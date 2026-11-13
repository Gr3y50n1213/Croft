// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Messages. Every function takes the `account_id` and puts `account_id = ?` in its WHERE
//! clause (and sets it on insert), even when it is also given a `folder_id` or message id, so one
//! account can never read or change another account's rows by guessing an id.

use std::collections::{BTreeMap, BTreeSet};

use croft_mail_core::category::{classify, Category, SenderRule};
use croft_protocols::{FlagChange, Flags, Keyword, RemoteHeader, Uid};
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
    /// True while a move or delete is waiting for the server: the row is kept but left out of
    /// lists.
    pub local_hidden: bool,
}

/// Reads a [`Message`] from the first 23 columns of a row, in the order of [`LIST_SQL`].
pub(crate) fn map_row(row: &Row<'_>) -> rusqlite::Result<Message> {
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
        local_hidden: row.get(22)?,
    })
}

/// The list query: newest first, leaving out snoozed rows and rows hidden by a waiting move or
/// delete. It is a constant so a test can check that its plan uses the `message_folder_date` index without a sort step.
pub(crate) const LIST_SQL: &str = r#"SELECT id, account_id, folder_id, uid, message_id_hdr, thread_key, subject,
       from_name, from_addr, date_ts, size, snippet, is_seen, is_flagged, is_answered, is_draft,
       keywords_json, has_attachments, category, snoozed_until, pending_ops, body_state, local_hidden
  FROM message
 WHERE account_id = ?1 AND folder_id = ?2 AND local_hidden = 0
   AND (snoozed_until IS NULL OR snoozed_until <= ?3)
 ORDER BY date_ts DESC, id
 LIMIT ?4 OFFSET ?5"#;

/// Messages of one folder, newest first, leaving out messages snoozed past `now_ms` and messages
/// hidden while a move or delete waits for the server (`local_hidden`).
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
                      body_state, local_hidden
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
/// `is_flagged` keep their local values instead of taking the server's. Snooze, category (decided
/// at the first store, T35), snippet,
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
    // T35: the category is decided once, when the message is first stored; a later sync keeps it
    // (the ON CONFLICT branch leaves it alone), so the person's corrections stay.
    let category = classify(
        header.from_addr.as_deref(),
        &header.list,
        &category_rules(conn, account_id)?,
    );
    let id = conn
        .query_row(
            r#"INSERT INTO message (account_id, folder_id, uid, message_id_hdr, thread_key, subject,
                                    from_name, from_addr, date_ts, size, is_seen, is_flagged,
                                    is_answered, is_draft, keywords_json, category, list_id,
                                    list_unsubscribe, list_one_click, is_pinned, follows_up)
               SELECT ?1, f.id, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17,
                      ?18, ?19, ?20, ?21
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
                   -- T36b: like the flags, a queued keyword change is not undone by a sync.
                   keywords_json = CASE WHEN message.pending_ops > 0 THEN message.keywords_json
                                        ELSE excluded.keywords_json END,
                   is_pinned  = CASE WHEN message.pending_ops > 0 THEN message.is_pinned
                                     ELSE excluded.is_pinned END,
                   follows_up = CASE WHEN message.pending_ops > 0 THEN message.follows_up
                                     ELSE excluded.follows_up END,
                   list_id          = excluded.list_id,
                   list_unsubscribe = excluded.list_unsubscribe,
                   list_one_click   = excluded.list_one_click
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
                keywords,
                category.as_str(),
                header.list.list_id,
                header.list.unsubscribe,
                header.list.one_click,
                header.flags.has(Keyword::Pinned),
                header.flags.has(Keyword::FollowUp)
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
        FlagChange::Keyword(k, v) => {
            let Some(stored) = conn
                .query_row(
                    "SELECT keywords_json FROM message WHERE account_id = ?1 AND id = ?2",
                    params![account_id, message_id],
                    |r| r.get::<_, String>(0),
                )
                .optional()?
            else {
                return Ok(false);
            };
            let mut flags = Flags {
                keywords: serde_json::from_str(&stored)?,
                ..Flags::default()
            };
            flags.apply(change);
            let keywords = serde_json::to_string(&flags.keywords)?;
            let column = match k {
                Keyword::Pinned => {
                    "UPDATE message SET keywords_json = ?3, is_pinned = ?4 WHERE account_id = ?1 AND id = ?2"
                }
                Keyword::FollowUp => {
                    "UPDATE message SET keywords_json = ?3, follows_up = ?4 WHERE account_id = ?1 AND id = ?2"
                }
            };
            conn.execute(column, params![account_id, message_id, keywords, v])?
        }
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

