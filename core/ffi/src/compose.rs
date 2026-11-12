// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Composing and sending (T18c): drafts, the Outbox actions, recipient suggestions, reply and
//! forward fields, and identities. The calls are on [`Core`]; `core/ffi/README.md` has the
//! "compose lifecycle" for the screens.
//!
//! # Where things live
//!
//! - A **draft** is a row in the database (version 6 of the schema): saved on the device by
//!   [`Core::save_draft`], copied to the server's Drafts folder by [`Core::sync_draft_to_server`]
//!   (APPEND the new version, then delete the old one, found by the draft's Message-ID, which never
//!   changes). Attachments are stored with the draft.
//! - **Send** ([`Core::send`]) writes the Outbox row (with the draft's id in it, in one statement)
//!   and leaves the draft in place, locked, until the message is sent; Undo, Edit and Discard find
//!   the draft again through it.
//! - The core finds out that a message was **sent** by watching the Outbox ([`reconcile`]): the
//!   draft is then finished, its copy on the server deleted, and the recipients remembered for
//!   the suggestions.
//!
//! # Never twice
//!
//! The rules that keep a message from being sent twice are in the Outbox and the engine (the row is
//! marked `sending` on disk before the server is contacted; a message that may have reached the
//! server is Failed and never retried by itself). This file adds three more:
//!
//! - A draft that has an Outbox message is locked: `save_draft`, `send` and `discard_draft` refuse
//!   it ([`ComposeError::InOutbox`]), so a second Send cannot make a second message from it.
//! - The link between the draft and its Outbox message is written with the Outbox row, so there is
//!   no moment in which the message exists and the draft does not know it.
//! - Before a message is taken back (Undo, Edit, Discard) the draft's link is cleared, and only
//!   afterwards is the Outbox asked. A crash in between is read as "taken back", never as "sent".
//!   All of these, and the check that finds sent messages, run under one lock.

use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::path::{Component, Path};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};

use croft_engine::compose::{
    self as mail, Address, AttachmentData, BuildError, DraftFields, Outgoing, ReplyKind,
    SourceMessage,
};
use croft_engine::{
    Change, NewMessage, OutboxActionError, OutboxEntry, OutboxId, OutboxState, SendError,
};
use croft_protocols::drafts::{delete_draft_copies, drafts_folder, save_draft_copy};
use croft_protocols::smtp::{
    MAYBE_SENT_REASON, REASON_MESSAGE, REASON_RECIPIENT, REASON_SENDER, REASON_TOO_LARGE,
};
use croft_protocols::BackendError;
use croft_storage::accounts::{self, Account};
use croft_storage::drafts::{self, Draft, DraftContent as StoredContent, DraftState};
use croft_storage::{folders, identities, messages, recipients, Db, StorageError};
use rusqlite::{params, Connection};
use tokio::sync::broadcast;

use crate::core::{now_ms, Gone, Inner};
use crate::error::{CoreError, StorageReason};
use crate::Core;

// ------------------------------------------------------------------------------------------
// Limits
// ------------------------------------------------------------------------------------------

/// The size above which the Send button is disabled and the attachment chip warns, when the server
/// has said nothing else: 25 MB, the limit most servers have.
pub const ATTACHMENT_WARN_BYTES: u64 = 25 * 1024 * 1024;
/// The most one attachment may hold. The app may add attachments beyond
/// [`ATTACHMENT_WARN_BYTES`] to show the warning; the core refuses what would not fit in memory.
pub const MAX_ATTACHMENT_BYTES: u64 = 40 * 1024 * 1024;
/// The most all attachments of one draft may hold together.
pub const MAX_DRAFT_ATTACHMENT_BYTES: u64 = 40 * 1024 * 1024;
/// The most attachments one draft may have.
pub const MAX_ATTACHMENTS: u32 = 50;
/// The most recipients one draft may have (To, Cc and Bcc together).
pub const MAX_RECIPIENTS: u32 = 500;
/// The longest subject, in characters (RFC 5322 line limit).
pub const MAX_SUBJECT_CHARS: u32 = 998;
/// The longest body, in bytes.
pub const MAX_BODY_BYTES: u32 = 2 * 1024 * 1024;
/// The longest address text and display name the core stores, in characters.
const MAX_ADDRESS_CHARS: usize = 320;
const MAX_NAME_CHARS: usize = 200;
/// The most suggestions one call returns.
pub const MAX_SUGGESTIONS: u32 = 20;
/// The shortest time between two copies of one draft to the server unless the app forces it (on
/// pause): 20 s, as `docs/ux/state-machines.md` says.
const SERVER_COPY_INTERVAL_MS: i64 = 20_000;

/// The numbers the compose screen needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Record)]
pub struct ComposeLimits {
    /// Attachments above this make Send disabled (the server would refuse the message), unless the
    /// server said otherwise.
    pub attachment_warn_bytes: u64,
    /// The largest single attachment the core accepts.
    pub max_attachment_bytes: u64,
    /// The largest total of attachments in one draft the core accepts.
    pub max_attachments_total_bytes: u64,
    /// The most attachments in one draft.
    pub max_attachments: u32,
    /// The most recipients in one draft.
    pub max_recipients: u32,
    /// The longest subject, in characters.
    pub max_subject_chars: u32,
    /// The longest body, in bytes.
    pub max_body_bytes: u32,
    /// The most suggestions [`Core::suggest_recipients`] returns.
    pub max_suggestions: u32,
}

/// The limits of this version of the core.
#[uniffi::export]
pub fn compose_limits() -> ComposeLimits {
    ComposeLimits {
        attachment_warn_bytes: ATTACHMENT_WARN_BYTES,
        max_attachment_bytes: MAX_ATTACHMENT_BYTES,
        max_attachments_total_bytes: MAX_DRAFT_ATTACHMENT_BYTES,
        max_attachments: MAX_ATTACHMENTS,
        max_recipients: MAX_RECIPIENTS,
        max_subject_chars: MAX_SUBJECT_CHARS,
        max_body_bytes: MAX_BODY_BYTES,
        max_suggestions: MAX_SUGGESTIONS,
    }
}

// ------------------------------------------------------------------------------------------
// Errors
// ------------------------------------------------------------------------------------------

/// Why a compose call was refused. Fixed categories: no variant carries text, an address or a path.
/// Nothing was changed unless the call says otherwise.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Error)]
pub enum ComposeError {
    /// No account with this id.
    NoSuchAccount,
    /// No draft with this id in this account (or it is finished).
    NoSuchDraft,
    /// No Outbox message with this id in this account (it was sent, or it is gone).
    NoSuchOutboxItem,
    /// No stored message with this id in this account (reply and forward).
    NoSuchMessage,
    /// The identity is not one of this account's.
    NoSuchIdentity,
    /// No attachment with this id in this draft.
    NoSuchAttachment,
    /// The draft has been sent or is waiting in the Outbox: it cannot be changed, sent again or
    /// discarded. Take it back with Undo or Edit first.
    InOutbox,
    /// Nobody to send it to: To, Cc and Bcc are all empty.
    NoRecipient,
    /// A recipient (or the sender) is not a valid address. [`Core::check_draft`] says which.
    InvalidAddress,
    /// The account cannot send: it has no SMTP server in its settings.
    CannotSend,
    /// Undo is too late: the window has closed or the message is already on its way.
    TooLate,
    /// The Outbox message is being sent right now and cannot be edited or discarded.
    WrongState,
    /// The attachment is larger than [`MAX_ATTACHMENT_BYTES`], or the attachments together would
    /// be larger than [`MAX_DRAFT_ATTACHMENT_BYTES`].
    AttachmentTooLarge,
    /// The draft already has [`MAX_ATTACHMENTS`] attachments.
    TooManyAttachments,
    /// The file could not be read: it is outside the attachment directory, not a plain file, or the
    /// path has a link or `..` in it, or the system refused. Nothing outside the one file is ever
    /// read.
    AttachmentUnreadable,
    /// `add_draft_attachment_file` before the app registered its attachment directory
    /// ([`Core::set_attachment_directory`]).
    NoAttachmentDirectory,
    /// [`Core::set_attachment_directory`]: the path is not an absolute existing folder, or the
    /// core's own database is inside it.
    BadAttachmentDirectory,
    /// [`Core::retry_outbox_item`]: the message may already have been sent and `confirmed` was
    /// false. Nothing was changed. Ask the person ("This message may already have been sent"), and
    /// call again with `confirmed = true` only if they choose Retry anyway.
    NeedsConfirmation,
    /// A field is longer than the limits ([`compose_limits`]).
    DraftTooLarge,
    /// The database failed.
    Storage {
        /// Fixed category.
        reason: StorageReason,
    },
    /// Something inside the core failed (a task panicked, the system's random source failed).
    Internal,
}

impl std::fmt::Display for ComposeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::NoSuchAccount => "no such account",
            Self::NoSuchDraft => "no such draft",
            Self::NoSuchOutboxItem => "no such outbox message",
            Self::NoSuchMessage => "no such message",
            Self::NoSuchIdentity => "no such identity",
            Self::NoSuchAttachment => "no such attachment",
            Self::InOutbox => "the draft is in the outbox",
            Self::NoRecipient => "no recipient",
            Self::InvalidAddress => "an address is not valid",
            Self::CannotSend => "the account cannot send mail",
            Self::TooLate => "too late to undo",
            Self::WrongState => "the message is being sent",
            Self::AttachmentTooLarge => "the attachments are too large",
            Self::TooManyAttachments => "too many attachments",
            Self::AttachmentUnreadable => "the file could not be read",
            Self::NoAttachmentDirectory => "no attachment directory was registered",
            Self::BadAttachmentDirectory => "the attachment directory cannot be used",
            Self::NeedsConfirmation => "the message may already have been sent",
            Self::DraftTooLarge => "a field is too long",
            Self::Storage { .. } => "storage error",
            Self::Internal => "internal error",
        })
    }
}

impl std::error::Error for ComposeError {}

impl From<StorageError> for ComposeError {
    fn from(e: StorageError) -> Self {
        match CoreError::from(e) {
            CoreError::Storage { reason } => Self::Storage { reason },
            CoreError::SchemaTooNew => Self::Storage {
                reason: StorageReason::Migration,
            },
            _ => Self::Internal,
        }
    }
}

impl From<rusqlite::Error> for ComposeError {
    fn from(e: rusqlite::Error) -> Self {
        StorageError::Sqlite(e).into()
    }
}

impl From<CoreError> for ComposeError {
    fn from(e: CoreError) -> Self {
        match e {
            CoreError::Storage { reason } => Self::Storage { reason },
            CoreError::NoSuchAccount => Self::NoSuchAccount,
            CoreError::NoSuchMessage => Self::NoSuchMessage,
            _ => Self::Internal,
        }
    }
}

impl From<Gone> for ComposeError {
    fn from(_: Gone) -> Self {
        Self::Internal
    }
}

impl From<OutboxActionError> for ComposeError {
    fn from(e: OutboxActionError) -> Self {
        match e {
            OutboxActionError::NoSuchMessage => Self::NoSuchOutboxItem,
            OutboxActionError::TooLate => Self::TooLate,
            OutboxActionError::WrongState => Self::WrongState,
            OutboxActionError::Storage(e) => e.into(),
            OutboxActionError::NoOutbox => Self::Internal,
        }
    }
}

