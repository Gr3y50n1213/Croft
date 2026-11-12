// This Source Code Form is subject to the terms of the GNU Affero General Public License,
// version 3 or later. See LICENSE-AGPL-3.0 at the repository root.

//! The Companion's SQLite database (T23): forward-only migrations, users and their wrapped data
//! keys, the audit log, and the master key check value. Every query on a user's data is scoped by
//! the user id. Nothing here stores a key in the clear.

use std::path::Path;

use rusqlite::{params, Connection, OptionalExtension};

use crate::crypto::{self, CryptoError, Key, MASTER_CHECK_AAD};

/// The migrations, in order. `PRAGMA user_version` is how many have run.
const MIGRATIONS: &[&str] = &[
    include_str!("../migrations/0001_init.sql"),
    include_str!("../migrations/0002_sign_in.sql"),
    include_str!("../migrations/0003_oidc.sql"),
    include_str!("../migrations/0004_mail.sql"),
];

/// The plaintext of the master key check value.
const MASTER_CHECK: &[u8] = b"croft-companion master key";

/// What went wrong with the database. No variant carries a secret.
#[derive(Debug)]
pub enum StoreError {
    /// SQLite failed.
    Sql(rusqlite::Error),
    /// A key did not open (the wrong master key, or damaged data).
    Crypto(CryptoError),
    /// The database was written by a newer Companion.
    TooNew,
    /// The name is taken.
    NameTaken,
    /// No such user.
    NoSuchUser,
    /// The row already exists (a passkey registered twice).
    Conflict,
    /// The last passkey cannot be removed.
    LastPasskey,
    /// A stored row does not read back (damaged data).
    Damaged,
    /// A login name must be 1 to 64 letters, digits, `.`, `_` or `-`.
    BadName,
}

impl std::fmt::Display for StoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Sql(_) => f.write_str("the database failed"),
            Self::Crypto(e) => write!(f, "{e}"),
            Self::TooNew => f.write_str("the database was written by a newer Companion"),
            Self::NameTaken => f.write_str("that name is taken"),
            Self::NoSuchUser => f.write_str("no such user"),
            Self::Conflict => f.write_str("that already exists"),
            Self::LastPasskey => f.write_str("the last passkey cannot be removed"),
            Self::Damaged => f.write_str("stored data is damaged"),
            Self::BadName => f.write_str(
                "a login name must be 1 to 64 letters, digits, dots, underscores or dashes",
            ),
        }
    }
}

impl std::error::Error for StoreError {}

impl From<rusqlite::Error> for StoreError {
    fn from(e: rusqlite::Error) -> Self {
        Self::Sql(e)
    }
}

impl From<CryptoError> for StoreError {
    fn from(e: CryptoError) -> Self {
        Self::Crypto(e)
    }
}

/// A user's role.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// Manages the instance and its users.
    Admin,
    /// Uses it.
    User,
}

impl Role {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Admin => "admin",
            Self::User => "user",
        }
    }

    pub(crate) fn parse(s: &str) -> Self {
        if s == "admin" {
            Self::Admin
        } else {
            Self::User
        }
    }
}

/// A user as the admin sees it. No key, no secret.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct User {
    /// Row id.
    pub id: i64,
    /// Login name.
    pub name: String,
    /// Role.
    pub role: Role,
    /// Created, Unix milliseconds.
    pub created_ms: i64,
    /// Cannot sign in.
    pub disabled: bool,
}

/// The database.
pub struct Store {
    /// The connection. The sign-in module (`auth`) keeps its own queries scoped by user.
    pub(crate) conn: Connection,
}

impl std::fmt::Debug for Store {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Store")
    }
}

/// Now, Unix milliseconds.
pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}

impl Store {
    /// Opens (creating it if needed) and migrates the database at `path`.
    pub fn open(path: &Path) -> Result<Self, StoreError> {
        let conn = Connection::open(path)?;
        Self::setup(conn)
    }

    /// An in-memory database (tests).
    pub fn in_memory() -> Result<Self, StoreError> {
        Self::setup(Connection::open_in_memory()?)
    }

