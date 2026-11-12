// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Reading (S20 conversation, S21 blocked content and attachments): `open_conversation`,
//! `load_body`, `save_attachment`, `allow_remote_content` and the known-senders list.
//!
//! See `core/ffi/README.md`, "Reading a message", for how the app renders what comes back.

use std::fs::OpenOptions;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use croft_engine::BodyError;
use croft_protocols::BackendError;
use croft_reader::{files, normalise_address, remote_allowed, RemoteImages};
use croft_storage::reading::{self, ConversationRow};
use croft_storage::{accounts, StorageError};
use zeroize::Zeroizing;

use crate::core::Core;
use crate::error::{CoreError, StorageReason};

/// How many numbered names (`report (1).pdf`, ...) `save_attachment` tries when the name is taken.
const MAX_NAME_TRIES: u32 = 100;

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|d| i64::try_from(d.as_millis()).ok())
        .unwrap_or(0)
}

/// The person's "Remote images" setting (S49 and S57; default `Ask`). The app passes it to
/// [`Core::load_body`]; the core decides.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, uniffi::Enum)]
pub enum RemoteImagesSetting {
    /// Blocked until the person taps "Show images" on the message ([`Core::allow_remote_content`]).
    #[default]
    Ask,
    /// As `Ask`, and loaded automatically for the account's known senders ([`Core::trust_sender`]).
    KnownSenders,
    /// Never loaded, even on a message where the person allowed them before.
    Never,
}

impl From<RemoteImagesSetting> for RemoteImages {
    fn from(s: RemoteImagesSetting) -> Self {
        match s {
            RemoteImagesSetting::Ask => Self::Ask,
            RemoteImagesSetting::KnownSenders => Self::KnownSenders,
            RemoteImagesSetting::Never => Self::Never,
        }
    }
}

/// Whether a message's body is on the device.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum BodyState {
    /// Not fetched yet: [`Core::load_body`] needs the network.
    NotLoaded,
    /// Stored: [`Core::load_body`] works offline.
    Loaded,
}

/// One mailbox of an address header. `Debug` does not print it.
#[derive(Clone, PartialEq, Eq, uniffi::Record)]
pub struct MailAddress {
    /// Display name, if any.
    pub name: Option<String>,
    /// The address, if any (a group or a broken header can have a name only).
    pub address: Option<String>,
}

impl std::fmt::Debug for MailAddress {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("MailAddress { .. }")
    }
}

/// One message of a conversation (S20). `Debug` prints ids only.
#[derive(Clone, PartialEq, Eq, uniffi::Record)]
pub struct ConversationMessage {
    /// Row id (what `load_body`, `mark_seen` and the other actions take).
    pub id: i64,
    /// The account.
    pub account: i64,
    /// The folder's row id.
    pub folder: i64,
    /// The folder's special use (`inbox`, `sent`, `archive`, `trash`, ...); the screen can show
    /// "You" for `sent`.
    pub folder_special_use: String,
    /// Sender display name.
    pub sender_name: Option<String>,
    /// Sender address.
    pub sender_address: Option<String>,
    /// `To`. Empty until the body was loaded once (the sync stores headers only).
    pub to: Vec<MailAddress>,
    /// `Cc`. Empty until the body was loaded once.
    pub cc: Vec<MailAddress>,
    /// Date, Unix milliseconds.
    pub date_ms: i64,
    /// Subject.
    pub subject: String,
    /// Preview text (for the collapsed rows).
    pub snippet: String,
    /// Read.
    pub seen: bool,
    /// Starred.
    pub flagged: bool,
    /// Answered.
    pub answered: bool,
    /// A draft.
    pub draft: bool,
    /// Has attachments (other than images drawn in the body).
    pub has_attachments: bool,
    /// A local change has not been confirmed by the server yet.
    pub pending: bool,
    /// Whether the body is on the device.
    pub body_state: BodyState,
    /// The person tapped "Show images" on this message.
    pub remote_allowed: bool,
}

impl std::fmt::Debug for ConversationMessage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConversationMessage")
            .field("id", &self.id)
            .field("account", &self.account)
            .field("folder", &self.folder)
            .field("body_state", &self.body_state)
            .finish_non_exhaustive()
    }
}