/// The message of this account at `uid` in `folder_id`, hidden or not.
pub fn find_by_uid(
    conn: &Connection,
    account_id: i64,
    folder_id: i64,
    uid: Uid,
) -> Result<Option<Message>> {
    Ok(conn
        .query_row(
            r#"SELECT id, account_id, folder_id, uid, message_id_hdr, thread_key, subject,
                      from_name, from_addr, date_ts, size, snippet, is_seen, is_flagged, is_answered,
                      is_draft, keywords_json, has_attachments, category, snoozed_until, pending_ops,
                      body_state, local_hidden
                 FROM message WHERE account_id = ?1 AND folder_id = ?2 AND uid = ?3"#,
            params![account_id, folder_id, uid],
            map_row,
        )
        .optional()?)
}

/// The flags of every message row in a folder of this account (hidden rows included), by UID: the
/// local side of a sync plan. `deleted` is always false (it is not stored).
pub fn folder_flags(
    conn: &Connection,
    account_id: i64,
    folder_id: i64,
) -> Result<BTreeMap<Uid, Flags>> {
    let mut stmt = conn.prepare_cached(
        r#"SELECT uid, is_seen, is_flagged, is_answered, is_draft, keywords_json
             FROM message WHERE account_id = ?1 AND folder_id = ?2"#,
    )?;
    let rows = stmt.query_map(params![account_id, folder_id], |r| {
        let keywords: String = r.get(5)?;
        let keywords: BTreeSet<String> = serde_json::from_str(&keywords).map_err(|_| {
            rusqlite::Error::FromSqlConversionFailure(
                5,
                rusqlite::types::Type::Text,
                "keywords_json is not a JSON array of strings".into(),
            )
        })?;
        Ok((
            r.get::<_, i64>(0)?,
            Flags {
                seen: r.get(1)?,
                flagged: r.get(2)?,
                answered: r.get(3)?,
                draft: r.get(4)?,
                deleted: false,
                keywords,
            },
        ))
    })?;
    let mut out = BTreeMap::new();
    for row in rows {
        let (uid, flags) = row?;
        let uid = Uid::try_from(uid).map_err(|_| StorageError::OutOfRange("uid"))?;
        out.insert(uid, flags);
    }
    Ok(out)
}

/// Takes the server's flags for the message at `uid` in a folder of this account. As in
/// [`upsert`], `is_seen` and `is_flagged` keep their local values while `pending_ops > 0`. Returns
/// false when there is no such row.
pub fn set_server_flags(
    conn: &Connection,
    account_id: i64,
    folder_id: i64,
    uid: Uid,
    flags: &Flags,
) -> Result<bool> {
    let keywords = serde_json::to_string(&flags.keywords)?;
    let n = conn.execute(
        r#"UPDATE message
              SET is_seen    = CASE WHEN pending_ops > 0 THEN is_seen ELSE ?4 END,
                  is_flagged = CASE WHEN pending_ops > 0 THEN is_flagged ELSE ?5 END,
                  is_answered = ?6, is_draft = ?7, keywords_json = ?8
            WHERE account_id = ?1 AND folder_id = ?2 AND uid = ?3"#,
        params![
            account_id,
            folder_id,
            uid,
            flags.seen,
            flags.flagged,
            flags.answered,
            flags.draft,
            keywords
        ],
    )?;
    Ok(n > 0)
}

/// Deletes the rows at these UIDs in a folder of this account (the server no longer has them).
/// Returns how many rows went.
pub fn delete_uids(
    conn: &Connection,
    account_id: i64,
    folder_id: i64,
    uids: &[Uid],
) -> Result<usize> {
    let mut stmt = conn.prepare_cached(
        "DELETE FROM message WHERE account_id = ?1 AND folder_id = ?2 AND uid = ?3",
    )?;
    let mut n = 0;
    for uid in uids {
        n += stmt.execute(params![account_id, folder_id, uid])?;
    }
    Ok(n)
}

/// How many message rows a folder of this account has, hidden ones included.
pub fn count_in_folder(conn: &Connection, account_id: i64, folder_id: i64) -> Result<u64> {
    let n: i64 = conn.query_row(
        "SELECT COUNT(*) FROM message WHERE account_id = ?1 AND folder_id = ?2",
        params![account_id, folder_id],
        |r| r.get(0),
    )?;
    Ok(u64::try_from(n).unwrap_or(0))
}

