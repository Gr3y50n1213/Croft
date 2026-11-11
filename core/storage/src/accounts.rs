// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Accounts. The `account` table is the root of the account scope, so these queries are the one
//! place (with [`crate::settings`]) that does not filter by `account_id`.

use rusqlite::{params, Connection, OptionalExtension, Row};

use crate::error::{Result, StorageError};
use crate::settings;

/// Which protocol an account speaks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Protocol {
    /// IMAP + SMTP.
    Imap,
    /// JMAP.
    Jmap,
}

impl Protocol {
    fn as_db_str(self) -> &'static str {
        match self {
            Self::Imap => "imap",
            Self::Jmap => "jmap",
        }
    }

    fn from_db_str(s: &str) -> Option<Self> {
        match s {
            "imap" => Some(Self::Imap),
            "jmap" => Some(Self::Jmap),
            _ => None,
        }
    }
}

/// Sync state of an account, as shown in the account list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// Idle and healthy.
    Ok,
    /// A sync is running.
    Syncing,
    /// Credentials were rejected.
    NeedsSignIn,
    /// No connection.
    Offline,
    /// Any other failure.
    Error,
}

impl Status {
    fn as_db_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Syncing => "syncing",
            Self::NeedsSignIn => "needs_sign_in",
            Self::Offline => "offline",
            Self::Error => "error",
        }
    }

    fn from_db_str(s: &str) -> Option<Self> {
        match s {
            "ok" => Some(Self::Ok),
            "syncing" => Some(Self::Syncing),
            "needs_sign_in" => Some(Self::NeedsSignIn),
            "offline" => Some(Self::Offline),
            "error" => Some(Self::Error),
            _ => None,
        }
    }
}

/// An account to create.
#[derive(Debug, Clone, PartialEq)]
pub struct NewAccount {
    /// Name shown in the account list.
    pub label: String,
    /// The account's own address.
    pub email: String,
    /// IMAP or JMAP.
    pub protocol: Protocol,
    /// `generic`, `gmail`, `outlook`, ...
    pub provider: String,
    /// Hosts, ports, security and auth kind. Never secrets.
    pub config: serde_json::Value,
    /// Optional ARGB colour override.
    pub color: Option<i64>,
    /// Position in the account list.
    pub position: i64,
}

/// A stored account.
#[derive(Debug, Clone, PartialEq)]
pub struct Account {
    /// Row id; every other table refers to it as `account_id`.
    pub id: i64,
    /// Name shown in the account list.
    pub label: String,
    /// The account's own address.
    pub email: String,
    /// IMAP or JMAP.
    pub protocol: Protocol,
    /// `generic`, `gmail`, `outlook`, ...
    pub provider: String,
    /// Hosts, ports, security and auth kind. Never secrets.
    pub config: serde_json::Value,
    /// Optional ARGB colour override.
    pub color: Option<i64>,
    /// Position in the account list.
    pub position: i64,
    /// Sync state.
    pub status: Status,
    /// Creation time, Unix milliseconds.
    pub created_at: i64,
}

fn bad_column(idx: usize, what: &str) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(
        idx,
        rusqlite::types::Type::Text,
        format!("unknown {what}").into(),
    )
}

fn map_row(row: &Row<'_>) -> rusqlite::Result<Account> {
    let protocol: String = row.get(3)?;
    let config: String = row.get(5)?;
    let status: String = row.get(8)?;
    Ok(Account {
        id: row.get(0)?,
        label: row.get(1)?,
        email: row.get(2)?,
        protocol: Protocol::from_db_str(&protocol).ok_or_else(|| bad_column(3, "protocol"))?,
        provider: row.get(4)?,
        config: serde_json::from_str(&config).map_err(|_| bad_column(5, "config"))?,
        color: row.get(6)?,
        position: row.get(7)?,
        status: Status::from_db_str(&status).ok_or_else(|| bad_column(8, "status"))?,
        created_at: row.get(9)?,
    })
}