/// A conversation (S20): its messages, oldest first.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct Conversation {
    /// The account.
    pub account: i64,
    /// The message the conversation was opened from (always in `messages`): the screen expands it
    /// and the newest one, and collapses the others.
    pub opened_id: i64,
    /// The messages, oldest first.
    pub messages: Vec<ConversationMessage>,
}

/// One attachment chip (S21).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct AttachmentInfo {
    /// Row id (what `save_attachment` takes, with the message id).
    pub id: i64,
    /// A file name safe to show: never a path; bidirectional-override and invisible characters
    /// removed so the real extension shows.
    pub file_name: String,
    /// MIME type, lower case.
    pub mime_type: String,
    /// Size in bytes.
    pub size: u64,
    /// `Content-ID`, if any.
    pub content_id: Option<String>,
    /// Drawn inside the HTML body (a `cid:` image): no chip needed.
    pub shown_inline: bool,
    /// A program, script, installer or disk image (`.exe`, `.apk`, `.bat`, `.js`, `.scr`, ...):
    /// show the chip as blocked. `save_attachment` refuses it.
    pub blocked: bool,
}

/// A message's body for display (S20, S21). `Debug` prints sizes and counts only.
#[derive(Clone, PartialEq, Eq, uniffi::Record)]
pub struct MessageBody {
    /// The message.
    pub message_id: i64,
    /// Plain text (an HTML-only message's HTML converted to text). Shown when `html_sanitised` is
    /// `None`.
    pub text: Option<String>,
    /// The **sanitised** HTML, the only HTML the renderer may get. `None` for a text-only message
    /// (and for HTML too deeply nested to render safely: then `truncated` is true and `text` has
    /// its text).
    pub html_sanitised: Option<String>,
    /// True when remote images were allowed for this load: `html_sanitised` then refers to
    /// `https` images and the renderer may use the network for images. False: the renderer has
    /// no network at all.
    pub remote_allowed: bool,
    /// Remote images left out (0 when allowed). Above 0 and not allowed: show the "Remote images
    /// blocked" bar with "Show images".
    pub blocked_remote_count: u32,
    /// Tracking pixels removed (in every mode; "Show images" does not bring them back).
    pub trackers_removed: u32,
    /// The sender is on the account's known-senders list.
    pub sender_known: bool,
    /// Something was cut at a size limit (very long text, too many parts), or the HTML was too
    /// complex to render safely (past the sanitiser's limits or its 3 s deadline): then
    /// `html_sanitised` is `None` and `text` holds the message's text. The core remembers that
    /// and never tries the HTML of that message again.
    pub truncated: bool,
    /// The attachments, in document order.
    pub attachments: Vec<AttachmentInfo>,
}

impl std::fmt::Debug for MessageBody {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MessageBody")
            .field("message_id", &self.message_id)
            .field("text_bytes", &self.text.as_ref().map(String::len))
            .field("html_bytes", &self.html_sanitised.as_ref().map(String::len))
            .field("remote_allowed", &self.remote_allowed)
            .field("blocked_remote_count", &self.blocked_remote_count)
            .field("attachments", &self.attachments.len())
            .finish_non_exhaustive()
    }
}

/// Why a reading call failed. Fixed categories: never message content, an address, a path or
/// server text.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Error)]
pub enum ReadError {
    /// No message with this id (in any account).
    NoSuchMessage,
    /// The message has no attachment with this id.
    NoSuchAttachment,
    /// No account with this id.
    NoSuchAccount,
    /// The account is not running (its settings are unusable). Nothing to fetch with.
    AccountUnavailable,
    /// No connection, and the body is not on the device yet. Try again when online.
    Offline,
    /// The server refused the sign-in; the account waits for the person (as for any action).
    SignInNeeded,
    /// The server no longer has this message (moved or deleted elsewhere). The list updates at
    /// the next sync.
    NotOnServer,
    /// The message is larger than 50 MiB; it is not downloaded.
    TooLarge {
        /// Its size in bytes.
        size: u64,
    },
    /// What the server sent is not an email message.
    Unreadable,
    /// The server did not answer or is busy. Try again.
    ServerUnavailable,
    /// The server refused the request.
    ServerRefused,
    /// `save_attachment`: the attachment is a program or script and is never saved.
    AttachmentBlocked,
    /// `save_attachment`: `dest_dir` is not an absolute path to an existing directory.
    BadDestination,
    /// `save_attachment`: the file could not be written.
    WriteFailed,
    /// `trust_sender`: not one plain email address.
    InvalidAddress,
    /// The database failed.
    Storage {
        /// Fixed category.
        reason: StorageReason,
    },
    /// Something inside the core failed.
    Internal,
}

