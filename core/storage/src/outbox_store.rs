// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The Outbox in SQLite: [`SqliteOutboxStore`] implements `croft_mail_core::OutboxStore` over the
//! `outbox` table (version 4 of the schema, see `migrations/0004_outbox_append.sql`).
//!
//! Every statement carries `account_id`. The message bytes are stored as given in `mime_blob`;
//! they are read only by [`OutboxStore::mime`] and never appear in an error, a log or `Debug`
//! output. Recipients are a JSON array, so no address can be split into two.
//!
//! Message ids come from `AUTOINCREMENT`, so SQLite itself keeps the high-water mark
//! (`sqlite_sequence`) and an id is never reused after its row is deleted.
//!
//! The store never writes the state `sent`: a finished message is deleted (the copy in the Sent
//! folder is the record), together with its bytes. A `sent` row left by some other writer is
//! deleted when the store is loaded.

use std::borrow::{Borrow, BorrowMut};

use croft_mail_core::outbox::REASON_UNREADABLE;
use croft_mail_core::{OutboxBatch, OutboxEntry, OutboxState, OutboxStore, StoredOutbox};
use rusqlite::{params, OptionalExtension, Transaction};

use crate::error::{Result, StorageError};
use crate::Db;

/// The Outbox's store over an open database. `D` is the [`Db`] itself, or anything that lends one
/// mutably (such as `&mut Db`).
#[derive(Debug)]
pub struct SqliteOutboxStore<D: BorrowMut<Db> = Db> {
    db: D,
}

impl<D: BorrowMut<Db>> SqliteOutboxStore<D> {
    /// Wraps an open database.
    pub fn new(db: D) -> Self {
        Self { db }
    }

    /// The database, for reads such as the message list.
    pub fn db(&self) -> &Db {
        Borrow::<Db>::borrow(&self.db)
    }

    /// Gives the database back.
    pub fn into_inner(self) -> D {
        self.db
    }
}

impl<D: BorrowMut<Db>> OutboxStore for SqliteOutboxStore<D> {
    type Error = StorageError;

    fn load(&mut self) -> Result<StoredOutbox> {
        BorrowMut::<Db>::borrow_mut(&mut self.db).transaction(load_all)
    }

    fn commit(&mut self, batch: &OutboxBatch) -> Result<()> {
        BorrowMut::<Db>::borrow_mut(&mut self.db).transaction(|tx| write_batch(tx, batch))
    }

    fn mime(&mut self, account: i64, id: u64) -> Result<Option<Vec<u8>>> {
        let id = db_id(id)?;
        let conn = Borrow::<Db>::borrow(&self.db).connection();
        Ok(conn
            .query_row(
                "SELECT mime_blob FROM outbox WHERE account_id = ?1 AND id = ?2",
                params![account, id],
                |r| r.get(0),
            )
            .optional()?)
    }
}

fn db_id(id: u64) -> Result<i64> {
    i64::try_from(id).map_err(|_| StorageError::OutOfRange("outbox id"))
}

fn state_name(state: OutboxState) -> &'static str {
    match state {
        OutboxState::Queued => "queued",
        OutboxState::Sending => "sending",
        OutboxState::Submitted => "submitted",
        OutboxState::Filing => "filing",
        OutboxState::Sent => "sent",
        OutboxState::Failed => "failed",
    }
}

fn state_from(name: &str) -> Option<OutboxState> {
    Some(match name {
        "queued" => OutboxState::Queued,
        "sending" => OutboxState::Sending,
        "submitted" => OutboxState::Submitted,
        "filing" => OutboxState::Filing,
        "sent" => OutboxState::Sent,
        "failed" => OutboxState::Failed,
        _ => return None,
    })
}

fn recipients_json(recipients: &[String]) -> Result<String> {
    Ok(serde_json::to_string(recipients)?)
}

struct Row {
    id: i64,
    identity: Option<i64>,
    draft: Option<i64>,
    state: String,
    send_at: i64,
    attempts: i64,
    last_error: Option<String>,
    envelope_from: String,
    recipients: String,
}

