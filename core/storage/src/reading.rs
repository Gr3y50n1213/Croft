// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Reading messages: the stored body and attachments of a message, the fetched message itself,
//! the remote-content choices, and the messages of a conversation.
//!
//! Every function takes the `account_id` and checks it: `message_body` and `attachment` have no
//! `account_id` column of their own, so they are always reached through a `message` row of the
//! account (a join or a check first), and an id of another account answers like a missing one.

use std::collections::{BTreeSet, HashMap, HashSet};

use rusqlite::{params, Connection, OptionalExtension};

use crate::error::Result;
use crate::messages::{self, Message};

/// The most messages one conversation returns (the newest are kept).
pub const MAX_CONVERSATION: usize = 500;
/// How many rounds of "messages that answer a message already in the conversation" are followed
/// when the References headers were cut short (each round is one query per new key).
const CONVERSATION_ROUNDS: usize = 4;

/// What [`store_body`] writes for one fetched message. `Debug` is not derived: it holds content.
pub struct NewBody<'a> {
    /// The message as the server sent it.
    pub raw: &'a [u8],
    /// SHA-256 of `raw`.
    pub raw_sha256: &'a [u8],
    /// When it was fetched, Unix milliseconds.
    pub fetched_at: i64,
    /// The header block as text.
    pub headers_raw: &'a str,
    /// The plain text body (feeds search).
    pub text: Option<&'a str>,
    /// The sanitised HTML with remote content blocked.
    pub html_clean: Option<&'a str>,
    /// Remote images left out of `html_clean`.
    pub remote_blocked: u32,
    /// Tracking pixels removed.
    pub trackers_removed: u32,
    /// A body was cut at a limit.
    pub truncated: bool,
    /// The HTML is too complex to render (a deterministic outcome: never tried again).
    pub too_complex: bool,
    /// The HTML was not rendered this time (sanitiser busy, deadline missed): try again later.
    pub html_pending: bool,
    /// Recipients for `message.to_json` (a JSON array of `{name, address}`).
    pub to_json: &'a str,
    /// Copy recipients for `message.cc_json`.
    pub cc_json: &'a str,
    /// The list preview for `message.snippet`.
    pub snippet: &'a str,
    /// The attachments.
    pub attachments: &'a [NewAttachment],
}

/// One attachment for [`store_body`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewAttachment {
    /// The part's number in the stored message.
    pub part_id: String,
    /// A safe file name.
    pub file_name: String,
    /// MIME type.
    pub mime_type: String,
    /// Decoded size in bytes.
    pub size: u64,
    /// `Content-ID` without angle brackets.
    pub content_id: Option<String>,
    /// Drawn inside the HTML body.
    pub shown_inline: bool,
    /// A program or script.
    pub executable: bool,
}

/// A stored body. `Debug` prints sizes only.
#[derive(Clone, PartialEq, Eq)]
pub struct StoredBody {
    /// The message's row id.
    pub message_id: i64,
    /// The header block.
    pub headers_raw: Option<String>,
    /// Plain text.
    pub text: Option<String>,
    /// Sanitised HTML, remote content blocked.
    pub html_clean: Option<String>,
    /// Remote images left out of `html_clean`.
    pub remote_blocked: u32,
    /// Tracking pixels removed.
    pub trackers_removed: u32,
    /// A body was cut at a limit.
    pub truncated: bool,
    /// The HTML was not rendered and must not be tried again.
    pub too_complex: bool,
    /// The HTML was not rendered this time; the next open tries again ([`store_html`]).
    pub html_pending: bool,
}

impl std::fmt::Debug for StoredBody {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StoredBody")
            .field("message_id", &self.message_id)
            .field("text_bytes", &self.text.as_ref().map(String::len))
            .field("html_bytes", &self.html_clean.as_ref().map(String::len))
            .field("remote_blocked", &self.remote_blocked)
            .finish_non_exhaustive()
    }
}

/// A stored attachment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredAttachment {
    /// Row id (what the app names it by).
    pub id: i64,
    /// The message's row id.
    pub message_id: i64,
    /// The part's number in the stored message.
    pub part_id: String,
    /// A safe file name.
    pub file_name: String,
    /// MIME type.
    pub mime_type: String,
    /// Decoded size in bytes.
    pub size: u64,
    /// `Content-ID` without angle brackets.
    pub content_id: Option<String>,
    /// Drawn inside the HTML body.
    pub shown_inline: bool,
    /// A program or script.
    pub executable: bool,
}