impl std::fmt::Display for ReadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoSuchMessage => f.write_str("no such message"),
            Self::NoSuchAttachment => f.write_str("no such attachment"),
            Self::NoSuchAccount => f.write_str("no such account"),
            Self::AccountUnavailable => f.write_str("the account is not available"),
            Self::Offline => f.write_str("offline, and the message is not on this device"),
            Self::SignInNeeded => f.write_str("sign-in needed"),
            Self::NotOnServer => f.write_str("the message is no longer on the server"),
            Self::TooLarge { size } => write!(f, "the message is too large ({size} bytes)"),
            Self::Unreadable => f.write_str("the message could not be read"),
            Self::ServerUnavailable => f.write_str("the server is not available"),
            Self::ServerRefused => f.write_str("the server refused"),
            Self::AttachmentBlocked => f.write_str("the attachment is a program and is blocked"),
            Self::BadDestination => f.write_str("the destination is not a directory"),
            Self::WriteFailed => f.write_str("the file could not be written"),
            Self::InvalidAddress => f.write_str("not an email address"),
            Self::Storage { .. } => f.write_str("storage error"),
            Self::Internal => f.write_str("internal error"),
        }
    }
}

impl std::error::Error for ReadError {}

impl From<CoreError> for ReadError {
    fn from(e: CoreError) -> Self {
        match e {
            CoreError::NoSuchMessage => Self::NoSuchMessage,
            CoreError::NoSuchAccount => Self::NoSuchAccount,
            CoreError::Storage { reason } => Self::Storage { reason },
            CoreError::SchemaTooNew => Self::Storage {
                reason: StorageReason::Migration,
            },
            _ => Self::Internal,
        }
    }
}

impl From<StorageError> for ReadError {
    fn from(e: StorageError) -> Self {
        CoreError::from(e).into()
    }
}

impl From<BodyError> for ReadError {
    fn from(e: BodyError) -> Self {
        match e {
            BodyError::NoSuchMessage => Self::NoSuchMessage,
            BodyError::UnknownAccount(_) => Self::AccountUnavailable,
            BodyError::Offline => Self::Offline,
            BodyError::NeedsSignIn | BodyError::Server(BackendError::Auth(_)) => Self::SignInNeeded,
            BodyError::NotOnServer => Self::NotOnServer,
            BodyError::TooLarge { size } => Self::TooLarge { size },
            BodyError::Unreadable => Self::Unreadable,
            BodyError::Server(BackendError::Transient(_)) => Self::ServerUnavailable,
            BodyError::Server(BackendError::Permanent(_)) => Self::ServerRefused,
            BodyError::Storage(e) => e.into(),
        }
    }
}

/// Addresses from a stored `to_json` / `cc_json` (objects with `name` and `address`, or plain
/// strings). A damaged value gives none.
fn addresses(json: &str) -> Vec<MailAddress> {
    let Ok(serde_json::Value::Array(items)) = serde_json::from_str::<serde_json::Value>(json)
    else {
        return Vec::new();
    };
    let text = |v: Option<&serde_json::Value>| {
        v.and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
    };
    items
        .iter()
        .filter_map(|item| match item {
            serde_json::Value::String(s) => Some(MailAddress {
                name: None,
                address: text(Some(&serde_json::Value::String(s.clone()))),
            }),
            serde_json::Value::Object(o) => Some(MailAddress {
                name: text(o.get("name")),
                address: text(o.get("address")),
            }),
            _ => None,
        })
        .filter(|a| a.name.is_some() || a.address.is_some())
        .collect()
}

fn conversation_message(r: ConversationRow) -> ConversationMessage {
    let m = r.message;
    ConversationMessage {
        id: m.id,
        account: m.account_id,
        folder: m.folder_id,
        folder_special_use: r.folder_special_use,
        sender_name: m.from_name,
        sender_address: m.from_addr,
        to: addresses(&r.to_json),
        cc: addresses(&r.cc_json),
        date_ms: m.date_ts,
        subject: m.subject,
        snippet: m.snippet,
        seen: m.is_seen,
        flagged: m.is_flagged,
        answered: m.is_answered,
        draft: m.is_draft,
        has_attachments: m.has_attachments,
        pending: m.pending_ops > 0,
        body_state: if m.body_state == "full" {
            BodyState::Loaded
        } else {
            BodyState::NotLoaded
        },
        remote_allowed: r.remote_allowed,
    }
}

