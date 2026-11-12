// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The operation queue in SQLite: [`SqliteOpStore`] implements `croft_mail_core::OpStore` over the
//! `op_queue` and `op_awaiting_uid` tables.
//!
//! [`OpStore::commit`] writes the queue rows and the local effects (the new `is_seen`, hidden rows,
//! `pending_ops`, moved or removed message rows) in **one** transaction, so after a kill the
//! person's view and the queue always agree.
//!
//! Every statement carries `account_id`. Message rows are addressed by account, folder path and
//! UID (joined through `folder`), never by row id.
//!
//! `payload_json` is `{"v":1, ...}`. The JSON is written and read here rather than in mail-core, so
//! mail-core needs no serde. A row with another version, or one that cannot be read, is marked
//! failed with a plain reason when it is loaded; nothing guesses what it meant. Such a row is kept,
//! and listed again by every load, until `PersistentQueue::acknowledge_unreadable` deletes it, so a
//! kill before the person was told cannot lose it.
//!
//! Operation ids are never reused: the largest id ever written is kept in the app-wide `setting`
//! table (key `op_queue.max_id`), updated in the same transaction as the insert, because the table's
//! own `MAX(id)` shrinks when finished rows are purged or discarded.

use std::borrow::{Borrow, BorrowMut};

use croft_mail_core::{
    AwaitingUid, Batch, LocalEffect, Op, OpKind, OpState, OpStore, StoredQueue, UnreadableOp,
};
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use serde_json::{json, Value};

use crate::error::{Result, StorageError};
use crate::Db;
use crate::{folders, messages, settings};

/// The payload format version written by this build.
const PAYLOAD_VERSION: i64 = 1;

/// Shown when a row was written by a newer app.
const REASON_NEWER: &str = "This change was saved by a newer version of the app and was not sent";
/// Name of the app-wide setting that holds the largest operation id ever written. It is the one
/// place this module touches the `setting` table, which has no `account_id`.
const MAX_ID_KEY: &str = "op_queue.max_id";

/// Shown when a row cannot be understood.
const REASON_DAMAGED: &str = "This change could not be read and was not sent";

/// SQL for the id of a folder given `?1` (account) and a path parameter.
const FOLDER_OF_1_2: &str = "(SELECT id FROM folder WHERE account_id = ?1 AND path = ?2)";

/// The queue's store over an open database. `D` is the [`Db`] itself, or anything that lends one
/// mutably (such as `&mut Db`).
#[derive(Debug)]
pub struct SqliteOpStore<D: BorrowMut<Db> = Db> {
    db: D,
}

impl<D: BorrowMut<Db>> SqliteOpStore<D> {
    /// Wraps an open database.
    pub fn new(db: D) -> Self {
        Self { db }
    }

    /// The database, for reads such as the message list.
    pub fn db(&self) -> &Db {
        Borrow::<Db>::borrow(&self.db)
    }

    /// The database, mutably, for transactions that are not queue rows (the sync engine applies a
    /// folder sync through this). Do not write `op_queue` or `op_awaiting_uid` through it: the
    /// queue in memory would no longer match.
    pub fn db_mut(&mut self) -> &mut Db {
        BorrowMut::<Db>::borrow_mut(&mut self.db)
    }

    /// Gives the database back.
    pub fn into_inner(self) -> D {
        self.db
    }
}

impl<D: BorrowMut<Db>> OpStore for SqliteOpStore<D> {
    type Error = StorageError;

    fn load(&mut self) -> Result<StoredQueue> {
        BorrowMut::<Db>::borrow_mut(&mut self.db).transaction(load_all)
    }

    fn commit(&mut self, batch: &Batch) -> Result<()> {
        BorrowMut::<Db>::borrow_mut(&mut self.db).transaction(|tx| write_batch(tx, batch))
    }
}

// ---- payload ----------------------------------------------------------------------------------

