// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Drafts and their attachments (version 6 of the schema, `migrations/0006_compose.sql`).
//!
//! Every statement carries `account_id`, also when it is given a draft id, so an account can never
//! read or change another account's draft by guessing an id. Bodies, addresses and attachment bytes
//! never appear in an error or in `Debug` output.
//!
//! A draft's `state` is `draft` (being written), `queued` (Send was tapped; the Outbox row owns it)
//! or `done` (sent or discarded, waiting for its copy on the server to be deleted). See the
//! migration for the full story, and `revision` / `synced_revision` for the server copy.

use croft_mail_core::compose::Address;
use rusqlite::{params, Connection, OptionalExtension, Row};

use crate::error::{Result, StorageError};

/// Where a draft is in its life.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DraftState {
    /// Being written.
    Draft,
    /// Send was tapped: the Outbox message made from it exists (or has been sent).
    Queued,
    /// Sent or discarded; only its copy on the server is left to delete.
    Done,
}

impl DraftState {
    fn name(self) -> &'static str {
        match self {
            Self::Draft => "draft",
            Self::Queued => "queued",
            Self::Done => "done",
        }
    }

    fn from_name(name: &str) -> Option<Self> {
        Some(match name {
            "draft" => Self::Draft,
            "queued" => Self::Queued,
            "done" => Self::Done,
            _ => return None,
        })
    }
}

/// What the person edits.
#[derive(Clone, PartialEq, Eq, Default)]
pub struct DraftContent {
    /// The identity it is sent as (`identity.id`), if chosen.
    pub identity_id: Option<i64>,
    /// To.
    pub to: Vec<Address>,
    /// Cc.
    pub cc: Vec<Address>,
    /// Bcc.
    pub bcc: Vec<Address>,
    /// Subject.
    pub subject: String,
    /// Body text.
    pub body: String,
    /// `In-Reply-To`.
    pub in_reply_to: Option<String>,
    /// `References`.
    pub references: Vec<String>,
}

impl std::fmt::Debug for DraftContent {
    /// Counts only.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "DraftContent({} recipients)",
            self.to.len() + self.cc.len() + self.bcc.len()
        )
    }
}

/// A stored draft.
#[derive(Clone, PartialEq, Eq)]
pub struct Draft {
    /// Row id, never reused.
    pub id: i64,
    /// Owning account.
    pub account_id: i64,
    /// Its own Message-ID, the same for every save and for the message when it is sent.
    pub message_id: String,
    /// What the person wrote.
    pub content: DraftContent,
    /// Where it is in its life.
    pub state: DraftState,
    /// The Outbox message made from it, once Send was tapped.
    pub outbox_id: Option<i64>,
    /// Goes up with every change.
    pub revision: i64,
    /// The revision the copy on the server has (0: none yet).
    pub synced_revision: i64,
    /// The server folder holding the copy, if there is one.
    pub server_folder: Option<String>,
    /// The UID of the copy in that folder, if the server reported it.
    pub server_uid: Option<i64>,
    /// Creation time, Unix milliseconds.
    pub created_at: i64,
    /// Time of the last change, Unix milliseconds.
    pub updated_at: i64,
}

impl std::fmt::Debug for Draft {
    /// Ids and states only.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Draft")
            .field("id", &self.id)
            .field("account_id", &self.account_id)
            .field("state", &self.state)
            .field("revision", &self.revision)
            .finish_non_exhaustive()
    }
}

/// An attachment without its bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttachmentInfo {
    /// Row id.
    pub id: i64,
    /// The file name to show and to send.
    pub name: String,
    /// The media type.
    pub mime_type: String,
    /// Size in bytes.
    pub size: i64,
}

fn addresses_json(list: &[Address]) -> Result<String> {
    let items: Vec<serde_json::Value> = list
        .iter()
        .map(|a| serde_json::json!({ "name": a.name, "address": a.address }))
        .collect();
    Ok(serde_json::to_string(&items)?)
}

