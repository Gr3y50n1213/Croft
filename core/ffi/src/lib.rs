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
//! # What it offers (API version 3)
//!
//! - [`Core::open`] with the database key and a [`CredentialProvider`] the app implements.
//! - Accounts: `discover`, `verify`, `add_account`, `list_accounts`, `remove_account`, `signed_in`.
//! - Reading: `list_folders`, `list_messages` (keyset paging, one folder or the unified Inbox).
//! - Observing: `next_change(after)`, a numbered change log the app waits on.
//! - Actions: `mark_seen`, `set_flagged`, `archive`, `trash`, `move_to`, `delete_forever`, `undo`.
//! - Sync and network: `sync_now`, `set_network_available`, `set_undo_window_ms`.
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

mod changes;
mod core;
mod credentials;
mod error;
mod list;
mod settings;
#[cfg(test)]
mod tests;
mod types;

pub use crate::core::{Core, DiscoveryObserver};
pub use credentials::{CredentialProvider, FfiCredential, ProviderError};
pub use error::{
    discover_error_default_message, discover_error_message_key, verify_failure_default_message,
    verify_failure_message_key, CoreError, DiscoverError, StorageReason, VerifyFailure,
};
pub use types::*;

uniffi::setup_scaffolding!();

/// Version of this FFI surface. Version 2 replaced the free-text storage reason with
/// `StorageReason`. Version 3 is the mail surface: accounts, discovery, message lists, change
/// notices, actions and the credential provider. Version 4: `add_account` takes the new account's
/// credential, so the provider is only asked about accounts the app already stored. Bump on any
/// change to what is exported.
pub const FFI_API_VERSION: u32 = 4;

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
