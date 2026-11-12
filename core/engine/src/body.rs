// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Fetching a message body for the conversation screen: the account's backend fetches the whole
//! message once ([`croft_protocols::MailBackend::fetch_body`]), [`croft_reader`] parses and
//! sanitises it on a blocking thread, and the result is stored (`croft_storage::reading`). From
//! then on the message opens without the network.
//!
//! The fetch does not take the account's turn: it runs next to the account's task, so opening a
//! message never waits for a whole folder sync (the IMAP adapter's own connection lock orders the
//! commands). The queue's lock is held only to look the message up and to store the result,
//! never during the download.

use std::fmt;
use std::sync::atomic::Ordering;

use croft_mail_core::AccountId;
use croft_protocols::{BackendError, RawMessage};
use croft_reader::{Address, ParseError, Rendered};
use croft_storage::reading::{self, NewAttachment, NewBody};
use croft_storage::{folders, messages, StorageError};

use crate::engine::{Change, Shared};
use crate::handle::EngineHandle;

/// Why [`EngineHandle::fetch_body`] could not fetch a body. Fixed categories: never message
/// content, addresses or server text (a [`BackendError`] carries the adapter's fixed wording).
#[derive(Debug)]
pub enum BodyError {
    /// No message with this row id in this account.
    NoSuchMessage,
    /// No backend was added for this account.
    UnknownAccount(AccountId),
    /// The device is offline for this account, and the body is not stored.
    Offline,
    /// The server refused the sign-in (the account now waits for [`EngineHandle::signed_in`]).
    NeedsSignIn,
    /// The server no longer has the message (moved or deleted elsewhere); the next sync updates
    /// the list.
    NotOnServer,
    /// The message is larger than the limit (`croft_protocols::MAX_MESSAGE_BYTES`, 50 MiB).
    TooLarge {
        /// Its size in bytes, as the server reported it.
        size: u64,
    },
    /// What the server sent is not an email message.
    Unreadable,
    /// The server failed: `Transient` (try again) or `Permanent`.
    Server(BackendError),
    /// The database failed.
    Storage(StorageError),
}

impl fmt::Display for BodyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoSuchMessage => f.write_str("no such message"),
            Self::UnknownAccount(a) => write!(f, "no backend for account {a}"),
            Self::Offline => f.write_str("offline, and the message is not on this device"),
            Self::NeedsSignIn => f.write_str("sign-in needed"),
            Self::NotOnServer => f.write_str("the message is no longer on the server"),
            Self::TooLarge { size } => write!(f, "the message is too large ({size} bytes)"),
            Self::Unreadable => f.write_str("the server sent something that is not a message"),
            Self::Server(e) => e.fmt(f),
            Self::Storage(e) => write!(f, "database error: {e}"),
        }
    }
}

impl std::error::Error for BodyError {}

impl From<StorageError> for BodyError {
    fn from(e: StorageError) -> Self {
        Self::Storage(e)
    }
}

/// The addresses as `message.to_json` stores them: `[{"name": ..., "address": ...}]`.
fn addresses_json(list: &[Address]) -> String {
    let items: Vec<serde_json::Value> = list
        .iter()
        .map(|a| serde_json::json!({"name": a.name, "address": a.address}))
        .collect();
    serde_json::Value::Array(items).to_string()
}

