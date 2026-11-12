// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The people the person has sent mail to: the source of the suggestions in the recipient fields.
//!
//! Scoped by `account_id`: an address used from one account is never suggested in another. Only
//! addresses of mail the person actually sent are here (and, when the sync stores them, the To and
//! Cc of the messages in the Sent folder); there is no link to the device's contacts.

use croft_mail_core::compose::Address;
use rusqlite::{params, Connection};

use crate::error::Result;

/// A suggestion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Suggestion {
    /// The display name last used with this address, if any.
    pub name: Option<String>,
    /// The address, in lower case.
    pub address: String,
}

/// Notes that mail was sent to these addresses at `now_ms`. An address already known has its use
/// count raised and its name replaced by the new one when there is one.
pub fn record(conn: &Connection, account_id: i64, sent_to: &[Address], now_ms: i64) -> Result<()> {
    for a in sent_to {
        let address = a.address.trim().to_lowercase();
        if address.is_empty() {
            continue;
        }
        let name = a
            .name
            .as_deref()
            .map(str::trim)
            .filter(|n| !n.is_empty())
            .map(str::to_owned);
        conn.execute(
            "INSERT INTO sent_recipient (account_id, address, name, last_used, uses)
             VALUES (?1, ?2, ?3, ?4, 1)
             ON CONFLICT (account_id, address) DO UPDATE SET
                 name = COALESCE(excluded.name, name),
                 last_used = MAX(last_used, excluded.last_used),
                 uses = uses + 1",
            params![account_id, address, name, now_ms],
        )?;
    }
    Ok(())
}

/// Escapes `%`, `_` and the escape character for a `LIKE ... ESCAPE '\'` pattern.
fn like_escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if matches!(c, '%' | '_' | '\\') {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// Addresses whose address or name starts with `prefix`, or has a word in its name that does
/// (`smi` finds `John Smith`), most used first, then most recent. At most `limit` results, never
/// more than 50. An empty prefix gives none.
pub fn suggest(
    conn: &Connection,
    account_id: i64,
    prefix: &str,
    limit: u32,
) -> Result<Vec<Suggestion>> {
    let prefix = prefix.trim().to_lowercase();
    if prefix.is_empty() {
        return Ok(Vec::new());
    }
    let p = like_escape(&prefix);
    let mut stmt = conn.prepare(
        "SELECT address, name FROM sent_recipient
          WHERE account_id = ?1
            AND (address LIKE ?2 || '%' ESCAPE '\\'
                 OR lower(name) LIKE ?2 || '%' ESCAPE '\\'
                 OR lower(name) LIKE '% ' || ?2 || '%' ESCAPE '\\')
          ORDER BY uses DESC, last_used DESC, address
          LIMIT ?3",
    )?;
    let rows = stmt.query_map(params![account_id, p, limit.min(50)], |r| {
        Ok(Suggestion {
            address: r.get(0)?,
            name: r.get(1)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

#[cfg(test)]
mod tests;