/// One message of a conversation, with what the screen needs besides the list row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConversationRow {
    /// The message row.
    pub message: Message,
    /// The stored `to_json`.
    pub to_json: String,
    /// The stored `cc_json`.
    pub cc_json: String,
    /// The person allowed remote content on this message.
    pub remote_allowed: bool,
    /// The folder's `special_use` (`inbox`, `sent`, ...).
    pub folder_special_use: String,
    /// SHA-256 of the fetched message, when it was fetched.
    pub raw_sha256: Option<Vec<u8>>,
}

/// True when the message is a message of this account.
fn owned(conn: &Connection, account_id: i64, message_id: i64) -> Result<bool> {
    Ok(conn
        .query_row(
            "SELECT 1 FROM message WHERE account_id = ?1 AND id = ?2",
            params![account_id, message_id],
            |_| Ok(()),
        )
        .optional()?
        .is_some())
}

/// Stores a fetched message: the raw copy, the body, the attachments (replacing what was stored
/// before), and on the message row its recipients, preview, attachment flag and
/// `body_state = 'full'`. Returns false, writing nothing, when the message is not one of this
/// account (it may have been deleted meanwhile). Run it inside a transaction.
pub fn store_body(
    conn: &Connection,
    account_id: i64,
    message_id: i64,
    body: &NewBody<'_>,
) -> Result<bool> {
    if !owned(conn, account_id, message_id)? {
        return Ok(false);
    }
    conn.execute(
        "INSERT OR REPLACE INTO message_raw (message_id, account_id, raw, sha256, fetched_at)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            message_id,
            account_id,
            body.raw,
            body.raw_sha256,
            body.fetched_at
        ],
    )?;
    conn.execute(
        "DELETE FROM message_body WHERE message_id = ?1",
        params![message_id],
    )?;
    conn.execute(
        "INSERT INTO message_body (message_id, headers_raw, text_plain, html_clean, remote_blocked,
                                   trackers_removed, truncated, too_complex, html_pending)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            message_id,
            body.headers_raw,
            body.text,
            body.html_clean,
            body.remote_blocked,
            body.trackers_removed,
            body.truncated,
            body.too_complex,
            body.html_pending && !body.too_complex
        ],
    )?;
    conn.execute(
        "DELETE FROM attachment WHERE message_id = ?1",
        params![message_id],
    )?;
    let mut insert = conn.prepare_cached(
        "INSERT OR IGNORE INTO attachment (message_id, part_id, filename, mime_type, size,
                                           content_id, shown_inline, executable)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
    )?;
    for a in body.attachments {
        insert.execute(params![
            message_id,
            a.part_id,
            a.file_name,
            a.mime_type,
            i64::try_from(a.size).unwrap_or(i64::MAX),
            a.content_id,
            a.shown_inline,
            a.executable
        ])?;
    }
    let has_attachments = body.attachments.iter().any(|a| !a.shown_inline);
    conn.execute(
        "UPDATE message SET to_json = ?3, cc_json = ?4, snippet = ?5, has_attachments = ?6,
                            body_state = 'full'
          WHERE account_id = ?1 AND id = ?2",
        params![
            account_id,
            message_id,
            body.to_json,
            body.cc_json,
            body.snippet,
            has_attachments
        ],
    )?;
    Ok(true)
}

/// The stored body of a message of this account, if it has one.
pub fn body(conn: &Connection, account_id: i64, message_id: i64) -> Result<Option<StoredBody>> {
    Ok(conn
        .query_row(
            "SELECT b.message_id, b.headers_raw, b.text_plain, b.html_clean, b.remote_blocked,
                    b.trackers_removed, b.truncated, b.too_complex, b.html_pending
               FROM message_body b JOIN message m ON m.id = b.message_id
              WHERE m.account_id = ?1 AND m.id = ?2",
            params![account_id, message_id],
            |r| {
                Ok(StoredBody {
                    message_id: r.get(0)?,
                    headers_raw: r.get(1)?,
                    text: r.get(2)?,
                    html_clean: r.get(3)?,
                    remote_blocked: r.get(4)?,
                    trackers_removed: r.get(5)?,
                    truncated: r.get(6)?,
                    too_complex: r.get(7)?,
                    html_pending: r.get(8)?,
                })
            },
        )
        .optional()?)
}

