// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Folders. Every function takes the `account_id` and filters on it, so an id from another
//! account never matches.

use croft_protocols::{FolderStatus, RemoteFolder};
use rusqlite::{params, Connection, OptionalExtension, Row};

use crate::error::{Result, StorageError};

/// A stored folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Folder {
    /// Row id.
    pub id: i64,
    /// Owning account.
    pub account_id: i64,
    /// Full path with the server's delimiter.
    pub path: String,
    /// Hierarchy delimiter, if any.
    pub delimiter: Option<String>,
    /// `special_use` column text, for example `inbox` (see `SpecialUse::as_db_str`).
    pub special_use: String,
    /// UIDVALIDITY at the last sync.
    pub uid_validity: Option<i64>,
    /// UIDNEXT at the last sync.
    pub uid_next: Option<i64>,
    /// HIGHESTMODSEQ at the last sync.
    pub highest_modseq: Option<i64>,
    /// Unread messages, kept by triggers.
    pub unread_count: i64,
    /// All messages, kept by triggers.
    pub total_count: i64,
    /// Last successful sync, Unix milliseconds.
    pub last_synced_at: Option<i64>,
}

fn map_row(row: &Row<'_>) -> rusqlite::Result<Folder> {
    Ok(Folder {
        id: row.get(0)?,
        account_id: row.get(1)?,
        path: row.get(2)?,
        delimiter: row.get(3)?,
        special_use: row.get(4)?,
        uid_validity: row.get(5)?,
        uid_next: row.get(6)?,
        highest_modseq: row.get(7)?,
        unread_count: row.get(8)?,
        total_count: row.get(9)?,
        last_synced_at: row.get(10)?,
    })
}

/// Inserts the folder, or updates its delimiter and special use if the account already has a
/// folder with this path. Counters and sync state are kept. Returns the folder id.
pub fn upsert(conn: &Connection, account_id: i64, folder: &RemoteFolder) -> Result<i64> {
    let delimiter = folder.delimiter.map(String::from);
    let id = conn.query_row(
        r#"INSERT INTO folder (account_id, path, delimiter, special_use)
           VALUES (?1, ?2, ?3, ?4)
           ON CONFLICT (account_id, path) DO UPDATE
               SET delimiter = excluded.delimiter, special_use = excluded.special_use
           RETURNING id"#,
        params![
            account_id,
            folder.path,
            delimiter,
            folder.special_use.as_db_str()
        ],
        |r| r.get(0),
    )?;
    Ok(id)
}

/// Reads one folder of this account.
pub fn get(conn: &Connection, account_id: i64, folder_id: i64) -> Result<Option<Folder>> {
    Ok(conn
        .query_row(
            r#"SELECT id, account_id, path, delimiter, special_use, uid_validity, uid_next,
                      highest_modseq, unread_count, total_count, last_synced_at
                 FROM folder WHERE account_id = ?1 AND id = ?2"#,
            params![account_id, folder_id],
            map_row,
        )
        .optional()?)
}

/// Reads the folder of this account with this path (paths are unique per account).
pub fn find_by_path(conn: &Connection, account_id: i64, path: &str) -> Result<Option<Folder>> {
    Ok(conn
        .query_row(
            r#"SELECT id, account_id, path, delimiter, special_use, uid_validity, uid_next,
                      highest_modseq, unread_count, total_count, last_synced_at
                 FROM folder WHERE account_id = ?1 AND path = ?2"#,
            params![account_id, path],
            map_row,
        )
        .optional()?)
}

/// Every folder of this account, ordered by path.
pub fn list(conn: &Connection, account_id: i64) -> Result<Vec<Folder>> {
    let mut stmt = conn.prepare_cached(
        r#"SELECT id, account_id, path, delimiter, special_use, uid_validity, uid_next,
                  highest_modseq, unread_count, total_count, last_synced_at
             FROM folder WHERE account_id = ?1 ORDER BY path"#,
    )?;
    let rows = stmt.query_map([account_id], map_row)?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

/// Records the folder's server state after a sync. Returns false when the folder is not in this
/// account.
pub fn update_status(
    conn: &Connection,
    account_id: i64,
    folder_id: i64,
    status: &FolderStatus,
    synced_at_ms: i64,
) -> Result<bool> {
    let modseq = status
        .highest_modseq
        .map(i64::try_from)
        .transpose()
        .map_err(|_| StorageError::OutOfRange("highest_modseq"))?;
    let n = conn.execute(
        r#"UPDATE folder
              SET uid_validity = ?3, uid_next = ?4, highest_modseq = ?5, last_synced_at = ?6
            WHERE account_id = ?1 AND id = ?2"#,
        params![
            account_id,
            folder_id,
            status.uid_validity,
            status.uid_next,
            modseq,
            synced_at_ms
        ],
    )?;
    Ok(n > 0)
}

/// Stores the folder's UIDVALIDITY alone, and sets UIDNEXT and HIGHESTMODSEQ to NULL (the sync time
/// is left as it was). For the middle of a UIDVALIDITY reset: the folder is not synced yet, so no
/// later sync may take it for "unchanged" until [`update_status`] stores the full status at the
/// end of a completed sync. Returns false when the folder is not in this account.
pub fn set_validity_only(
    conn: &Connection,
    account_id: i64,
    folder_id: i64,
    uid_validity: u32,
) -> Result<bool> {
    let n = conn.execute(
        r#"UPDATE folder
              SET uid_validity = ?3, uid_next = NULL, highest_modseq = NULL
            WHERE account_id = ?1 AND id = ?2"#,
        params![account_id, folder_id, uid_validity],
    )?;
    Ok(n > 0)
}

/// Deletes a folder and, by cascade, its messages. Returns false when the folder is not in this
/// account.
pub fn delete(conn: &Connection, account_id: i64, folder_id: i64) -> Result<bool> {
    let n = conn.execute(
        "DELETE FROM folder WHERE account_id = ?1 AND id = ?2",
        params![account_id, folder_id],
    )?;
    Ok(n > 0)
}