/// Name of the app-wide setting that holds the largest account id ever used. It is one of the
/// few places that touches the `setting` table, which has no `account_id`.
const MAX_ID_KEY: &str = "account.max_id";

/// Creates an account and returns its id.
///
/// Ids are never reused. The account's id is the key of its secrets in the platform keystore
/// (outside this database) and of the app's other per-account state, so an id given to a removed
/// account and later to a new one could hand the new account the old one's stored secrets. The
/// table's own `MAX(id)` shrinks when the last account is removed, so the largest id ever used is
/// kept in the `setting` table (key `account.max_id`), and the new id is
/// `max(that, MAX(id)) + 1`. The id is chosen, the row inserted and the mark updated in one
/// transaction: a savepoint, so it also works inside the caller's transaction. No migration is
/// needed; a database without the key starts from `MAX(id)`.
pub fn insert(conn: &Connection, new: &NewAccount, now_ms: i64) -> Result<i64> {
    let config = serde_json::to_string(&new.config)?;
    conn.execute_batch("SAVEPOINT account_insert")?;
    let result = insert_in_savepoint(conn, new, &config, now_ms);
    match result {
        Ok(id) => {
            conn.execute_batch("RELEASE account_insert")?;
            Ok(id)
        }
        Err(e) => {
            // Undo and close the savepoint; the original error is the one to report.
            let _ = conn.execute_batch("ROLLBACK TO account_insert; RELEASE account_insert");
            Err(e)
        }
    }
}

fn insert_in_savepoint(
    conn: &Connection,
    new: &NewAccount,
    config: &str,
    now_ms: i64,
) -> Result<i64> {
    let table_max: i64 =
        conn.query_row("SELECT COALESCE(MAX(id), 0) FROM account", [], |r| r.get(0))?;
    // A mark that is not an integer (damaged or written by hand) is treated as absent: the id then
    // comes from `MAX(id)`, and the mark is overwritten below. Failing here would make every
    // insert fail for good.
    let high_water = match settings::get::<i64>(conn, MAX_ID_KEY) {
        Ok(mark) => mark.unwrap_or(0),
        // Only a document that cannot be read is ignored; a database error still fails the insert
        // (guessing then could reuse an id).
        Err(StorageError::Json(_)) => 0,
        Err(e) => return Err(e),
    };
    let id = table_max
        .max(high_water)
        .checked_add(1)
        .ok_or(StorageError::OutOfRange("account id"))?;
    conn.execute(
        r#"INSERT INTO account (id, label, email, protocol, provider, config_json, color, position, created_at)
           VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)"#,
        params![
            id,
            new.label,
            new.email,
            new.protocol.as_db_str(),
            new.provider,
            config,
            new.color,
            new.position,
            now_ms
        ],
    )?;
    settings::set(conn, MAX_ID_KEY, &id, now_ms)?;
    Ok(id)
}

/// Reads one account.
pub fn get(conn: &Connection, id: i64) -> Result<Option<Account>> {
    Ok(conn
        .query_row(
            r#"SELECT id, label, email, protocol, provider, config_json, color, position, status, created_at
                 FROM account WHERE id = ?1"#,
            [id],
            map_row,
        )
        .optional()?)
}

