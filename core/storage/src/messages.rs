// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Messages. Every function takes the `account_id` and puts `account_id = ?` in its WHERE
//! clause (and sets it on insert), even when it is also given a `folder_id` or message id, so one
//! account can never read or change another account's rows by guessing an id.

use croft_protocols::{FlagChange, RemoteHeader};
use rusqlite::{params, Connection, OptionalExtension, Row};

use crate::error::{Result, StorageError};

/// A stored message row (headers and state, no body).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    /// Row id.
    pub id: i64,
    /// Owning account.
    pub account_id: i64,
    /// Folder the message is in.
    pub folder_id: i64,
    /// IMAP UID in the folder.
    pub uid: i64,
    /// RFC 5322 Message-ID.
    pub message_id_hdr: Option<String>,
    /// Conversation key, see [`thread_key`].
    pub thread_key: String,
    /// Subject.
    pub subject: String,
    /// Sender display name.
    pub from_name: Option<String>,
    /// Sender address.
    pub from_addr: Option<String>,
    /// Date, Unix milliseconds.
    pub date_ts: i64,
    /// Size in bytes.
    pub size: i64,
    /// Preview text.
    pub snippet: String,
    /// `\Seen`.
    pub is_seen: bool,
    /// `\Flagged`.
    pub is_flagged: bool,
    /// `\Answered`.
    pub is_answered: bool,
    /// `\Draft`.
    pub is_draft: bool,
    /// Custom keywords, sorted.
    pub keywords: Vec<String>,
    /// Whether the message has attachments.
    pub has_attachments: bool,
    /// Inbox tab: `primary`, `updates`, `promotions` or `social`.
    pub category: String,
    /// Hidden from lists until this time (Unix milliseconds), if snoozed.
    pub snoozed_until: Option<i64>,
    /// Above 0 while a local change has not reached the server.
    pub pending_ops: i64,
    /// `none`, `partial` or `full`.
    pub body_state: String,
}

fn map_row(row: &Row<'_>) -> rusqlite::Result<Message> {
    let keywords: String = row.get(16)?;
    let keywords = serde_json::from_str(&keywords).map_err(|_| {
        rusqlite::Error::FromSqlConversionFailure(
            16,
            rusqlite::types::Type::Text,
            "keywords_json is not a JSON array of strings".into(),
        )
    })?;
    Ok(Message {
        id: row.get(0)?,
        account_id: row.get(1)?,
        folder_id: row.get(2)?,
        uid: row.get(3)?,
        message_id_hdr: row.get(4)?,
        thread_key: row.get(5)?,
        subject: row.get(6)?,
        from_name: row.get(7)?,
        from_addr: row.get(8)?,
        date_ts: row.get(9)?,
        size: row.get(10)?,
        snippet: row.get(11)?,
        is_seen: row.get(12)?,
        is_flagged: row.get(13)?,
        is_answered: row.get(14)?,
        is_draft: row.get(15)?,
        keywords,
        has_attachments: row.get(17)?,
        category: row.get(18)?,
        snoozed_until: row.get(19)?,
        pending_ops: row.get(20)?,
        body_state: row.get(21)?,
    })
}

/// The list query: newest first, snoozed rows hidden. It is a constant so a test can check that
/// its plan uses the `message_folder_date` index without a sort step.
pub(crate) const LIST_SQL: &str = r#"SELECT id, account_id, folder_id, uid, message_id_hdr, thread_key, subject,
       from_name, from_addr, date_ts, size, snippet, is_seen, is_flagged, is_answered, is_draft,
       keywords_json, has_attachments, category, snoozed_until, pending_ops, body_state
  FROM message
 WHERE account_id = ?1 AND folder_id = ?2
   AND (snoozed_until IS NULL OR snoozed_until <= ?3)
 ORDER BY date_ts DESC, id
 LIMIT ?4 OFFSET ?5"#;