    fn setup(conn: Connection) -> Result<Self, StoreError> {
        conn.pragma_update(None, "foreign_keys", true)?;
        let _: String = conn.query_row("PRAGMA journal_mode = WAL", [], |r| r.get(0))?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        let mut s = Self { conn };
        s.migrate()?;
        Ok(s)
    }

    fn migrate(&mut self) -> Result<(), StoreError> {
        let version: i64 = self
            .conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))?;
        let known = i64::try_from(MIGRATIONS.len()).unwrap_or(i64::MAX);
        if version > known {
            return Err(StoreError::TooNew);
        }
        for (i, sql) in MIGRATIONS
            .iter()
            .enumerate()
            .skip(usize::try_from(version).unwrap_or(0))
        {
            let tx = self.conn.transaction()?;
            tx.execute_batch(sql)?;
            tx.pragma_update(
                None,
                "user_version",
                i64::try_from(i + 1).unwrap_or(i64::MAX),
            )?;
            tx.commit()?;
        }
        Ok(())
    }

    /// Answers when the database does (`/healthz`).
    pub fn ping(&self) -> bool {
        self.conn
            .query_row("SELECT 1", [], |r| r.get::<_, i64>(0))
            .is_ok()
    }

    /// Checks `master` is this instance's master key: it must open the check value **and every
    /// user's wrapped data key**. The check value lives in the same database, so on its own it
    /// proves nothing about a key file someone else wrote. A database without a check value takes
    /// one only while it has no user key: a wrong key can never "initialise" data that exists.
    pub fn check_master_key(&self, master: &Key) -> Result<(), StoreError> {
        check_key(&self.conn, master)
    }

    /// Adds a user with a fresh data key wrapped by `master`. The key is checked inside the same
    /// write transaction (taken at once), so a rotation that committed meanwhile is never
    /// undercut by a key wrapped under the old master key. Audited.
    pub fn add_user(&mut self, master: &Key, name: &str, role: Role) -> Result<User, StoreError> {
        let tx = self
            .conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let user = insert_user(&tx, master, name, role, now_ms())?;
        tx.commit()?;
        Ok(user)
    }

    /// Every user, oldest first.
    pub fn users(&self) -> Result<Vec<User>, StoreError> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, name, role, created_ms, disabled FROM user ORDER BY id")?;
        let rows = stmt.query_map([], |r| {
            Ok(User {
                id: r.get(0)?,
                name: r.get(1)?,
                role: Role::parse(&r.get::<_, String>(2)?),
                created_ms: r.get(3)?,
                disabled: r.get(4)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// `user_id`'s data key, unwrapped with `master`.
    pub fn data_key(&self, master: &Key, user_id: i64) -> Result<Key, StoreError> {
        let wrapped: Option<Vec<u8>> = self
            .conn
            .query_row(
                "SELECT wrapped FROM user_key WHERE user_id = ?1",
                params![user_id],
                |r| r.get(0),
            )
            .optional()?;
        let wrapped = wrapped.ok_or(StoreError::NoSuchUser)?;
        Ok(crypto::unwrap_data_key(master, user_id, &wrapped)?)
    }

    /// Rewraps every data key and the check value from `old` to `new` in one transaction: either
    /// all of them move to the new master key or none does. Audited.
    pub fn rotate_master_key(&mut self, old: &Key, new: &Key) -> Result<usize, StoreError> {
        self.check_master_key(old)?;
        let tx = self.conn.transaction()?;
        let keys: Vec<(i64, Vec<u8>)> = {
            let mut stmt = tx.prepare("SELECT user_id, wrapped FROM user_key")?;
            let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
            rows.collect::<rusqlite::Result<Vec<_>>>()?
        };
        for (user_id, wrapped) in &keys {
            let rewrapped = crypto::rewrap_data_key(old, new, *user_id, wrapped)?;
            tx.execute(
                "UPDATE user_key SET wrapped = ?1, key_version = key_version + 1 WHERE user_id = ?2",
                params![rewrapped, user_id],
            )?;
        }
        let check = new.seal(MASTER_CHECK_AAD, MASTER_CHECK)?;
        tx.execute(
            "UPDATE meta SET value = ?1 WHERE key = 'master_check'",
            params![check],
        )?;
        tx.execute(
            "INSERT INTO audit_log(at_ms, user_id, action, outcome) VALUES (?1, NULL, 'master_key.rotate', 'ok')",
            params![now_ms()],
        )?;
        tx.commit()?;
        Ok(keys.len())
    }

    /// Records an event. `action` and `outcome` are fixed words chosen by the code, never input.
    pub fn audit(
        &self,
        user_id: Option<i64>,
        action: &'static str,
        outcome: &'static str,
    ) -> Result<(), StoreError> {
        self.conn.execute(
            "INSERT INTO audit_log(at_ms, user_id, action, outcome) VALUES (?1, ?2, ?3, ?4)",
            params![now_ms(), user_id, action, outcome],
        )?;
        Ok(())
    }

    /// The tables of the schema and whether each has a `user_id` column (the scoping test).
    pub fn tables_and_user_scope(&self) -> Result<Vec<(String, bool)>, StoreError> {
        let mut stmt = self.conn.prepare(
            "SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%' ORDER BY name",
        )?;
        let names = stmt
            .query_map([], |r| r.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let mut out = Vec::new();
        for n in names {
            let mut cols = self.conn.prepare(&format!("PRAGMA table_info({n})"))?;
            let has = cols
                .query_map([], |r| r.get::<_, String>(1))?
                .collect::<rusqlite::Result<Vec<_>>>()?
                .iter()
                .any(|c| c == "user_id");
            out.push((n, has));
        }
        Ok(out)
    }
}

/// See [`Store::check_master_key`]. Runs on a connection or inside a transaction.
/// Whether `name` is a valid login name (1 to 64 letters, digits, `.`, `_` or `-`).
pub fn name_is_valid(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

/// Adds a user with a fresh data key inside the caller's write transaction (the master key is
/// checked in it, see [`Store::add_user`]). Audited.
pub(crate) fn insert_user(
    tx: &Connection,
    master: &Key,
    name: &str,
    role: Role,
    now: i64,
) -> Result<User, StoreError> {
    let name = name.trim();
    if !name_is_valid(name) {
        return Err(StoreError::BadName);
    }
    check_key(tx, master)?;
    let taken: bool = tx
        .query_row(
            "SELECT 1 FROM user WHERE name = ?1 COLLATE NOCASE",
            params![name],
            |_| Ok(true),
        )
        .optional()?
        .unwrap_or(false);
    if taken {
        return Err(StoreError::NameTaken);
    }
    tx.execute(
        "INSERT INTO user(name, role, created_ms) VALUES (?1, ?2, ?3)",
        params![name, role.as_str(), now],
    )?;
    let id = tx.last_insert_rowid();
    let (_, wrapped) = crypto::new_data_key(master, id)?;
    tx.execute(
        "INSERT INTO user_key(user_id, wrapped, key_version) VALUES (?1, ?2, 1)",
        params![id, wrapped],
    )?;
    tx.execute(
        "INSERT INTO audit_log(at_ms, user_id, action, outcome) VALUES (?1, ?2, 'user.add', 'ok')",
        params![now, id],
    )?;
    Ok(User {
        id,
        name: name.to_owned(),
        role,
        created_ms: now,
        disabled: false,
    })
}

fn check_key(conn: &Connection, master: &Key) -> Result<(), StoreError> {
    let stored: Option<Vec<u8>> = conn
        .query_row(
            "SELECT value FROM meta WHERE key = 'master_check'",
            [],
            |r| r.get(0),
        )
        .optional()?;
    let keys: Vec<(i64, Vec<u8>)> = {
        let mut stmt = conn.prepare("SELECT user_id, wrapped FROM user_key")?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
        rows.collect::<rusqlite::Result<Vec<_>>>()?
    };
    match stored {
        Some(sealed) => {
            let plain = master.open(MASTER_CHECK_AAD, &sealed)?;
            if plain.as_slice() != MASTER_CHECK {
                return Err(StoreError::Crypto(CryptoError::Open));
            }
        }
        // The check value is missing while users exist: someone removed it. Never re-create it
        // here (that would accept any key); the key must prove itself on the users' keys below.
        None if !keys.is_empty() => {}
        None => {
            let sealed = master.seal(MASTER_CHECK_AAD, MASTER_CHECK)?;
            conn.execute(
                "INSERT INTO meta(key, value) VALUES ('master_check', ?1)",
                params![sealed],
            )?;
        }
    }
    for (user_id, wrapped) in &keys {
        crypto::unwrap_data_key(master, *user_id, wrapped)?;
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn migrations_apply_once_and_a_newer_database_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("c.db");
        Store::open(&path).unwrap();
        Store::open(&path).unwrap(); // idempotent
        let conn = Connection::open(&path).unwrap();
        conn.pragma_update(None, "user_version", 99).unwrap();
        drop(conn);
        assert!(matches!(Store::open(&path), Err(StoreError::TooNew)));
    }

    #[test]
    fn every_table_has_a_user_scope_or_is_global() {
        // Global, by design: `meta` (instance settings), `user` (the users themselves).
        let global = ["meta", "user"];
        let s = Store::in_memory().unwrap();
        for (table, scoped) in s.tables_and_user_scope().unwrap() {
            assert!(
                scoped || global.contains(&table.as_str()),
                "{table} has no user_id"
            );
        }
    }

    #[test]
    fn users_get_a_wrapped_data_key_and_nothing_secret_is_listed() {
        let master = Key::generate().unwrap();
        let mut s = Store::in_memory().unwrap();
        s.check_master_key(&master).unwrap();
        let a = s.add_user(&master, "admin", Role::Admin).unwrap();
        let b = s.add_user(&master, " bob ", Role::User).unwrap();
        assert_eq!(b.name, "bob");
        assert!(matches!(
            s.add_user(&master, "bob", Role::User),
            Err(StoreError::NameTaken)
        ));
        let listed = s.users().unwrap();
        assert_eq!(listed, vec![a.clone(), b.clone()]);
        // Each data key opens only with the master key, and only for its user.
        let ka = s.data_key(&master, a.id).unwrap();
        let sealed = crypto::seal_value(&ka, a.id, "test", b"made-up").unwrap();
        assert!(
            crypto::open_value(&s.data_key(&master, b.id).unwrap(), b.id, "test", &sealed).is_err()
        );
        let other = Key::generate().unwrap();
        assert!(matches!(
            s.data_key(&other, a.id),
            Err(StoreError::Crypto(_))
        ));
        assert!(matches!(
            s.check_master_key(&other),
            Err(StoreError::Crypto(_))
        ));
    }

    #[test]
    fn a_removed_check_value_never_lets_a_wrong_key_in() {
        let master = Key::generate().unwrap();
        let mut s = Store::in_memory().unwrap();
        s.check_master_key(&master).unwrap();
        s.add_user(&master, "u", Role::User).unwrap();
        s.conn
            .execute("DELETE FROM meta WHERE key = 'master_check'", [])
            .unwrap();
        let wrong = Key::generate().unwrap();
        assert!(
            s.check_master_key(&wrong).is_err(),
            "the users' keys do not open"
        );
        assert!(s.add_user(&wrong, "v", Role::User).is_err());
        s.check_master_key(&master).unwrap();
    }

    #[test]
    fn rotation_moves_every_key_and_the_old_master_key_then_opens_nothing() {
        let old = Key::generate().unwrap();
        let mut s = Store::in_memory().unwrap();
        s.check_master_key(&old).unwrap();
        let u = s.add_user(&old, "u", Role::User).unwrap();
        let sealed = crypto::seal_value(&s.data_key(&old, u.id).unwrap(), u.id, "c", b"v").unwrap();
        let new = Key::generate().unwrap();
        assert_eq!(s.rotate_master_key(&old, &new).unwrap(), 1);
        s.check_master_key(&new).unwrap();
        assert!(s.check_master_key(&old).is_err());
        let k = s.data_key(&new, u.id).unwrap();
        assert_eq!(&**crypto::open_value(&k, u.id, "c", &sealed).unwrap(), b"v");
        assert!(s.data_key(&old, u.id).is_err());
        // A rotation with the wrong "old" key changes nothing.
        let wrong = Key::generate().unwrap();
        assert!(s
            .rotate_master_key(&wrong, &Key::generate().unwrap())
            .is_err());
        s.check_master_key(&new).unwrap();
    }
}