// ------------------------------------------------------------------------------------------
// Records
// ------------------------------------------------------------------------------------------

/// An address with an optional display name, as a recipient chip holds it. Prints `***` in `Debug`.
#[derive(Clone, PartialEq, Eq, uniffi::Record)]
pub struct FfiAddress {
    /// The display name, if the person gave one (`Ann Example`).
    pub name: Option<String>,
    /// The address as typed (`ann@example.org`). Not necessarily valid: see [`Core::check_draft`].
    pub address: String,
}

impl std::fmt::Debug for FfiAddress {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("FfiAddress(***)")
    }
}

impl From<&Address> for FfiAddress {
    fn from(a: &Address) -> Self {
        Self {
            name: a.name.clone(),
            address: a.address.clone(),
        }
    }
}

impl From<&FfiAddress> for Address {
    fn from(a: &FfiAddress) -> Self {
        Self {
            name: a.name.clone(),
            address: a.address.clone(),
        }
    }
}

/// Reads one recipient as a person types or pastes it (`ann@example.org`, `Ann <ann@example.org>`,
/// `"Example, Ann" <ann@example.org>`), for turning text into a chip. `None` when no address can
/// be found. The address is **not** checked: use [`is_valid_address`] for the error chip.
#[uniffi::export]
pub fn parse_recipient(text: String) -> Option<FfiAddress> {
    mail::parse_address(&text).map(|a| FfiAddress::from(&a))
}

/// Reads a pasted list (`a@x.org, Bob <b@x.org>; c@x.org`) into recipients. At most
/// [`MAX_RECIPIENTS`].
#[uniffi::export]
pub fn parse_recipients(text: String) -> Vec<FfiAddress> {
    mail::parse_address_list(&text, MAX_RECIPIENTS as usize)
        .iter()
        .map(FfiAddress::from)
        .collect()
}

/// True when the bare address is one a server could deliver to (see `check_draft`). For the error
/// chip ("Fix the address"); never contacts a server.
#[uniffi::export]
pub fn is_valid_address(address: String) -> bool {
    mail::is_valid_address(address.trim())
}

/// What the person writes.
#[derive(Clone, PartialEq, Eq, uniffi::Record)]
pub struct DraftContent {
    /// The identity it is sent from (`FfiIdentity::id`). `None`: the account's default identity.
    pub identity: Option<i64>,
    /// To.
    pub to: Vec<FfiAddress>,
    /// Cc.
    pub cc: Vec<FfiAddress>,
    /// Bcc. Never written into a message that is sent; kept in the draft copy on the server.
    pub bcc: Vec<FfiAddress>,
    /// The subject.
    pub subject: String,
    /// The body, plain text.
    pub body: String,
    /// The Message-ID of the message this answers (`<...>`), for threading. Set by the reply
    /// helpers; pass it back unchanged.
    pub in_reply_to: Option<String>,
    /// The `References` ids, oldest first. Set by the reply helpers; pass it back unchanged.
    pub references: Vec<String>,
}

impl std::fmt::Debug for DraftContent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "DraftContent({} recipients)",
            self.to.len() + self.cc.len() + self.bcc.len()
        )
    }
}

/// An attachment in a draft, without its bytes.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct DraftAttachment {
    /// The attachment's id, for [`Core::remove_draft_attachment`].
    pub id: i64,
    /// The file name shown on the chip and sent to the recipient.
    pub name: String,
    /// The media type.
    pub mime_type: String,
    /// The size in bytes.
    pub size_bytes: u64,
}

/// Whether the draft's copy on the server is up to date.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum ServerCopy {
    /// No copy on the server yet.
    NotCopied,
    /// The copy is the latest version.
    Current,
    /// There is a copy, but it is an older version (offline, or a copy is due).
    Behind,
}

/// A stored draft, to reopen compose.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct FfiDraft {
    /// The draft's id.
    pub id: i64,
    /// Its account.
    pub account_id: i64,
    /// Its Message-ID, which it keeps for its whole life and which the sent message has too.
    pub message_id: String,
    /// What the person wrote.
    pub content: DraftContent,
    /// The attachments.
    pub attachments: Vec<DraftAttachment>,
    /// Where the copy on the server stands.
    pub server_copy: ServerCopy,
    /// The Outbox message made from this draft, if Send was tapped and the message is still the
    /// Outbox's. The draft is then locked.
    pub outbox_id: Option<u64>,
    /// When it was started, Unix milliseconds.
    pub created_ms: i64,
    /// When it was last changed, Unix milliseconds.
    pub updated_ms: i64,
}

/// A line of the Drafts list.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct DraftSummary {
    /// The draft's id.
    pub id: i64,
    /// Its account.
    pub account_id: i64,
    /// The subject (may be empty).
    pub subject: String,
    /// The first words of the body, on one line (at most 120 characters).
    pub snippet: String,
    /// Names (else addresses) of the first recipients, at most 3.
    pub to_preview: Vec<String>,
    /// Whether it has attachments.
    pub has_attachments: bool,
    /// The Outbox message made from it, if any.
    pub outbox_id: Option<u64>,
    /// When it was last changed, Unix milliseconds.
    pub updated_ms: i64,
}

/// The answer to [`Core::save_draft`].
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct SavedDraft {
    /// The draft's id (new for a first save).
    pub id: i64,
    /// Its Message-ID.
    pub message_id: String,
    /// When it was saved, Unix milliseconds.
    pub saved_ms: i64,
}

/// What stops a draft from being sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum DraftProblem {
    /// No recipient: Send is disabled, tap explains "Add a recipient".
    NoRecipient,
    /// A recipient chip has an invalid address: Send is disabled, tap explains "Fix the address".
    InvalidRecipient,
    /// The attachments together are above [`ATTACHMENT_WARN_BYTES`]: Send is disabled and the
    /// attachment chip shows the limit.
    AttachmentsTooLarge,
}

/// The facts the Send button's condition table (`docs/ux/controls.md`) is built from.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct DraftCheck {
    /// Why Send is disabled. Empty: Send is enabled (offline and "needs sign-in" never disable it).
    pub problems: Vec<DraftProblem>,
    /// The invalid recipient addresses as typed, for the error chips.
    pub invalid_recipients: Vec<String>,
    /// The subject is empty (after trimming).
    pub subject_empty: bool,
    /// The body is empty (after trimming) and there is no attachment: with no subject either, the
    /// app asks "Send without subject or text?".
    pub body_empty: bool,
    /// The attachments' total size.
    pub attachments_bytes: u64,
    /// The size above which Send is disabled ([`ATTACHMENT_WARN_BYTES`]).
    pub size_limit_bytes: u64,
}

/// What [`Core::sync_draft_to_server`] did. None of these is an error: the draft is safe on the
/// device either way, and the core copies it later when something says "later".
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum DraftSyncOutcome {
    /// A new copy is on the server and the old one is deleted.
    Saved,
    /// The copy on the server is already the latest version (or the draft is empty and has none).
    AlreadyCurrent,
    /// The last copy was less than 20 seconds ago; nothing was done. Ask again later or with
    /// `force`.
    Deferred,
    /// The device is offline. The core copies the draft when the connection returns.
    Offline,
    /// The account has no server to copy to (the sample mailbox). The draft stays on the device.
    NotApplicable,
    /// The server could not be reached or was busy. The core tries again when the connection
    /// returns.
    TryLater,
    /// The account needs the person to sign in again.
    NeedsSignIn,
    /// The server refused the copy (for example the folder is read-only). The draft stays on the
    /// device.
    Refused,
}

/// Where an Outbox message is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum OutboxItemState {
    /// Waiting: for the Undo window to end, for the network, or for a retry.
    Queued,
    /// Being sent now (progress ring). Cannot be edited or discarded.
    Sending,
    /// Not sent. Retry, Edit and Discard are possible; `reason` says why.
    Failed,
}

/// Why an Outbox message waits or failed, as a fixed category. Never the server's words.
/// [`outbox_reason_key`] gives the key to translate and [`outbox_reason_default_message`] the
/// English fallback.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum OutboxReason {
    /// The server may have accepted it but never said so (the connection dropped, or the app was
    /// stopped while it was going out). It is not sent again by itself: look in Sent, then Retry,
    /// Edit or Discard.
    MayNotHaveBeenSent,
    /// The server refused the sender address.
    SenderRefused,
    /// The server refused a recipient address.
    RecipientRefused,
    /// The server refused the message.
    MessageRefused,
    /// The message is larger than the server accepts.
    TooLarge,
    /// The server refused it for a reason that only needs another kind of server (8-bit text or an
    /// address with accents).
    ServerLimits,
    /// The server could not be reached; the core keeps trying (Queued rows).
    TryingAgain,
    /// The server could not be reached after several tries.
    GaveUp,
    /// The account needs the person to sign in again; the message waits.
    NeedsSignIn,
    /// Something inside Croft failed for this message.
    Internal,
    /// Any other reason.
    Other,
}

impl OutboxReason {
    fn key(self) -> &'static str {
        match self {
            Self::MayNotHaveBeenSent => "outbox.reason.may_not_have_been_sent",
            Self::SenderRefused => "outbox.reason.sender_refused",
            Self::RecipientRefused => "outbox.reason.recipient_refused",
            Self::MessageRefused => "outbox.reason.message_refused",
            Self::TooLarge => "outbox.reason.too_large",
            Self::ServerLimits => "outbox.reason.server_limits",
            Self::TryingAgain => "outbox.reason.trying_again",
            Self::GaveUp => "outbox.reason.gave_up",
            Self::NeedsSignIn => "outbox.reason.needs_sign_in",
            Self::Internal => "outbox.reason.internal",
            Self::Other => "outbox.reason.other",
        }
    }

    fn default_message(self) -> &'static str {
        match self {
            Self::MayNotHaveBeenSent => "May not have been sent. Check Sent before trying again.",
            Self::SenderRefused => "The server refused the sender address.",
            Self::RecipientRefused => "The server refused a recipient address.",
            Self::MessageRefused => "The server refused the message.",
            Self::TooLarge => "The message is larger than the server accepts.",
            Self::ServerLimits => "The server cannot take this message.",
            Self::TryingAgain => "Couldn't reach the server. Trying again.",
            Self::GaveUp => "Couldn't reach the server after several tries.",
            Self::NeedsSignIn => "Sign in again to send this message.",
            Self::Internal => "Something went wrong inside Croft.",
            Self::Other => "The message could not be sent.",
        }
    }
}

/// The key the screens look up in their own translations for an [`OutboxReason`], for example
/// `outbox.reason.too_large`.
#[uniffi::export]
pub fn outbox_reason_key(reason: OutboxReason) -> String {
    reason.key().to_owned()
}

/// The English text of an [`OutboxReason`] (fixed wording, no data), for when the screens have no
/// translation for the key.
#[uniffi::export]
pub fn outbox_reason_default_message(reason: OutboxReason) -> String {
    reason.default_message().to_owned()
}