fn kind_name(kind: &OpKind) -> &'static str {
    match kind {
        OpKind::SetSeen { .. } => "set_seen",
        OpKind::SetFlagged { .. } => "set_flagged",
        OpKind::MoveTo { .. } => "move",
        OpKind::DeleteForever { .. } => "delete_forever",
    }
}

fn payload_of(kind: &OpKind) -> String {
    let value = match kind {
        OpKind::SetSeen { folder, uid, value } | OpKind::SetFlagged { folder, uid, value } => {
            json!({"v": PAYLOAD_VERSION, "folder": folder, "uid": uid, "value": value})
        }
        OpKind::MoveTo { folder, uid, dest } => {
            json!({"v": PAYLOAD_VERSION, "folder": folder, "uid": uid, "dest": dest})
        }
        OpKind::DeleteForever { folder, uid } => {
            json!({"v": PAYLOAD_VERSION, "folder": folder, "uid": uid})
        }
    };
    value.to_string()
}

/// The address the operation acts on, kept in `target_key` as `<uid>@<folder path>`. It does not
/// depend on the payload version, so a row that cannot be read can still be cleaned up.
fn target_key(kind: &OpKind) -> String {
    let (folder, uid) = kind.target();
    format!("{uid}@{folder}")
}

fn parse_target_key(key: &str) -> Option<(String, u32)> {
    let (uid, folder) = key.split_once('@')?;
    Some((folder.to_owned(), uid.parse().ok()?))
}

/// Reads a payload back. `Err` carries the plain reason to show.
fn kind_from(kind: &str, payload: &str) -> std::result::Result<OpKind, &'static str> {
    let value: Value = serde_json::from_str(payload).map_err(|_| REASON_DAMAGED)?;
    match value.get("v").and_then(Value::as_i64) {
        Some(PAYLOAD_VERSION) => {}
        Some(_) => return Err(REASON_NEWER),
        None => return Err(REASON_DAMAGED),
    }
    let text = |key: &str| value.get(key).and_then(Value::as_str).map(str::to_owned);
    let folder = text("folder").ok_or(REASON_DAMAGED)?;
    let uid = value
        .get("uid")
        .and_then(Value::as_u64)
        .and_then(|u| u32::try_from(u).ok())
        .ok_or(REASON_DAMAGED)?;
    let flag = || {
        value
            .get("value")
            .and_then(Value::as_bool)
            .ok_or(REASON_DAMAGED)
    };
    match kind {
        "set_seen" => Ok(OpKind::SetSeen {
            folder,
            uid,
            value: flag()?,
        }),
        "set_flagged" => Ok(OpKind::SetFlagged {
            folder,
            uid,
            value: flag()?,
        }),
        "move" => Ok(OpKind::MoveTo {
            folder,
            uid,
            dest: text("dest").ok_or(REASON_DAMAGED)?,
        }),
        "delete_forever" => Ok(OpKind::DeleteForever { folder, uid }),
        _ => Err(REASON_DAMAGED),
    }
}

/// An operation id as a SQLite integer.
fn db_id(id: u64) -> Result<i64> {
    i64::try_from(id).map_err(|_| StorageError::OutOfRange("operation id"))
}

fn state_name(state: OpState) -> &'static str {
    match state {
        OpState::Queued => "queued",
        OpState::Applying => "applying",
        OpState::Confirmed => "confirmed",
        OpState::Failed => "failed",
        OpState::Cancelled => "cancelled",
    }
}

fn state_from(name: &str) -> Option<OpState> {
    Some(match name {
        "queued" => OpState::Queued,
        "applying" => OpState::Applying,
        "confirmed" => OpState::Confirmed,
        "failed" => OpState::Failed,
        "cancelled" => OpState::Cancelled,
        _ => return None,
    })
}

// ---- load -------------------------------------------------------------------------------------

/// One `op_queue` row as stored.
struct Row {
    id: i64,
    kind: String,
    payload: String,
    target: Option<String>,
    state: String,
    created_at: i64,
    not_before: i64,
    attempts: i64,
    last_error: Option<String>,
}

