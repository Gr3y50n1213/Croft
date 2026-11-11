// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The addresses an account can send as (the From picker). Every account has at least one: its own
//! address, made the first time it is asked for. Scoped by `account_id`.

use rusqlite::{params, Connection, OptionalExtension};

use crate::error::Result;

/// One sender identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Identity {
    /// Row id.
    pub id: i64,
    /// Owning account.
    pub account_id: i64,
    /// The name shown to recipients.
    pub display_name: String,
    /// The address mail is sent from.
    pub address: String,
    /// The one the compose screen starts with.
    pub is_default: bool,
}

fn map(r: &rusqlite::Row<'_>) -> rusqlite::Result<Identity> {
    Ok(Identity {
        id: r.get(0)?,
        account_id: r.get(1)?,
        display_name: r.get(2)?,
        address: r.get(3)?,
        is_default: r.get(4)?,
    })
}

/// The account's identities, default first. When it has none, one is made from `address` and
/// `display_name` (the account's own) and returned.
pub fn list_or_create(
    conn: &Connection,
    account_id: i64,
    display_name: &str,
    address: &str,
) -> Result<Vec<Identity>> {
    let mut stmt = conn.prepare(
        "SELECT id, account_id, display_name, address, is_default FROM identity
          WHERE account_id = ?1 ORDER BY is_default DESC, id",
    )?;
    let existing: Vec<Identity> = stmt
        .query_map([account_id], map)?
        .collect::<rusqlite::Result<_>>()?;
    if !existing.is_empty() {
        return Ok(existing);
    }
    drop(stmt);
    conn.execute(
        "INSERT OR IGNORE INTO identity (account_id, display_name, address, is_default)
         VALUES (?1, ?2, ?3, 1)",
        params![account_id, display_name, address],
    )?;
    list_existing(conn, account_id)
}

fn list_existing(conn: &Connection, account_id: i64) -> Result<Vec<Identity>> {
    let mut stmt = conn.prepare(
        "SELECT id, account_id, display_name, address, is_default FROM identity
          WHERE account_id = ?1 ORDER BY is_default DESC, id",
    )?;
    let rows = stmt.query_map([account_id], map)?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// One identity of this account.
pub fn get(conn: &Connection, account_id: i64, id: i64) -> Result<Option<Identity>> {
    Ok(conn
        .query_row(
            "SELECT id, account_id, display_name, address, is_default FROM identity
              WHERE account_id = ?1 AND id = ?2",
            params![account_id, id],
            map,
        )
        .optional()?)
}

#[cfg(test)]
mod tests;