/// The UIDs of every row of a folder of this account (hidden ones included), ascending. Lighter
/// than [`folder_flags`] for the engine, which only needs to cut the rows into chunks.
pub fn uids_in_folder(conn: &Connection, account_id: i64, folder_id: i64) -> Result<Vec<Uid>> {
    let mut stmt = conn.prepare_cached(
        "SELECT uid FROM message WHERE account_id = ?1 AND folder_id = ?2 ORDER BY uid",
    )?;
    let rows = stmt.query_map(params![account_id, folder_id], |r| r.get::<_, i64>(0))?;
    let mut out = Vec::new();
    for row in rows {
        out.push(Uid::try_from(row?).map_err(|_| StorageError::OutOfRange("uid"))?);
    }
    Ok(out)
}

/// How many rows of a folder of this account have this `Message-ID` header, not counting the row
/// at `except_uid`. The engine uses it to tell whether a Message-ID names more than one copy.
pub fn count_other_copies(
    conn: &Connection,
    account_id: i64,
    folder_id: i64,
    message_id_hdr: &str,
    except_uid: Uid,
) -> Result<u64> {
    let n: i64 = conn.query_row(
        r#"SELECT COUNT(*) FROM message
            WHERE account_id = ?1 AND folder_id = ?2 AND message_id_hdr = ?3 AND uid <> ?4"#,
        params![account_id, folder_id, message_id_hdr, except_uid],
        |r| r.get(0),
    )?;
    Ok(u64::try_from(n).unwrap_or(0))
}

/// The UIDs of the rows of a folder of this account whose `Message-ID` header is one of
/// `message_ids`. The engine uses it to keep every copy of a message that has a destructive
/// operation waiting while a reset deletes the folder's other rows.
pub fn uids_with_message_ids(
    conn: &Connection,
    account_id: i64,
    folder_id: i64,
    message_ids: &[&str],
) -> Result<BTreeSet<Uid>> {
    let mut stmt = conn.prepare_cached(
        "SELECT uid FROM message WHERE account_id = ?1 AND folder_id = ?2 AND message_id_hdr = ?3",
    )?;
    let mut out = BTreeSet::new();
    for id in message_ids {
        let rows = stmt.query_map(params![account_id, folder_id, id], |r| r.get::<_, i64>(0))?;
        for row in rows {
            out.insert(Uid::try_from(row?).map_err(|_| StorageError::OutOfRange("uid"))?);
        }
    }
    Ok(out)
}

/// Deletes every row of a folder of this account, keeping the folder (its UIDVALIDITY changed, so
/// the cached UIDs mean nothing). Returns how many rows went.
pub fn delete_all_in_folder(conn: &Connection, account_id: i64, folder_id: i64) -> Result<usize> {
    Ok(conn.execute(
        "DELETE FROM message WHERE account_id = ?1 AND folder_id = ?2",
        params![account_id, folder_id],
    )?)
}

/// Clears `local_hidden` and `pending_ops` on every row of a folder of this account whose UID is
/// not in `keep`. The sync engine passes the UIDs that still have an unfinished operation (or a
/// move waiting for its new UID): any other hidden row or pending count is left over from an
/// operation that is gone, and would otherwise stay stuck. Returns how many rows changed.
pub fn release_local_state(
    conn: &Connection,
    account_id: i64,
    folder_id: i64,
    keep: &BTreeSet<Uid>,
) -> Result<usize> {
    let mut stmt = conn.prepare_cached(
        r#"SELECT uid FROM message
            WHERE account_id = ?1 AND folder_id = ?2 AND (local_hidden <> 0 OR pending_ops <> 0)"#,
    )?;
    let stuck: Vec<i64> = stmt
        .query_map(params![account_id, folder_id], |r| r.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    let mut update = conn.prepare_cached(
        r#"UPDATE message SET local_hidden = 0, pending_ops = 0
            WHERE account_id = ?1 AND folder_id = ?2 AND uid = ?3"#,
    )?;
    let mut n = 0;
    for uid in stuck {
        let keep_it = Uid::try_from(uid).is_ok_and(|u| keep.contains(&u));
        if !keep_it {
            n += update.execute(params![account_id, folder_id, uid])?;
        }
    }
    Ok(n)
}

// ---- T35: inbox categories ----

/// The account's "Always do this" rules.
pub fn category_rules(conn: &Connection, account_id: i64) -> Result<Vec<SenderRule>> {
    let mut stmt =
        conn.prepare_cached("SELECT sender, category FROM category_rule WHERE account_id = ?1")?;
    let rows = stmt.query_map(params![account_id], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
    })?;
    let mut out = Vec::new();
    for row in rows {
        let (sender, category) = row?;
        if let Some(category) = Category::parse(&category) {
            out.push(SenderRule { sender, category });
        }
    }
    Ok(out)
}