fn addresses_from(text: &str) -> Vec<Address> {
    let Ok(serde_json::Value::Array(items)) = serde_json::from_str::<serde_json::Value>(text)
    else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|v| {
            let address = v.get("address")?.as_str()?.to_owned();
            let name = v.get("name").and_then(|n| n.as_str()).map(str::to_owned);
            Some(Address { name, address })
        })
        .collect()
}

const COLUMNS: &str =
    "id, account_id, identity_id, message_id, to_json, cc_json, bcc_json, subject, body,
       in_reply_to, references_json, state, outbox_id, revision, synced_revision, server_folder,
       server_uid, created_at, updated_at";

fn map_row(r: &Row<'_>) -> rusqlite::Result<Draft> {
    let state: String = r.get(11)?;
    let references: String = r.get(10)?;
    Ok(Draft {
        id: r.get(0)?,
        account_id: r.get(1)?,
        message_id: r.get(3)?,
        content: DraftContent {
            identity_id: r.get(2)?,
            to: addresses_from(&r.get::<_, String>(4)?),
            cc: addresses_from(&r.get::<_, String>(5)?),
            bcc: addresses_from(&r.get::<_, String>(6)?),
            subject: r.get(7)?,
            body: r.get(8)?,
            in_reply_to: r.get(9)?,
            references: serde_json::from_str(&references).unwrap_or_default(),
        },
        state: DraftState::from_name(&state).unwrap_or(DraftState::Draft),
        outbox_id: r.get(12)?,
        revision: r.get(13)?,
        synced_revision: r.get(14)?,
        server_folder: r.get(15)?,
        server_uid: r.get(16)?,
        created_at: r.get(17)?,
        updated_at: r.get(18)?,
    })
}

/// Stores a new draft (state `draft`, revision 1) with the Message-ID it will keep. Returns its id.
pub fn insert(
    conn: &Connection,
    account_id: i64,
    message_id: &str,
    content: &DraftContent,
    now_ms: i64,
) -> Result<i64> {
    conn.execute(
        "INSERT INTO draft (account_id, identity_id, message_id, to_json, cc_json, bcc_json, subject,
                            body, in_reply_to, references_json, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?11)",
        params![
            account_id,
            content.identity_id,
            message_id,
            addresses_json(&content.to)?,
            addresses_json(&content.cc)?,
            addresses_json(&content.bcc)?,
            content.subject,
            content.body,
            content.in_reply_to,
            serde_json::to_string(&content.references)?,
            now_ms,
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

/// Replaces what the person wrote, only while the draft is in state `draft` (a queued or finished
/// draft is not edited). The revision goes up. Returns false when there is no such draft in that
/// state for this account.
pub fn update(
    conn: &Connection,
    account_id: i64,
    id: i64,
    content: &DraftContent,
    now_ms: i64,
) -> Result<bool> {
    let changed = conn.execute(
        "UPDATE draft SET identity_id = ?3, to_json = ?4, cc_json = ?5, bcc_json = ?6, subject = ?7,
                          body = ?8, in_reply_to = ?9, references_json = ?10,
                          revision = revision + 1, updated_at = ?11
          WHERE account_id = ?1 AND id = ?2 AND state = 'draft'",
        params![
            account_id,
            id,
            content.identity_id,
            addresses_json(&content.to)?,
            addresses_json(&content.cc)?,
            addresses_json(&content.bcc)?,
            content.subject,
            content.body,
            content.in_reply_to,
            serde_json::to_string(&content.references)?,
            now_ms,
        ],
    )?;
    Ok(changed == 1)
}

/// One draft of this account, in any state.
pub fn get(conn: &Connection, account_id: i64, id: i64) -> Result<Option<Draft>> {
    Ok(conn
        .query_row(
            &format!("SELECT {COLUMNS} FROM draft WHERE account_id = ?1 AND id = ?2"),
            params![account_id, id],
            map_row,
        )
        .optional()?)
}

/// The account's drafts that are being written or queued (not the finished ones), newest change
/// first.
pub fn list(conn: &Connection, account_id: i64) -> Result<Vec<Draft>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {COLUMNS} FROM draft
          WHERE account_id = ?1 AND state IN ('draft', 'queued')
          ORDER BY updated_at DESC, id DESC"
    ))?;
    let rows = stmt.query_map([account_id], map_row)?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// Every draft of the account in one state, oldest first.