/// Messages of one folder, newest first, leaving out messages snoozed past `now_ms`.
///
/// Returns at most `limit` rows after skipping `offset`. Rows with the same date keep a stable
/// order (by id), so paging does not repeat or skip them.
pub fn list_by_folder(
    conn: &Connection,
    account_id: i64,
    folder_id: i64,
    now_ms: i64,
    limit: u32,
    offset: u32,
) -> Result<Vec<Message>> {
    let mut stmt = conn.prepare_cached(LIST_SQL)?;
    let rows = stmt.query_map(
        params![account_id, folder_id, now_ms, limit, offset],
        map_row,
    )?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

/// Reads one message of this account.
pub fn get(conn: &Connection, account_id: i64, message_id: i64) -> Result<Option<Message>> {
    Ok(conn
        .query_row(
            r#"SELECT id, account_id, folder_id, uid, message_id_hdr, thread_key, subject,
                      from_name, from_addr, date_ts, size, snippet, is_seen, is_flagged, is_answered,
                      is_draft, keywords_json, has_attachments, category, snoozed_until, pending_ops,
                      body_state
                 FROM message WHERE account_id = ?1 AND id = ?2"#,
            params![account_id, message_id],
            map_row,
        )
        .optional()?)
}

/// Strips any number of leading `Re:`, `Fwd:` and `Fw:` prefixes (case-insensitive, with
/// surrounding spaces), trims, and lowercases.
fn normalise_subject(subject: &str) -> String {
    let mut s = subject.trim();
    loop {
        let lower_prefix = |p: &str| {
            s.get(..p.len())
                .is_some_and(|head| head.eq_ignore_ascii_case(p))
        };
        let cut = ["re:", "fwd:", "fw:"]
            .iter()
            .find(|p| lower_prefix(p))
            .map(|p| p.len());
        match cut {
            Some(n) => s = s[n..].trim_start(),
            None => break,
        }
    }
    s.trim().to_lowercase()
}

/// The conversation key stored in `message.thread_key` for a header.
///
/// In order of preference:
/// 1. the first entry of `references` (the root of the conversation), when it is not blank;
/// 2. the message's own `Message-ID`;
/// 3. the subject with leading `Re:`/`Fwd:`/`Fw:` removed (case-insensitive), trimmed and
///    lowercased.
///
/// The result is empty only when all three are blank; [`upsert`] then uses a key unique to the
/// message instead, so unrelated blank messages are not merged into one conversation.
pub fn thread_key(header: &RemoteHeader) -> String {
    if let Some(first) = header
        .references
        .iter()
        .map(|r| r.trim())
        .find(|r| !r.is_empty())
    {
        return first.to_owned();
    }
    if let Some(id) = header
        .message_id
        .as_deref()
        .map(str::trim)
        .filter(|i| !i.is_empty())
    {
        return id.to_owned();
    }
    normalise_subject(&header.subject)
}

/// Inserts the message by `(folder_id, uid)`, or updates the existing row's headers and flags.
/// Returns the row id.
///
/// If the existing row has `pending_ops > 0` a local change is in flight, so `is_seen` and
/// `is_flagged` keep their local values instead of taking the server's. Snooze, category, snippet,
/// body state, attachment flag and recipients are never touched by an upsert.
///
/// Fails with [`StorageError::NotFound`] when `folder_id` is not a folder of `account_id`.
pub fn upsert(
    conn: &Connection,
    account_id: i64,
    folder_id: i64,
    header: &RemoteHeader,
) -> Result<i64> {
    let mut key = thread_key(header);
    if key.is_empty() {
        key = format!("uid:{folder_id}:{}", header.uid);
    }
    let keywords = serde_json::to_string(&header.flags.keywords)?;
    let id = conn
        .query_row(
            r#"INSERT INTO message (account_id, folder_id, uid, message_id_hdr, thread_key, subject,
                                    from_name, from_addr, date_ts, size, is_seen, is_flagged,
                                    is_answered, is_draft, keywords_json)
               SELECT ?1, f.id, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15
                 FROM folder f
                WHERE f.account_id = ?1 AND f.id = ?2
               ON CONFLICT (folder_id, uid) DO UPDATE SET
                   message_id_hdr = excluded.message_id_hdr,
                   thread_key     = excluded.thread_key,
                   subject        = excluded.subject,
                   from_name      = excluded.from_name,
                   from_addr      = excluded.from_addr,
                   date_ts        = excluded.date_ts,
                   size           = excluded.size,
                   is_seen    = CASE WHEN message.pending_ops > 0 THEN message.is_seen
                                     ELSE excluded.is_seen END,
                   is_flagged = CASE WHEN message.pending_ops > 0 THEN message.is_flagged
                                     ELSE excluded.is_flagged END,
                   is_answered   = excluded.is_answered,
                   is_draft      = excluded.is_draft,
                   keywords_json = excluded.keywords_json
               WHERE message.account_id = excluded.account_id
               RETURNING id"#,
            params![
                account_id,
                folder_id,
                header.uid,
                header.message_id,
                key,
                header.subject,
                header.from_name,
                header.from_addr,
                header.date_ms,
                header.size,
                header.flags.seen,
                header.flags.flagged,
                header.flags.answered,
                header.flags.draft,
                keywords
            ],
            |r| r.get(0),
        )
        .optional()?;
    id.ok_or(StorageError::NotFound("folder"))
}

/// Applies one local flag change to a message of this account. Returns false when there is no
/// such message in this account. Does not touch `pending_ops`; the operation queue does that in
/// the same transaction.
pub fn update_flags(
    conn: &Connection,
    account_id: i64,
    message_id: i64,
    change: FlagChange,
) -> Result<bool> {
    let n = match change {
        FlagChange::Seen(v) => conn.execute(
            "UPDATE message SET is_seen = ?3 WHERE account_id = ?1 AND id = ?2",
            params![account_id, message_id, v],
        )?,
        FlagChange::Flagged(v) => conn.execute(
            "UPDATE message SET is_flagged = ?3 WHERE account_id = ?1 AND id = ?2",
            params![account_id, message_id, v],
        )?,
    };
    Ok(n > 0)
}

/// Deletes a message of this account. Returns false when there is no such message in this account.
pub fn delete(conn: &Connection, account_id: i64, message_id: i64) -> Result<bool> {
    let n = conn.execute(
        "DELETE FROM message WHERE account_id = ?1 AND id = ?2",
        params![account_id, message_id],
    )?;
    Ok(n > 0)
}
