// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The JMAP id map in SQLite: [`SqliteJmapIdStore`] implements `croft_protocols::JmapIdStore` over
//! the `jmap_mailbox`, `jmap_email_uid` and `jmap_account_state` tables (version 5 of the schema,
//! see `migrations/0005_jmap_ids.sql`).
//!
//! One store serves one account: every statement carries its `account_id`. The rules the JMAP
//! adapter relies on are kept by the schema as well as by this code: UIDs of a mailbox increase from
//! 1 and are never reused (`next_uid` only goes up, a trigger refuses to lower it, and a row's UID
//! must be below it), a UID is never given to two emails, and resetting a mailbox deletes its entries
//! with it. A constraint that fires anyway means the tables are inconsistent, and is reported as
//! [`IdStoreError::Corrupt`], which makes the adapter reset that mailbox; any other SQLite failure is
//! [`IdStoreError::Unavailable`] (retry later). Nothing in an error carries an id or a value.
//!
//! The store owns its database handle behind a mutex, so it is `Send + Sync` as the trait requires.
//! Give it a [`Db`] of its own (SQLite allows several connections to one file in WAL mode), or the
//! one the engine uses if the engine can lend it for these short calls.

use std::borrow::{Borrow, BorrowMut};
use std::collections::{BTreeMap, HashMap};
use std::sync::Mutex;

use croft_protocols::jmap::{fresh_uid_validity, IdStoreError, JmapIdStore};
use croft_protocols::Uid;
use rusqlite::{params, Connection, ErrorCode, OptionalExtension};

use crate::error::StorageError;
use crate::Db;

/// What can go wrong inside one call, before it is reduced to an [`IdStoreError`].
enum Fail {
    Sql(rusqlite::Error),
    Id(IdStoreError),
}

impl From<rusqlite::Error> for Fail {
    fn from(e: rusqlite::Error) -> Self {
        Self::Sql(e)
    }
}

impl From<StorageError> for Fail {
    fn from(e: StorageError) -> Self {
        match e {
            StorageError::Sqlite(e) => Self::Sql(e),
            _ => Self::Id(IdStoreError::Unavailable),
        }
    }
}

impl From<IdStoreError> for Fail {
    fn from(e: IdStoreError) -> Self {
        Self::Id(e)
    }
}

impl From<Fail> for IdStoreError {
    fn from(f: Fail) -> Self {
        match f {
            Fail::Id(e) => e,
            // A trigger, CHECK, UNIQUE or foreign key that refuses what the code just did: the
            // tables hold something they must not.
            Fail::Sql(rusqlite::Error::SqliteFailure(e, _))
                if e.code == ErrorCode::ConstraintViolation =>
            {
                IdStoreError::Corrupt
            }
            Fail::Sql(_) => IdStoreError::Unavailable,
        }
    }
}

/// The id map of one account over an open database. `D` is the [`Db`] itself, or anything that
/// lends one mutably.
#[derive(Debug)]
pub struct SqliteJmapIdStore<D: BorrowMut<Db> + Send = Db> {
    db: Mutex<D>,
    account_id: i64,
}

impl<D: BorrowMut<Db> + Send> SqliteJmapIdStore<D> {
    /// The id map of account `account_id` (a row of the `account` table) in `db`.
    pub fn new(db: D, account_id: i64) -> Self {
        Self {
            db: Mutex::new(db),
            account_id,
        }
    }