/// Sorts the Outbox's recorded text into a fixed category. The text itself never leaves this
/// function (it could, for an adapter that was careless, hold an address).
fn reason_of(text: &str, state: OutboxState) -> OutboxReason {
    if text == MAYBE_SENT_REASON {
        OutboxReason::MayNotHaveBeenSent
    } else if text == REASON_SENDER {
        OutboxReason::SenderRefused
    } else if text == REASON_RECIPIENT {
        OutboxReason::RecipientRefused
    } else if text == REASON_MESSAGE {
        OutboxReason::MessageRefused
    } else if text == REASON_TOO_LARGE {
        OutboxReason::TooLarge
    } else if text.starts_with("The server does not accept") {
        OutboxReason::ServerLimits
    } else if text.starts_with("Gave up after") {
        OutboxReason::GaveUp
    } else if text == "sign-in needed" {
        OutboxReason::NeedsSignIn
    } else if text.starts_with("This message was not sent: it was handed")
        || text.starts_with("This message could not be read")
    {
        OutboxReason::Internal
    } else if state == OutboxState::Failed {
        OutboxReason::Other
    } else {
        OutboxReason::TryingAgain
    }
}

/// One row of the Outbox screen (S26).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct OutboxItem {
    /// The id for Undo, Retry, Edit and Discard.
    pub id: u64,
    /// Queued, Sending or Failed.
    pub state: OutboxItemState,
    /// When the message becomes due, Unix milliseconds: the end of the Undo window for a new
    /// message, the next try for one that is retrying.
    pub send_at_ms: i64,
    /// How many times it has been handed to the server.
    pub attempts: u32,
    /// Why it waits or failed.
    pub reason: Option<OutboxReason>,
    /// True for [`OutboxReason::MayNotHaveBeenSent`]: ask the person to check Sent before Retry.
    pub may_have_been_sent: bool,
    /// The draft it was made from (what Edit opens).
    pub draft_id: Option<i64>,
    /// The subject, from the draft (empty when there is none).
    pub subject: String,
    /// How many recipients it goes to (To, Cc and Bcc).
    pub recipient_count: u32,
    /// The first two recipients, as bare addresses.
    pub to_preview: Vec<String>,
}

/// The answer to [`Core::send`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Record)]
pub struct SendStarted {
    /// The Outbox id, for the "Sending... Undo" snackbar ([`Core::undo_send`]).
    pub outbox_id: u64,
    /// When the Undo window ends and the message becomes due, Unix milliseconds.
    pub send_at_ms: i64,
}

/// A sender identity for the From picker.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct FfiIdentity {
    /// The identity's id, for [`DraftContent::identity`].
    pub id: i64,
    /// The name recipients see.
    pub display_name: String,
    /// The address it sends from.
    pub address: String,
    /// The one compose starts with.
    pub is_default: bool,
}

/// The fields of a draft that a reply or forward fills in (nothing is stored until the app calls
/// [`Core::save_draft`]).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ReplyDraft {
    /// The fields to put in compose.
    pub content: DraftContent,
    /// False when the original's body is not stored on the device: the quote is empty (a reply) or
    /// the forward has only the original's header lines. The app can fetch the body first (when
    /// the reading calls exist) and ask again.
    pub quote_included: bool,
}

// ------------------------------------------------------------------------------------------
// The compose state of the core
// ------------------------------------------------------------------------------------------

/// What [`Core`] keeps for composing: its own connection to the database file (so a large
/// attachment never holds the engine's lock), the lock that orders the Outbox actions against the
/// check for sent messages, and the pacing of copies to the server.
pub(crate) struct Compose {
    /// The core's runtime, to start background work from calls made on the app's threads.
    rt: tokio::runtime::Handle,
    db: Mutex<Db>,
    /// Held by Send, Undo, Edit, Discard of an Outbox message, and the check for sent messages:
    /// they never interleave.
    actions: Mutex<()>,
    /// Held while a draft's copy on the server is made or deleted: one at a time, so a draft is
    /// never appended twice at once.
    server: tokio::sync::Mutex<()>,
    /// (account, draft) -> time of the last attempt to copy it, Unix milliseconds.
    last_attempt: Mutex<HashMap<(i64, i64), i64>>,
    interval_ms: AtomicI64,
    /// The one folder attachment files may be read from (registered by the app).
    attachment_dir: Mutex<Option<AttachmentDir>>,
    /// The database file, to keep it out of the attachment directory.
    db_path: std::path::PathBuf,
    /// Background runs of `kick_compose` that have started and not finished (tests wait for 0).
    kicks: std::sync::atomic::AtomicUsize,
}

impl Compose {
    pub(crate) fn new(db: Db, rt: tokio::runtime::Handle, db_path: std::path::PathBuf) -> Self {
        Self {
            rt,
            attachment_dir: Mutex::new(None),
            db_path,
            db: Mutex::new(db),
            actions: Mutex::new(()),
            server: tokio::sync::Mutex::new(()),
            last_attempt: Mutex::new(HashMap::new()),
            interval_ms: AtomicI64::new(SERVER_COPY_INTERVAL_MS),
            kicks: std::sync::atomic::AtomicUsize::new(0),
        }
    }

    fn db(&self) -> MutexGuard<'_, Db> {
        self.db.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn actions(&self) -> MutexGuard<'_, ()> {
        self.actions.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn attachment_dir(&self) -> Option<AttachmentDir> {
        self.attachment_dir
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn attempts(&self) -> MutexGuard<'_, HashMap<(i64, i64), i64>> {
        self.last_attempt
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// True when no background run of the check for sent messages, the clean-up and the copies is
    /// going on (tests).
    #[cfg(test)]
    pub(crate) fn is_idle(&self) -> bool {
        self.kicks.load(Ordering::SeqCst) == 0
    }

    /// Changes the shortest time between two unforced copies of one draft (tests).
    #[cfg(test)]
    pub(crate) fn set_interval_ms(&self, ms: i64) {
        self.interval_ms.store(ms, Ordering::SeqCst);
    }
}

fn random<const N: usize>() -> Result<[u8; N], ComposeError> {
    let mut bytes = [0u8; N];
    getrandom::fill(&mut bytes).map_err(|_| ComposeError::Internal)?;
    Ok(bytes)
}

fn account_of(conn: &Connection, id: i64) -> Result<Account, ComposeError> {
    accounts::get(conn, id)?.ok_or(ComposeError::NoSuchAccount)
}

// ------------------------------------------------------------------------------------------
// Converting what the app passes
// ------------------------------------------------------------------------------------------

fn clean_name(name: &Option<String>) -> Result<Option<String>, ComposeError> {
    let Some(name) = name else { return Ok(None) };
    let name = name.trim();
    if name.chars().count() > MAX_NAME_CHARS {
        return Err(ComposeError::DraftTooLarge);
    }
    Ok((!name.is_empty()).then(|| name.to_owned()))
}

fn clean_list(list: &[FfiAddress]) -> Result<Vec<Address>, ComposeError> {
    let mut out = Vec::with_capacity(list.len());
    for a in list {
        let address = a.address.trim();
        if address.chars().count() > MAX_ADDRESS_CHARS {
            return Err(ComposeError::DraftTooLarge);
        }
        if address.is_empty() {
            continue;
        }
        out.push(Address {
            name: clean_name(&a.name)?,
            address: address.to_owned(),
        });
    }
    Ok(out)
}

fn stored_content(c: &DraftContent) -> Result<StoredContent, ComposeError> {
    if c.to.len() + c.cc.len() + c.bcc.len() > MAX_RECIPIENTS as usize
        || c.subject.chars().count() > MAX_SUBJECT_CHARS as usize
        || c.body.len() > MAX_BODY_BYTES as usize
        || c.references.len() > 500
        || c.in_reply_to.as_ref().is_some_and(|i| i.len() > 400)
        || c.references.iter().any(|r| r.len() > 400)
    {
        return Err(ComposeError::DraftTooLarge);
    }
    Ok(StoredContent {
        identity_id: c.identity,
        to: clean_list(&c.to)?,
        cc: clean_list(&c.cc)?,
        bcc: clean_list(&c.bcc)?,
        subject: c.subject.clone(),
        body: c.body.clone(),
        in_reply_to: c.in_reply_to.clone(),
        references: c.references.clone(),
    })
}

fn public_content(c: &StoredContent) -> DraftContent {
    let list = |l: &[Address]| l.iter().map(FfiAddress::from).collect();
    DraftContent {
        identity: c.identity_id,
        to: list(&c.to),
        cc: list(&c.cc),
        bcc: list(&c.bcc),
        subject: c.subject.clone(),
        body: c.body.clone(),
        in_reply_to: c.in_reply_to.clone(),
        references: c.references.clone(),
    }
}

fn size(bytes: i64) -> u64 {
    u64::try_from(bytes).unwrap_or(0)
}

fn attachment_infos(
    conn: &Connection,
    account: i64,
    draft: i64,
) -> Result<Vec<DraftAttachment>, ComposeError> {
    Ok(drafts::attachments(conn, account, draft)?
        .into_iter()
        .map(|a| DraftAttachment {
            id: a.id,
            name: a.name,
            mime_type: a.mime_type,
            size_bytes: size(a.size),
        })
        .collect())
}

fn server_copy_of(d: &Draft) -> ServerCopy {
    // A folder noted before a copy was made (`plan_server_copy`) is not a copy: only a recorded
    // revision is.
    if d.server_folder.is_none() || d.synced_revision == 0 {
        ServerCopy::NotCopied
    } else if d.synced_revision >= d.revision {
        ServerCopy::Current
    } else {
        ServerCopy::Behind
    }
}

/// The Outbox message that belongs to the draft, if there is one (in any state, including the ones
/// the Outbox screen does not list).
fn entry_for(entries: &[OutboxEntry], draft: i64) -> Option<&OutboxEntry> {
    entries.iter().find(|e| e.draft == Some(draft))
}

fn public_draft(
    conn: &Connection,
    d: &Draft,
    entries: &[OutboxEntry],
) -> Result<FfiDraft, ComposeError> {
    Ok(FfiDraft {
        id: d.id,
        account_id: d.account_id,
        message_id: d.message_id.clone(),
        content: public_content(&d.content),
        attachments: attachment_infos(conn, d.account_id, d.id)?,
        server_copy: server_copy_of(d),
        outbox_id: entry_for(entries, d.id).map(|e| e.id),
        created_ms: d.created_at,
        updated_ms: d.updated_at,
    })
}

// ------------------------------------------------------------------------------------------
// Building the message
// ------------------------------------------------------------------------------------------

/// The sender of a draft: its identity, else the account's default one.
fn sender_of(
    conn: &Connection,
    account: &Account,
    identity: Option<i64>,
) -> Result<(i64, Address), ComposeError> {
    let all = identities::list_or_create(conn, account.id, &account.label, &account.email)?;
    let chosen = match identity {
        Some(id) => all.iter().find(|i| i.id == id),
        None => all.first(),
    }
    .ok_or(ComposeError::NoSuchIdentity)?;
    let name = chosen.display_name.trim();
    Ok((
        chosen.id,
        Address {
            name: (!name.is_empty()).then(|| name.to_owned()),
            address: chosen.address.clone(),
        },
    ))
}

/// Writes the draft as a message: `as_draft` for the copy on the server, else the message to send.
fn build(
    conn: &Connection,
    account: &Account,
    draft: &Draft,
    as_draft: bool,
) -> Result<(Vec<u8>, i64, Address), ComposeError> {
    let (identity_id, from) = sender_of(conn, account, draft.content.identity_id)?;
    let files = drafts::attachment_data(conn, account.id, draft.id)?;
    let attachments: Vec<AttachmentData<'_>> = files
        .iter()
        .map(|(name, mime_type, bytes)| AttachmentData {
            name,
            mime_type,
            bytes,
        })
        .collect();
    let c = &draft.content;
    let mime = mail::build_message(&Outgoing {
        from: &from,
        to: &c.to,
        cc: &c.cc,
        bcc: &c.bcc,
        subject: &c.subject,
        body: &c.body,
        in_reply_to: c.in_reply_to.as_deref(),
        references: &c.references,
        message_id: &draft.message_id,
        date_ms: now_ms(),
        as_draft,
        attachments: &attachments,
        boundary_seed: random()?,
    })
    .map_err(|e| match e {
        BuildError::NoRecipient => ComposeError::NoRecipient,
        BuildError::InvalidAddress => ComposeError::InvalidAddress,
        BuildError::BadMessageId => ComposeError::Internal,
    })?;
    Ok((mime, identity_id, from))
}

/// To, Cc and Bcc as bare addresses, each once (compared without regard to case).
fn envelope_recipients(c: &StoredContent) -> Vec<String> {
    let mut seen = HashSet::new();
    c.to.iter()
        .chain(&c.cc)
        .chain(&c.bcc)
        .map(|a| a.address.trim().to_owned())
        .filter(|a| seen.insert(a.to_lowercase()))
        .collect()
}

fn draft_is_empty(conn: &Connection, d: &Draft) -> Result<bool, ComposeError> {
    let c = &d.content;
    Ok(c.to.is_empty()
        && c.cc.is_empty()
        && c.bcc.is_empty()
        && c.subject.trim().is_empty()
        && c.body.trim().is_empty()
        && drafts::attachments(conn, d.account_id, d.id)?.is_empty())
}

// ------------------------------------------------------------------------------------------
// Reading a file the app names
// ------------------------------------------------------------------------------------------

/// The one folder attachment files may be read from, as the app registered it
/// ([`Core::set_attachment_directory`]).
#[derive(Clone)]
pub(crate) struct AttachmentDir {
    /// The path as the app gave it (absolute, no `..`). The app's cache path may itself go through
    /// a link (`/data/user/0/...` is `/data/data/...` on Android), so a file path is accepted under
    /// this spelling or under the canonical one.
    given: std::path::PathBuf,
    /// The same folder with every link resolved.
    canonical: std::path::PathBuf,
}

/// Reads one file for an attachment, and nothing else. The path must lie under the registered
/// attachment folder, with **no link anywhere below it** and no `..`; it must name a plain file with
/// exactly one name (no hard link), of at most `cap` bytes.
///
/// The check and the open are not one step, so the open is made to refuse what the check forbids:
/// `O_NOFOLLOW` (a final link is an error), `O_NONBLOCK` (a pipe swapped in at the last moment is
/// opened without waiting, and refused by the next step), then the opened file itself is examined
/// (`fstat`): a regular file, one link, the same device and inode as the file that was checked, and
/// its size. The path is resolved once more after the open and must still be where it was. What
/// remains is a swap of a folder **inside** the app's private cache folder during these few
/// microseconds, which needs a process that can already write there.
#[cfg(unix)]
fn read_capped_file(dir: &AttachmentDir, path: &str, cap: u64) -> Result<Vec<u8>, ComposeError> {
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt};

