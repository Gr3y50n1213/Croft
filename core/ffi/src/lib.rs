// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The Croft FFI surface: the only Rust API the apps call.
//!
//! The surface is deliberately small and versioned. [`FFI_API_VERSION`] (exposed as
//! `ffi_api_version()`) must be bumped on every change to anything exported here (while the
//! surface is pre-1.0 additions count too). The apps check the version at start-up and refuse to
//! run against one they do not know.
//!
//! # What it offers (API version 9)
//!
//! - [`Core::open`] with the database key and a [`CredentialProvider`] the app implements.
//! - Accounts: `discover`, `verify`, `add_account`, `add_demo_account` (the sample mailbox),
//!   `list_accounts`, `remove_account`, `signed_in`.
//! - Reading: `list_folders`, `list_messages` (keyset paging, one folder or the unified Inbox).
//! - Observing: `next_change(after)`, a numbered change log the app waits on.
//! - Actions: `mark_seen`, `set_flagged`, `archive`, `trash`, `move_to`, `delete_forever`, `undo`.
//! - Sync and network: `sync_now`, `set_network_available`, `set_undo_window_ms`.
//! - Composing and sending (version 8, `compose.rs`): drafts, `send`, `undo_send`, the Outbox
//!   actions, recipient suggestions, reply and forward fields, identities. Errors: [`ComposeError`].
//! - Reading (version 9, `reading.rs`, S20, S21): `open_conversation`, `load_body` (sanitised HTML, remote images blocked
//!   unless allowed), `save_attachment`, `allow_remote_content`, `trust_sender`,
//!   `untrust_sender`, `trusted_senders`. Their errors are [`ReadError`].
//!
//! # Errors
//!
//! Errors are fixed categories: [`CoreError`], [`DiscoverError`], [`VerifyFailure`]. None carries
//! free text, a path, an address or a row value (the one string in them is a certificate
//! fingerprint, which is public and meant to be shown).
//!
//! # Secrets
//!
//! The core never stores a secret. `account.config_json` holds server settings only. Credentials
//! come from the app's [`CredentialProvider`] when an account's adapter is built, and live in the
//! protocol layer's wiping strings. No type here prints one in `Debug`.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

// The test server is for the host library the JVM tests load; it must never reach an Android build.
#[cfg(all(feature = "test-server", target_os = "android"))]
compile_error!("the test-server feature is host-only and must never be enabled for Android");

mod background;
mod changes;
mod compose;
mod core;
mod credentials;
mod error;
mod jmap_account;
mod list;
mod oauth;
mod reading;
mod search;
mod settings;
mod submit;
#[cfg(feature = "test-server")]
mod test_server;
#[cfg(test)]
mod tests;
mod types;

pub use crate::core::{Core, DiscoveryObserver};
pub use background::{BackgroundCheck, NewMailInfo};
pub use compose::{
    compose_limits, is_valid_address, outbox_reason_default_message, outbox_reason_key,
    parse_recipient, parse_recipients, ComposeError, ComposeLimits, DraftAttachment, DraftCheck,
    DraftContent, DraftProblem, DraftSummary, DraftSyncOutcome, FfiAddress, FfiDraft, FfiIdentity,
    OutboxItem, OutboxItemState, OutboxReason, ReplyDraft, SavedDraft, SendStarted, ServerCopy,
    ATTACHMENT_WARN_BYTES, MAX_ATTACHMENTS, MAX_ATTACHMENT_BYTES, MAX_BODY_BYTES,
    MAX_DRAFT_ATTACHMENT_BYTES, MAX_RECIPIENTS, MAX_SUBJECT_CHARS, MAX_SUGGESTIONS,
};
pub use credentials::{CredentialProvider, FfiCredential, ProviderError};
pub use error::{
    discover_error_default_message, discover_error_message_key, verify_failure_default_message,
    verify_failure_message_key, CoreError, DiscoverError, StorageReason, VerifyFailure,
};
pub use oauth::{OAuthFailure, OAuthSignedIn, OAuthStart};
pub use reading::{
    AttachmentInfo, BodyState, Conversation, ConversationMessage, MailAddress, MessageBody,
    ReadError, RemoteImagesSetting,
};
pub use search::{LocalResults, SearchFailure, SearchHit, ServerResults, TextRange};
#[cfg(feature = "test-server")]
pub use test_server::{TestImapServer, TestJmapServer, TestTokenEndpoint};
pub use types::*;

uniffi::setup_scaffolding!();

/// Version of this FFI surface. Version 2 replaced the free-text storage reason with
/// `StorageReason`. Version 3 is the mail surface: accounts, discovery, message lists, change
/// notices, actions and the credential provider. Version 4: `add_account` takes the new account's
/// credential, so the provider is only asked about accounts the app already stored. Version 5:
/// `MessageRow.thread_count` and `.recipients`, `AccountInfo.last_synced_ms` and
/// `MessageScope::Starred`. Version 6: `add_demo_account` (the sample mailbox) and
/// `AccountInfo.is_demo`. Version 7: `add_account` for an address that is already added, with the
/// same servers, updates that account and returns `AddedAccount { id, updated, update_token }`; it
/// takes `confirmed_fingerprint` and fails with `CoreError::CertificateChanged` when it would
/// replace, add or drop a trusted certificate pin without the person's confirmation of that
/// fingerprint (a pin is never dropped); the other servers give the new `CoreError::AccountExists`;
/// `verify` returns the pin it checked against (and checks against the stored pin of the same
/// account, and reports a self-signed certificate for an account stored without a pin as changed);
/// `commit_account_update` and `rollback_account_update` finish an update, each with its token.
/// Version 8 (T18c) is composing and sending: real accounts can send (the engine has an Outbox and an
/// SMTP submitter); drafts (`save_draft`, `load_draft`, `list_drafts`, `discard_draft`,
/// `sync_draft_to_server`, attachments), `check_draft`, `send`, `undo_send`, the Outbox actions
/// (`list_outbox`, `retry_outbox_item`, `edit_outbox_item`, `discard_outbox_item`),
/// `suggest_recipients`, `reply_draft`, `reply_all_draft`, `forward_draft`, `identities`, and the
/// helpers `parse_recipient`, `is_valid_address`, `compose_limits`; the new error `ComposeError`.
/// Version 9: reading (T18a): `open_conversation`, `load_body`, `save_attachment`,
/// `allow_remote_content`, `trust_sender`, `untrust_sender`, `trusted_senders`, with the records
/// `Conversation`, `ConversationMessage`, `MailAddress`, `MessageBody`, `AttachmentInfo`, the enums
/// `BodyState` and `RemoteImagesSetting`, and the error `ReadError`.
/// Bump on any change to what is
/// exported. (The `test-server` feature adds
/// a test-only object to the host library the JVM tests load; it is not part of the surface and
/// is never in the Android library.)
pub const FFI_API_VERSION: u32 = 12;

/// The version of the Croft core library, for example `0.1.0`.
#[uniffi::export]
pub fn croft_version() -> String {
    env!("CARGO_PKG_VERSION").to_owned()
}

/// The version of the FFI surface itself ([`FFI_API_VERSION`]).
#[uniffi::export]
pub fn ffi_api_version() -> u32 {
    FFI_API_VERSION
}