/// Writes `bytes` to a new file named after `name` (made safe again here) **inside** `dir` and
/// returns its path. Never writes outside `dir`: the name is reduced to one plain component, the
/// result's parent must be `dir` itself (after resolving `dir`), and the file is created with
/// `create_new`, which never follows a link left at that name and never overwrites; a taken name
/// gets ` (1)`, ` (2)`, ... A file that fails halfway is removed.
pub(crate) fn write_into(
    dir: &Path,
    name: &str,
    mime_type: &str,
    bytes: &[u8],
) -> Result<PathBuf, ReadError> {
    if !dir.is_absolute() {
        return Err(ReadError::BadDestination);
    }
    let dir = std::fs::canonicalize(dir).map_err(|_| ReadError::BadDestination)?;
    if !std::fs::metadata(&dir).is_ok_and(|m| m.is_dir()) {
        return Err(ReadError::BadDestination);
    }
    let safe = files::safe_file_name(Some(name), mime_type);
    // The name is checked again as it will be written: an executable is never written, whatever
    // the stored flag said.
    if files::is_executable(&safe, mime_type) {
        return Err(ReadError::AttachmentBlocked);
    }
    let (stem, ext) = match safe.rfind('.') {
        Some(i) if i > 0 => (&safe[..i], &safe[i..]),
        _ => (safe.as_str(), ""),
    };
    for n in 0..=MAX_NAME_TRIES {
        let candidate = if n == 0 {
            safe.clone()
        } else {
            format!("{stem} ({n}){ext}")
        };
        if files::is_executable(&candidate, mime_type) {
            return Err(ReadError::AttachmentBlocked);
        }
        let path = dir.join(&candidate);
        // One plain component, directly in `dir`.
        if path.parent() != Some(dir.as_path())
            || path.file_name().and_then(|f| f.to_str()) != Some(candidate.as_str())
        {
            return Err(ReadError::BadDestination);
        }
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(mut file) => {
                let written = file.write_all(bytes).and_then(|()| file.sync_all());
                return match written {
                    Ok(()) => Ok(path),
                    Err(_) => {
                        drop(file);
                        let _ = std::fs::remove_file(&path);
                        Err(ReadError::WriteFailed)
                    }
                };
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(_) => return Err(ReadError::WriteFailed),
        }
    }
    Err(ReadError::WriteFailed)
}

impl Core {
    /// The message's account and the message row, or `NoSuchMessage`.
    fn owner(&self, message_id: i64) -> Result<croft_storage::messages::Message, ReadError> {
        self.inner
            .resolve(&[message_id])?
            .into_iter()
            .next()
            .ok_or(ReadError::NoSuchMessage)
    }
}

#[uniffi::export]
impl Core {
    /// The conversation `message_id` belongs to (S20), oldest first, across the account's folders:
    /// Sent is included; Trash, Junk and Drafts only when the conversation is opened from a
    /// message there; a message in two folders is listed once. Built from the stored thread key
    /// (References / In-Reply-To). Reads the database only: quick, works offline.
    pub fn open_conversation(&self, message_id: i64) -> Result<Conversation, ReadError> {
        let m = self.owner(message_id)?;
        let rows = self
            .inner
            .handle
            .read(|db| reading::conversation(db.connection(), m.account_id, m.id))?;
        Ok(Conversation {
            account: m.account_id,
            opened_id: m.id,
            messages: rows.into_iter().map(conversation_message).collect(),
        })
    }