    let bad = || ComposeError::AttachmentUnreadable;
    let p = Path::new(path);
    if !p.is_absolute() || p.components().any(|c| matches!(c, Component::ParentDir)) {
        return Err(bad());
    }
    let rel = p
        .strip_prefix(&dir.given)
        .or_else(|_| p.strip_prefix(&dir.canonical))
        .map_err(|_| bad())?;
    let parts: Vec<_> = rel.components().collect();
    if parts.is_empty() || !parts.iter().all(|c| matches!(c, Component::Normal(_))) {
        return Err(bad());
    }
    // Walk down from the canonical folder: nothing on the way may be a link; the last part must be
    // a plain file.
    let mut here = dir.canonical.clone();
    let mut checked = None;
    for (i, part) in parts.iter().enumerate() {
        here.push(part);
        let md = std::fs::symlink_metadata(&here).map_err(|_| bad())?;
        let last = i + 1 == parts.len();
        if md.file_type().is_symlink() || (last && !md.is_file()) || (!last && !md.is_dir()) {
            return Err(bad());
        }
        if last {
            checked = Some(md);
        }
    }
    let checked = checked.ok_or_else(bad)?;
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC)
        .open(&here)
        .map_err(|_| bad())?;
    let opened = file.metadata().map_err(|_| bad())?;
    if !opened.is_file()
        || opened.nlink() != 1
        || opened.dev() != checked.dev()
        || opened.ino() != checked.ino()
    {
        return Err(bad());
    }
    if opened.len() > cap {
        return Err(ComposeError::AttachmentTooLarge);
    }
    // Still the same place after the open (no folder on the way became a link meanwhile).
    if std::fs::canonicalize(&here).map_err(|_| bad())? != here {
        return Err(bad());
    }
    let mut bytes = Vec::with_capacity(usize::try_from(opened.len()).unwrap_or(0));
    // One byte more than the cap shows a file that grew (or a file that reports no size, as the
    // files of `/proc` do); the cap holds whatever the size said.
    file.take(cap.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|_| bad())?;
    if bytes.len() as u64 > cap {
        return Err(ComposeError::AttachmentTooLarge);
    }
    Ok(bytes)
}

/// Where the checks above cannot be made (no Unix file model), no file is read.
#[cfg(not(unix))]
fn read_capped_file(_: &AttachmentDir, _: &str, _: u64) -> Result<Vec<u8>, ComposeError> {
    Err(ComposeError::AttachmentUnreadable)
}

/// A file name for the chip: no folders, no control characters, at most 200 characters.
fn clean_file_name(name: &str) -> String {
    let base = name.rsplit(['/', '\\']).next().unwrap_or("");
    let text: String = base
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .take(MAX_NAME_CHARS)
        .collect();
    let text = text.trim();
    if text.is_empty() || text == "." || text == ".." {
        "attachment".to_owned()
    } else {
        text.to_owned()
    }
}

fn clean_type(mime_type: &str) -> String {
    let t: String = mime_type.trim().chars().take(100).collect();
    if t.is_empty() {
        "application/octet-stream".to_owned()
    } else {
        t
    }
}

// ------------------------------------------------------------------------------------------
// Work that needs the Inner
// ------------------------------------------------------------------------------------------

impl Inner {
    /// Runs `f` with the compose database.
    fn compose_db<T>(
        &self,
        f: impl FnOnce(&Connection) -> Result<T, ComposeError>,
    ) -> Result<T, ComposeError> {
        let db = self.compose.db();
        f(db.connection())
    }

    /// The draft of this account, in any state. `NoSuchDraft` for another account's id.
    fn draft_of(conn: &Connection, account: i64, id: i64) -> Result<Draft, ComposeError> {
        drafts::get(conn, account, id)?.ok_or(ComposeError::NoSuchDraft)
    }

    /// An editable draft: not finished, and without an Outbox message. A draft whose message exists
    /// but whose state was not updated yet (a crash between the two writes) is repaired and
    /// refused.
    fn editable_draft(
        &self,
        conn: &Connection,
        account: i64,
        id: i64,
    ) -> Result<Draft, ComposeError> {
        let d = Self::draft_of(conn, account, id)?;
        match d.state {
            DraftState::Done => Err(ComposeError::NoSuchDraft),
            DraftState::Queued => Err(ComposeError::InOutbox),
            DraftState::Draft => {
                // Without an Outbox nothing can be sent: the answer would mean nothing.
                let entries = self
                    .handle
                    .outbox_entries(account)
                    .ok_or(ComposeError::Internal)?;
                if let Some(e) = entry_for(&entries, id) {
                    let outbox_id = i64::try_from(e.id).map_err(|_| ComposeError::Internal)?;
                    drafts::set_queued(conn, account, id, outbox_id)?;
                    return Err(ComposeError::InOutbox);
                }
                Ok(d)
            }
        }
    }

    /// Whether the account can use the network at all: a real account that is running.
    fn has_server(&self, account: i64) -> bool {
        self.backends().contains_key(&account)
    }

    /// The path of the account's Drafts folder from the folder mapping, if it is known.
    fn drafts_folder_path(conn: &Connection, account: i64) -> Option<String> {
        folders::list(conn, account)
            .ok()?
            .into_iter()
            .find(|f| f.special_use == "drafts")
            .map(|f| f.path)
    }

    /// Finds the messages the Outbox has finished with and settles their drafts (see the module
    /// notes, "Never twice"). Returns true when a draft changed. Blocking; runs under the actions
    /// lock.
    pub(crate) fn reconcile(&self, account: i64) -> Result<bool, ComposeError> {
        let _actions = self.compose.actions();
        // No Outbox: nothing is known about any message, so no draft may be called sent.
        let Some(entries) = self.handle.outbox_entries(account) else {
            return Ok(false);
        };
        let now = now_ms();
        self.compose_db(|conn| {
            let mut changed = false;
            for d in drafts::list(conn, account)? {
                let linked = entry_for(&entries, d.id);
                let link =
                    |e: &OutboxEntry| i64::try_from(e.id).map_err(|_| ComposeError::Internal);
                match (d.state, linked) {
                    (DraftState::Draft | DraftState::Queued, Some(e)) => match e.state {
                        // The server has it: sent.
                        OutboxState::Submitted | OutboxState::Filing | OutboxState::Sent => {
                            finish_sent(conn, &d, now)?;
                            changed = true;
                        }
                        // Still the Outbox's. If the draft does not know (a crash between the
                        // Outbox write and the draft's), it learns now.
                        _ => {
                            if d.state != DraftState::Queued || d.outbox_id != Some(link(e)?) {
                                drafts::set_queued(conn, account, d.id, link(e)?)?;
                                changed = d.state != DraftState::Queued;
                            }
                        }
                    },
                    // Gone from the Outbox with the link still set: sent and filed (a discard or
                    // an Undo clears the link first).
                    (DraftState::Queued, None) if d.outbox_id.is_some() => {
                        finish_sent(conn, &d, now)?;
                        changed = true;
                    }
                    // Gone with the link cleared: taken back (Undo, Edit, Discard, or a crash in
                    // the middle of one). It is a draft again.
                    (DraftState::Queued, None) => {
                        drafts::set_editable(conn, account, d.id, now)?;
                        changed = true;
                    }
                    _ => {}
                }
            }
            Ok(changed)
        })
    }
}

