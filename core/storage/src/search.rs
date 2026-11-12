// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Local search (S27, S28) over the `message_fts` index and the message table.
//!
//! Every statement filters on `account_id` (the accounts in the scope); the index is joined on
//! `message.id`. Text terms go through [`SearchQuery::fts`], which only ever writes quoted
//! strings, so a query never raises an FTS syntax error. Ranking: FTS5's bm25 with the subject
//! weighted above the sender and the body, then the newest first, then the row id, so the order is
//! stable. Trash and Junk are left out unless the query names a folder (`in:`); rows hidden while
//! a move or delete waits for the server are always left out.
//!
//! Matched words come back between [`MARK_START`] and [`MARK_END`] (FTS5 `highlight` and
//! `snippet`). The body column is only filled once a body is stored, so until then the body is not
//! searched (the screen says so).

use croft_mail_core::search::SearchQuery;
use rusqlite::types::Value;
use rusqlite::{params_from_iter, Connection};

use crate::error::Result;
use crate::folders;

/// Put before a matched word in [`LocalHit::subject_marked`] and [`LocalHit::snippet_marked`].
pub const MARK_START: char = '\u{2}';
/// Put after a matched word.
pub const MARK_END: char = '\u{3}';

/// The most hits one search returns.
pub const MAX_HITS: u32 = 200;

/// One local result.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LocalHit {
    /// The message's row id.
    pub message_id: i64,
    /// The subject with matches marked (`None` when only filters ran).
    pub subject_marked: Option<String>,
    /// A short piece of the stored body around the matches, marked (`None` when the body did
    /// not match or is not stored).
    pub snippet_marked: Option<String>,
}

/// The folder ids an `in:` value names in one account: a role (`inbox`, `sent`, `drafts`,
/// `archive`, `trash`, `junk`/`spam`, `all`) or a folder's path or last path part, ignoring case.
pub fn named_folders(conn: &Connection, account: i64, name: &str) -> Result<Vec<i64>> {
    let want = name.to_lowercase();
    let role = match want.as_str() {
        "spam" => "junk",
        other => other,
    };
    Ok(folders::list(conn, account)?
        .into_iter()
        .filter(|f| {
            let path = f.path.to_lowercase();
            let last = match f.delimiter.as_deref().filter(|d| !d.is_empty()) {
                Some(d) => path
                    .rsplit(&d.to_lowercase())
                    .next()
                    .unwrap_or(&path)
                    .to_owned(),
                None => path.clone(),
            };
            f.special_use == role || path == want || last == want
        })
        .map(|f| f.id)
        .collect())
}

/// `%` and `_` written literally for `LIKE ... ESCAPE '\'`.
fn like_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('%');
    for c in s.chars() {
        if matches!(c, '%' | '_' | '\\') {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('%');
    out
}

/// Searches the stored mail of `accounts` (a scope of one account, or all). Up to `limit` hits
/// (at most [`MAX_HITS`]), best first. An empty query, or an `in:` naming no folder, finds nothing.
pub fn search(
    conn: &Connection,
    accounts: &[i64],
    q: &SearchQuery,
    limit: u32,
) -> Result<Vec<LocalHit>> {
    let limit = limit.min(MAX_HITS);
    if q.is_empty() || accounts.is_empty() || limit == 0 {
        return Ok(Vec::new());
    }
    let mut args: Vec<Value> = Vec::new();
    let mut wheres: Vec<String> = Vec::new();

    let placeholders = |args: &mut Vec<Value>, vals: Vec<i64>| -> String {
        let mut p = Vec::with_capacity(vals.len());
        for v in vals {
            args.push(Value::Integer(v));
            p.push(format!("?{}", args.len()));
        }
        p.join(", ")
    };

    let fts = q.fts();
    if let Some(expr) = &fts {
        args.push(Value::Text(expr.clone()));
        wheres.push(format!("message_fts MATCH ?{}", args.len()));
    }
    let scope = placeholders(&mut args, accounts.to_vec());
    wheres.push(format!("m.account_id IN ({scope})"));
    wheres.push("m.local_hidden = 0".into());

    match &q.folder {
        Some(name) => {
            let mut ids = Vec::new();
            for &a in accounts {
                ids.extend(named_folders(conn, a, name)?);
            }
            if ids.is_empty() {
                return Ok(Vec::new());
            }
            let list = placeholders(&mut args, ids);
            wheres.push(format!("m.folder_id IN ({list})"));
        }
        None => wheres.push(
            "m.folder_id NOT IN (SELECT id FROM folder f WHERE f.account_id = m.account_id \
             AND f.special_use IN ('trash', 'junk'))"
                .into(),
        ),
    }
    if q.unread {
        wheres.push("m.is_seen = 0".into());
    }
    if q.starred {
        wheres.push("m.is_flagged = 1".into());
    }
    if q.has_attachment {
        wheres.push("m.has_attachments = 1".into());
    }
    if let Some(b) = q.before_ms {
        args.push(Value::Integer(b));
        wheres.push(format!("m.date_ts < ?{}", args.len()));
    }
    if let Some(a) = q.after_ms {
        args.push(Value::Integer(a));
        wheres.push(format!("m.date_ts >= ?{}", args.len()));
    }
    for to in &q.to {
        args.push(Value::Text(like_escape(&to.to_lowercase())));
        let n = args.len();
        wheres.push(format!(
            "(lower(m.to_json) LIKE ?{n} ESCAPE '\\' OR lower(m.cc_json) LIKE ?{n} ESCAPE '\\')"
        ));
    }
    args.push(Value::Integer(i64::from(limit)));
    let lim = args.len();
    let wh = wheres.join(" AND ");

    let sql = if fts.is_some() {
        format!(
            "SELECT m.id,
                    highlight(message_fts, 0, char(2), char(3)),
                    CASE WHEN length(message_fts.body) > 0
                         THEN snippet(message_fts, 2, char(2), char(3), '…', 12) END
               FROM message_fts JOIN message m ON m.id = message_fts.rowid
              WHERE {wh}
              ORDER BY bm25(message_fts, 10.0, 5.0, 1.0), m.date_ts DESC, m.id DESC
              LIMIT ?{lim}"
        )
    } else {
        format!(
            "SELECT m.id, NULL, NULL FROM message m
              WHERE {wh}
              ORDER BY m.date_ts DESC, m.id DESC
              LIMIT ?{lim}"
        )
    };
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params_from_iter(args.iter()), |r| {
        let snippet: Option<String> = r.get(2)?;
        Ok(LocalHit {
            message_id: r.get(0)?,
            subject_marked: r.get(1)?,
            // A body snippet with no mark matched nothing in the body: not worth showing.
            snippet_marked: snippet.filter(|s| s.contains(MARK_START)),
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

/// True when some message of these accounts has no stored body (its text was not searched).
pub fn bodies_missing(conn: &Connection, accounts: &[i64]) -> Result<bool> {
    let mut stmt = conn.prepare_cached(
        "SELECT EXISTS (SELECT 1 FROM message m
                         WHERE m.account_id = ?1
                           AND NOT EXISTS (SELECT 1 FROM message_body b WHERE b.message_id = m.id))",
    )?;
    for &a in accounts {
        if stmt.query_row([a], |r| r.get::<_, bool>(0))? {
            return Ok(true);
        }
    }
    Ok(false)
}
