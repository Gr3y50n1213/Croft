// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The database connection: open, key, migrate, transactions.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use rusqlite::config::DbConfig;
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
    /// Order matters. For a file that exists, the key is applied and `user_version` is read on a
    /// separate **read-only** connection first. A database newer than this build is refused with
    /// [`StorageError::SchemaTooNew`], and a wrong key with
    /// [`StorageError::WrongKeyOrNotADatabase`], before anything is written. A read-only
    /// connection never checkpoints on close, so a leftover `-wal` file from a crashed app is not
    /// merged into (or deleted from) a database we are about to refuse. Only after that check
    /// passes is the file opened read-write, with foreign keys and WAL enabled and the pending
    /// migrations run, each in its own `BEGIN IMMEDIATE` transaction that re-checks
    /// `user_version`, so two openers of the same new file cannot both run the same migration.
    /// The read-write connection also checks the version before anything else (another process
    /// may have upgraded the file in between) and does not checkpoint on close until that check
    /// and the migrations are done, so even a refusal there leaves the file and its `-wal` alone.
    ///
    /// A `-journal` left by a crash beside a rollback-mode file makes the read-only check fail;
    /// the file is then opened read-write once, only so SQLite can roll the journal back, and the
    /// version is checked on that connection before anything else.
    ///
    /// Opening a WAL database, even one that is then refused, may create empty `-wal` and `-shm`
    /// files next to it. They hold no data of their own; the data in the database is never
    /// changed by a refused open.
    ///
    /// The open can block: each wait for another connection's lock lasts up to five seconds, and
    /// several can follow one another. Call it from a worker thread, never from the UI thread.
    ///
    /// A relative `path` is taken as relative to the current directory and is never read as a
    /// `file:` URI.
    pub fn open(path: &Path, key: Option<&[u8]>) -> Result<Self> {
        validate_key(key)?;
        let path = literal_path(path);
        let path = path.as_path();
        if exists(path) {
            match check_version_read_only(path, key) {
                Err(StorageError::Sqlite(rusqlite::Error::SqliteFailure(f, _)))
                    if f.extended_code == SQLITE_READONLY_ROLLBACK =>
                {
                    recover_hot_journal(path, key)?;
                }
                other => other?,
            }
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
        // A refusal below drops this connection; closing must not merge the -wal into a database
        // that turns out to be too new (or has the wrong key). Re-enabled once we are past that.
        set_checkpoint_on_close(&conn, false)?;
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
        // Checked and migrated: from here on a normal close checkpoints as usual.
        set_checkpoint_on_close(&db.conn, true)?;
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

/// Flags shared by the read-only and read-write opens. `SQLITE_OPEN_URI` is not set, but the
/// bundled SQLite is compiled with `SQLITE_USE_URI`, so a name starting with `file:` would be read
/// as a URI anyway. [`literal_path`] makes sure no path we pass on can start that way.
const FLAGS: OpenFlags = OpenFlags::SQLITE_OPEN_NO_MUTEX;

/// `SQLITE_READONLY_ROLLBACK`: a read-only connection met a hot rollback journal it may not play back.
const SQLITE_READONLY_ROLLBACK: std::ffi::c_int = 776;

/// Makes a relative path unable to start with `file:` (and so be read as a URI) by prefixing `./`.
/// An absolute path cannot start with `file:` either.
fn literal_path(path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        Path::new(".").join(path)
    }
}

/// True when something exists at `path`. (A zero-byte file is how a brand-new database starts out;
/// the read-only check reads it as version 0.)
fn exists(path: &Path) -> bool {
    match std::fs::metadata(path) {
        Ok(_) => true,
        Err(e) => e.kind() != std::io::ErrorKind::NotFound,
    }
}

/// Turns checkpoint-on-close off (`false`) or back on (`true`) for this connection.
fn set_checkpoint_on_close(conn: &Connection, on: bool) -> Result<()> {
    conn.set_db_config(DbConfig::SQLITE_DBCONFIG_NO_CKPT_ON_CLOSE, !on)?;
    Ok(())
}

/// Applies the key and refuses a too-new database or a wrong key, on a read-only connection.
fn check_version_read_only(path: &Path, key: Option<&[u8]>) -> Result<()> {
    let ro = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY | FLAGS)?;
    ro.busy_timeout(BUSY_TIMEOUT)?;
    apply_key(&ro, key)?;
    let version = read_version(&ro)?;
    pending(version)?;
    // Dropped here: a read-only connection leaves the file and its -wal untouched.
    Ok(())
}

