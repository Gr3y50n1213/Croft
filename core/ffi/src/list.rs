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

/// The most recipients one row carries (a mailing to hundreds would only make the row heavy).
const MAX_RECIPIENTS: usize = 20;

const COLUMNS: &str =
    "id, account_id, folder_id, from_name, from_addr, subject, snippet, date_ts, \
     is_seen, is_flagged, has_attachments, pending_ops, to_json, \
     (SELECT COUNT(*) FROM message t
       WHERE t.account_id = message.account_id AND t.folder_id = message.folder_id
         AND t.thread_key = message.thread_key AND t.local_hidden = 0)";

/// Names (else addresses) from a stored `to_json`: an array whose items are strings or objects
/// with `name` and/or `address`. Anything else gives no recipients (a damaged value must not
/// fail the list).
fn recipients(json: &str) -> Vec<String> {
    let Ok(serde_json::Value::Array(items)) = serde_json::from_str::<serde_json::Value>(json)
    else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|item| {
            let text = match item {
                serde_json::Value::String(s) => Some(s.as_str()),
                serde_json::Value::Object(o) => ["name", "address"]
                    .iter()
                    .filter_map(|k| o.get(*k).and_then(serde_json::Value::as_str))
                    .find(|s| !s.trim().is_empty()),
                _ => None,
            }?;
            let text = text.trim();
            (!text.is_empty()).then(|| text.to_owned())
        })
        .take(MAX_RECIPIENTS)
        .collect()
}

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
        recipients: recipients(&r.get::<_, String>(12)?),
        thread_count: u32::try_from(r.get::<_, i64>(13)?.max(1)).unwrap_or(u32::MAX),
    })
}

/// The rows of these message ids, in the given order. Ids not in `accounts`, unknown ones and
/// rows hidden by a waiting move or delete are left out (search results use this).
pub(crate) fn rows_for(
    conn: &Connection,
    accounts: &[i64],
    ids: &[i64],
) -> Result<Vec<MessageRow>, CoreError> {
    let sql = format!("SELECT {COLUMNS} FROM message WHERE id = ?1 AND local_hidden = 0");
    let mut stmt = conn.prepare_cached(&sql)?;
    let mut out = Vec::with_capacity(ids.len());
    for &id in ids {
        let found = stmt.query_map(params![id], row)?.next().transpose()?;
        if let Some(r) = found.filter(|r| accounts.contains(&r.account)) {
            out.push(r);
        }
    }
    Ok(out)
}

/// Up to `limit` rows of one folder after the cursor, newest first; with `category`, only its
/// rows (T35).
fn folder_rows(
    conn: &Connection,
    account: i64,
    folder_id: i64,
    now_ms: i64,
    before: Option<RowCursor>,
    limit: u32,
    category: Option<&str>,
) -> Result<Vec<MessageRow>, CoreError> {
    let rows = match before {
        None => {
            let sql = format!(
                "SELECT {COLUMNS} FROM message
                  WHERE account_id = ?1 AND folder_id = ?2 AND local_hidden = 0
                    AND (snoozed_until IS NULL OR snoozed_until <= ?3)
                    AND (?5 IS NULL OR category = ?5)
                  ORDER BY date_ts DESC, id DESC LIMIT ?4"
            );
            let mut stmt = conn.prepare_cached(&sql)?;
            let rows = stmt.query_map(params![account, folder_id, now_ms, limit, category], row)?;
            rows.collect::<rusqlite::Result<Vec<_>>>()?
        }
        Some(c) => {
            let sql = format!(
                "SELECT {COLUMNS} FROM message
                  WHERE account_id = ?1 AND folder_id = ?2 AND local_hidden = 0
                    AND (snoozed_until IS NULL OR snoozed_until <= ?3)
                    AND (date_ts < ?4 OR (date_ts = ?4 AND id < ?5))
                    AND (?7 IS NULL OR category = ?7)
                  ORDER BY date_ts DESC, id DESC LIMIT ?6"
            );
            let mut stmt = conn.prepare_cached(&sql)?;
            let rows = stmt.query_map(
                params![account, folder_id, now_ms, c.date_ms, c.id, limit, category],
                row,
            )?;
            rows.collect::<rusqlite::Result<Vec<_>>>()?
        }
    };
    Ok(rows)
}

/// Up to `limit` flagged rows of one account after the cursor, newest first. Trash and Junk are
/// left out, like rows hidden by a waiting move or delete and snoozed rows.
fn starred_rows(
    conn: &Connection,
    account: i64,
    now_ms: i64,
    before: Option<RowCursor>,
    limit: u32,
) -> Result<Vec<MessageRow>, CoreError> {
    // `i64::MAX` as the cursor date and id means "from the start", so one statement serves both.
    let (date, id) = before.map_or((i64::MAX, i64::MAX), |c| (c.date_ms, c.id));
    let sql = format!(
        "SELECT {COLUMNS} FROM message
          WHERE account_id = ?1 AND is_flagged = 1 AND local_hidden = 0
            AND (snoozed_until IS NULL OR snoozed_until <= ?2)
            AND (date_ts < ?3 OR (date_ts = ?3 AND id < ?4))
            AND folder_id NOT IN (SELECT id FROM folder
                                   WHERE account_id = ?1 AND special_use IN ('trash', 'junk'))
          ORDER BY date_ts DESC, id DESC LIMIT ?5"
    );
    let mut stmt = conn.prepare_cached(&sql)?;
    let rows = stmt.query_map(params![account, now_ms, date, id, limit], row)?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

/// One page of the scope. `accounts` is every account id (for the unified scopes).
pub(crate) fn page(
    conn: &Connection,
    accounts: &[i64],
    scope: MessageScope,
    now_ms: i64,
    before: Option<RowCursor>,
    limit: u32,
    category: Option<&str>,
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
            folder_rows(conn, account, folder_id, now_ms, before, want, category)?
        }
        MessageScope::UnifiedInbox => {
            let mut all = Vec::new();
            for &account in accounts {
                for f in folders::list(conn, account)? {
                    if f.special_use == "inbox" {
                        all.extend(folder_rows(
                            conn, account, f.id, now_ms, before, want, category,
                        )?);
                    }
                }
            }
            all.sort_by_key(|r| std::cmp::Reverse((r.date_ms, r.id)));
            all.truncate(want as usize);
            all
        }
        MessageScope::Starred { account } => {
            let mut all = Vec::new();
            // A named account is read even if it is not in `accounts` (it then has no rows: the
            // statement filters on it), so an unknown id is an empty list, not an error.
            let wanted: Vec<i64> = match account {
                Some(a) => vec![a],
                None => accounts.to_vec(),
            };
            for a in wanted {
                all.extend(starred_rows(conn, a, now_ms, before, want)?);
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