/// Every account, in list order.
pub fn list(conn: &Connection) -> Result<Vec<Account>> {
    let mut stmt = conn.prepare_cached(
        r#"SELECT id, label, email, protocol, provider, config_json, color, position, status, created_at
             FROM account ORDER BY position, id"#,
    )?;
    let rows = stmt.query_map([], map_row)?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

/// Sets the sync state. Returns false when the account does not exist.
pub fn set_status(conn: &Connection, id: i64, status: Status) -> Result<bool> {
    let n = conn.execute(
        "UPDATE account SET status = ?2 WHERE id = ?1",
        params![id, status.as_db_str()],
    )?;
    Ok(n > 0)
}

/// Replaces an account's provider name and stored settings, and nothing else: its id, address,
/// label, position, colour and every row that belongs to it stay. Returns false when the account
/// does not exist. Like `config`, the new settings never hold a secret.
pub fn set_settings(
    conn: &Connection,
    id: i64,
    provider: &str,
    config: &serde_json::Value,
) -> Result<bool> {
    let config = serde_json::to_string(config)?;
    let n = conn.execute(
        "UPDATE account SET provider = ?2, config_json = ?3 WHERE id = ?1",
        params![id, provider, config],
    )?;
    Ok(n > 0)
}

/// Removes an account and, by cascade, all its folders, messages, identities, queue and outbox rows.
/// Returns false when the account does not exist.
pub fn delete(conn: &Connection, id: i64) -> Result<bool> {
    Ok(conn.execute("DELETE FROM account WHERE id = ?1", [id])? > 0)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::Db;

    fn new_account(label: &str) -> NewAccount {
        NewAccount {
            label: label.to_owned(),
            email: format!("{label}@example.org"),
            protocol: Protocol::Imap,
            provider: "generic".to_owned(),
            config: serde_json::json!({}),
            color: None,
            position: 0,
        }
    }

    #[test]
    fn an_account_id_is_never_reused() {
        let db = Db::open_in_memory().unwrap();
        let c = db.connection();
        assert_eq!(insert(c, &new_account("one"), 1).unwrap(), 1);
        let second = insert(c, &new_account("two"), 2).unwrap();
        assert_eq!(second, 2);
        assert!(delete(c, second).unwrap());
        assert_eq!(insert(c, &new_account("three"), 3).unwrap(), 3);
        // Removing every account still does not bring the counter back.
        for a in list(c).unwrap() {
            assert!(delete(c, a.id).unwrap());
        }
        assert_eq!(insert(c, &new_account("four"), 4).unwrap(), 4);
    }

    #[test]
    fn a_database_without_the_mark_starts_from_the_largest_id() {
        // An account row written before the mark existed (no migration): the next id is above it.
        let db = Db::open_in_memory().unwrap();
        let c = db.connection();
        c.execute(
            "INSERT INTO account (id, label, email, protocol, config_json, created_at)
             VALUES (7, 'old', 'old@example.org', 'imap', '{}', 1)",
            [],
        )
        .unwrap();
        assert_eq!(insert(c, &new_account("new"), 2).unwrap(), 8);
    }

    #[test]
    fn a_mark_that_is_not_an_integer_is_ignored_and_overwritten() {
        let db = Db::open_in_memory().unwrap();
        let c = db.connection();
        assert_eq!(insert(c, &new_account("one"), 1).unwrap(), 1);
        for bad in ["\"seven\"", "{\"a\":1}", "not json", "1.5", "null"] {
            c.execute(
                "INSERT INTO setting (key, value_json, updated_at) VALUES ('account.max_id', ?1, 1)
                 ON CONFLICT (key) DO UPDATE SET value_json = excluded.value_json",
                [bad],
            )
            .unwrap();
            let id = insert(c, &new_account("next"), 2).unwrap();
            // The id falls back to MAX(id) + 1, and the mark is repaired.
            let max: i64 = c
                .query_row("SELECT MAX(id) FROM account", [], |r| r.get(0))
                .unwrap();
            assert_eq!(id, max, "bad mark {bad}");
            assert_eq!(settings::get::<i64>(c, MAX_ID_KEY).unwrap(), Some(id));
        }
    }

    #[test]
    fn a_rolled_back_insert_leaves_the_counter_alone() {
        let mut db = Db::open_in_memory().unwrap();
        let failed: Result<()> = db.transaction(|tx| {
            insert(tx, &new_account("gone"), 1)?;
            Err(StorageError::NotFound("test"))
        });
        assert!(failed.is_err());
        assert_eq!(insert(db.connection(), &new_account("kept"), 2).unwrap(), 1);
    }
}