/// The message of this draft was sent: the recipients are remembered for the suggestions and the
/// draft is finished (its copy on the server is deleted afterwards).
fn finish_sent(conn: &Connection, d: &Draft, now: i64) -> Result<(), ComposeError> {
    let c = &d.content;
    let all: Vec<Address> = c.to.iter().chain(&c.cc).chain(&c.bcc).cloned().collect();
    recipients::record(conn, d.account_id, &all, now)?;
    drafts::mark_done(conn, d.account_id, d.id, now)?;
    Ok(())
}

fn outbox_id_i64(id: OutboxId) -> Result<i64, ComposeError> {
    i64::try_from(id).map_err(|_| ComposeError::Internal)
}

/// Runs blocking work off the runtime's workers.
async fn blocking<T: Send + 'static>(
    inner: &Arc<Inner>,
    f: impl FnOnce(&Inner) -> Result<T, ComposeError> + Send + 'static,
) -> Result<T, ComposeError> {
    let inner = inner.clone();
    tokio::task::spawn_blocking(move || f(&inner))
        .await
        .map_err(|_| ComposeError::Internal)?
}

/// What is needed to copy a draft to the server, read in one go.
struct Prepared {
    draft: Draft,
    mime: Vec<u8>,
    folder_hint: Option<String>,
}

/// The state of a draft's copy: what [`sync_draft`] decided before it touches the network.
enum Plan {
    Done(DraftSyncOutcome),
    Copy(Box<Prepared>),
}

fn plan_copy(
    inner: &Inner,
    account: i64,
    draft_id: i64,
    deferred: bool,
) -> Result<Plan, ComposeError> {
    inner.compose_db(|conn| {
        let acct = account_of(conn, account)?;
        let d = Inner::draft_of(conn, account, draft_id)?;
        if d.state != DraftState::Draft {
            return Ok(Plan::Done(DraftSyncOutcome::AlreadyCurrent));
        }
        if d.server_folder.is_some() && d.synced_revision >= d.revision {
            return Ok(Plan::Done(DraftSyncOutcome::AlreadyCurrent));
        }
        if d.server_folder.is_none() && draft_is_empty(conn, &d)? {
            // Nothing worth a copy on the server.
            return Ok(Plan::Done(DraftSyncOutcome::AlreadyCurrent));
        }
        if deferred {
            return Ok(Plan::Done(DraftSyncOutcome::Deferred));
        }
        let (mime, _, _) = build(conn, &acct, &d, true)?;
        let folder_hint = Inner::drafts_folder_path(conn, account);
        Ok(Plan::Copy(Box::new(Prepared {
            draft: d,
            mime,
            folder_hint,
        })))
    })
}

fn outcome_of(e: &BackendError) -> DraftSyncOutcome {
    match e {
        BackendError::Transient(_) => DraftSyncOutcome::TryLater,
        BackendError::Auth(_) => DraftSyncOutcome::NeedsSignIn,
        BackendError::Permanent(_) => DraftSyncOutcome::Refused,
    }
}

/// Copies one draft to the server's Drafts folder. See [`Core::sync_draft_to_server`].
pub(crate) async fn sync_draft(
    inner: &Arc<Inner>,
    account: i64,
    draft_id: i64,
    force: bool,
) -> Result<DraftSyncOutcome, ComposeError> {
    // Unknown account or draft are refused before anything else.
    {
        let first = blocking(inner, move |i| {
            i.compose_db(|conn| {
                account_of(conn, account)?;
                let d = Inner::draft_of(conn, account, draft_id)?;
                Ok(d.state)
            })
        })
        .await?;
        if first != DraftState::Draft {
            return Ok(DraftSyncOutcome::AlreadyCurrent);
        }
    }
    let Some(backend) = inner.backends().get(&account).cloned() else {
        return Ok(DraftSyncOutcome::NotApplicable);
    };
    // T22: a JMAP account's drafts stay on the device (JMAP draft copies are a later task).
    if backend.is_jmap() {
        return Ok(DraftSyncOutcome::NotApplicable);
    }
    if inner.hub.is_offline() {
        return Ok(DraftSyncOutcome::Offline);
    }
    let _one_at_a_time = inner.compose.server.lock().await;
    let deferred = {
        let now = now_ms();
        let interval = inner.compose.interval_ms.load(Ordering::SeqCst);
        let last = inner.compose.attempts().get(&(account, draft_id)).copied();
        !force && last.is_some_and(|t| now.saturating_sub(t) < interval)
    };
    let plan = blocking(inner, move |i| plan_copy(i, account, draft_id, deferred)).await?;
    let prepared = match plan {
        Plan::Done(outcome) => return Ok(outcome),
        Plan::Copy(p) => p,
    };
    inner
        .compose
        .attempts()
        .insert((account, draft_id), now_ms());
    let Prepared {
        draft,
        mime,
        folder_hint,
    } = *prepared;
    let (imap, folder) = match async {
        let imap = backend.imap().await?;
        let folder = drafts_folder(&imap, folder_hint.as_deref()).await?;
        Ok::<_, BackendError>((imap, folder))
    }
    .await
    {
        Ok(found) => found,
        Err(e) => return Ok(outcome_of(&e)),
    };
    // Written before the copy is made: if the copy is made and its record is not (a failure, a
    // crash), the draft still knows which folder to look in when it is sent or discarded, so the
    // copy is never left behind.
    {
        let (id, folder) = (draft.id, folder.clone());
        blocking(inner, move |i| {
            i.compose_db(|conn| Ok(drafts::plan_server_copy(conn, account, id, &folder)?))
        })
        .await?;
    }
    match save_draft_copy(&imap, &folder, &draft.message_id, &mime).await {
        Ok(copy) => {
            // Earlier Message-IDs of this draft (it was taken back from a message that may have
            // been sent): their copies go now. Those that cannot be deleted stay on the list.
            let mut removed = Vec::new();
            for stale in &draft.stale_message_ids {
                if delete_draft_copies(&imap, &copy.folder, stale)
                    .await
                    .is_ok()
                {
                    removed.push(stale.clone());
                }
            }
            let revision = draft.revision;
            let id = draft.id;
            blocking(inner, move |i| {
                i.compose_db(|conn| {
                    drafts::record_server_copy(
                        conn,
                        account,
                        id,
                        &copy.folder,
                        copy.uid.map(i64::from),
                        revision,
                    )?;
                    drafts::clear_stale(conn, account, id, &removed)?;
                    Ok(())
                })
            })
            .await?;
            Ok(DraftSyncOutcome::Saved)
        }
        Err(e) => Ok(outcome_of(&e)),
    }
}

/// Deletes the server copies of finished drafts (sent or discarded) and then the drafts. Anything
/// that cannot be done now (offline, signed out) is left for the next time.
pub(crate) async fn cleanup_finished(inner: &Arc<Inner>, account: i64) {
    let _one_at_a_time = inner.compose.server.lock().await;
    let finished = blocking(inner, move |i| {
        i.compose_db(|conn| Ok(drafts::in_state(conn, account, DraftState::Done)?))
    })
    .await
    .unwrap_or_default();
    for d in finished {
        let has_copy = d.server_folder.is_some();
        let backend = inner.backends().get(&account).cloned();
        let removed = match (&d.server_folder, backend) {
            (Some(folder), Some(backend)) => {
                if inner.hub.is_offline() {
                    false
                } else {
                    let r = async {
                        let imap = backend.imap().await?;
                        for id in std::iter::once(&d.message_id).chain(&d.stale_message_ids) {
                            delete_draft_copies(&imap, folder, id).await?;
                        }
                        Ok::<_, BackendError>(())
                    }
                    .await;
                    // Only "try again later" and "sign in" keep the draft; a refusal (the folder
                    // is gone) must not keep it for ever.
                    !matches!(r, Err(BackendError::Transient(_) | BackendError::Auth(_)))
                }
            }
            // No copy was ever recorded, or the account has no server (the sample mailbox).
            (None, _) | (Some(_), None) => !has_copy || !inner.has_server(account),
        };
        if removed {
            let id = d.id;
            let _ = blocking(inner, move |i| {
                i.compose_db(|conn| Ok(drafts::delete(conn, account, id)?))
            })
            .await;
            inner.compose.attempts().remove(&(account, d.id));
        }
    }
}

/// Copies every draft of the account that has changes the server does not have.
pub(crate) async fn sync_pending(inner: &Arc<Inner>, account: i64) {
    let pending = blocking(inner, move |i| {
        i.compose_db(|conn| {
            Ok(drafts::in_state(conn, account, DraftState::Draft)?
                .into_iter()
                .filter(|d| d.revision > d.synced_revision)
                .map(|d| d.id)
                .collect::<Vec<_>>())
        })
    })
    .await
    .unwrap_or_default();
    for id in pending {
        // A background copy is not the app's "after 5 s": it is not held back by the pacing.
        let _ = sync_draft(inner, account, id, true).await;
    }
}

impl Inner {
    /// Runs the check for sent messages, the clean-up of finished drafts and (with `copy`) the
    /// copies that are due, in the background. Does nothing when there is no runtime.
    pub(crate) fn kick_compose(self: &Arc<Self>, account: i64, copy: bool) {
        let inner = self.clone();
        self.compose.kicks.fetch_add(1, Ordering::SeqCst);
        self.compose.rt.spawn(async move {
            let changed = {
                let i = inner.clone();
                blocking(&i, move |i| i.reconcile(account)).await
            };
            if matches!(changed, Ok(true)) {
                inner.hub.accounts_changed([account]);
            }
            cleanup_finished(&inner, account).await;
            if copy {
                sync_pending(&inner, account).await;
            }
            inner.compose.kicks.fetch_sub(1, Ordering::SeqCst);
        });
    }
}

/// Watches the engine's changes: whenever an account's Outbox changed, the drafts of that account
/// are settled. Holds the core weakly, so it never keeps it alive.
pub(crate) fn watch_outbox(inner: &Arc<Inner>, mut changes: broadcast::Receiver<Change>) {
    let weak: Weak<Inner> = Arc::downgrade(inner);
    inner.compose.rt.spawn(async move {
        loop {
            let accounts: Vec<i64> = match changes.recv().await {
                Ok(Change::Outbox { account }) => vec![account],
                Ok(_) => continue,
                Err(broadcast::error::RecvError::Lagged(_)) => {
                    let Some(inner) = weak.upgrade() else { return };
                    inner.account_ids().unwrap_or_default()
                }
                Err(broadcast::error::RecvError::Closed) => return,
            };
            let Some(inner) = weak.upgrade() else { return };
            for account in accounts {
                inner.kick_compose(account, false);
            }
        }
    });
}

// ------------------------------------------------------------------------------------------
// The calls
// ------------------------------------------------------------------------------------------

/// How an Outbox action finds the draft behind a message.
struct Taken {
    draft: Draft,
    entry: OutboxEntry,
}

