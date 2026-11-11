// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Credentials: the callback interface the app implements, and the backend wrapper that holds
//! one account's adapter.
//!
//! The core never stores a secret. It asks the [`CredentialProvider`] when it builds an account's
//! adapter for a stored account (at `open`, at `signed_in`) and hands the answer straight to
//! `ImapBackend::new`, which keeps it in a `Zeroizing` string for as long as the adapter lives.
//! Nothing here writes a secret to SQLite, a log line or an error.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use croft_protocols::{
    BackendError, BackendFuture, Capabilities, Credential, FlagChange, FolderStatus, ImapBackend,
    ImapConfig, MailBackend, MoveOutcome, RemoteFolder, RemoteHeader, RemoteSnapshot, Uid,
};
use zeroize::{Zeroize, Zeroizing};

/// What proves who the person is, as the app hands it over: build one with [`FfiCredential::password`]
/// or [`FfiCredential::oauth2`].
///
/// This is an object, not an enum with fields, on purpose. UniFFI generates a Kotlin `data class`
/// for every enum variant that has fields, and a data class's `toString()` prints all of its
/// properties: a `String` field prints the secret, and so does a `ByteArray` field (Kotlin prints
/// the array's elements). An object has no fields on the Kotlin side, so it prints as
/// `FfiCredential@1a2b3c` and nothing else. The secret bytes live in Rust, in a `Zeroizing`
/// buffer that is wiped when the object is released (Kotlin `close()`, or the cleaner). The
/// constructors take the app's `ByteArray` (UTF-8), which the app can `fill(0)` as soon as the
/// constructor has returned. Bytes that are not UTF-8 count as "no credential". The core copies
/// the secret into the protocol layer's wiping credential for each use, so one object can serve
/// several calls. No `Clone`, no `PartialEq`, and `Debug` prints `***`. See the README for the
/// copies the core cannot wipe.
#[derive(uniffi::Object)]
pub struct FfiCredential {
    kind: Kind,
}

/// The two shapes of credential. The secrets are wiped on drop.
enum Kind {
    Password(Zeroizing<Vec<u8>>),
    OAuth2 {
        user: String,
        access_token: Zeroizing<Vec<u8>>,
    },
}

#[uniffi::export]
impl FfiCredential {
    /// An account password (or app password), as UTF-8 bytes.
    #[uniffi::constructor]
    pub fn password(secret: Vec<u8>) -> Arc<Self> {
        Arc::new(Self {
            kind: Kind::Password(Zeroizing::new(secret)),
        })
    }

    /// An OAuth 2 access token (UTF-8 bytes) and the account name it belongs to.
    #[uniffi::constructor]
    pub fn oauth2(user: String, access_token: Vec<u8>) -> Arc<Self> {
        Arc::new(Self {
            kind: Kind::OAuth2 {
                user,
                access_token: Zeroizing::new(access_token),
            },
        })
    }
}

impl std::fmt::Debug for FfiCredential {
    /// Prints `***` for everything.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.kind {
            Kind::Password(_) => f.write_str("FfiCredential::Password(***)"),
            Kind::OAuth2 { .. } => f.write_str("FfiCredential::OAuth2(***)"),
        }
    }
}

/// Copies secret bytes into a string that wipes itself. Bytes that are not UTF-8 are wiped and
/// give `None`.
fn secret_string(bytes: &[u8]) -> Option<Zeroizing<String>> {
    match String::from_utf8(bytes.to_vec()) {
        Ok(s) => Some(Zeroizing::new(s)),
        Err(e) => {
            // The error owns the copy: wipe it before it is dropped.
            let mut copy = e.into_bytes();
            copy.zeroize();
            None
        }
    }
}

impl FfiCredential {
    /// The protocol layer's wiping credential. The one copy made here is moved into it (no
    /// further copy); the bytes in `self` are wiped when the object is released. `None` when a
    /// secret is not valid UTF-8.
    pub(crate) fn to_credential(&self) -> Option<Credential> {
        match &self.kind {
            Kind::Password(secret) => {
                let mut secret = secret_string(secret)?;
                Some(Credential::password(std::mem::take(&mut *secret)))
            }
            Kind::OAuth2 { user, access_token } => {
                let mut token = secret_string(access_token)?;
                Some(Credential::oauth2(
                    user.clone(),
                    std::mem::take(&mut *token),
                ))
            }
        }
    }
}

/// The only error a callback into the app can report. It has no fields and no text on purpose:
/// when Kotlin code throws, UniFFI would otherwise turn the exception into a panic whose message
/// holds the exception text, and that text could carry a secret. A thrown exception maps to
/// `Failed` and the text is dropped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Error)]
pub enum ProviderError {
    /// The app's code failed (it threw, or reported a failure). The core treats it like "no
    /// credential".
    Failed,
}

impl std::fmt::Display for ProviderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("the app's callback failed")
    }
}

impl From<uniffi::UnexpectedUniFFICallbackError> for ProviderError {
    /// Any exception that is not a [`ProviderError`]: its text is discarded here.
    fn from(_: uniffi::UnexpectedUniFFICallbackError) -> Self {
        Self::Failed
    }
}

/// Implemented by the app (T11 on Android: the keystore). Called from Rust threads, never from a
/// UI thread; keep the answers quick.
///
/// A callback that throws is reported as [`ProviderError::Failed`] (see there); the core treats
/// `credential` failing like having no credential.
#[uniffi::export(with_foreign)]
pub trait CredentialProvider: Send + Sync {
    /// The credential for the account, or `None` when there is none (the account then starts in
    /// the needs-sign-in state and `sign_in_needed` is called).
    ///
    /// Called only for accounts the app has already stored a secret for (at `open` and
    /// `signed_in`). It is never called for an account being added: `add_account` takes that
    /// account's credential as an argument, and the app stores it under the returned id. The
    /// secret lives in the returned object, which wipes it when released; the app should wipe
    /// the `ByteArray` it built the object from.
    fn credential(&self, account_id: i64) -> Result<Option<Arc<FfiCredential>>, ProviderError>;

    /// The account needs the person to sign in again (no credential, or the server refused it).
    /// Called once each time the account enters that state; `signed_in` ends it.
    fn sign_in_needed(&self, account_id: i64) -> Result<(), ProviderError>;
}

/// Asks the provider. A failure on the app's side (or a credential that is not UTF-8) counts as
/// "no credential".
pub(crate) fn ask(provider: &dyn CredentialProvider, account: i64) -> Option<Credential> {
    provider
        .credential(account)
        .ok()
        .flatten()
        .and_then(|c| c.to_credential())
}

/// Tells the provider the account needs a sign-in. A failure on the app's side is ignored.
pub(crate) fn tell_sign_in_needed(provider: &dyn CredentialProvider, account: i64) {
    let _ = provider.sign_in_needed(account);
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
