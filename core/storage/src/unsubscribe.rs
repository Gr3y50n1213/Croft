// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! One-tap unsubscribe (T37): a message's stored list headers, the older mail from its sender, and
//! the senders the person unsubscribed from. Every statement is scoped by account.

use rusqlite::{params, Connection, OptionalExtension};

use crate::error::Result;

/// What a message offers for unsubscribing, as stored when it was fetched (T35).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListHeaders {
    /// `List-Unsubscribe`, as stored (bounded and unfolded at fetch).
    pub unsubscribe: Option<String>,
    /// `List-Unsubscribe-Post: List-Unsubscribe=One-Click` came with it.
    pub one_click: bool,
    /// The sender's address.
    pub from_addr: Option<String>,
    /// The sender's display name.
    pub from_name: Option<String>,
    /// The message's date.
    pub date_ms: i64,
}

/// The list headers of message `id` of `account`, or `None` when there is no such message.
pub fn headers(conn: &Connection, account: i64, id: i64) -> Result<Option<ListHeaders>> {
    Ok(conn
        .query_row(
            "SELECT list_unsubscribe, list_one_click, from_addr, from_name, date_ts
               FROM message WHERE account_id = ?1 AND id = ?2",
            params![account, id],
            |r| {
                Ok(ListHeaders {
                    unsubscribe: r.get(0)?,
                    one_click: r.get(1)?,
                    from_addr: r.get(2)?,
                    from_name: r.get(3)?,
                    date_ms: r.get(4)?,
                })
            },
        )
        .optional()?)
}

/// The other messages of `account` from `sender` (any case) dated no later than `until_ms`,
/// newest first, leaving out message `except`, Trash, Junk, Sent and Drafts, and rows hidden by a
/// waiting move or delete: S31's "Also delete the N older messages". At most `limit`. (The From
/// address is not authenticated: never the person's own sent mail, whatever a message claims.)
pub fn older_from(
    conn: &Connection,
    account: i64,
    sender: &str,
    until_ms: i64,
    except: i64,
    limit: u32,
) -> Result<Vec<i64>> {
    let mut stmt = conn.prepare_cached(
        r#"SELECT id FROM message
            WHERE account_id = ?1 AND from_addr = ?2 COLLATE NOCASE AND date_ts <= ?3 AND id <> ?4
              AND local_hidden = 0
              AND folder_id NOT IN (SELECT id FROM folder
                                     WHERE account_id = ?1
                                       AND special_use IN ('trash', 'junk', 'sent', 'drafts'))
            ORDER BY date_ts DESC, id DESC LIMIT ?5"#,
    )?;
    let rows = stmt.query_map(params![account, sender, until_ms, except, limit], |r| {
        r.get(0)
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<i64>>>()?)
}

/// Whether `address` (any case) is the account's own: its address or one of its identities. A
/// message that claims to come from it is never trusted to name mail to delete (T37 review).
pub fn is_own_address(conn: &Connection, account: i64, address: &str) -> Result<bool> {
    Ok(conn
        .query_row(
            r#"SELECT 1 WHERE EXISTS (SELECT 1 FROM account WHERE id = ?1 AND email = ?2 COLLATE NOCASE)
                         OR EXISTS (SELECT 1 FROM identity
                                     WHERE account_id = ?1 AND address = ?2 COLLATE NOCASE)"#,
            params![account, address.trim()],
            |_| Ok(()),
        )
        .optional()?
        .is_some())
}

/// Remembers that the person unsubscribed from `sender` in `account`.
pub fn remember(conn: &Connection, account: i64, sender: &str, now_ms: i64) -> Result<()> {
    conn.execute(
        "INSERT INTO unsubscribed (account_id, sender, created_at) VALUES (?1, ?2, ?3)
         ON CONFLICT (account_id, sender) DO NOTHING",
        params![account, sender.to_ascii_lowercase(), now_ms],
    )?;
    Ok(())
}

/// Whether the person unsubscribed from `sender` in `account`.
pub fn is_unsubscribed(conn: &Connection, account: i64, sender: &str) -> Result<bool> {
    Ok(conn
        .query_row(
            "SELECT 1 FROM unsubscribed WHERE account_id = ?1 AND sender = ?2",
            params![account, sender.to_ascii_lowercase()],
            |_| Ok(()),
        )
        .optional()?
        .is_some())
}
