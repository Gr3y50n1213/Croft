// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The message list query: keyset paging by (date, id), newest first.
//!
//! Offset paging would skip or repeat rows when new mail arrives between two pages. Here a page
//! starts "just after the row with this date and id", so the position is stable.
//!
//! Hidden rows: snoozed rows (until their time) and rows hidden while a move or delete waits for
//! the server (`local_hidden`) are left out, as in `croft_storage::messages::list_by_folder`.
//! Every statement filters on `account_id`; the unified Inbox first lists the `inbox` folders of
//! each account and then runs the same scoped statement per folder, merging in memory, so each
//! folder is read through the `message_folder_date` index and no sort over the whole mailbox is
//! needed.

use croft_storage::folders;
use rusqlite::{params, Connection, Row};

use crate::error::CoreError;
use crate::types::{MessagePage, MessageRow, MessageScope, RowCursor};

/// The most rows one page may hold.
pub(crate) const MAX_PAGE: u32 = 500;

const COLUMNS: &str =
    "id, account_id, folder_id, from_name, from_addr, subject, snippet, date_ts, \
     is_seen, is_flagged, has_attachments, pending_ops";

fn row(r: &Row<'_>) -> rusqlite::Result<MessageRow> {
    Ok(MessageRow {
        id: r.get(0)?,
        account: r.get(1)?,
        folder: r.get(2)?,
        sender_name: r.get(3)?,
        sender_address: r.get(4)?,
        subject: r.get(5)?,
        snippet: r.get(6)?,
        date_ms: r.get(7)?,
        seen: r.get(8)?,
        flagged: r.get(9)?,
        has_attachments: r.get(10)?,
        pending: r.get::<_, i64>(11)? > 0,
    })
}

/// Up to `limit` rows of one folder after the cursor, newest first.
fn folder_rows(
    conn: &Connection,
    account: i64,
    folder_id: i64,
    now_ms: i64,
    before: Option<RowCursor>,
    limit: u32,
) -> Result<Vec<MessageRow>, CoreError> {
    let rows = match before {
        None => {
            let sql = format!(
                "SELECT {COLUMNS} FROM message
                  WHERE account_id = ?1 AND folder_id = ?2 AND local_hidden = 0
                    AND (snoozed_until IS NULL OR snoozed_until <= ?3)
                  ORDER BY date_ts DESC, id DESC LIMIT ?4"
            );
            let mut stmt = conn.prepare_cached(&sql)?;
            let rows = stmt.query_map(params![account, folder_id, now_ms, limit], row)?;
            rows.collect::<rusqlite::Result<Vec<_>>>()?
        }
        Some(c) => {
            let sql = format!(
                "SELECT {COLUMNS} FROM message
                  WHERE account_id = ?1 AND folder_id = ?2 AND local_hidden = 0
                    AND (snoozed_until IS NULL OR snoozed_until <= ?3)
                    AND (date_ts < ?4 OR (date_ts = ?4 AND id < ?5))
                  ORDER BY date_ts DESC, id DESC LIMIT ?6"
            );
            let mut stmt = conn.prepare_cached(&sql)?;
            let rows = stmt.query_map(
                params![account, folder_id, now_ms, c.date_ms, c.id, limit],
                row,
            )?;
            rows.collect::<rusqlite::Result<Vec<_>>>()?
        }
    };
    Ok(rows)
}

/// One page of the scope. `accounts` is every account id (for the unified Inbox).
pub(crate) fn page(
    conn: &Connection,
    accounts: &[i64],
    scope: MessageScope,
    now_ms: i64,
    before: Option<RowCursor>,
    limit: u32,
) -> Result<MessagePage, CoreError> {
    let limit = limit.min(MAX_PAGE);
    if limit == 0 {
        return Ok(MessagePage {
            rows: vec![],
            next: None,
        });
    }
    // One extra row tells whether another page exists.
    let want = limit + 1;
    let mut rows = match scope {
        MessageScope::Folder { account, folder_id } => {
            folder_rows(conn, account, folder_id, now_ms, before, want)?
        }
        MessageScope::UnifiedInbox => {
            let mut all = Vec::new();
            for &account in accounts {
                for f in folders::list(conn, account)? {
                    if f.special_use == "inbox" {
                        all.extend(folder_rows(conn, account, f.id, now_ms, before, want)?);
                    }
                }
            }
            all.sort_by_key(|r| std::cmp::Reverse((r.date_ms, r.id)));
            all.truncate(want as usize);
            all
        }
    };
    let more = rows.len() > limit as usize;
    rows.truncate(limit as usize);
    let next = if more {
        rows.last().map(|r| RowCursor {
            date_ms: r.date_ms,
            id: r.id,
        })
    } else {
        None
    };
    Ok(MessagePage { rows, next })
}
