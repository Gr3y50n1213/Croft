// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The database connection: open, key, migrate, transactions.

use std::path::Path;
use std::time::Duration;

use rusqlite::{Connection, Transaction, TransactionBehavior};

use crate::error::{Result, StorageError};
use crate::pending;

/// How long a statement waits for another connection's lock before failing with "busy".
const BUSY_TIMEOUT: Duration = Duration::from_secs(5);

/// An open, migrated Croft database.
///
/// Repository functions in [`crate::accounts`], [`crate::folders`], [`crate::messages`] and
/// [`crate::settings`] take a `&Connection`; both [`Db::connection`] and the `&Transaction` given to
/// [`Db::transaction`] can be passed to them.
pub struct Db {
    conn: Connection,
    migrations_run: usize,
}

impl Db {
    /// Opens (creating it if needed) the database at `path` and brings it to the latest schema.
    ///
    /// `key` is the 32-byte database key held in the platform keystore. It is only accepted when
    /// the crate is built with the `sqlcipher` feature; without that feature a key is refused with
    /// [`StorageError::EncryptionUnavailable`] rather than ignored, so an encrypted store never
    /// silently becomes a plain one. Pass `None` for an unencrypted database.
    ///
    /// Order matters: the key is applied first, then `user_version` is read, and a database newer
    /// than this build is refused with [`StorageError::SchemaTooNew`] before anything is written
    /// (not even the WAL switch, which changes the file header). Only then are foreign keys and WAL
    /// enabled and the pending migrations run, each in its own transaction.
    pub fn open(path: &Path, key: Option<&[u8]>) -> Result<Self> {
        let conn = Connection::open(path)?;
        Self::init(conn, key, true)
    }

    /// Opens a private in-memory database at the latest schema. For tests and previews.
    pub fn open_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        Self::init(conn, None, false)
    }

    fn init(conn: Connection, key: Option<&[u8]>, on_disk: bool) -> Result<Self> {
        apply_key(&conn, key)?;
        // The first read: with a wrong key (or a non-database file) it fails here.
        let version: u32 = conn
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .map_err(|e| match e {
                rusqlite::Error::SqliteFailure(f, _)
                    if f.code == rusqlite::ErrorCode::NotADatabase =>
                {
                    StorageError::WrongKeyOrNotADatabase
                }
                other => StorageError::Sqlite(other),
            })?;
        let todo = pending(version)?;

        conn.busy_timeout(BUSY_TIMEOUT)?;
        conn.pragma_update(None, "foreign_keys", true)?;
        let mode: String =
            conn.pragma_update_and_check(None, "journal_mode", "WAL", |r| r.get(0))?;
        let mode_ok = mode.eq_ignore_ascii_case("wal") || (!on_disk && mode == "memory");
        if !mode_ok {
            return Err(StorageError::WalUnavailable);
        }

        let mut db = Self {
            conn,
            migrations_run: 0,
        };
        for m in todo {
            let tx = db
                .conn
                .transaction_with_behavior(TransactionBehavior::Immediate)?;
            let step = tx
                .execute_batch(m.sql)
                // user_version changes with the transaction, so a failed migration leaves no trace.
                .and_then(|()| tx.pragma_update(None, "user_version", m.version))
                .and_then(|()| tx.commit());
            step.map_err(|cause| StorageError::Migration {
                version: m.version,
                cause,
            })?;
            db.migrations_run += 1;
        }
        Ok(db)
    }

    /// How many migrations ran while opening this database (0 when it was already up to date).
    pub fn migrations_run_on_open(&self) -> usize {
        self.migrations_run
    }

    /// The schema version stored in the file (`PRAGMA user_version`).
    pub fn schema_version(&self) -> Result<u32> {
        Ok(self
            .conn
            .pragma_query_value(None, "user_version", |r| r.get(0))?)
    }

    /// The underlying connection, for repository functions and read-only queries.
    pub fn connection(&self) -> &Connection {
        &self.conn
    }

    /// Runs `f` inside one write transaction: committed if it returns `Ok`, rolled back if it
    /// returns `Err` or panics. Later tasks use this to change `message` and `op_queue` together.
    ///
    /// The transaction takes the write lock immediately, so it cannot fail halfway with "busy".
    /// `E` can be [`StorageError`] or any error type that converts from it.
    pub fn transaction<T, E>(
        &mut self,
        f: impl FnOnce(&Transaction<'_>) -> Result<T, E>,
    ) -> Result<T, E>
    where
        E: From<StorageError>,
    {
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|e| E::from(StorageError::from(e)))?;
        let out = f(&tx)?;
        tx.commit().map_err(|e| E::from(StorageError::from(e)))?;
        Ok(out)
    }
}

impl std::fmt::Debug for Db {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Db { .. }")
    }
}

#[cfg(feature = "sqlcipher")]
fn apply_key(conn: &Connection, key: Option<&[u8]>) -> Result<()> {
    let Some(key) = key else { return Ok(()) };
    if key.len() != 32 {
        return Err(StorageError::InvalidKey);
    }
    // SQLCipher raw-key syntax: x'<64 hex digits>' skips the passphrase KDF.
    let mut hex = String::with_capacity(67);
    hex.push_str("x'");
    for b in key {
        hex.push_str(&format!("{b:02x}"));
    }
    hex.push('\'');
    conn.pragma_update(None, "key", hex)?;
    Ok(())
}

#[cfg(not(feature = "sqlcipher"))]
fn apply_key(_conn: &Connection, key: Option<&[u8]>) -> Result<()> {
    match key {
        None => Ok(()),
        Some(_) => Err(StorageError::EncryptionUnavailable),
    }
}
