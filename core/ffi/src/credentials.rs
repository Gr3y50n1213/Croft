// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Credentials: the callback interface the app implements, and the backend wrapper that holds
//! one account's adapter.
//!
//! The core never stores a secret. It asks the [`CredentialProvider`] when it builds an account's
//! adapter (at `open`, at `add_account`, at `signed_in`) and hands the answer straight to
//! `ImapBackend::new`, which keeps it in a `Zeroizing` string for as long as the adapter lives.
//! Nothing here writes a secret to SQLite, a log line or an error.

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use croft_protocols::{
    BackendError, BackendFuture, Capabilities, Credential, FlagChange, FolderStatus, ImapBackend,
    ImapConfig, MailBackend, MoveOutcome, RemoteFolder, RemoteHeader, RemoteSnapshot, Uid,
};

/// What proves who the person is, as the app hands it over. The core wraps the strings in
/// `Zeroizing` on arrival.
#[derive(Clone, PartialEq, Eq, uniffi::Enum)]
pub enum FfiCredential {
    /// An account password (or app password).
    Password {
        /// The secret.
        secret: String,
    },
    /// An OAuth 2 access token.
    OAuth2 {
        /// The account name the token belongs to.
        user: String,
        /// The current access token.
        access_token: String,
    },
}

impl std::fmt::Debug for FfiCredential {
    /// Prints `***` for everything.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Password { .. } => f.write_str("FfiCredential::Password(***)"),
            Self::OAuth2 { .. } => f.write_str("FfiCredential::OAuth2(***)"),
        }
    }
}

impl FfiCredential {
    /// Moves the strings into the protocol layer's wiping credential (no copy is made).
    pub(crate) fn into_credential(self) -> Credential {
        match self {
            Self::Password { secret } => Credential::password(secret),
            Self::OAuth2 { user, access_token } => Credential::oauth2(user, access_token),
        }
    }
}

/// Implemented by the app (T11 on Android: the keystore). Called from Rust threads, never from a
/// UI thread; keep the answers quick.
#[uniffi::export(with_foreign)]
pub trait CredentialProvider: Send + Sync {
    /// The credential for the account, or `None` when there is none (the account then starts in
    /// the needs-sign-in state and `sign_in_needed` is called).
    ///
    /// For an account being added, this is called before `add_account` returns, with the id of the
    /// new row: the app answers with the secret it has just verified and stores it under that id.
    fn credential(&self, account_id: i64) -> Option<FfiCredential>;

    /// The account needs the person to sign in again (no credential, or the server refused it).
    /// Called once each time the account enters that state; `signed_in` ends it.
    fn sign_in_needed(&self, account_id: i64);
}

/// Asks the provider. A panic or exception on the app's side counts as "no credential".
pub(crate) fn ask(provider: &dyn CredentialProvider, account: i64) -> Option<Credential> {
    catch_unwind(AssertUnwindSafe(|| provider.credential(account)))
        .ok()
        .flatten()
        .map(FfiCredential::into_credential)
}

/// Tells the provider the account needs a sign-in. A panic on the app's side is ignored.
pub(crate) fn tell_sign_in_needed(provider: &dyn CredentialProvider, account: i64) {
    let _ = catch_unwind(AssertUnwindSafe(|| provider.sign_in_needed(account)));
}

/// What an account's backend can do right now.
#[derive(Clone)]
enum State {
    /// No credential: every call answers `Auth`, so the engine pauses the account and reports the
    /// sign-in.
    NeedsSignIn,
    /// Ready.
    Ready(Arc<ImapBackend>),
    /// The settings are unusable: every call answers `Permanent`.
    Broken,
}

/// Where an account's backend stands after a credential was set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Readiness {
    /// A credential was given and the adapter was built.
    Ready,
    /// No credential.
    NeedsSignIn,
    /// The settings are unusable.
    Broken,
}

/// One account's mail backend. The engine holds it for the life of the account; `set_credential`
/// swaps the adapter inside when the person signs in again, so the engine's slot never changes.
pub(crate) struct AccountBackend {
    config: ImapConfig,
    state: Mutex<State>,
}