/// Whether the message of this account has its fetched copy stored. Reads no content (the copy
/// can be 50 MiB, and callers ask this while holding the engine's lock).
pub fn has_raw(conn: &Connection, account_id: i64, message_id: i64) -> Result<bool> {
    Ok(conn
        .query_row(
            "SELECT 1 FROM message_raw r JOIN message m ON m.id = r.message_id
              WHERE r.account_id = ?1 AND m.account_id = ?1 AND m.id = ?2",
            params![account_id, message_id],
            |_| Ok(()),
        )
        .optional()?
        .is_some())
}

/// Whether the message of this account has a stored body. Reads no content.
pub fn has_body(conn: &Connection, account_id: i64, message_id: i64) -> Result<bool> {
    Ok(conn
        .query_row(
            "SELECT 1 FROM message_body b JOIN message m ON m.id = b.message_id
              WHERE m.account_id = ?1 AND m.id = ?2",
            params![account_id, message_id],
            |_| Ok(()),
        )
        .optional()?
        .is_some())
}

/// Whether the stored body of a message of this account waits for its HTML (stored with
/// `html_pending`). Reads no content.
pub fn html_pending(conn: &Connection, account_id: i64, message_id: i64) -> Result<bool> {
    Ok(conn
        .query_row(
            "SELECT b.html_pending FROM message_body b JOIN message m ON m.id = b.message_id
              WHERE m.account_id = ?1 AND m.id = ?2",
            params![account_id, message_id],
            |r| r.get::<_, bool>(0),
        )
        .optional()?
        .unwrap_or(false))
}

/// The fetched message of a message of this account, if it was fetched.
pub fn raw(conn: &Connection, account_id: i64, message_id: i64) -> Result<Option<Vec<u8>>> {
    Ok(conn
        .query_row(
            "SELECT r.raw FROM message_raw r JOIN message m ON m.id = r.message_id
              WHERE r.account_id = ?1 AND m.account_id = ?1 AND m.id = ?2",
            params![account_id, message_id],
            |r| r.get(0),
        )
        .optional()?)
}

const ATTACHMENT_COLUMNS: &str = "a.id, a.message_id, a.part_id, a.filename, a.mime_type, a.size, \
                                  a.content_id, a.shown_inline, a.executable";

fn attachment_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<StoredAttachment> {
    Ok(StoredAttachment {
        id: r.get(0)?,
        message_id: r.get(1)?,
        part_id: r.get(2)?,
        file_name: r.get::<_, Option<String>>(3)?.unwrap_or_default(),
        mime_type: r.get(4)?,
        size: u64::try_from(r.get::<_, i64>(5)?).unwrap_or(0),
        content_id: r.get(6)?,
        shown_inline: r.get(7)?,
        executable: r.get(8)?,
    })
}

