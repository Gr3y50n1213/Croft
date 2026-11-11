// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The database connection: open, key, migrate, transactions.

use std::path::Path;
use std::time::{Duration, Instant};

use rusqlite::{Connection, OpenFlags, Transaction, TransactionBehavior};
#[cfg(feature = "sqlcipher")]
use zeroize::Zeroizing;

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
    /// silently becomes a plain one. Pass `None` for an unencrypted database. The key is checked
    /// before the file is touched, so a refused key never leaves an empty file behind.
    ///
    /// Order matters. For a file that already has content, the key is applied and `user_version`
    /// is read on a separate **read-only** connection first. A database newer than this build is
    /// refused with [`StorageError::SchemaTooNew`], and a wrong key with
    /// [`StorageError::WrongKeyOrNotADatabase`], before anything is written. A read-only
    /// connection never checkpoints on close, so a leftover `-wal` file from a crashed app is not
    /// merged into (or deleted from) a database we are about to refuse. (SQLite may still create
    /// a missing `-shm` index file next to a `-wal` it has to read; that file holds no data and
    /// is rebuilt from the `-wal` on every open.) Only after that check passes is the file opened
    /// read-write, with foreign keys and WAL enabled and the pending migrations run, each in its
    /// own `BEGIN IMMEDIATE` transaction that re-checks `user_version`, so two openers of the
    /// same new file cannot both run the same migration.
    pub fn open(path: &Path, key: Option<&[u8]>) -> Result<Self> {
        validate_key(key)?;
        if has_content(path) {
            let ro = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY | FLAGS)?;
            ro.busy_timeout(BUSY_TIMEOUT)?;
            apply_key(&ro, key)?;
            let version = read_version(&ro)?;
            pending(version)?;
            // Dropped here: a read-only connection leaves the file and its -wal untouched.
            drop(ro);
        }
        let conn = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE | FLAGS,
        )?;
        Self::init(conn, key, true)
    }

    /// Opens a private in-memory database at the latest schema. For tests and previews.
    pub fn open_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        Self::init(conn, None, false)
    }

    fn init(conn: Connection, key: Option<&[u8]>, on_disk: bool) -> Result<Self> {
        // Before the first read, so concurrent openers wait for each other instead of failing.
        conn.busy_timeout(BUSY_TIMEOUT)?;
        apply_key(&conn, key)?;
        // The first read: with a wrong key (or a non-database file) it fails here.
        let version = read_version(&conn)?;
        let todo = pending(version)?;

        conn.pragma_update(None, "foreign_keys", true)?;
        let mode = enable_wal(&conn)?;
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
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .map_err(|cause| StorageError::Migration {
                    version: m.version,
                    cause,
                })?;
            // Another connection may have migrated between our read and our lock: look again now
            // that we hold the write lock, and skip what is already applied.
            let current = read_version(&tx)?;
            pending(current)?;
            if current >= m.version {
                continue;
            }
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

/// Flags shared by the read-only and read-write opens. No `SQLITE_OPEN_URI`: a path is a path, so
/// a file name that happens to start with `file:` cannot smuggle in connection options.
const FLAGS: OpenFlags = OpenFlags::SQLITE_OPEN_NO_MUTEX;

/// True when `path` is a file that may already hold a database: anything but a missing or empty
/// file. (An empty file is how a brand-new database starts out, so there is nothing to protect.)
fn has_content(path: &Path) -> bool {
    match std::fs::metadata(path) {
        Ok(m) => m.len() > 0,
        Err(e) => e.kind() != std::io::ErrorKind::NotFound,
    }
}

/// Switches the file to WAL and returns the journal mode SQLite reports.
///
/// Changing the journal mode needs an exclusive lock, and SQLite answers "busy" at once, without
/// calling the busy handler, when another connection is reading the file at that moment (two
/// openers of a brand-new file). So retry on busy for as long as the busy timeout allows. The
/// switch is permanent in the file, so only the first opener ever has to win it.
fn enable_wal(conn: &Connection) -> Result<String> {
    let started = Instant::now();
    loop {
        match conn.pragma_update_and_check(None, "journal_mode", "WAL", |r| r.get(0)) {
            Err(rusqlite::Error::SqliteFailure(f, _))
                if f.code == rusqlite::ErrorCode::DatabaseBusy
                    && started.elapsed() < BUSY_TIMEOUT =>
            {
                std::thread::sleep(Duration::from_millis(5));
            }
            other => return Ok(other?),
        }
    }
}

/// Reads `PRAGMA user_version`. With a wrong key (or a non-database file) this is where it fails.
fn read_version(conn: &Connection) -> Result<u32> {
    conn.pragma_query_value(None, "user_version", |r| r.get(0))
        .map_err(|e| match e {
            rusqlite::Error::SqliteFailure(f, _) if f.code == rusqlite::ErrorCode::NotADatabase => {
                StorageError::WrongKeyOrNotADatabase
            }
            other => StorageError::Sqlite(other),
        })
}

/// Checks the key before any file is opened or created.
#[cfg(feature = "sqlcipher")]
fn validate_key(key: Option<&[u8]>) -> Result<()> {
    match key {
        Some(k) if k.len() != 32 => Err(StorageError::InvalidKey),
        _ => Ok(()),
    }
}

/// Checks the key before any file is opened or created.
#[cfg(not(feature = "sqlcipher"))]
fn validate_key(key: Option<&[u8]>) -> Result<()> {
    match key {
        None => Ok(()),
        Some(_) => Err(StorageError::EncryptionUnavailable),
    }
}

/// Issues `PRAGMA key` with the raw 32-byte key (call [`validate_key`] first).
///
/// The statement text is the only copy of the key we build, and it is wiped when it goes out of
/// scope. It is made without `format!` so no temporary strings are left behind, and run with
/// `execute_batch`, which hands the text to SQLite as-is. SQLite and SQLCipher keep their own
/// copies of the key inside the connection; those are outside our control.
#[cfg(feature = "sqlcipher")]
fn apply_key(conn: &Connection, key: Option<&[u8]>) -> Result<()> {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let Some(key) = key else { return Ok(()) };
    if key.len() != 32 {
        return Err(StorageError::InvalidKey);
    }
    // SQLCipher raw-key syntax: "x'<64 hex digits>'" skips the passphrase KDF.
    // Exact capacity (16 for `PRAGMA key = "x'`, 64 digits, 2 for `'"`), so the buffer is never reallocated.
    let mut stmt = Zeroizing::new(String::with_capacity(16 + 64 + 2));
    stmt.push_str("PRAGMA key = \"x'");
    for b in key {
        stmt.push(char::from(HEX[usize::from(b >> 4)]));
        stmt.push(char::from(HEX[usize::from(b & 0x0f)]));
    }
    stmt.push_str("'\"");
    conn.execute_batch(&stmt)?;
    Ok(())
}

#[cfg(not(feature = "sqlcipher"))]
fn apply_key(_conn: &Connection, key: Option<&[u8]>) -> Result<()> {
    match key {
        None => Ok(()),
        Some(_) => Err(StorageError::EncryptionUnavailable),
    }
}