impl AccountBackend {
    /// A backend with no credential yet.
    pub(crate) fn new(config: ImapConfig) -> Self {
        Self {
            config,
            state: Mutex::new(State::NeedsSignIn),
        }
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Builds the adapter from the credential (or marks the backend as needing a sign-in when
    /// there is none). The previous adapter is dropped, which closes its connection.
    pub(crate) fn set_credential(&self, credential: Option<Credential>) -> Readiness {
        let (state, readiness) = match credential {
            None => (State::NeedsSignIn, Readiness::NeedsSignIn),
            Some(c) => match ImapBackend::new(self.config.clone(), c) {
                Ok(b) => (State::Ready(Arc::new(b)), Readiness::Ready),
                Err(_) => (State::Broken, Readiness::Broken),
            },
        };
        *self.lock() = state;
        readiness
    }

    fn current(&self) -> Result<Arc<ImapBackend>, BackendError> {
        match &*self.lock() {
            State::Ready(b) => Ok(b.clone()),
            State::NeedsSignIn => Err(BackendError::Auth("sign-in needed".into())),
            State::Broken => Err(BackendError::Permanent("unusable server settings".into())),
        }
    }
}

impl MailBackend for AccountBackend {
    fn capabilities(&self) -> BackendFuture<'_, Capabilities> {
        match self.current() {
            Ok(b) => Box::pin(async move { b.capabilities().await }),
            Err(e) => Box::pin(async move { Err(e) }),
        }
    }

    fn list_folders(&self) -> BackendFuture<'_, Vec<RemoteFolder>> {
        match self.current() {
            Ok(b) => Box::pin(async move { b.list_folders().await }),
            Err(e) => Box::pin(async move { Err(e) }),
        }
    }

    fn folder_status<'a>(&'a self, folder: &'a str) -> BackendFuture<'a, FolderStatus> {
        match self.current() {
            Ok(b) => Box::pin(async move { b.folder_status(folder).await }),
            Err(e) => Box::pin(async move { Err(e) }),
        }
    }

    fn snapshot<'a>(&'a self, folder: &'a str) -> BackendFuture<'a, RemoteSnapshot> {
        match self.current() {
            Ok(b) => Box::pin(async move { b.snapshot(folder).await }),
            Err(e) => Box::pin(async move { Err(e) }),
        }
    }

    fn fetch_headers<'a>(
        &'a self,
        folder: &'a str,
        uids: &'a [Uid],
    ) -> BackendFuture<'a, Vec<RemoteHeader>> {
        match self.current() {
            Ok(b) => Box::pin(async move { b.fetch_headers(folder, uids).await }),
            Err(e) => Box::pin(async move { Err(e) }),
        }
    }

    fn find_by_message_id<'a>(
        &'a self,
        folder: &'a str,
        message_id: &'a str,
    ) -> BackendFuture<'a, Option<Uid>> {
        match self.current() {
            Ok(b) => Box::pin(async move { b.find_by_message_id(folder, message_id).await }),
            Err(e) => Box::pin(async move { Err(e) }),
        }
    }

    fn set_flag<'a>(
        &'a self,
        folder: &'a str,
        uids: &'a [Uid],
        change: FlagChange,
    ) -> BackendFuture<'a, ()> {
        match self.current() {
            Ok(b) => Box::pin(async move { b.set_flag(folder, uids, change).await }),
            Err(e) => Box::pin(async move { Err(e) }),
        }
    }

    fn move_to<'a>(
        &'a self,
        folder: &'a str,
        uids: &'a [Uid],
        dest: &'a str,
    ) -> BackendFuture<'a, MoveOutcome> {
        match self.current() {
            Ok(b) => Box::pin(async move { b.move_to(folder, uids, dest).await }),
            Err(e) => Box::pin(async move { Err(e) }),
        }
    }

    fn delete_forever<'a>(&'a self, folder: &'a str, uids: &'a [Uid]) -> BackendFuture<'a, ()> {
        match self.current() {
            Ok(b) => Box::pin(async move { b.delete_forever(folder, uids).await }),
            Err(e) => Box::pin(async move { Err(e) }),
        }
    }
}