fn load_all(tx: &Transaction<'_>) -> Result<StoredQueue> {
    // Ids are one counter for the whole table, so the largest id is the one global read here.
    let table_max: i64 = tx.query_row("SELECT COALESCE(MAX(id), 0) FROM op_queue", [], |r| {
        r.get(0)
    })?;
    let table_max =
        u64::try_from(table_max).map_err(|_| StorageError::OutOfRange("operation id"))?;
    let mut out = StoredQueue {
        max_id: stored_high_water(tx)?.max(table_max),
        ..StoredQueue::default()
    };

    let accounts: Vec<i64> = tx
        .prepare("SELECT id FROM account ORDER BY id")?
        .query_map([], |r| r.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    for account in accounts {
        // Finished rows are history; their effects were applied when they finished. Rows that
        // could not be read stay until the engine has reported them.
        tx.execute(
            "DELETE FROM op_queue WHERE account_id = ?1
                AND state IN ('confirmed', 'failed', 'cancelled')
                AND COALESCE(last_error, '') NOT IN (?2, ?3)",
            params![account, REASON_NEWER, REASON_DAMAGED],
        )?;
        // Rows marked unreadable by an earlier load and not yet acknowledged. Their message rows
        // were repaired then, so nothing is released again here.
        let mut stmt = tx.prepare(
            "SELECT id, last_error FROM op_queue
              WHERE account_id = ?1 AND state = 'failed' AND last_error IN (?2, ?3) ORDER BY id",
        )?;
        let kept = stmt.query_map(params![account, REASON_NEWER, REASON_DAMAGED], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
        })?;
        for entry in kept {
            let (id, reason) = entry?;
            out.unreadable.push(UnreadableOp {
                id: u64::try_from(id).map_err(|_| StorageError::OutOfRange("operation id"))?,
                account,
                reason,
            });
        }
        drop(stmt);

        let rows: Vec<Row> = tx
            .prepare(
                "SELECT id, kind, payload_json, target_key, state, created_at, not_before, attempts, last_error
                   FROM op_queue WHERE account_id = ?1 AND state IN ('queued', 'applying')
                  ORDER BY id",
            )?
            .query_map([account], |r| {
                Ok(Row {
                    id: r.get(0)?,
                    kind: r.get(1)?,
                    payload: r.get(2)?,
                    target: r.get(3)?,
                    state: r.get(4)?,
                    created_at: r.get(5)?,
                    not_before: r.get(6)?,
                    attempts: r.get(7)?,
                    last_error: r.get(8)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?;
        for row in rows {
            match op_from_row(account, &row) {
                Ok(op) => out.ops.push(op),
                Err(reason) => {
                    abandon_unreadable(tx, account, &row, reason)?;
                    out.unreadable.push(UnreadableOp {
                        id: u64::try_from(row.id).unwrap_or(0),
                        account,
                        reason: reason.to_owned(),
                    });
                }
            }
        }
        let mut stmt = tx.prepare(
            "SELECT folder, uid, dest_folder, message_id_hdr, lookups, created_at
               FROM op_awaiting_uid WHERE account_id = ?1 ORDER BY folder, uid",
        )?;
        let awaiting = stmt.query_map([account], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, Option<String>>(3)?,
                r.get::<_, i64>(4)?,
                r.get::<_, i64>(5)?,
            ))
        })?;
        for entry in awaiting {
            let (folder, uid, dest_folder, message_id, lookups, created_ms) = entry?;
            out.awaiting.push(AwaitingUid {
                account,
                folder,
                uid: u32::try_from(uid).map_err(|_| StorageError::OutOfRange("uid"))?,
                dest_folder,
                message_id,
                lookups: u32::try_from(lookups)
                    .map_err(|_| StorageError::OutOfRange("lookup count"))?,
                created_ms,
            });
        }
    }
    Ok(out)
}

fn op_from_row(account: i64, row: &Row) -> std::result::Result<Op, &'static str> {
    let kind = kind_from(&row.kind, &row.payload)?;
    Ok(Op {
        id: u64::try_from(row.id).map_err(|_| REASON_DAMAGED)?,
        account,
        kind,
        state: state_from(&row.state).ok_or(REASON_DAMAGED)?,
        created_ms: row.created_at,
        not_before_ms: row.not_before,
        attempts: u32::try_from(row.attempts).map_err(|_| REASON_DAMAGED)?,
        last_error: row.last_error.clone(),
    })
}