    /// The message's body for display (S20, S21). When it is not on the device yet it is fetched
    /// first (the whole message, once; never marking it read: mark it read with `mark_seen` after
    /// the setting's delay), so this waits for the network then; a stored body is returned
    /// offline. `remote_images` is the person's setting: the core decides from it, the message's
    /// "Show images" ([`Core::allow_remote_content`]) and the known-senders list whether remote
    /// images are in `html_sanitised` (see [`MessageBody::remote_allowed`]).
    ///
    /// Async: runs on the core's runtime (parsing and sanitising on a blocking thread). If the
    /// call is cancelled the fetch still finishes and is stored.
    pub async fn load_body(
        &self,
        message_id: i64,
        remote_images: RemoteImagesSetting,
    ) -> Result<MessageBody, ReadError> {
        let m = self.owner(message_id)?;
        let inner = self.inner.clone();
        let task = self.spawn(
            async move {
                let account = m.account_id;
                let id = m.id;
                inner.handle.fetch_body(account, id).await?;
                let handle = inner.handle.clone();
                tokio::task::spawn_blocking(move || {
                    let (stored, attachments, message_allowed, sender_known) =
                        handle.read(|db| {
                            let c = db.connection();
                            let sender = m.from_addr.as_deref().and_then(normalise_address);
                            Ok::<_, ReadError>((
                                reading::body(c, account, id)?.ok_or(ReadError::NoSuchMessage)?,
                                reading::attachments(c, account, id)?,
                                reading::remote_allowed(c, account, id)?,
                                match sender {
                                    Some(s) => reading::is_trusted(c, account, &s)?,
                                    None => false,
                                },
                            ))
                        })?;
                    let allowed =
                        remote_allowed(remote_images.into(), message_allowed, sender_known);
                    // Marked when its HTML could not be rendered once: never tried again.
                    let too_complex = stored.too_complex;
                    let mut body = MessageBody {
                        message_id: id,
                        text: stored.text,
                        html_sanitised: stored.html_clean,
                        remote_allowed: false,
                        blocked_remote_count: stored.remote_blocked,
                        trackers_removed: stored.trackers_removed,
                        sender_known,
                        truncated: stored.truncated,
                        attachments: attachments
                            .into_iter()
                            .map(|a| AttachmentInfo {
                                id: a.id,
                                file_name: a.file_name,
                                mime_type: a.mime_type,
                                size: a.size,
                                content_id: a.content_id,
                                shown_inline: a.shown_inline,
                                blocked: a.executable,
                            })
                            .collect(),
                    };
                    if allowed && !too_complex && body.html_sanitised.is_some() {
                        // Sanitised again from the stored message, keeping remote images.
                        let raw = Zeroizing::new(
                            handle
                                .read(|db| reading::raw(db.connection(), account, id))?
                                .ok_or(ReadError::Unreadable)?,
                        );
                        let rendered =
                            croft_reader::render(&raw, true).map_err(|_| ReadError::Unreadable)?;
                        if rendered.too_complex {
                            // Too complex with remote images (a deterministic outcome): keep the
                            // stored (blocked) HTML, and never try again.
                            handle.read(|db| {
                                reading::set_too_complex(db.connection(), account, id)
                            })?;
                        } else if rendered.html_pending || rendered.html.is_none() {
                            // The sanitiser was busy or past its deadline: the stored (blocked)
                            // HTML for now, nothing marked; the next open tries again.
                        } else {
                            body.html_sanitised = rendered.html;
                            body.blocked_remote_count = rendered.blocked_remote;
                            body.trackers_removed = rendered.trackers_removed;
                            body.remote_allowed = true;
                        }
                    }
                    Ok(body)
                })
                .await
                .map_err(|_| ReadError::Internal)?
            },
            false,
        );
        task.await.map_err(|_| ReadError::Internal)?
    }

    /// Saves attachment `attachment_id` of message `message_id` into `dest_dir` and returns the
    /// file's path. `dest_dir` is the app's own cache directory (absolute, existing); the app
    /// shares the file through its `FileProvider` with a temporary grant, never as `file://`.
    ///
    /// The file is always created **inside** `dest_dir` (the name is the attachment's safe name,
    /// with ` (1)`, ` (2)`, ... when taken; never a path, never overwriting, never following a
    /// link). Executables are refused (`AttachmentBlocked`). Uses the stored message when it is
    /// there (offline), else fetches it first. Async: runs on the core's runtime.
    pub async fn save_attachment(
        &self,
        message_id: i64,
        attachment_id: i64,
        dest_dir: String,
    ) -> Result<String, ReadError> {
        let m = self.owner(message_id)?;
        let inner = self.inner.clone();
        let task = self.spawn(
            async move {
                let (account, id) = (m.account_id, m.id);
                let attachment = inner
                    .handle
                    .read(|db| reading::attachment(db.connection(), account, id, attachment_id))?
                    .ok_or(ReadError::NoSuchAttachment)?;
                if attachment.executable {
                    return Err(ReadError::AttachmentBlocked);
                }
                inner.handle.fetch_body(account, id).await?;
                let handle = inner.handle.clone();
                tokio::task::spawn_blocking(move || {
                    let raw = Zeroizing::new(
                        handle
                            .read(|db| reading::raw(db.connection(), account, id))?
                            .ok_or(ReadError::NoSuchAttachment)?,
                    );
                    let bytes = Zeroizing::new(
                        croft_reader::part_content(&raw, &attachment.part_id)
                            .map_err(|_| ReadError::Unreadable)?
                            .ok_or(ReadError::NoSuchAttachment)?,
                    );
                    let path = write_into(
                        Path::new(&dest_dir),
                        &attachment.file_name,
                        &attachment.mime_type,
                        &bytes,
                    )?;
                    path.to_str()
                        .map(str::to_owned)
                        .ok_or(ReadError::WriteFailed)
                })
                .await
                .map_err(|_| ReadError::Internal)?
            },
            false,
        );
        task.await.map_err(|_| ReadError::Internal)?
    }