#[uniffi::export]
impl Core {
    /// Saves a draft on the device. Call it about one second after the person stops typing (the
    /// core only stores). `draft_id: None` starts a new draft (its Message-ID is made now and never
    /// changes); `Some(id)` replaces the content of that draft.
    ///
    /// Refused with [`ComposeError::InOutbox`] while the draft has an Outbox message (Send was
    /// tapped and not taken back), [`ComposeError::NoSuchDraft`] for a draft that is not this
    /// account's, and [`ComposeError::DraftTooLarge`] over the limits ([`compose_limits`]).
    /// Addresses are stored as typed, including invalid ones ([`Core::check_draft`] reports them).
    pub fn save_draft(
        &self,
        account_id: i64,
        draft_id: Option<i64>,
        content: DraftContent,
    ) -> Result<SavedDraft, ComposeError> {
        let inner = &self.inner;
        let stored = stored_content(&content)?;
        let now = now_ms();
        let _actions = inner.compose.actions();
        inner.compose_db(|conn| {
            let acct = account_of(conn, account_id)?;
            if let Some(identity) = stored.identity_id {
                if identities::get(conn, account_id, identity)?.is_none() {
                    return Err(ComposeError::NoSuchIdentity);
                }
            }
            match draft_id {
                Some(id) => {
                    let d = inner.editable_draft(conn, account_id, id)?;
                    if !drafts::update(conn, account_id, id, &stored, now)? {
                        return Err(ComposeError::NoSuchDraft);
                    }
                    Ok(SavedDraft {
                        id,
                        message_id: d.message_id,
                        saved_ms: now,
                    })
                }
                None => {
                    let (_, from) = sender_of(conn, &acct, stored.identity_id)?;
                    let message_id = mail::new_message_id(random()?, &from.address);
                    let id = drafts::insert(conn, account_id, &message_id, &stored, now)?;
                    Ok(SavedDraft {
                        id,
                        message_id,
                        saved_ms: now,
                    })
                }
            }
        })
    }

    /// One draft, to reopen compose (after a restart, from the Drafts list, or after Undo and
    /// Edit). A draft of another account, and a finished one, is [`ComposeError::NoSuchDraft`].
    pub fn load_draft(&self, account_id: i64, draft_id: i64) -> Result<FfiDraft, ComposeError> {
        let entries = self
            .inner
            .handle
            .outbox_entries(account_id)
            .unwrap_or_default();
        self.inner.compose_db(|conn| {
            account_of(conn, account_id)?;
            let d = Inner::draft_of(conn, account_id, draft_id)?;
            if d.state == DraftState::Done {
                return Err(ComposeError::NoSuchDraft);
            }
            public_draft(conn, &d, &entries)
        })
    }

    /// The account's drafts, newest change first: the ones being written and the ones waiting in
    /// the Outbox. After a crash this is how the app finds the draft that was open ("kill after
    /// 1 s: local draft restored").
    pub fn list_drafts(&self, account_id: i64) -> Result<Vec<DraftSummary>, ComposeError> {
        let entries = self
            .inner
            .handle
            .outbox_entries(account_id)
            .unwrap_or_default();
        self.inner.compose_db(|conn| {
            account_of(conn, account_id)?;
            let mut out = Vec::new();
            for d in drafts::list(conn, account_id)? {
                let c = &d.content;
                let to_preview =
                    c.to.iter()
                        .chain(&c.cc)
                        .chain(&c.bcc)
                        .take(3)
                        .map(|a| {
                            a.name
                                .clone()
                                .filter(|n| !n.is_empty())
                                .unwrap_or_else(|| a.address.clone())
                        })
                        .collect();
                let snippet: String = c
                    .body
                    .split_whitespace()
                    .collect::<Vec<_>>()
                    .join(" ")
                    .chars()
                    .take(120)
                    .collect();
                out.push(DraftSummary {
                    id: d.id,
                    account_id,
                    subject: c.subject.clone(),
                    snippet,
                    to_preview,
                    has_attachments: !drafts::attachments(conn, account_id, d.id)?.is_empty(),
                    outbox_id: entry_for(&entries, d.id).map(|e| e.id),
                    updated_ms: d.updated_at,
                });
            }
            Ok(out)
        })
    }

    /// Discards a draft (the person chose Discard, or closed an empty compose). The draft
    /// disappears from the lists at once; its copy on the server is deleted in the background (now,
    /// or when the connection returns). Refused with [`ComposeError::InOutbox`] while it has an
    /// Outbox message.
    pub fn discard_draft(&self, account_id: i64, draft_id: i64) -> Result<(), ComposeError> {
        let inner = &self.inner;
        {
            let _actions = inner.compose.actions();
            inner.compose_db(|conn| {
                account_of(conn, account_id)?;
                inner.editable_draft(conn, account_id, draft_id)?;
                drafts::mark_done(conn, account_id, draft_id, now_ms())?;
                Ok(())
            })?;
        }
        inner.kick_compose(account_id, false);
        Ok(())
    }

    /// Copies the draft to the server's Drafts folder: the new version is appended with `\Draft`,
    /// then the old copy is deleted (found by the draft's Message-ID, so a copy is never lost and
    /// never kept twice). Call it 5 s after typing stops, at most every 20 s while typing goes on,
    /// and with `force` when the app is paused or compose is closed. The core also paces it: a
    /// call less than 20 s after the last copy of this draft is [`DraftSyncOutcome::Deferred`]
    /// unless `force`.
    ///
    /// The outcome is never an error for the person: offline and server trouble leave the draft on
    /// the device, and the core copies it when the connection returns
    /// ([`Core::set_network_available`]) and at the next start. Errors are only for ids that are
    /// not this account's.
    pub async fn sync_draft_to_server(
        &self,
        account_id: i64,
        draft_id: i64,
        force: bool,
    ) -> Result<DraftSyncOutcome, ComposeError> {
        let inner = self.inner.clone();
        let task = self.spawn(
            async move { sync_draft(&inner, account_id, draft_id, force).await },
            false,
        );
        task.await?
    }

    /// Adds a file to the draft from bytes the app has (the system picker's result). `name` is the
    /// file name to show and send (folders in it are dropped). Refused over the size limits
    /// ([`ComposeError::AttachmentTooLarge`], [`ComposeError::TooManyAttachments`]); an attachment
    /// larger than [`ATTACHMENT_WARN_BYTES`] is accepted (the chip warns and Send is disabled, see
    /// [`Core::check_draft`]).
    ///
    /// **The bytes are already in memory when this is called** (a `ByteArray` crossing the FFI is
    /// copied): check `compose_limits().max_attachment_bytes` before reading a file into memory, and
    /// prefer [`Core::add_draft_attachment_file`] for anything large. The size is the first thing
    /// checked here.
    pub async fn add_draft_attachment_bytes(
        &self,
        account_id: i64,
        draft_id: i64,
        name: String,
        mime_type: String,
        bytes: Vec<u8>,
    ) -> Result<DraftAttachment, ComposeError> {
        // The bytes are already in memory when they get here; refuse what could never be kept
        // before anything else is done with them (no task, no database).
        if bytes.len() as u64 > MAX_ATTACHMENT_BYTES {
            return Err(ComposeError::AttachmentTooLarge);
        }
        let inner = self.inner.clone();
        let task = self.spawn(
            async move {
                blocking(&inner, move |i| {
                    add_attachment(i, account_id, draft_id, &name, &mime_type, bytes)
                })
                .await
            },
            false,
        );
        task.await?
    }

    /// Adds a file to the draft from a path in the app's attachment folder (a copy the system
    /// picker's result was written to; see [`Core::set_attachment_directory`], which must have been
    /// called). The core reads **only that file**: the path must be absolute, without `..`, under
    /// the registered folder with **no link anywhere below it**, and name a plain file with one name
    /// (not a link, folder, pipe, device or hard link) of at most [`MAX_ATTACHMENT_BYTES`]. Anything
    /// else, the core's own database and the files of `/proc` included, is
    /// [`ComposeError::AttachmentUnreadable`] (or `AttachmentTooLarge`), and
    /// [`ComposeError::NoAttachmentDirectory`] when no folder was registered. `name` is the name to
    /// show and send; the path's own name is never used. Delete the cache file afterwards.
    pub async fn add_draft_attachment_file(
        &self,
        account_id: i64,
        draft_id: i64,
        path: String,
        name: String,
        mime_type: String,
    ) -> Result<DraftAttachment, ComposeError> {
        let inner = self.inner.clone();
        let task = self.spawn(
            async move {
                blocking(&inner, move |i| {
                    // Checked before the file is read: an unknown draft reads nothing.
                    i.compose_db(|conn| i.editable_draft(conn, account_id, draft_id).map(|_| ()))?;
                    let dir = i
                        .compose
                        .attachment_dir()
                        .ok_or(ComposeError::NoAttachmentDirectory)?;
                    let bytes = read_capped_file(&dir, &path, MAX_ATTACHMENT_BYTES)?;
                    add_attachment(i, account_id, draft_id, &name, &mime_type, bytes)
                })
                .await
            },
            false,
        );
        task.await?
    }

    /// Registers the one folder `add_draft_attachment_file` may read from. **Call it once, right
    /// after `Core.open`** (before any attachment is added from a path), with a folder that only
    /// holds the copies the system picker's results are written to, for example a `compose-attach`
    /// folder inside the app's cache folder. A later call replaces the folder.
    ///
    /// The path must be absolute and an existing folder. The core's own database must not be inside
    /// it (`BadAttachmentDirectory`): never pass the app's data folder or the folder the database
    /// lives in. Nothing is created, listed or deleted there by the core.
    pub fn set_attachment_directory(&self, path: String) -> Result<(), ComposeError> {
        let given = Path::new(&path);
        let bad = ComposeError::BadAttachmentDirectory;
        if !given.is_absolute()
            || given
                .components()
                .any(|c| matches!(c, Component::ParentDir))
        {
            return Err(bad);
        }
        let canonical = std::fs::canonicalize(given).map_err(|_| bad.clone())?;
        if !canonical.is_dir() {
            return Err(bad);
        }
        // The database file (and the files beside it) must not be reachable through this folder.
        let db = std::fs::canonicalize(&self.inner.compose.db_path)
            .unwrap_or_else(|_| self.inner.compose.db_path.clone());
        if db.starts_with(&canonical) {
            return Err(bad);
        }
        *self
            .inner
            .compose
            .attachment_dir
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(AttachmentDir {
            given: given.to_path_buf(),
            canonical,
        });
        Ok(())
    }

    /// Removes an attachment from the draft.
    pub fn remove_draft_attachment(
        &self,
        account_id: i64,
        draft_id: i64,
        attachment_id: i64,
    ) -> Result<(), ComposeError> {
        let inner = &self.inner;
        let _actions = inner.compose.actions();
        inner.compose_db(|conn| {
            inner.editable_draft(conn, account_id, draft_id)?;
            if drafts::remove_attachment(conn, account_id, draft_id, attachment_id, now_ms())? {
                Ok(())
            } else {
                Err(ComposeError::NoSuchAttachment)
            }
        })
    }