    /// Gives the database back.
    pub fn into_inner(self) -> D {
        self.db
            .into_inner()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Runs `f` in one write transaction.
    fn tx<T>(
        &self,
        f: impl FnOnce(&rusqlite::Transaction<'_>, i64) -> Result<T, Fail>,
    ) -> Result<T, IdStoreError> {
        let mut guard = self.db.lock().map_err(|_| IdStoreError::Unavailable)?;
        let account = self.account_id;
        BorrowMut::<Db>::borrow_mut(&mut *guard)
            .transaction(|tx| f(tx, account))
            .map_err(IdStoreError::from)
    }

    /// Runs a read with the connection.
    fn read<T>(
        &self,
        f: impl FnOnce(&Connection, i64) -> Result<T, Fail>,
    ) -> Result<T, IdStoreError> {
        let guard = self.db.lock().map_err(|_| IdStoreError::Unavailable)?;
        f(Borrow::<Db>::borrow(&*guard).connection(), self.account_id).map_err(IdStoreError::from)
    }
}

fn to_uid(n: i64) -> Result<Uid, Fail> {
    Uid::try_from(n).map_err(|_| Fail::Id(IdStoreError::Corrupt))
}

/// The mailbox's row (validity, next UID), created with a random validity if it is new.
fn ensure_mailbox(conn: &Connection, account: i64, mailbox: &str) -> Result<(u32, i64), Fail> {
    let row: Option<(i64, i64)> = conn
        .query_row(
            "SELECT uid_validity, next_uid FROM jmap_mailbox WHERE account_id = ?1 AND mailbox_id = ?2",
            params![account, mailbox],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    match row {
        Some((v, n)) => Ok((to_uid(v)?, n)),
        None => {
            let v = fresh_uid_validity(None);
            conn.execute(
                "INSERT INTO jmap_mailbox (account_id, mailbox_id, uid_validity, next_uid) VALUES (?1, ?2, ?3, 1)",
                params![account, mailbox, i64::from(v)],
            )?;
            Ok((v, 1))
        }
    }
}

impl<D: BorrowMut<Db> + Send> JmapIdStore for SqliteJmapIdStore<D> {
    fn uid_validity(&self, mailbox: &str) -> Result<u32, IdStoreError> {
        self.tx(|tx, account| Ok(ensure_mailbox(tx, account, mailbox)?.0))
    }

    fn reset_mailbox(&self, mailbox: &str) -> Result<u32, IdStoreError> {
        self.tx(|tx, account| {
            let old: Option<i64> = tx
                .query_row(
                    "SELECT uid_validity FROM jmap_mailbox WHERE account_id = ?1 AND mailbox_id = ?2",
                    params![account, mailbox],
                    |r| r.get(0),
                )
                .optional()?;
            // Deleting the row removes every entry with it.
            tx.execute(
                "DELETE FROM jmap_mailbox WHERE account_id = ?1 AND mailbox_id = ?2",
                params![account, mailbox],
            )?;
            let v = fresh_uid_validity(old.and_then(|o| u32::try_from(o).ok()));
            tx.execute(
                "INSERT INTO jmap_mailbox (account_id, mailbox_id, uid_validity, next_uid) VALUES (?1, ?2, ?3, 1)",
                params![account, mailbox, i64::from(v)],
            )?;
            Ok(v)
        })
    }

    fn forget_mailbox(&self, mailbox: &str) -> Result<(), IdStoreError> {
        self.tx(|tx, account| {
            tx.execute(
                "DELETE FROM jmap_mailbox WHERE account_id = ?1 AND mailbox_id = ?2",
                params![account, mailbox],
            )?;
            Ok(())
        })
    }

    fn next_uid(&self, mailbox: &str) -> Result<Uid, IdStoreError> {
        self.tx(|tx, account| {
            let (_, next) = ensure_mailbox(tx, account, mailbox)?;
            Uid::try_from(next).map_err(|_| Fail::Id(IdStoreError::Exhausted))
        })
    }

    fn assign(&self, mailbox: &str, email_ids: &[String]) -> Result<Vec<Uid>, IdStoreError> {
        self.tx(|tx, account| {
            let (_, next) = ensure_mailbox(tx, account, mailbox)?;
            let mut known: HashMap<&str, Uid> = HashMap::new();
            let mut fresh: Vec<&str> = Vec::new();
            {
                let mut find = tx.prepare_cached(
                    "SELECT uid FROM jmap_email_uid WHERE account_id = ?1 AND mailbox_id = ?2 AND email_id = ?3",
                )?;
                for id in email_ids {
                    if known.contains_key(id.as_str()) || fresh.contains(&id.as_str()) {
                        continue;
                    }
                    match find
                        .query_row(params![account, mailbox, id], |r| r.get::<_, i64>(0))
                        .optional()?
                    {
                        Some(uid) => {
                            known.insert(id, to_uid(uid)?);
                        }
                        None => fresh.push(id),
                    }
                }
            }
            if !fresh.is_empty() {
                let last = next - 1 + fresh.len() as i64;
                if last > i64::from(u32::MAX) {
                    return Err(Fail::Id(IdStoreError::Exhausted));
                }
                // The counter moves first: a new row's UID has to be below it.
                tx.execute(
                    "UPDATE jmap_mailbox SET next_uid = ?3 WHERE account_id = ?1 AND mailbox_id = ?2",
                    params![account, mailbox, last + 1],
                )?;
                let mut insert = tx.prepare_cached(
                    "INSERT INTO jmap_email_uid (account_id, mailbox_id, email_id, uid) VALUES (?1, ?2, ?3, ?4)",
                )?;
                for (i, id) in fresh.iter().enumerate() {
                    let uid = next + i as i64;
                    insert.execute(params![account, mailbox, id, uid])?;
                    known.insert(id, to_uid(uid)?);
                }
            }
            Ok(email_ids.iter().map(|id| known[id.as_str()]).collect())
        })
    }

    fn email_ids(
        &self,
        mailbox: &str,
        uids: &[Uid],
    ) -> Result<BTreeMap<Uid, String>, IdStoreError> {
        self.read(|conn, account| {
            let mut find = conn.prepare_cached(
                "SELECT email_id FROM jmap_email_uid WHERE account_id = ?1 AND mailbox_id = ?2 AND uid = ?3",
            )?;
            let mut out = BTreeMap::new();
            for uid in uids {
                if let Some(id) = find
                    .query_row(params![account, mailbox, i64::from(*uid)], |r| {
                        r.get::<_, String>(0)
                    })
                    .optional()?
                {
                    out.insert(*uid, id);
                }
            }
            Ok(out)
        })
    }

    fn entries(&self, mailbox: &str) -> Result<BTreeMap<Uid, String>, IdStoreError> {
        self.read(|conn, account| {
            let next: Option<i64> = conn
                .query_row(
                    "SELECT next_uid FROM jmap_mailbox WHERE account_id = ?1 AND mailbox_id = ?2",
                    params![account, mailbox],
                    |r| r.get(0),
                )
                .optional()?;
            let mut stmt = conn.prepare_cached(
                "SELECT uid, email_id FROM jmap_email_uid WHERE account_id = ?1 AND mailbox_id = ?2 ORDER BY uid",
            )?;
            let rows = stmt.query_map(params![account, mailbox], |r| {
                Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
            })?;
            let mut out = BTreeMap::new();
            for row in rows {
                let (uid, id) = row?;
                // An entry at or above the counter would let a UID be handed out twice.
                if next.is_none_or(|n| uid >= n) {
                    return Err(Fail::Id(IdStoreError::Corrupt));
                }
                out.insert(to_uid(uid)?, id);
            }
            Ok(out)
        })
    }

    fn remove(&self, mailbox: &str, email_ids: &[String]) -> Result<(), IdStoreError> {
        self.tx(|tx, account| {
            let mut del = tx.prepare_cached(
                "DELETE FROM jmap_email_uid WHERE account_id = ?1 AND mailbox_id = ?2 AND email_id = ?3",
            )?;
            for id in email_ids {
                del.execute(params![account, mailbox, id])?;
            }
            Ok(())
        })
    }

    fn email_state(&self) -> Result<Option<String>, IdStoreError> {
        self.read(|conn, account| {
            Ok(conn
                .query_row(
                    "SELECT email_state FROM jmap_account_state WHERE account_id = ?1",
                    params![account],
                    |r| r.get(0),
                )
                .optional()?)
        })
    }

    fn set_email_state(&self, state: Option<&str>) -> Result<(), IdStoreError> {
        self.tx(|tx, account| {
            match state {
                Some(s) => tx.execute(
                    "INSERT INTO jmap_account_state (account_id, email_state) VALUES (?1, ?2)
                     ON CONFLICT (account_id) DO UPDATE SET email_state = excluded.email_state",
                    params![account, s],
                )?,
                None => tx.execute(
                    "DELETE FROM jmap_account_state WHERE account_id = ?1",
                    params![account],
                )?,
            };
            Ok(())
        })
    }
}