/// Lets SQLite roll back a crash's hot `-journal`, then checks the version on that connection.
///
/// Rolling back a hot journal is recovery, not a change of meaning: it restores the last committed
/// state, which is what every reader would see anyway, so it is done even for a database that is
/// then refused as too new. Nothing else is done on this connection: it is not created, nothing is
/// written and it does not checkpoint on close.
fn recover_hot_journal(path: &Path, key: Option<&[u8]>) -> Result<()> {
    let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_WRITE | FLAGS)?;
    conn.busy_timeout(BUSY_TIMEOUT)?;
    set_checkpoint_on_close(&conn, false)?;
    apply_key(&conn, key)?;
    // Any read rolls the hot journal back first.
    let version = read_version(&conn)?;
    pending(version)?;
    Ok(())
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

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)] // a panic is the failure report
mod tests {
    use super::*;
    use crate::{latest_version, SchemaTooNew};

    fn bytes(p: &Path) -> Vec<u8> {
        std::fs::read(p).unwrap()
    }

    /// A WAL database at `<dir>/live.db` whose latest changes (user_version included) exist only
    /// in a non-empty `-wal`, copied while the writer is still open (the state a crash leaves).
    fn live_wal(dir: &Path, version: u32) -> PathBuf {
        let src = tempfile::tempdir().unwrap();
        let src_db = src.path().join("live.db");
        let w = Connection::open(&src_db).unwrap();
        let mode: String = w
            .pragma_update_and_check(None, "journal_mode", "WAL", |r| r.get(0))
            .unwrap();
        assert_eq!(mode, "wal");
        w.pragma_update(None, "wal_autocheckpoint", 0).unwrap();
        w.execute_batch("CREATE TABLE t (x INTEGER); INSERT INTO t VALUES (1);")
            .unwrap();
        w.pragma_update(None, "user_version", version).unwrap();
        let dst = dir.join("live.db");
        std::fs::copy(&src_db, &dst).unwrap();
        std::fs::copy(src.path().join("live.db-wal"), dir.join("live.db-wal")).unwrap();
        drop(w);
        assert!(std::fs::metadata(dir.join("live.db-wal")).unwrap().len() > 0);
        dst
    }

    /// A rollback-mode database at `<dir>/hot.db` with one row and `user_version = version`, plus
    /// a hot `-journal` from a write transaction that never finished (a second row).
    fn hot_journal(dir: &Path, version: u32) -> PathBuf {
        let src = tempfile::tempdir().unwrap();
        let src_db = src.path().join("hot.db");
        let w = Connection::open(&src_db).unwrap();
        w.pragma_update(None, "synchronous", "OFF").unwrap();
        let mode: String = w
            .pragma_update_and_check(None, "journal_mode", "DELETE", |r| r.get(0))
            .unwrap();
        assert_eq!(mode, "delete");
        w.execute_batch("CREATE TABLE t (x INTEGER); INSERT INTO t VALUES (1);")
            .unwrap();
        w.pragma_update(None, "user_version", version).unwrap();
        w.execute_batch("BEGIN; INSERT INTO t VALUES (2);").unwrap();
        let dst = dir.join("hot.db");
        std::fs::copy(&src_db, &dst).unwrap();
        std::fs::copy(
            src.path().join("hot.db-journal"),
            dir.join("hot.db-journal"),
        )
        .unwrap();
        drop(w);
        assert!(std::fs::metadata(dir.join("hot.db-journal")).unwrap().len() > 0);
        dst
    }