    /// What the Send button needs to know (`docs/ux/controls.md`, "Send button"). Does not change
    /// anything; call it after each save.
    pub fn check_draft(&self, account_id: i64, draft_id: i64) -> Result<DraftCheck, ComposeError> {
        self.inner.compose_db(|conn| {
            account_of(conn, account_id)?;
            let d = Inner::draft_of(conn, account_id, draft_id)?;
            let c = &d.content;
            let total = size(drafts::attachments_total(conn, account_id, draft_id)?);
            let all: Vec<&Address> = c.to.iter().chain(&c.cc).chain(&c.bcc).collect();
            let invalid: Vec<String> = all
                .iter()
                .filter(|a| !mail::is_valid_address(&a.address))
                .map(|a| a.address.clone())
                .collect();
            let mut problems = Vec::new();
            if all.is_empty() {
                problems.push(DraftProblem::NoRecipient);
            }
            if !invalid.is_empty() {
                problems.push(DraftProblem::InvalidRecipient);
            }
            if total > ATTACHMENT_WARN_BYTES {
                problems.push(DraftProblem::AttachmentsTooLarge);
            }
            Ok(DraftCheck {
                problems,
                invalid_recipients: invalid,
                subject_empty: c.subject.trim().is_empty(),
                body_empty: c.body.trim().is_empty() && total == 0,
                attachments_bytes: total,
                size_limit_bytes: ATTACHMENT_WARN_BYTES,
            })
        })
    }

    /// Sends the draft: the message is written (its Message-ID is the draft's), stored in the
    /// Outbox with `send_at = now + the undo window` ([`Core::set_undo_window_ms`]), and goes out
    /// when the window ends and the network allows, **exactly once**. Nothing reaches the server
    /// before the window ends, and nothing while the device is offline: the message waits, and
    /// the Outbox shows it as Queued.
    ///
    /// The draft is locked until the message is sent ([`ComposeError::InOutbox`] for a second
    /// Send); [`Core::undo_send`] or [`Core::edit_outbox_item`] unlock it. When the message has
    /// been sent the core deletes the draft and its copy on the server by itself.
    ///
    /// Refused with [`ComposeError::NoRecipient`] or [`ComposeError::InvalidAddress`] (see
    /// [`Core::check_draft`]), [`ComposeError::CannotSend`] for an account with no SMTP server.
    /// Bcc recipients receive the message but are not in it.
    pub fn send(&self, account_id: i64, draft_id: i64) -> Result<SendStarted, ComposeError> {
        let inner = &self.inner;
        let _actions = inner.compose.actions();
        let (envelope, mime, identity_id, from, draft) = inner.compose_db(|conn| {
            let acct = account_of(conn, account_id)?;
            let d = inner.editable_draft(conn, account_id, draft_id)?;
            let (mime, identity_id, from) = build(conn, &acct, &d, false)?;
            Ok((envelope_recipients(&d.content), mime, identity_id, from, d))
        })?;
        let queued = inner
            .handle
            .send(
                NewMessage {
                    account: account_id,
                    identity: Some(identity_id),
                    draft: Some(draft.id),
                    envelope_from: from.address,
                    recipients: envelope,
                },
                mime,
            )
            .map_err(|e| match e {
                SendError::NoSubmitter(_) => ComposeError::CannotSend,
                SendError::UnknownAccount(_) => ComposeError::NoSuchAccount,
                SendError::NoRecipients => ComposeError::NoRecipient,
                SendError::Storage(e) => e.into(),
                SendError::NoOutbox
                | SendError::BccHeader
                | SendError::NoMessageId
                | SendError::MessageIdNotAscii
                | SendError::NoSender => ComposeError::Internal,
            })?;
        // The link in the draft is a hint written after the message (the message carries the
        // draft's id itself): a failure here is repaired by the next check, not fatal.
        let send_at_ms = inner
            .handle
            .outbox_entries(account_id)
            .unwrap_or_default()
            .iter()
            .find(|e| e.id == queued)
            .map_or_else(now_ms, |e| e.send_at_ms);
        let linked = outbox_id_i64(queued).and_then(|oid| {
            inner.compose_db(|conn| Ok(drafts::set_queued(conn, account_id, draft.id, oid)?))
        });
        if linked.is_err() {
            log::warn!(
                "account {account_id}: could not mark a draft as queued; it is repaired later"
            );
        }
        Ok(SendStarted {
            outbox_id: queued,
            send_at_ms,
        })
    }

    /// Undo send, inside the window: nothing is sent, the message leaves the Outbox and the draft
    /// is editable again. Returns the draft's id: reopen compose with
    /// [`Core::load_draft`] (the cursor position is the app's to restore).
    /// [`ComposeError::TooLate`] once the window has closed or the message started going out.
    pub fn undo_send(&self, account_id: i64, outbox_id: u64) -> Result<i64, ComposeError> {
        self.take_back(account_id, outbox_id, false, |handle, account, id| {
            handle.undo_send(account, id).map(|_| ())
        })
    }

    /// The Outbox screen's rows for the account (S26): Queued, Sending and Failed messages, oldest
    /// first. A message that was sent is not listed.
    pub fn list_outbox(&self, account_id: i64) -> Result<Vec<OutboxItem>, ComposeError> {
        let entries: Vec<OutboxEntry> = self.inner.handle.outbox(account_id);
        self.inner.compose_db(|conn| {
            account_of(conn, account_id)?;
            let mut out = Vec::new();
            for e in entries {
                let state = match e.state {
                    OutboxState::Queued => OutboxItemState::Queued,
                    OutboxState::Sending => OutboxItemState::Sending,
                    OutboxState::Failed => OutboxItemState::Failed,
                    // The screen does not list these (the message is out; only the copy is left).
                    OutboxState::Submitted | OutboxState::Filing | OutboxState::Sent => continue,
                };
                let subject = match e.draft {
                    Some(d) => drafts::get(conn, account_id, d)?
                        .map(|d| d.content.subject)
                        .unwrap_or_default(),
                    None => String::new(),
                };
                let reason = e.last_error.as_deref().map(|t| reason_of(t, e.state));
                out.push(OutboxItem {
                    id: e.id,
                    state,
                    send_at_ms: e.send_at_ms,
                    attempts: e.attempts,
                    reason,
                    may_have_been_sent: e.may_have_been_sent(),
                    draft_id: e.draft,
                    subject,
                    recipient_count: u32::try_from(e.recipients.len()).unwrap_or(u32::MAX),
                    to_preview: e.recipients.iter().take(2).cloned().collect(),
                });
            }
            Ok(out)
        })
    }

    /// Retry on a Failed message: it is queued again, due now, with fresh attempts.
    ///
    /// For a message with [`OutboxItem::may_have_been_sent`] the server may already have it, and a
    /// Retry can deliver it twice. The call is therefore refused with
    /// [`ComposeError::NeedsConfirmation`] unless `confirmed` is true. The screen shows "This message
    /// may already have been sent" (suggesting a look in Sent) with Retry anyway and Cancel, and
    /// passes `confirmed = true` only for Retry anyway. For any other Failed message pass `false`.
    pub fn retry_outbox_item(
        &self,
        account_id: i64,
        outbox_id: u64,
        confirmed: bool,
    ) -> Result<(), ComposeError> {
        let _actions = self.inner.compose.actions();
        self.inner
            .compose_db(|conn| account_of(conn, account_id).map(|_| ()))?;
        let entry = self
            .inner
            .handle
            .outbox_entries(account_id)
            .ok_or(ComposeError::Internal)?
            .into_iter()
            .find(|e| e.id == outbox_id)
            .ok_or(ComposeError::NoSuchOutboxItem)?;
        if entry.may_have_been_sent() && !confirmed {
            return Err(ComposeError::NeedsConfirmation);
        }
        self.inner.handle.retry_send(account_id, outbox_id)?;
        Ok(())
    }

    /// Edit on a Queued or Failed message: it leaves the Outbox and returns to its draft. Returns
    /// the draft's id: reopen compose with [`Core::load_draft`]. A message that is being sent
    /// cannot be taken back ([`ComposeError::WrongState`]).
    pub fn edit_outbox_item(&self, account_id: i64, outbox_id: u64) -> Result<i64, ComposeError> {
        self.take_back(account_id, outbox_id, true, |handle, account, id| {
            handle.edit_back_to_draft(account, id).map(|_| ())
        })
    }

    /// Discard on a Queued or Failed message (the screen has asked first): the message and its
    /// draft are dropped, and so is the draft's copy on the server.
    pub fn discard_outbox_item(&self, account_id: i64, outbox_id: u64) -> Result<(), ComposeError> {
        let draft_id = self.take_back(account_id, outbox_id, false, |handle, account, id| {
            handle.discard_send(account, id)
        })?;
        self.discard_draft(account_id, draft_id)
    }

    /// Recipient suggestions for what the person has typed so far: addresses of mail the person
    /// **sent** from this account (no device contacts), whose address or display name (or a word
    /// of it) starts with `prefix`, most used first. At most `limit` (never more than
    /// [`MAX_SUGGESTIONS`]); nothing for an empty prefix. Another account's history is never
    /// used.
    ///
    /// The history is filled when mail is sent through [`Core::send`], and from the To and Cc of
    /// the Sent folder's messages when the sync stores them (it does not yet: see the README's
    /// "Known gaps").
    pub fn suggest_recipients(
        &self,
        account_id: i64,
        prefix: String,
        limit: u32,
    ) -> Result<Vec<FfiAddress>, ComposeError> {
        let limit = limit.clamp(1, MAX_SUGGESTIONS);
        self.inner.compose_db(|conn| {
            account_of(conn, account_id)?;
            let mut out: Vec<FfiAddress> = recipients::suggest(conn, account_id, &prefix, limit)?
                .into_iter()
                .map(|s| FfiAddress {
                    name: s.name,
                    address: s.address,
                })
                .collect();
            if out.len() < limit as usize {
                for a in sent_folder_recipients(conn, account_id, &prefix)? {
                    if out.len() >= limit as usize {
                        break;
                    }
                    if !out
                        .iter()
                        .any(|x| x.address.eq_ignore_ascii_case(&a.address))
                    {
                        out.push(a);
                    }
                }
            }
            Ok(out)
        })
    }

    /// The fields for a reply to the stored message `message_id` (a row id of this account): To is
    /// the sender (or `Reply-To`), the subject has `Re:` once, `In-Reply-To` and `References` are
    /// set, and the body is the quote of the stored text when the body is stored (otherwise empty:
    /// [`ReplyDraft::quote_included`] is false). Nothing is stored; the app calls
    /// [`Core::save_draft`] when the person starts to write.
    pub fn reply_draft(
        &self,
        account_id: i64,
        message_id: i64,
    ) -> Result<ReplyDraft, ComposeError> {
        self.reply_fields(account_id, message_id, ReplyKind::Reply)
    }

    /// Like [`Core::reply_draft`], with everyone else on To and Cc of the original as well, except
    /// the person's own addresses (the account's and its identities'). The original's To and Cc are
    /// only as complete as what is stored (the sync does not store them yet; the header block of
    /// the body does, once it is fetched).
    pub fn reply_all_draft(
        &self,
        account_id: i64,
        message_id: i64,
    ) -> Result<ReplyDraft, ComposeError> {
        self.reply_fields(account_id, message_id, ReplyKind::ReplyAll)
    }

    /// The fields for forwarding the stored message: no recipients, `Fwd:` once in the subject, and
    /// a "Forwarded message" block with the original's From, Date, Subject and To followed by its
    /// text when stored (see [`ReplyDraft::quote_included`]). The original's attachments are not
    /// carried over yet.
    pub fn forward_draft(
        &self,
        account_id: i64,
        message_id: i64,
    ) -> Result<ReplyDraft, ComposeError> {
        self.reply_fields(account_id, message_id, ReplyKind::Forward)
    }