/// Marks a row nobody can read as failed, and cleans up its message row as far as the payload-free
/// columns allow: the count goes down and a hidden message is shown again. A flag value is not
/// put back (it is not known); the next sync brings the server's value. A row with no usable
/// `target_key` releases nothing: the message is not known, so it may stay hidden or counted.
/// This runs once per row: the failed row is kept (see `load_all`) and is not released again.
fn abandon_unreadable(tx: &Transaction<'_>, account: i64, row: &Row, reason: &str) -> Result<()> {
    tx.execute(
        "UPDATE op_queue SET state = 'failed', last_error = ?3 WHERE account_id = ?1 AND id = ?2",
        params![account, row.id, reason],
    )?;
    if let Some((folder, uid)) = row.target.as_deref().and_then(parse_target_key) {
        apply_effect(
            tx,
            &LocalEffect::PendingDelta {
                account,
                folder: folder.clone(),
                uid,
                delta: -1,
            },
        )?;
        if matches!(row.kind.as_str(), "move" | "delete_forever") {
            apply_effect(
                tx,
                &LocalEffect::Unhide {
                    account,
                    folder,
                    uid,
                },
            )?;
        }
    }
    Ok(())
}

// ---- commit -----------------------------------------------------------------------------------

/// The largest operation id ever written, from the `setting` table (0 when never written).
fn stored_high_water(tx: &Transaction<'_>) -> Result<u64> {
    Ok(settings::get::<u64>(tx, MAX_ID_KEY)?.unwrap_or(0))
}

fn write_batch(tx: &Transaction<'_>, batch: &Batch) -> Result<()> {
    for op in &batch.ops {
        upsert_op(tx, op)?;
    }
    if let Some(newest) = batch.ops.iter().max_by_key(|op| op.id) {
        if newest.id > stored_high_water(tx)? {
            settings::set(tx, MAX_ID_KEY, &newest.id, newest.created_ms)?;
        }
    }
    for (account, id) in &batch.purged {
        tx.execute(
            "DELETE FROM op_queue WHERE account_id = ?1 AND id = ?2
                AND state IN ('confirmed', 'failed', 'cancelled')",
            params![account, db_id(*id)?],
        )?;
    }
    for a in &batch.awaiting_put {
        tx.execute(
            "INSERT INTO op_awaiting_uid (account_id, folder, uid, dest_folder, message_id_hdr, lookups, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT (account_id, folder, uid) DO UPDATE SET
                 dest_folder = excluded.dest_folder,
                 message_id_hdr = excluded.message_id_hdr,
                 lookups = excluded.lookups,
                 created_at = excluded.created_at",
            params![
                a.account,
                a.folder,
                a.uid,
                a.dest_folder,
                a.message_id,
                a.lookups,
                a.created_ms
            ],
        )?;
    }
    for (account, folder, uid) in &batch.awaiting_removed {
        tx.execute(
            "DELETE FROM op_awaiting_uid WHERE account_id = ?1 AND folder = ?2 AND uid = ?3",
            params![account, folder, uid],
        )?;
    }
    for effect in &batch.effects {
        apply_effect(tx, effect)?;
    }
    Ok(())
}