/// Sets one message's category (Move to Primary, ...). False when there is no such message in
/// this account.
pub fn set_category(
    conn: &Connection,
    account_id: i64,
    message_id: i64,
    category: Category,
) -> Result<bool> {
    let n = conn.execute(
        "UPDATE message SET category = ?3 WHERE account_id = ?1 AND id = ?2",
        params![account_id, message_id, category.as_str()],
    )?;
    Ok(n > 0)
}

/// "Always do this": mail from `address` (lower case) goes to `category` from now on, and the
/// account's mail already stored from that address is re-filed. The number of messages re-filed.
pub fn add_category_rule(
    conn: &Connection,
    account_id: i64,
    address: &str,
    category: Category,
) -> Result<usize> {
    conn.execute(
        "INSERT INTO category_rule (account_id, sender, category) VALUES (?1, ?2, ?3)
         ON CONFLICT (account_id, sender) DO UPDATE SET category = excluded.category",
        params![account_id, address, category.as_str()],
    )?;
    Ok(conn.execute(
        "UPDATE message SET category = ?3 WHERE account_id = ?1 AND lower(from_addr) = ?2",
        params![account_id, address, category.as_str()],
    )?)
}

/// Unread messages per category in one folder of this account (hidden and snoozed ones left out,
/// as in the list), for S10's chips. Every category is present, with 0 when it has none.
pub fn unread_by_category(
    conn: &Connection,
    account_id: i64,
    folder_id: i64,
    now_ms: i64,
) -> Result<Vec<(Category, u64)>> {
    let mut stmt = conn.prepare_cached(
        r#"SELECT category, COUNT(*) FROM message
            WHERE account_id = ?1 AND folder_id = ?2 AND is_seen = 0 AND local_hidden = 0
              AND (snoozed_until IS NULL OR snoozed_until <= ?3)
            GROUP BY category"#,
    )?;
    let rows = stmt.query_map(params![account_id, folder_id, now_ms], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
    })?;
    let mut counts: Vec<(Category, u64)> = Category::ALL.iter().map(|c| (*c, 0)).collect();
    for row in rows {
        let (name, n) = row?;
        if let Some(c) = Category::parse(&name) {
            if let Some(slot) = counts.iter_mut().find(|(k, _)| *k == c) {
                slot.1 = u64::try_from(n).unwrap_or(0);
            }
        }
    }
    Ok(counts)
}

// ---- T36: snooze on this device ----

/// The longest a message can be snoozed: a year and a day.
pub const MAX_SNOOZE_MS: i64 = 366 * 24 * 60 * 60 * 1000;

/// Snoozes a message of this account until `until_ms` (`None` unsnoozes it). Snoozing clears an
/// earlier back-from-snooze mark. False when there is no such message in this account.
pub fn set_snooze(
    conn: &Connection,
    account_id: i64,
    message_id: i64,
    until_ms: Option<i64>,
) -> Result<bool> {
    let n = conn.execute(
        "UPDATE message SET snoozed_until = ?3, snooze_back_at = NULL WHERE account_id = ?1 AND id = ?2",
        params![account_id, message_id, until_ms],
    )?;
    Ok(n > 0)
}

/// The account's messages whose wake time has come (`snoozed_until <= now_ms`).
pub fn due_snoozes(conn: &Connection, account_id: i64, now_ms: i64) -> Result<Vec<i64>> {
    let mut stmt = conn.prepare_cached(
        r#"SELECT id FROM message
            WHERE account_id = ?1 AND snoozed_until IS NOT NULL AND snoozed_until <= ?2
            ORDER BY snoozed_until, id"#,
    )?;
    let rows = stmt.query_map(params![account_id, now_ms], |r| r.get(0))?;
    Ok(rows.collect::<rusqlite::Result<Vec<i64>>>()?)
}

/// Wakes one message: no longer snoozed, marked back from snooze at `now_ms`. False when there is
/// no such snoozed message in this account (woken already).
pub fn wake(conn: &Connection, account_id: i64, message_id: i64, now_ms: i64) -> Result<bool> {
    let n = conn.execute(
        r#"UPDATE message SET snoozed_until = NULL, snooze_back_at = ?3
            WHERE account_id = ?1 AND id = ?2 AND snoozed_until IS NOT NULL"#,
        params![account_id, message_id, now_ms],
    )?;
    Ok(n > 0)
}

/// The account's soonest wake time, if anything is snoozed.
pub fn next_wake(conn: &Connection, account_id: i64) -> Result<Option<i64>> {
    Ok(conn.query_row(
        "SELECT MIN(snoozed_until) FROM message WHERE account_id = ?1 AND snoozed_until IS NOT NULL",
        params![account_id],
        |r| r.get(0),
    )?)
}