/// The attachments of a message of this account, in document order.
pub fn attachments(
    conn: &Connection,
    account_id: i64,
    message_id: i64,
) -> Result<Vec<StoredAttachment>> {
    let mut stmt = conn.prepare_cached(&format!(
        "SELECT {ATTACHMENT_COLUMNS} FROM attachment a JOIN message m ON m.id = a.message_id
          WHERE m.account_id = ?1 AND m.id = ?2 ORDER BY a.id"
    ))?;
    let rows = stmt.query_map(params![account_id, message_id], attachment_row)?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// One attachment of a message of this account.
pub fn attachment(
    conn: &Connection,
    account_id: i64,
    message_id: i64,
    attachment_id: i64,
) -> Result<Option<StoredAttachment>> {
    Ok(conn
        .query_row(
            &format!(
                "SELECT {ATTACHMENT_COLUMNS} FROM attachment a JOIN message m ON m.id = a.message_id
                  WHERE m.account_id = ?1 AND m.id = ?2 AND a.id = ?3"
            ),
            params![account_id, message_id, attachment_id],
            attachment_row,
        )
        .optional()?)
}

/// Marks the stored body of a message of this account as too complex to render: later opens show
/// its text and never try the HTML again. Returns false when it has no stored body.
pub fn set_too_complex(conn: &Connection, account_id: i64, message_id: i64) -> Result<bool> {
    Ok(conn.execute(
        "UPDATE message_body SET too_complex = 1, html_pending = 0
          WHERE message_id = (SELECT id FROM message WHERE account_id = ?1 AND id = ?2)",
        params![account_id, message_id],
    )? > 0)
}

/// What a later render of a message whose HTML was pending gave (see [`store_html`]).
/// `Debug` is not derived: it holds content.
pub struct RenderedHtml<'a> {
    /// The sanitised HTML with remote content blocked, or `None` when it still was not rendered.
    pub html_clean: Option<&'a str>,
    /// Remote images left out of `html_clean`.
    pub remote_blocked: u32,
    /// Tracking pixels removed.
    pub trackers_removed: u32,
    /// A body was cut at a limit.
    pub truncated: bool,
    /// The HTML is too complex to render: never tried again.
    pub too_complex: bool,
    /// Still not rendered this time: tried again on the next open.
    pub html_pending: bool,
    /// The Content-IDs drawn inside `html_clean` (their attachments get `shown_inline`).
    pub cids_used: &'a [String],
}

/// Stores the outcome of rendering again a message whose HTML was pending (its body was stored
/// with `html_pending`): the HTML and its counts, the marks, and which attachments are now drawn
/// inline. Attachment rows keep their ids. Returns false when it has no stored body.
pub fn store_html(
    conn: &Connection,
    account_id: i64,
    message_id: i64,
    html: &RenderedHtml<'_>,
) -> Result<bool> {
    if !owned(conn, account_id, message_id)? {
        return Ok(false);
    }
    let updated = conn.execute(
        "UPDATE message_body SET html_clean = ?2, remote_blocked = ?3, trackers_removed = ?4,
                                 truncated = ?5, too_complex = ?6, html_pending = ?7
          WHERE message_id = ?1",
        params![
            message_id,
            html.html_clean,
            html.remote_blocked,
            html.trackers_removed,
            html.truncated,
            html.too_complex,
            html.html_pending && !html.too_complex
        ],
    )?;
    if updated == 0 {
        return Ok(false);
    }
    let mut inline = conn.prepare_cached(
        "UPDATE attachment SET shown_inline = 1 WHERE message_id = ?1 AND content_id = ?2",
    )?;
    for cid in html.cids_used {
        inline.execute(params![message_id, cid])?;
    }
    conn.execute(
        "UPDATE message SET has_attachments = EXISTS (
             SELECT 1 FROM attachment WHERE message_id = ?2 AND shown_inline = 0)
          WHERE account_id = ?1 AND id = ?2",
        params![account_id, message_id],
    )?;
    Ok(true)
}

/// Records whether the person allowed remote content on a message of this account. Returns false
/// when there is no such message.
pub fn set_remote_allowed(
    conn: &Connection,
    account_id: i64,
    message_id: i64,
    allowed: bool,
) -> Result<bool> {
    Ok(conn.execute(
        "UPDATE message SET remote_allowed = ?3 WHERE account_id = ?1 AND id = ?2",
        params![account_id, message_id, allowed],
    )? > 0)
}

/// Whether the person allowed remote content on a message of this account.
pub fn remote_allowed(conn: &Connection, account_id: i64, message_id: i64) -> Result<bool> {
    Ok(conn
        .query_row(
            "SELECT remote_allowed FROM message WHERE account_id = ?1 AND id = ?2",
            params![account_id, message_id],
            |r| r.get(0),
        )
        .optional()?
        .unwrap_or(false))
}

/// Adds a known sender (an address already normalised: trimmed, lower case) to the account.
/// Returns false when there is no such account. Adding one twice is harmless.
pub fn trust_sender(
    conn: &Connection,
    account_id: i64,
    address: &str,
    now_ms: i64,
) -> Result<bool> {
    let exists = conn
        .query_row(
            "SELECT 1 FROM account WHERE id = ?1",
            params![account_id],
            |_| Ok(()),
        )
        .optional()?
        .is_some();
    if !exists {
        return Ok(false);
    }
    conn.execute(
        "INSERT OR IGNORE INTO trusted_sender (account_id, address, created_at) VALUES (?1, ?2, ?3)",
        params![account_id, address, now_ms],
    )?;
    Ok(true)
}