fn upsert_op(tx: &Transaction<'_>, op: &Op) -> Result<()> {
    let n = tx.execute(
        "INSERT INTO op_queue (id, account_id, kind, payload_json, target_key, state, created_at, not_before, attempts, last_error)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
         ON CONFLICT (id) DO UPDATE SET
             kind = excluded.kind,
             payload_json = excluded.payload_json,
             target_key = excluded.target_key,
             state = excluded.state,
             not_before = excluded.not_before,
             attempts = excluded.attempts,
             last_error = excluded.last_error
         WHERE op_queue.account_id = excluded.account_id",
        params![
            db_id(op.id)?,
            op.account,
            kind_name(&op.kind),
            payload_of(&op.kind),
            target_key(&op.kind),
            state_name(op.state),
            op.created_ms,
            op.not_before_ms,
            op.attempts,
            op.last_error
        ],
    )?;
    if n == 0 {
        // The id exists for another account: never touch it.
        return Err(StorageError::NotFound("operation"));
    }
    Ok(())
}

// ---- local effects ----------------------------------------------------------------------------

fn apply_effect(tx: &Transaction<'_>, effect: &LocalEffect) -> Result<()> {
    match effect {
        LocalEffect::SetSeen {
            account,
            folder,
            uid,
            value,
        } => update_row(tx, "is_seen = ?4", *account, folder, *uid, *value),
        LocalEffect::SetFlagged {
            account,
            folder,
            uid,
            value,
        } => update_row(tx, "is_flagged = ?4", *account, folder, *uid, *value),
        LocalEffect::Hide {
            account,
            folder,
            uid,
        } => update_row(tx, "local_hidden = ?4", *account, folder, *uid, 1),
        LocalEffect::Unhide {
            account,
            folder,
            uid,
        } => update_row(tx, "local_hidden = ?4", *account, folder, *uid, 0),
        LocalEffect::PendingDelta {
            account,
            folder,
            uid,
            delta,
        } => update_row(
            tx,
            "pending_ops = MAX(0, pending_ops + ?4)",
            *account,
            folder,
            *uid,
            *delta,
        ),
        LocalEffect::RemoveMessage {
            account,
            folder,
            uid,
        } => {
            tx.execute(
                &format!(
                    "DELETE FROM message WHERE account_id = ?1 AND uid = ?3 AND folder_id = {FOLDER_OF_1_2}"
                ),
                params![account, folder, uid],
            )?;
            Ok(())
        }
        LocalEffect::CopyMessage { account, from, to } => copy_message(tx, *account, from, to),
        LocalEffect::ClearFolder { account, folder } => {
            if let Some(id) = folder_id(tx, *account, folder)? {
                messages::delete_all_in_folder(tx, *account, id)?;
            }
            Ok(())
        }
        LocalEffect::PutHeader {
            account,
            folder,
            header,
        } => {
            if let Some(id) = folder_id(tx, *account, folder)? {
                messages::upsert(tx, *account, id, header)?;
            }
            Ok(())
        }
        LocalEffect::SetFolderValidity {
            account,
            folder,
            uid_validity,
        } => {
            if let Some(id) = folder_id(tx, *account, folder)? {
                folders::set_validity_only(tx, *account, id, *uid_validity)?;
            }
            Ok(())
        }
    }
}

/// `UPDATE message SET <set> WHERE <the message at (account, folder, uid)>`. `?4` is the value.
fn update_row(
    tx: &Transaction<'_>,
    set: &str,
    account: i64,
    folder: &str,
    uid: u32,
    value: impl rusqlite::ToSql,
) -> Result<()> {
    tx.execute(
        &format!(
            "UPDATE message SET {set} WHERE account_id = ?1 AND uid = ?3 AND folder_id = {FOLDER_OF_1_2}"
        ),
        params![account, folder, uid, value],
    )?;
    Ok(())
}

fn folder_id(conn: &Connection, account: i64, path: &str) -> Result<Option<i64>> {
    Ok(conn
        .query_row(
            "SELECT id FROM folder WHERE account_id = ?1 AND path = ?2",
            params![account, path],
            |r| r.get(0),
        )
        .optional()?)
}

