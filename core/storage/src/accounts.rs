// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Accounts. The `account` table is the root of the account scope, so these queries are the one
//! place (with [`crate::settings`]) that does not filter by `account_id`.

use rusqlite::{params, Connection, OptionalExtension, Row};

use crate::error::Result;

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

/// Creates an account and returns its id.
pub fn insert(conn: &Connection, new: &NewAccount, now_ms: i64) -> Result<i64> {
    let config = serde_json::to_string(&new.config)?;
    conn.execute(
        r#"INSERT INTO account (label, email, protocol, provider, config_json, color, position, created_at)
           VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)"#,
        params![
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
    Ok(conn.last_insert_rowid())
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

/// Removes an account and, by cascade, all its folders, messages, identities, queue and outbox rows.
/// Returns false when the account does not exist.
pub fn delete(conn: &Connection, id: i64) -> Result<bool> {
    Ok(conn.execute("DELETE FROM account WHERE id = ?1", [id])? > 0)
}