    /// "Show images" on one message: from now on `load_body` with the `Ask` or `KnownSenders`
    /// setting includes its remote images (`Never` still blocks them). Call `load_body` again
    /// afterwards.
    pub fn allow_remote_content(&self, message_id: i64) -> Result<(), ReadError> {
        let m = self.owner(message_id)?;
        let done = self
            .inner
            .handle
            .read(|db| reading::set_remote_allowed(db.connection(), m.account_id, m.id, true))?;
        if done {
            Ok(())
        } else {
            Err(ReadError::NoSuchMessage)
        }
    }

    /// "Always show images from this sender": adds `address` to the account's known senders
    /// (stored in lower case). With the `KnownSenders` setting their messages load remote images
    /// automatically. Adding one twice is harmless.
    pub fn trust_sender(&self, account_id: i64, address: String) -> Result<(), ReadError> {
        let address = normalise_address(&address).ok_or(ReadError::InvalidAddress)?;
        let done = self
            .inner
            .handle
            .read(|db| reading::trust_sender(db.connection(), account_id, &address, now_ms()))?;
        if done {
            Ok(())
        } else {
            Err(ReadError::NoSuchAccount)
        }
    }

    /// Removes `address` from the account's known senders (S49's list). Returns true when it was
    /// there.
    pub fn untrust_sender(&self, account_id: i64, address: String) -> Result<bool, ReadError> {
        let address = normalise_address(&address).ok_or(ReadError::InvalidAddress)?;
        self.known_account(account_id)?;
        Ok(self
            .inner
            .handle
            .read(|db| reading::untrust_sender(db.connection(), account_id, &address))?)
    }

    /// The account's known senders, sorted (S49's list).
    pub fn trusted_senders(&self, account_id: i64) -> Result<Vec<String>, ReadError> {
        self.known_account(account_id)?;
        Ok(self
            .inner
            .handle
            .read(|db| reading::trusted_senders(db.connection(), account_id))?)
    }
}