    /// The addresses the account can send from, default first, for the From picker. An account that
    /// has none gets one made from its own address and name, so the list is never empty.
    pub fn identities(&self, account_id: i64) -> Result<Vec<FfiIdentity>, ComposeError> {
        self.inner.compose_db(|conn| {
            let acct = account_of(conn, account_id)?;
            Ok(
                identities::list_or_create(conn, account_id, &acct.label, &acct.email)?
                    .into_iter()
                    .map(|i| FfiIdentity {
                        id: i.id,
                        display_name: i.display_name,
                        address: i.address,
                        is_default: i.is_default,
                    })
                    .collect(),
            )
        })
    }
}

impl Core {
    /// Takes an Outbox message back (Undo, Edit or Discard) with the order that keeps it safe: the
    /// draft's link is cleared first, then the Outbox acts, and the link is put back when the
    /// Outbox refused. Returns the draft's id.
    fn take_back(
        &self,
        account_id: i64,
        outbox_id: u64,
        rotate_id_if_tried: bool,
        act: impl FnOnce(&croft_engine::EngineHandle, i64, OutboxId) -> Result<(), OutboxActionError>,
    ) -> Result<i64, ComposeError> {
        let inner = &self.inner;
        let _actions = inner.compose.actions();
        let acct = inner.compose_db(|conn| account_of(conn, account_id))?;
        let entry = inner
            .handle
            .outbox_entries(account_id)
            .ok_or(ComposeError::Internal)?
            .into_iter()
            .find(|e| e.id == outbox_id)
            .ok_or(ComposeError::NoSuchOutboxItem)?;
        let draft_id = entry.draft.ok_or(ComposeError::NoSuchDraft)?;
        let taken = inner.compose_db(|conn| {
            // A draft that is gone or finished means the message was sent (and filed) a moment
            // ago: there is nothing to take back.
            let draft = Inner::draft_of(conn, account_id, draft_id)
                .map_err(|_| ComposeError::NoSuchOutboxItem)?;
            if draft.state == DraftState::Done {
                return Err(ComposeError::NoSuchOutboxItem);
            }
            if rotate_id_if_tried && entry.attempts > 0 {
                // The server may have taken this message already. The corrected one gets a new
                // Message-ID, so that it is not taken for the old one (the Sent copy is found by
                // Message-ID and would not be filed; a recipient's server may drop a repeat). The
                // old id is kept only to delete the old draft copies on the server.
                let (_, from) = sender_of(conn, &acct, draft.content.identity_id)?;
                let fresh = mail::new_message_id(random()?, &from.address);
                drafts::unlink_outbox_with_new_id(conn, account_id, draft_id, &fresh)?;
            } else {
                drafts::unlink_outbox(conn, account_id, draft_id)?;
            }
            Ok(Taken { draft, entry })
        })?;
        match act(&inner.handle, account_id, outbox_id) {
            Ok(()) => {
                // The draft is a draft again. (A discard finishes it right after.)
                inner.compose_db(|conn| {
                    drafts::set_editable(conn, account_id, taken.draft.id, now_ms())?;
                    Ok(())
                })?;
                inner.hub.accounts_changed([account_id]);
                Ok(taken.draft.id)
            }
            Err(e) => {
                // Refused: the message is still the Outbox's, so the link goes back.
                let relinked = outbox_id_i64(taken.entry.id).and_then(|oid| {
                    inner.compose_db(|conn| {
                        Ok(drafts::set_queued(conn, account_id, taken.draft.id, oid)?)
                    })
                });
                if relinked.is_err() {
                    log::warn!("account {account_id}: could not restore a draft link; it is repaired later");
                }
                Err(e.into())
            }
        }
    }

    fn reply_fields(
        &self,
        account_id: i64,
        message_id: i64,
        kind: ReplyKind,
    ) -> Result<ReplyDraft, ComposeError> {
        self.inner.compose_db(|conn| {
            let acct = account_of(conn, account_id)?;
            let m =
                messages::get(conn, account_id, message_id)?.ok_or(ComposeError::NoSuchMessage)?;
            let ids = identities::list_or_create(conn, account_id, &acct.label, &acct.email)?;
            let (headers, text) = stored_body(conn, account_id, message_id)?;
            let header = |name: &str| headers.as_deref().and_then(|h| mail::header_value(h, name));
            let list = |name: &str, json: &str| -> Vec<Address> {
                match header(name) {
                    Some(v) => mail::parse_address_list(&v, MAX_RECIPIENTS as usize),
                    None => addresses_from_json(json),
                }
            };
            let to_json = stored_recipients(conn, account_id, message_id)?;
            let source = SourceMessage {
                message_id: m.message_id_hdr.clone(),
                subject: m.subject.clone(),
                from: m.from_addr.as_ref().map(|a| Address {
                    name: m.from_name.clone().filter(|n| !n.trim().is_empty()),
                    address: a.clone(),
                }),
                reply_to: header("Reply-To")
                    .map(|v| mail::parse_address_list(&v, 5))
                    .unwrap_or_default(),
                to: list("To", &to_json.0),
                cc: list("Cc", &to_json.1),
                date_ms: m.date_ts,
                references: header("References")
                    .map(|v| v.split_whitespace().map(str::to_owned).collect())
                    .unwrap_or_default(),
                text,
            };
            let own: Vec<String> = std::iter::once(acct.email.clone())
                .chain(ids.iter().map(|i| i.address.clone()))
                .collect();
            let fields: DraftFields = mail::reply_fields(kind, &source, &own);
            // Answer from the identity the message was sent to, if it was one of them.
            let identity = (kind != ReplyKind::Forward)
                .then(|| {
                    source
                        .to
                        .iter()
                        .chain(&source.cc)
                        .find_map(|a| {
                            ids.iter()
                                .find(|i| i.address.eq_ignore_ascii_case(a.address.trim()))
                        })
                        .map(|i| i.id)
                })
                .flatten();
            let list = |l: &[Address]| l.iter().map(FfiAddress::from).collect();
            Ok(ReplyDraft {
                content: DraftContent {
                    identity,
                    to: list(&fields.to),
                    cc: list(&fields.cc),
                    bcc: Vec::new(),
                    subject: fields.subject,
                    body: fields.body,
                    in_reply_to: fields.in_reply_to,
                    references: fields.references,
                },
                quote_included: fields.quote_included,
            })
        })
    }
}

/// The raw header block and the plain text of a stored message's body, if the body is stored.
/// Scoped by the message's account through the join (`message_body` has no account column).
fn stored_body(
    conn: &Connection,
    account: i64,
    message: i64,
) -> Result<(Option<String>, Option<String>), ComposeError> {
    let row = conn.query_row(
        "SELECT b.headers_raw, b.text_plain FROM message_body b
           JOIN message m ON m.id = b.message_id
          WHERE m.account_id = ?1 AND m.id = ?2",
        params![account, message],
        |r| {
            Ok((
                r.get::<_, Option<String>>(0)?,
                r.get::<_, Option<String>>(1)?,
            ))
        },
    );
    match row {
        Ok(v) => Ok(v),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok((None, None)),
        Err(e) => Err(e.into()),
    }
}

/// The stored `to_json` and `cc_json` of a message of this account.
fn stored_recipients(
    conn: &Connection,
    account: i64,
    message: i64,
) -> Result<(String, String), ComposeError> {
    Ok(conn.query_row(
        "SELECT to_json, cc_json FROM message WHERE account_id = ?1 AND id = ?2",
        params![account, message],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?)
}

/// Addresses from a stored `to_json` or `cc_json`: an array whose items are strings (`Name
/// <a@b>` or bare) or objects with `name` and `address`.
fn addresses_from_json(json: &str) -> Vec<Address> {
    let Ok(serde_json::Value::Array(items)) = serde_json::from_str::<serde_json::Value>(json)
    else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|item| match item {
            serde_json::Value::String(s) => mail::parse_address(s),
            serde_json::Value::Object(o) => {
                let address = o.get("address")?.as_str()?.trim().to_owned();
                (!address.is_empty()).then(|| Address {
                    name: o.get("name").and_then(|n| n.as_str()).map(str::to_owned),
                    address,
                })
            }
            _ => None,
        })
        .take(MAX_RECIPIENTS as usize)
        .collect()
}

/// To and Cc of the newest messages in the account's Sent folder, as far as the sync stored them,
/// that start with `prefix` (address or name or a word of it).
fn sent_folder_recipients(
    conn: &Connection,
    account: i64,
    prefix: &str,
) -> Result<Vec<FfiAddress>, ComposeError> {
    let p = prefix.trim().to_lowercase();
    if p.is_empty() {
        return Ok(Vec::new());
    }
    let mut stmt = conn.prepare(
        "SELECT m.to_json, m.cc_json FROM message m
           JOIN folder f ON f.id = m.folder_id AND f.account_id = m.account_id
          WHERE m.account_id = ?1 AND f.special_use = 'sent'
          ORDER BY m.date_ts DESC LIMIT 300",
    )?;
    let rows = stmt.query_map([account], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
    })?;
    let mut out: Vec<FfiAddress> = Vec::new();
    for row in rows {
        let (to, cc) = row?;
        for a in addresses_from_json(&to)
            .into_iter()
            .chain(addresses_from_json(&cc))
        {
            let address = a.address.to_lowercase();
            let name = a.name.as_deref().unwrap_or("").to_lowercase();
            let hit = address.starts_with(&p)
                || name.starts_with(&p)
                || name.split_whitespace().any(|w| w.starts_with(&p));
            if hit && !out.iter().any(|x| x.address.eq_ignore_ascii_case(&address)) {
                out.push(FfiAddress {
                    name: a.name,
                    address: a.address,
                });
            }
        }
    }
    Ok(out)
}

/// Adds an attachment under the limits. Blocking.
fn add_attachment(
    inner: &Inner,
    account: i64,
    draft: i64,
    name: &str,
    mime_type: &str,
    bytes: Vec<u8>,
) -> Result<DraftAttachment, ComposeError> {
    let len = bytes.len() as u64;
    if len > MAX_ATTACHMENT_BYTES {
        return Err(ComposeError::AttachmentTooLarge);
    }
    let _actions = inner.compose.actions();
    inner.compose_db(|conn| {
        inner.editable_draft(conn, account, draft)?;
        let existing = drafts::attachments(conn, account, draft)?;
        if existing.len() >= MAX_ATTACHMENTS as usize {
            return Err(ComposeError::TooManyAttachments);
        }
        let total: u64 = existing.iter().map(|a| size(a.size)).sum();
        if total.saturating_add(len) > MAX_DRAFT_ATTACHMENT_BYTES {
            return Err(ComposeError::AttachmentTooLarge);
        }
        let name = clean_file_name(name);
        let mime_type = clean_type(mime_type);
        let id = drafts::add_attachment(conn, account, draft, &name, &mime_type, &bytes, now_ms())?;
        Ok(DraftAttachment {
            id,
            name,
            mime_type,
            size_bytes: len,
        })
    })
}

#[cfg(test)]
mod tests;