fn load_all(tx: &Transaction<'_>) -> Result<StoredOutbox> {
    // `sqlite_sequence` is SQLite's own high-water mark for AUTOINCREMENT; the table's MAX(id) is
    // the fallback for a database where the sequence row is missing.
    let sequence: i64 = tx
        .query_row(
            "SELECT COALESCE((SELECT seq FROM sqlite_sequence WHERE name = 'outbox'), 0)",
            [],
            |r| r.get(0),
        )
        .unwrap_or(0);
    let table_max: i64 =
        tx.query_row("SELECT COALESCE(MAX(id), 0) FROM outbox", [], |r| r.get(0))?;
    let max_id = u64::try_from(sequence.max(table_max))
        .map_err(|_| StorageError::OutOfRange("outbox id"))?;
    let mut out = StoredOutbox {
        max_id,
        ..StoredOutbox::default()
    };
    let accounts: Vec<i64> = tx
        .prepare("SELECT id FROM account ORDER BY id")?
        .query_map([], |r| r.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    for account in accounts {
        tx.execute(
            "DELETE FROM outbox WHERE account_id = ?1 AND state = 'sent'",
            [account],
        )?;
        let rows: Vec<Row> = tx
            .prepare(
                "SELECT id, identity_id, state, send_at, attempts, last_error, envelope_from, recipients, draft_id
                   FROM outbox WHERE account_id = ?1 ORDER BY id",
            )?
            .query_map([account], |r| {
                Ok(Row {
                    id: r.get(0)?,
                    identity: r.get(1)?,
                    state: r.get(2)?,
                    send_at: r.get(3)?,
                    attempts: r.get(4)?,
                    last_error: r.get(5)?,
                    envelope_from: r.get(6)?,
                    recipients: r.get(7)?,
                    draft: r.get(8)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?;
        for row in rows {
            out.entries.push(entry_from_row(account, row)?);
        }
    }
    Ok(out)
}

/// A row that cannot be understood is not guessed at: it becomes Failed with a plain reason, so
/// the person sees it and chooses (Edit or Discard).
fn entry_from_row(account: i64, row: Row) -> Result<OutboxEntry> {
    let id = u64::try_from(row.id).map_err(|_| StorageError::OutOfRange("outbox id"))?;
    let recipients: Option<Vec<String>> = serde_json::from_str(&row.recipients).ok();
    let state = state_from(&row.state);
    let attempts = u32::try_from(row.attempts).ok();
    let readable = state.is_some() && recipients.is_some() && attempts.is_some();
    Ok(OutboxEntry {
        id,
        account,
        identity: row.identity,
        draft: row.draft,
        envelope_from: row.envelope_from,
        recipients: recipients.unwrap_or_default(),
        state: if readable {
            state.unwrap_or(OutboxState::Failed)
        } else {
            OutboxState::Failed
        },
        send_at_ms: row.send_at,
        attempts: attempts.unwrap_or(0),
        last_error: if readable {
            row.last_error
        } else {
            Some(REASON_UNREADABLE.to_owned())
        },
    })
}

fn write_batch(tx: &Transaction<'_>, batch: &OutboxBatch) -> Result<()> {
    for (entry, mime) in &batch.inserted {
        tx.execute(
            "INSERT INTO outbox (id, account_id, identity_id, mime_blob, state, send_at, attempts,
                                 last_error, envelope_from, recipients, draft_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![
                db_id(entry.id)?,
                entry.account,
                entry.identity,
                mime,
                state_name(entry.state),
                entry.send_at_ms,
                entry.attempts,
                entry.last_error,
                entry.envelope_from,
                recipients_json(&entry.recipients)?,
                entry.draft,
            ],
        )?;
    }
    for entry in &batch.updated {
        let changed = tx.execute(
            "UPDATE outbox SET state = ?3, send_at = ?4, attempts = ?5, last_error = ?6
              WHERE account_id = ?1 AND id = ?2",
            params![
                entry.account,
                db_id(entry.id)?,
                state_name(entry.state),
                entry.send_at_ms,
                entry.attempts,
                entry.last_error,
            ],
        )?;
        if changed != 1 {
            return Err(StorageError::NotFound("outbox message"));
        }
    }
    for (account, id) in &batch.removed {
        tx.execute(
            "DELETE FROM outbox WHERE account_id = ?1 AND id = ?2",
            params![account, db_id(*id)?],
        )?;
    }
    Ok(())
}