/// Copies the message at `from` to `to` (a confirmed move). The copy is visible, keeps the flags,
/// the pending count, the cached body and the attachment list. If the destination already has a
/// row for that UID (a sync got there first), that row is kept and only takes over the local flag
/// values and pending count when changes are still waiting; its `local_hidden` is left as it is
/// (the queue hides the copy with a separate `Hide` when a later move or delete is pending).
fn copy_message(
    tx: &Transaction<'_>,
    account: i64,
    from: &(String, u32),
    to: &(String, u32),
) -> Result<()> {
    if from == to {
        return Ok(());
    }
    let (Some(from_folder), Some(to_folder)) = (
        folder_id(tx, account, &from.0)?,
        folder_id(tx, account, &to.0)?,
    ) else {
        return Ok(());
    };
    let source: Option<(i64, i64, bool, bool)> = tx
        .query_row(
            "SELECT id, pending_ops, is_seen, is_flagged FROM message
              WHERE account_id = ?1 AND folder_id = ?2 AND uid = ?3",
            params![account, from_folder, from.1],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .optional()?;
    let Some((source_id, pending, seen, flagged)) = source else {
        return Ok(());
    };
    let existing: Option<i64> = tx
        .query_row(
            "SELECT id FROM message WHERE account_id = ?1 AND folder_id = ?2 AND uid = ?3",
            params![account, to_folder, to.1],
            |r| r.get(0),
        )
        .optional()?;
    if let Some(existing) = existing {
        if pending > 0 {
            tx.execute(
                "UPDATE message SET pending_ops = pending_ops + ?3, is_seen = ?4, is_flagged = ?5
                  WHERE account_id = ?1 AND id = ?2",
                params![account, existing, pending, seen, flagged],
            )?;
        }
        return Ok(());
    }
    tx.execute(
        "INSERT INTO message (account_id, folder_id, uid, message_id_hdr, thread_key, subject, from_name,
                              from_addr, to_json, cc_json, date_ts, size, snippet, is_seen, is_flagged,
                              is_answered, is_draft, keywords_json, has_attachments, category,
                              snoozed_until, pending_ops, body_state, local_hidden, remote_allowed)
         SELECT account_id, ?3, ?4, message_id_hdr, thread_key, subject, from_name,
                from_addr, to_json, cc_json, date_ts, size, snippet, is_seen, is_flagged,
                is_answered, is_draft, keywords_json, has_attachments, category,
                snoozed_until, pending_ops, body_state, 0, remote_allowed
           FROM message WHERE account_id = ?1 AND id = ?2",
        params![account, source_id, to_folder, to.1],
    )?;
    let new_id = tx.last_insert_rowid();
    tx.execute(
        "INSERT INTO message_body (message_id, headers_raw, text_plain, html_clean, remote_blocked,
                                   trackers_removed, truncated, too_complex)
         SELECT ?3, b.headers_raw, b.text_plain, b.html_clean, b.remote_blocked,
                b.trackers_removed, b.truncated, b.too_complex
           FROM message_body b JOIN message m ON m.id = b.message_id
          WHERE m.account_id = ?1 AND m.id = ?2",
        params![account, source_id, new_id],
    )?;
    tx.execute(
        "INSERT INTO attachment (message_id, part_id, filename, mime_type, size, cached_path,
                                 content_id, shown_inline, executable)
         SELECT ?3, a.part_id, a.filename, a.mime_type, a.size, a.cached_path,
                a.content_id, a.shown_inline, a.executable
           FROM attachment a JOIN message m ON m.id = a.message_id
          WHERE m.account_id = ?1 AND m.id = ?2",
        params![account, source_id, new_id],
    )?;
    // The fetched message goes with it, so the moved copy opens offline and keeps its attachments.
    tx.execute(
        "INSERT INTO message_raw (message_id, account_id, raw, sha256, fetched_at)
         SELECT ?3, r.account_id, r.raw, r.sha256, r.fetched_at
           FROM message_raw r JOIN message m ON m.id = r.message_id
          WHERE r.account_id = ?1 AND m.account_id = ?1 AND m.id = ?2",
        params![account, source_id, new_id],
    )?;
    Ok(())
}