    /// Finding 1: the read-write path itself must not checkpoint a too-new database it refuses
    /// (the race where the file is upgraded between the read-only check and the read-write open).
    #[test]
    fn read_write_init_refusing_a_too_new_database_does_not_checkpoint() {
        let dir = tempfile::tempdir().unwrap();
        let path = live_wal(dir.path(), latest_version() + 1);
        let wal = dir.path().join("live.db-wal");
        let (main_before, wal_before) = (bytes(&path), bytes(&wal));

        let conn = Connection::open_with_flags(
            &path,
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE | FLAGS,
        )
        .unwrap();
        let err = Db::init(conn, None, true).unwrap_err();
        assert!(matches!(
            err,
            StorageError::SchemaTooNew(SchemaTooNew { database, .. }) if database == latest_version() + 1
        ));
        assert_eq!(bytes(&path), main_before, "main file changed");
        assert_eq!(bytes(&wal), wal_before, "-wal changed or was removed");
    }

    /// A normal close still checkpoints (the flag is turned back off after migrating).
    #[test]
    fn a_normal_close_still_checkpoints() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.db");
        let db = Db::open(&path, None).unwrap();
        assert!(!db
            .conn
            .db_config(DbConfig::SQLITE_DBCONFIG_NO_CKPT_ON_CLOSE)
            .unwrap());
        drop(db);
        assert!(
            !dir.path().join("a.db-wal").exists(),
            "-wal not merged on close"
        );
    }

    #[test]
    fn a_zero_byte_file_is_a_new_database() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("empty.db");
        std::fs::File::create(&path).unwrap();
        let db = Db::open(&path, None).unwrap();
        assert_eq!(db.schema_version().unwrap(), latest_version());
    }

    /// Finding 2: a relative path never starts with `file:`.
    #[test]
    fn relative_paths_are_prefixed_so_they_are_never_uris() {
        let p = literal_path(Path::new("file:x.db?mode=memory"));
        assert_eq!(p, Path::new("./file:x.db?mode=memory"));
        let abs = std::env::temp_dir().join("x.db");
        assert_eq!(literal_path(&abs), abs);
    }

    /// Finding 2, end to end: a relative `file:` name is a literal file name in the current directory.
    #[test]
    fn a_relative_file_uri_looking_name_is_a_literal_file() {
        let dir = tempfile::tempdir().unwrap();
        let old = std::env::current_dir().unwrap();
        std::env::set_current_dir(dir.path()).unwrap();
        let result = Db::open(Path::new("file:x.db?mode=memory"), None).map(drop);
        let exists = std::fs::metadata("file:x.db?mode=memory").is_ok();
        std::env::set_current_dir(old).unwrap();
        result.unwrap();
        assert!(exists, "a literal file with that name must be created");
    }

    /// Finding 3: a crash's hot journal beside a rollback-mode file does not stop the open.
    #[test]
    fn a_hot_journal_is_rolled_back_and_the_database_opens() {
        let dir = tempfile::tempdir().unwrap();
        let path = hot_journal(dir.path(), 0);
        let db = Db::open(&path, None).unwrap();
        let rows: Vec<i64> = db
            .connection()
            .prepare("SELECT x FROM t ORDER BY x")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(rows, vec![1], "the unfinished transaction is rolled back");
        assert_eq!(db.schema_version().unwrap(), latest_version());
    }

    /// Finding 3: recovery does not let a too-new database through.
    #[test]
    fn a_too_new_database_with_a_hot_journal_is_still_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = hot_journal(dir.path(), latest_version() + 1);
        let err = Db::open(&path, None).unwrap_err();
        assert!(matches!(err, StorageError::SchemaTooNew(_)), "{err}");
        let conn = Connection::open(&path).unwrap();
        let v: u32 = conn
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .unwrap();
        assert_eq!(v, latest_version() + 1);
    }
}