impl Core {
    fn known_account(&self, account_id: i64) -> Result<(), ReadError> {
        let exists = self
            .inner
            .handle
            .read(|db| accounts::get(db.connection(), account_id))?
            .is_some();
        if exists {
            Ok(())
        } else {
            Err(ReadError::NoSuchAccount)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn addresses_read_objects_strings_and_nothing_else() {
        let a = addresses(r#"[{"name":"Ben","address":"ben@example.com"},"c@example.com",3,{}]"#);
        assert_eq!(a.len(), 2);
        assert_eq!(a[0].name.as_deref(), Some("Ben"));
        assert_eq!(a[1].address.as_deref(), Some("c@example.com"));
        assert!(addresses("not json").is_empty());
    }

    #[test]
    fn files_land_inside_the_folder_whatever_the_name() {
        let dir = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(dir.path()).unwrap();
        for name in [
            "../../escape.txt",
            "/etc/passwd",
            "..\\..\\win.ini",
            "..",
            "",
            "a/../../b.txt",
            "nul",
            "\u{202E}fdp.txt",
            "../\u{2800}/x.txt",
        ] {
            let path = write_into(dir.path(), name, "text/plain", b"x").unwrap();
            assert_eq!(path.parent(), Some(root.as_path()), "{name:?} -> {path:?}");
            assert_eq!(std::fs::read(&path).unwrap(), b"x");
        }
        // Nothing appeared next to the folder.
        let parent = root.parent().unwrap();
        assert!(!parent.join("escape.txt").exists());
    }

    #[test]
    fn an_executable_name_is_never_written_even_when_a_cut_reveals_it() {
        let dir = tempfile::tempdir().unwrap();
        let name = format!("{}.apk.{}", "A".repeat(115), "b".repeat(200));
        assert_eq!(
            write_into(dir.path(), &name, "application/octet-stream", b"x"),
            Err(ReadError::AttachmentBlocked)
        );
        assert_eq!(
            write_into(
                dir.path(),
                "setup.exe\u{2800}",
                "application/octet-stream",
                b"x"
            ),
            Err(ReadError::AttachmentBlocked)
        );
        assert_eq!(
            write_into(dir.path(), "\u{202E}txt.exe", "text/plain", b"x"),
            Err(ReadError::AttachmentBlocked)
        );
        assert_eq!(
            write_into(dir.path(), "notes.txt", "text/html", b"x"),
            Err(ReadError::AttachmentBlocked)
        );
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    }

    #[test]
    fn a_taken_name_gets_a_number_and_nothing_is_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        let first = write_into(dir.path(), "report.pdf", "application/pdf", b"one").unwrap();
        let second = write_into(dir.path(), "report.pdf", "application/pdf", b"two").unwrap();
        assert!(first.ends_with("report.pdf"));
        assert!(second.ends_with("report (1).pdf"));
        assert_eq!(std::fs::read(&first).unwrap(), b"one");
    }

    #[cfg(unix)]
    #[test]
    fn a_link_left_at_the_name_is_never_followed() {
        let dir = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let target = outside.path().join("victim.txt");
        std::fs::write(&target, b"keep").unwrap();
        std::os::unix::fs::symlink(&target, dir.path().join("note.txt")).unwrap();
        let path = write_into(dir.path(), "note.txt", "text/plain", b"new").unwrap();
        assert!(path.ends_with("note (1).txt"));
        assert_eq!(std::fs::read(&target).unwrap(), b"keep");
    }

    #[test]
    fn a_relative_or_missing_folder_is_refused() {
        assert_eq!(
            write_into(Path::new("relative/dir"), "a.txt", "text/plain", b"x"),
            Err(ReadError::BadDestination)
        );
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("nope");
        assert_eq!(
            write_into(&missing, "a.txt", "text/plain", b"x"),
            Err(ReadError::BadDestination)
        );
        let file = dir.path().join("file");
        std::fs::write(&file, b"").unwrap();
        assert_eq!(
            write_into(&file, "a.txt", "text/plain", b"x"),
            Err(ReadError::BadDestination)
        );
    }
}

#[cfg(test)]
mod too_complex_tests {
    use crate::reading::RemoteImagesSetting;
    use crate::tests::{block, wait_for, MemProvider, TempDb};
    use croft_storage::reading;

    #[test]
    fn a_message_marked_too_complex_is_never_rendered_again() {
        let db = TempDb::new();
        let core = db.open(&MemProvider::new());
        let account = block(core.add_demo_account()).unwrap().id;
        let rows = || {
            let folders = core.list_folders(account).unwrap();
            let Some(inbox) = folders.iter().find(|f| f.path == "INBOX") else {
                return Vec::new();
            };
            core.list_messages(
                crate::MessageScope::Folder {
                    account,
                    folder_id: inbox.id,
                },
                None,
                500,
            )
            .unwrap()
            .rows
        };
        block(wait_for(&core, "sample inbox", || rows().len() >= 10));
        // The sample message with a remote image.
        let (id, _) = rows()
            .iter()
            .map(|r| {
                (
                    r.id,
                    block(core.load_body(r.id, RemoteImagesSetting::Ask)).unwrap(),
                )
            })
            .find(|(_, b)| b.blocked_remote_count > 0)
            .unwrap();
        core.allow_remote_content(id).unwrap();
        assert!(
            block(core.load_body(id, RemoteImagesSetting::Ask))
                .unwrap()
                .remote_allowed
        );
        // A render that timed out once: the mark is stored, and no later load tries again.
        core.inner
            .handle
            .read(|db| reading::set_too_complex(db.connection(), account, id))
            .unwrap();
        let body = block(core.load_body(id, RemoteImagesSetting::Ask)).unwrap();
        assert!(!body.remote_allowed, "the slow path is not tried again");
        assert!(
            body.html_sanitised.is_some(),
            "the stored, blocked HTML is still shown"
        );
    }
}