/// Removes a known sender from the account. Returns true when it was there.
pub fn untrust_sender(conn: &Connection, account_id: i64, address: &str) -> Result<bool> {
    Ok(conn.execute(
        "DELETE FROM trusted_sender WHERE account_id = ?1 AND address = ?2",
        params![account_id, address],
    )? > 0)
}

/// Whether the (normalised) address is a known sender of the account.
pub fn is_trusted(conn: &Connection, account_id: i64, address: &str) -> Result<bool> {
    Ok(conn
        .query_row(
            "SELECT 1 FROM trusted_sender WHERE account_id = ?1 AND address = ?2",
            params![account_id, address],
            |_| Ok(()),
        )
        .optional()?
        .is_some())
}

/// The known senders of the account, sorted.
pub fn trusted_senders(conn: &Connection, account_id: i64) -> Result<Vec<String>> {
    let mut stmt = conn.prepare_cached(
        "SELECT address FROM trusted_sender WHERE account_id = ?1 ORDER BY address",
    )?;
    let rows = stmt.query_map(params![account_id], |r| r.get(0))?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

const CONVERSATION_COLUMNS: &str = "m.id, m.account_id, m.folder_id, m.uid, m.message_id_hdr, \
     m.thread_key, m.subject, m.from_name, m.from_addr, m.date_ts, m.size, m.snippet, m.is_seen, \
     m.is_flagged, m.is_answered, m.is_draft, m.keywords_json, m.has_attachments, m.category, \
     m.snoozed_until, m.pending_ops, m.body_state, m.local_hidden, m.to_json, m.cc_json, \
     m.remote_allowed, f.special_use,
     (SELECT r.sha256 FROM message_raw r WHERE r.message_id = m.id AND r.account_id = m.account_id)";

fn conversation_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<ConversationRow> {
    Ok(ConversationRow {
        message: messages::map_row(r)?,
        to_json: r.get(23)?,
        cc_json: r.get(24)?,
        remote_allowed: r.get(25)?,
        folder_special_use: r.get(26)?,
        raw_sha256: r.get(27)?,
    })
}

/// The messages of the conversation `message_id` (of this account) belongs to, oldest first.
///
/// Built from the stored thread key (`message.thread_key`: the first `References` id, else the
/// message's own `Message-ID`; see `messages::thread_key`), across every folder of the account,
/// and widened a few rounds to the messages whose `Message-ID` is a key already found (a reply
/// whose `References` were cut short still joins its thread). Then:
///
/// - **Sent is included**: the person's own replies are part of the conversation.
/// - **Trash and Junk are left out**, unless the conversation is opened from a message in that
///   folder. **Drafts** likewise (drafts are edited in the composer, not read here).
/// - Rows hidden while a move or delete waits for the server are left out, except the opened one.
/// - The same message in two folders (Inbox and Gmail's All Mail, say) is listed once: copies
///   are rows with the same `Message-ID`, **the same sender address (case ignored), the same date
///   and the same bytes**: the same SHA-256 of the fetched message when both were fetched, else
///   the same size on the server. The one kept is the opened one, else the one in the opened
///   message's folder, else one not in Junk or Trash, else one not in an `all` folder. A row with
///   a borrowed `Message-ID` but another sender, date or content is a different message and is
///   always listed, so a forged copy can never stand in for the real one.
/// - At most [`MAX_CONVERSATION`] messages (the newest).
///
/// Empty when the message is not one of this account.
pub fn conversation(
    conn: &Connection,
    account_id: i64,
    message_id: i64,
) -> Result<Vec<ConversationRow>> {
    let opened: Option<ConversationRow> = conn
        .query_row(
            &format!(
                "SELECT {CONVERSATION_COLUMNS} FROM message m
                   JOIN folder f ON f.id = m.folder_id AND f.account_id = m.account_id
                  WHERE m.account_id = ?1 AND m.id = ?2"
            ),
            params![account_id, message_id],
            conversation_row,
        )
        .optional()?;
    let Some(opened) = opened else {
        return Ok(Vec::new());
    };
    let by_key = conn.prepare_cached(&format!(
        "SELECT {CONVERSATION_COLUMNS} FROM message m
           JOIN folder f ON f.id = m.folder_id AND f.account_id = m.account_id
          WHERE m.account_id = ?1 AND (m.thread_key = ?2 OR m.message_id_hdr = ?2)
          LIMIT 2000"
    ));
    let mut by_key = by_key?;
    let mut keys_done: HashSet<String> = HashSet::new();
    let mut keys: Vec<String> = vec![opened.message.thread_key.clone()];
    let mut found: Vec<ConversationRow> = Vec::new();
    let mut seen_ids: BTreeSet<i64> = BTreeSet::new();
    for _ in 0..CONVERSATION_ROUNDS {
        let mut next = Vec::new();
        for key in keys.drain(..) {
            if !keys_done.insert(key.clone()) {
                continue;
            }
            let rows = by_key.query_map(params![account_id, key], conversation_row)?;
            for row in rows {
                let row = row?;
                if seen_ids.insert(row.message.id) {
                    for k in [
                        Some(&row.message.thread_key),
                        row.message.message_id_hdr.as_ref(),
                    ]
                    .into_iter()
                    .flatten()
                    {
                        if !keys_done.contains(k) {
                            next.push(k.clone());
                        }
                    }
                    found.push(row);
                }
            }
        }
        if next.is_empty() || found.len() > MAX_CONVERSATION * 2 {
            break;
        }
        keys = next;
    }
    if seen_ids.insert(opened.message.id) {
        found.push(opened.clone());
    }

    let opened_use = opened.folder_special_use.clone();
    let opened_folder = opened.message.folder_id;
    found.retain(|r| {
        if r.message.id == opened.message.id {
            return true;
        }
        if r.message.local_hidden {
            return false;
        }
        match r.folder_special_use.as_str() {
            u @ ("trash" | "junk" | "drafts") => u == opened_use,
            _ => true,
        }
    });

    // One row per copy of a message (same Message-ID, sender and date): the opened one, else in
    // the opened folder, else not in Junk or Trash, else not in All Mail.
    let rank = |r: &ConversationRow| {
        if r.message.id == opened.message.id {
            0
        } else if r.message.folder_id == opened_folder {
            1
        } else if matches!(r.folder_special_use.as_str(), "junk" | "trash") {
            4
        } else if r.folder_special_use != "all" {
            2
        } else {
            3
        }
    };
    found.sort_by_key(|r| (rank(r), r.message.id));
    // A later row is folded into a kept one only when they are the same bytes: the same
    // SHA-256 when both were fetched, else (not fetched yet) the same size on the server.
    type Copy = (i64, Option<Vec<u8>>);
    let mut copies_kept: HashMap<(String, String, i64), Vec<Copy>> = HashMap::new();
    found.retain(|r| match &r.message.message_id_hdr {
        Some(id) if !id.trim().is_empty() => {
            let key = (
                id.clone(),
                r.message
                    .from_addr
                    .as_deref()
                    .unwrap_or_default()
                    .trim()
                    .to_lowercase(),
                r.message.date_ts,
            );
            let kept = copies_kept.entry(key).or_default();
            // Round 3 review L2: the size alone counts only while neither copy is fetched; once
            // one is, only the same hash folds (a fetched copy never stands in for an unfetched
            // one, nor the other way round).
            let same = kept.iter().any(|(size, hash)| {
                *size == r.message.size
                    && match (hash, &r.raw_sha256) {
                        (Some(a), Some(b)) => a == b,
                        (None, None) => true,
                        _ => false,
                    }
            });
            if !same {
                kept.push((r.message.size, r.raw_sha256.clone()));
            }
            !same
        }
        _ => true,
    });

    found.sort_by_key(|r| (r.message.date_ts, r.message.id));
    if found.len() > MAX_CONVERSATION {
        let cut = found.len() - MAX_CONVERSATION;
        found.drain(..cut);
        if !found.iter().any(|r| r.message.id == opened.message.id) {
            // The opened message is always part of what is returned.
            found.remove(0);
            found.push(opened);
            found.sort_by_key(|r| (r.message.date_ts, r.message.id));
        }
    }
    Ok(found)
}