pub fn in_state(conn: &Connection, account_id: i64, state: DraftState) -> Result<Vec<Draft>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {COLUMNS} FROM draft WHERE account_id = ?1 AND state = ?2 ORDER BY id"
    ))?;
    let rows = stmt.query_map(params![account_id, state.name()], map_row)?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// Every draft of every account in state `queued`, for the check that finds sent messages.
/// Account ids come from the rows themselves.
pub fn queued_accounts(conn: &Connection) -> Result<Vec<i64>> {
    let mut stmt =
        conn.prepare("SELECT DISTINCT account_id FROM draft WHERE state IN ('queued', 'done')")?;
    let rows = stmt.query_map([], |r| r.get(0))?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// Marks the draft as queued for the Outbox message `outbox_id`. Only a draft in state `draft`.
pub fn set_queued(conn: &Connection, account_id: i64, id: i64, outbox_id: i64) -> Result<bool> {
    Ok(conn.execute(
        "UPDATE draft SET state = 'queued', outbox_id = ?3
          WHERE account_id = ?1 AND id = ?2 AND state IN ('draft', 'queued')",
        params![account_id, id, outbox_id],
    )? == 1)
}

/// Puts a queued draft back to `draft` (the message was taken back from the Outbox). The revision
/// goes up so the server copy is made again if needed. Returns false when it was not queued.
pub fn set_editable(conn: &Connection, account_id: i64, id: i64, now_ms: i64) -> Result<bool> {
    Ok(conn.execute(
        "UPDATE draft SET state = 'draft', outbox_id = NULL, revision = revision + 1, updated_at = ?3
          WHERE account_id = ?1 AND id = ?2 AND state = 'queued'",
        params![account_id, id, now_ms],
    )? == 1)
}

/// Marks the draft finished (sent or discarded). Its attachments are dropped. The row stays until
/// the copy on the server has been deleted (`delete`).
pub fn mark_done(conn: &Connection, account_id: i64, id: i64, now_ms: i64) -> Result<bool> {
    conn.execute(
        "DELETE FROM draft_attachment WHERE account_id = ?1 AND draft_id = ?2",
        params![account_id, id],
    )?;
    Ok(conn.execute(
        "UPDATE draft SET state = 'done', outbox_id = NULL, updated_at = ?3
          WHERE account_id = ?1 AND id = ?2",
        params![account_id, id, now_ms],
    )? == 1)
}

/// Removes the draft and its attachments.
pub fn delete(conn: &Connection, account_id: i64, id: i64) -> Result<bool> {
    conn.execute(
        "DELETE FROM draft_attachment WHERE account_id = ?1 AND draft_id = ?2",
        params![account_id, id],
    )?;
    Ok(conn.execute(
        "DELETE FROM draft WHERE account_id = ?1 AND id = ?2",
        params![account_id, id],
    )? == 1)
}

/// Records where the copy on the server is and which revision it has. `revision` only moves
/// forward (a slower, older save never overwrites a newer record).
pub fn record_server_copy(
    conn: &Connection,
    account_id: i64,
    id: i64,
    folder: &str,
    uid: Option<i64>,
    revision: i64,
) -> Result<bool> {
    Ok(conn.execute(
        "UPDATE draft SET server_folder = ?3, server_uid = ?4,
                          synced_revision = MAX(synced_revision, ?5)
          WHERE account_id = ?1 AND id = ?2",
        params![account_id, id, folder, uid, revision],
    )? == 1)
}

/// Forgets the copy on the server (it was deleted).
pub fn clear_server_copy(conn: &Connection, account_id: i64, id: i64) -> Result<bool> {
    Ok(conn.execute(
        "UPDATE draft SET server_folder = NULL, server_uid = NULL
          WHERE account_id = ?1 AND id = ?2",
        params![account_id, id],
    )? == 1)
}

// ------------------------------------------------------------------------------------------
// Attachments
// ------------------------------------------------------------------------------------------

/// The attachments of a draft, without bytes, in the order they were added.
pub fn attachments(
    conn: &Connection,
    account_id: i64,
    draft_id: i64,
) -> Result<Vec<AttachmentInfo>> {
    let mut stmt = conn.prepare(
        "SELECT id, name, mime_type, size FROM draft_attachment
          WHERE account_id = ?1 AND draft_id = ?2 ORDER BY id",
    )?;
    let rows = stmt.query_map(params![account_id, draft_id], |r| {
        Ok(AttachmentInfo {
            id: r.get(0)?,
            name: r.get(1)?,
            mime_type: r.get(2)?,
            size: r.get(3)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// The sum of the sizes of a draft's attachments.
pub fn attachments_total(conn: &Connection, account_id: i64, draft_id: i64) -> Result<i64> {
    Ok(conn.query_row(
        "SELECT COALESCE(SUM(size), 0) FROM draft_attachment WHERE account_id = ?1 AND draft_id = ?2",
        params![account_id, draft_id],
        |r| r.get(0),
    )?)
}

/// Adds an attachment to a draft that is in state `draft`, and raises the draft's revision.
/// Returns its id, or `NotFound` when the draft is not an editable draft of this account.
pub fn add_attachment(
    conn: &Connection,
    account_id: i64,
    draft_id: i64,
    name: &str,
    mime_type: &str,
    data: &[u8],
    now_ms: i64,
) -> Result<i64> {
    let size = i64::try_from(data.len()).map_err(|_| StorageError::OutOfRange("attachment"))?;
    let changed = conn.execute(
        "UPDATE draft SET revision = revision + 1, updated_at = ?3
          WHERE account_id = ?1 AND id = ?2 AND state = 'draft'",
        params![account_id, draft_id, now_ms],
    )?;
    if changed != 1 {
        return Err(StorageError::NotFound("draft"));
    }
    conn.execute(
        "INSERT INTO draft_attachment (account_id, draft_id, name, mime_type, size, data)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![account_id, draft_id, name, mime_type, size, data],
    )?;
    Ok(conn.last_insert_rowid())
}

/// Removes one attachment of an editable draft and raises its revision. Returns false when there
/// was none.
pub fn remove_attachment(
    conn: &Connection,
    account_id: i64,
    draft_id: i64,
    attachment_id: i64,
    now_ms: i64,
) -> Result<bool> {
    let removed = conn.execute(
        "DELETE FROM draft_attachment
          WHERE account_id = ?1 AND draft_id = ?2 AND id = ?3
            AND EXISTS (SELECT 1 FROM draft WHERE account_id = ?1 AND id = ?2 AND state = 'draft')",
        params![account_id, draft_id, attachment_id],
    )?;
    if removed == 1 {
        conn.execute(
            "UPDATE draft SET revision = revision + 1, updated_at = ?3
              WHERE account_id = ?1 AND id = ?2",
            params![account_id, draft_id, now_ms],
        )?;
    }
    Ok(removed == 1)
}

/// An attachment's name, type and bytes.
pub fn attachment_data(
    conn: &Connection,
    account_id: i64,
    draft_id: i64,
) -> Result<Vec<(String, String, Vec<u8>)>> {
    let mut stmt = conn.prepare(
        "SELECT name, mime_type, data FROM draft_attachment
          WHERE account_id = ?1 AND draft_id = ?2 ORDER BY id",
    )?;
    let rows = stmt.query_map(params![account_id, draft_id], |r| {
        Ok((r.get(0)?, r.get(1)?, r.get(2)?))
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

#[cfg(test)]
mod tests;