/// Parses `raw` (blocked mode) and stores it for the message. Returns the folder to announce, or
/// `None` when the message row is gone meanwhile.
fn parse_and_store(
    shared: &Shared,
    account: AccountId,
    message: i64,
    raw: &[u8],
) -> Result<Option<i64>, BodyError> {
    let rendered: Rendered = croft_reader::render(raw, false).map_err(|e| match e {
        ParseError::TooLarge { size } => BodyError::TooLarge { size },
        ParseError::NotAMessage => BodyError::Unreadable,
    })?;
    let to_json = addresses_json(&rendered.to);
    let cc_json = addresses_json(&rendered.cc);
    let snippet = croft_reader::snippet(rendered.text.as_deref().unwrap_or_default());
    let attachments: Vec<NewAttachment> = rendered
        .attachments
        .iter()
        .map(|a| NewAttachment {
            part_id: a.part_id.clone(),
            file_name: a.file_name.clone(),
            mime_type: a.mime_type.clone(),
            size: a.size,
            content_id: a.content_id.clone(),
            shown_inline: a.shown_inline,
            executable: a.executable,
        })
        .collect();
    let raw_sha256 = {
        use sha2::Digest;
        sha2::Sha256::digest(raw).to_vec()
    };
    let body = NewBody {
        raw,
        raw_sha256: &raw_sha256,
        fetched_at: shared.now(),
        headers_raw: &rendered.headers_raw,
        text: rendered.text.as_deref(),
        html_clean: rendered.html.as_deref(),
        remote_blocked: rendered.blocked_remote,
        trackers_removed: rendered.trackers_removed,
        truncated: rendered.truncated,
        too_complex: rendered.too_complex,
        to_json: &to_json,
        cc_json: &cc_json,
        snippet: &snippet,
        attachments: &attachments,
    };
    let mut q = shared.lock();
    let stored = q.store_mut().db_mut().transaction(|tx| {
        let folder = messages::get(tx, account, message)?.map(|m| m.folder_id);
        if folder.is_some() && !reading::store_body(tx, account, message, &body)? {
            return Ok::<_, StorageError>(None);
        }
        Ok(folder)
    })?;
    Ok(stored)
}

impl EngineHandle {
    /// Makes sure the message's body is stored: if it is not, fetches the whole message from the
    /// server, parses and sanitises it, and stores the raw copy, the text, the sanitised HTML
    /// (remote content blocked) and the attachment list. Returns at once when the body is already
    /// stored, also offline. **Waits for the network**: call it from a task, never the UI thread.
    ///
    /// Never marks the message read on the server (see `MailBackend::fetch_body`). A refused
    /// sign-in pauses the account like any other ([`Change::SignInNeeded`]). The folder's rows are
    /// announced as changed ([`Change::Messages`]) when a body was stored (the row's preview,
    /// recipients and attachment flag come from it).
    pub async fn fetch_body(&self, account: AccountId, message: i64) -> Result<(), BodyError> {
        let shared = self.shared().clone();
        let (path, uid) = {
            let q = shared.lock();
            let conn = q.store().db().connection();
            let m = messages::get(conn, account, message)?.ok_or(BodyError::NoSuchMessage)?;
            if reading::has_raw(conn, account, message)?
                && reading::has_body(conn, account, message)?
            {
                return Ok(());
            }
            let folder =
                folders::get(conn, account, m.folder_id)?.ok_or(BodyError::NoSuchMessage)?;
            if q.queue().is_offline(account) {
                return Err(BodyError::Offline);
            }
            let uid = u32::try_from(m.uid).map_err(|_| BodyError::NoSuchMessage)?;
            (folder.path, uid)
        };
        let slot = shared
            .slot(account)
            .ok_or(BodyError::UnknownAccount(account))?;
        if slot.needs_sign_in() {
            return Err(BodyError::NeedsSignIn);
        }
        let raw = match slot.backend.fetch_body(&path, uid).await {
            Ok(RawMessage::Found(raw)) => raw,
            Ok(RawMessage::Gone) => {
                // The list is out of date: let the account's task look.
                shared.request_full_sync(account);
                return Err(BodyError::NotOnServer);
            }
            Ok(RawMessage::TooLarge { size }) => return Err(BodyError::TooLarge { size }),
            Err(BackendError::Auth(_)) => {
                if !slot.needs_sign_in.swap(true, Ordering::SeqCst) {
                    shared.emit(Change::SignInNeeded { account });
                }
                return Err(BodyError::NeedsSignIn);
            }
            Err(e) => return Err(BodyError::Server(e)),
        };
        drop(slot);
        // Parsing and sanitising a big message takes a while: not on a runtime worker.
        let worker = shared.clone();
        let stored =
            tokio::task::spawn_blocking(move || parse_and_store(&worker, account, message, &raw))
                .await
                .map_err(|_| BodyError::Unreadable)??;
        match stored {
            Some(folder) => {
                shared.emit(Change::Messages {
                    account,
                    folders: vec![folder],
                });
                Ok(())
            }
            None => Err(BodyError::NoSuchMessage),
        }
    }
}
