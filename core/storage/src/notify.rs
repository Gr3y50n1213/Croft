// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Storage for notifications (T20): the per-account "notified up to" marker (in the `setting`
//! table, key `notify.marker.<account>`) and the Inbox rows a decision needs. Every query is
//! scoped by `account_id`.

use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};

use crate::error::Result;
use crate::settings;

/// An account's stored marker: the Inbox's UIDVALIDITY and the highest UID notified.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredMarker {
    /// UIDVALIDITY when it was set.
    pub uid_validity: u32,
    /// Highest UID notified (or the baseline).
    pub up_to: u32,
}

fn key(account_id: i64) -> String {
    format!("notify.marker.{account_id}")
}

/// The account's marker, if any.
pub fn marker(conn: &Connection, account_id: i64) -> Result<Option<StoredMarker>> {
    settings::get(conn, &key(account_id))
}

/// Stores the account's marker.
pub fn set_marker(conn: &Connection, account_id: i64, m: StoredMarker, now_ms: i64) -> Result<()> {
    settings::set(conn, &key(account_id), &m, now_ms)
}

/// Forgets the account's marker (the account was removed). Returns whether there was one.
pub fn remove_marker(conn: &Connection, account_id: i64) -> Result<bool> {
    settings::remove(conn, &key(account_id))
}

/// One Inbox row for the decision: `(row id, UID, read)`.
pub type InboxUid = (i64, u32, bool);

/// The rows of a folder of this account with a UID above `above` (rows hidden by a waiting move
/// or delete are left out).
pub fn rows_above(
    conn: &Connection,
    account_id: i64,
    folder_id: i64,
    above: u32,
) -> Result<Vec<InboxUid>> {
    let mut stmt = conn.prepare_cached(
        "SELECT id, uid, is_seen FROM message
          WHERE account_id = ?1 AND folder_id = ?2 AND uid > ?3 AND local_hidden = 0",
    )?;
    let rows = stmt.query_map(params![account_id, folder_id, above], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            u32::try_from(r.get::<_, i64>(1)?).unwrap_or(0),
            r.get::<_, bool>(2)?,
        ))
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

/// What a notification shows of a message: `(sender name, sender address, subject, date)`.
pub type Shown = (Option<String>, Option<String>, String, i64);

/// The sender, subject and date of a message of this account, if it exists.
pub fn shown(conn: &Connection, account_id: i64, message_id: i64) -> Result<Option<Shown>> {
    let mut stmt = conn.prepare_cached(
        "SELECT from_name, from_addr, subject, date_ts FROM message WHERE account_id = ?1 AND id = ?2",
    )?;
    let mut rows = stmt.query_map(params![account_id, message_id], |r| {
        Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
    })?;
    Ok(rows.next().transpose()?)
}
