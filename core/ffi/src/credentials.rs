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
use std::time::{Duration, Instant};

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
    /// The app's secure storage is not ready right now (the platform keystore is busy or
    /// restarting) and cannot say whether a credential exists. From `credential` this is a
    /// transient failure of the connection attempt: the account is not paused, `sign_in_needed`
    /// is not called, and the core asks again later with a growing delay. From `sign_in_needed`
    /// it is ignored, like `Failed`.
    NotReady,
}

impl std::fmt::Display for ProviderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Failed => f.write_str("the app's callback failed"),
            Self::NotReady => f.write_str("the app's secure storage is not ready"),
        }
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
/// `credential` failing like having no credential. [`ProviderError::NotReady`] is the one answer
/// that is not "no credential": it means "ask again later".
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

/// What the provider answered when asked for a stored account's credential.
pub(crate) enum Asked {
    /// A credential.
    Credential(Credential),
    /// None: no secret stored, a failure on the app's side, or a credential that is not UTF-8.
    NoCredential,
    /// The app's secure storage is not ready: ask again later, and do not call it a sign-in problem.
    NotReady,
}

/// Asks the provider. A failure on the app's side (or a credential that is not UTF-8) counts as
/// "no credential"; [`ProviderError::NotReady`] is kept apart.
pub(crate) fn ask(provider: &dyn CredentialProvider, account: i64) -> Asked {
    match provider.credential(account) {
        Ok(Some(c)) => c
            .to_credential()
            .map_or(Asked::NoCredential, Asked::Credential),
        Ok(None) | Err(ProviderError::Failed) => Asked::NoCredential,
        Err(ProviderError::NotReady) => Asked::NotReady,
    }
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
    /// The app's secure storage was not ready when the credential was asked for. Every call
    /// answers `Transient` (the engine retries) until `retry_at`, then the next call asks the
    /// provider again. `failures` counts the answers that were `NotReady` in a row.
    NotReady { retry_at: Instant, failures: u32 },
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
    /// The app's secure storage was not ready: not a sign-in problem; asked again later.
    NotReady,
    /// The settings are unusable.
    Broken,
}

/// The wait before asking the provider again after its `n`th `NotReady` in a row (`n >= 1`):
/// `base`, doubled each time, at most [`MAX_NOT_READY_DELAY`].
fn not_ready_delay(base: Duration, n: u32) -> Duration {
    base.saturating_mul(1u32 << n.saturating_sub(1).min(8))
        .min(MAX_NOT_READY_DELAY)
}

/// The longest wait between two asks while the app's secure storage is not ready.
const MAX_NOT_READY_DELAY: Duration = Duration::from_secs(300);

/// The first wait after a `NotReady` answer (short in this crate's own tests).
const NOT_READY_BASE_DELAY: Duration = if cfg!(test) {
    Duration::from_millis(20)
} else {
    Duration::from_secs(2)
};

/// One account's mail backend. The engine holds it for the life of the account; `set_credential`
/// swaps the adapter inside when the person signs in again, so the engine's slot never changes.
///
/// When the app's secure storage answers `NotReady` the backend is not signed out: it answers
/// `Transient` (so the engine keeps the account running and retries), and asks the provider again
/// on a later call, after a delay that doubles each time (2 s up to 5 min). `NoCredential` from
/// that later ask puts the account in the needs-sign-in state as usual.
pub(crate) struct AccountBackend {
    config: ImapConfig,
    provider: Arc<dyn CredentialProvider>,
    account: i64,
    not_ready_base: Duration,
    state: Mutex<State>,
}

impl AccountBackend {
    /// A backend with no credential yet. `provider` is asked again for account `account` when it
    /// said `NotReady`.
    pub(crate) fn new(
        config: ImapConfig,
        provider: Arc<dyn CredentialProvider>,
        account: i64,
    ) -> Self {
        Self::with_not_ready_base(config, provider, account, NOT_READY_BASE_DELAY)
    }

    /// Like [`AccountBackend::new`] with another first wait after `NotReady` (tests use a short one).
    pub(crate) fn with_not_ready_base(
        config: ImapConfig,
        provider: Arc<dyn CredentialProvider>,
        account: i64,
        not_ready_base: Duration,
    ) -> Self {
        Self {
            config,
            provider,
            account,
            not_ready_base,
            state: Mutex::new(State::NeedsSignIn),
        }
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The state an answer from the provider leads to, with the failures in a row so far.
    fn state_for(&self, asked: Asked, failures: u32) -> (State, Readiness) {
        match asked {
            Asked::NoCredential => (State::NeedsSignIn, Readiness::NeedsSignIn),
            Asked::NotReady => {
                let failures = failures.saturating_add(1);
                (
                    State::NotReady {
                        retry_at: Instant::now() + not_ready_delay(self.not_ready_base, failures),
                        failures,
                    },
                    Readiness::NotReady,
                )
            }
            Asked::Credential(c) => match ImapBackend::new(self.config.clone(), c) {
                Ok(b) => (State::Ready(Arc::new(b)), Readiness::Ready),
                Err(_) => (State::Broken, Readiness::Broken),
            },
        }
    }

    /// Builds the adapter from what the provider answered (or marks the backend as needing a
    /// sign-in when there is no credential, or as not ready when the app's storage is not). The
    /// previous adapter is dropped, which closes its connection.
    pub(crate) fn set_asked(&self, asked: Asked) -> Readiness {
        let (state, readiness) = self.state_for(asked, 0);
        *self.lock() = state;
        readiness
    }

    /// The adapter, asking the provider again first when it said `NotReady` and the wait is over.
    async fn current(&self) -> Result<Arc<ImapBackend>, BackendError> {
        let claimed = {
            let mut st = self.lock();
            match &*st {
                State::NotReady { retry_at, failures } if Instant::now() >= *retry_at => {
                    // This caller asks; the others find a later `retry_at` and answer Transient.
                    let failures = *failures;
                    *st = State::NotReady {
                        retry_at: Instant::now()
                            + not_ready_delay(self.not_ready_base, failures.saturating_add(1)),
                        failures: failures.saturating_add(1),
                    };
                    Some(failures)
                }
                _ => None,
            }
        };
        if let Some(failures) = claimed {
            let provider = self.provider.clone();
            let account = self.account;
            // The app's callback may take a moment: not on a runtime worker.
            let asked = tokio::task::spawn_blocking(move || ask(&*provider, account))
                .await
                .unwrap_or(Asked::NotReady);
            let (state, _) = self.state_for(asked, failures.saturating_add(1));
            let mut st = self.lock();
            // A sign-in that happened meanwhile (`set_asked`) is newer than this answer.
            if matches!(&*st, State::NotReady { .. }) {
                *st = state;
            }
        }
        self.ready()
    }

    /// The account's IMAP adapter, for what the [`MailBackend`] trait does not offer (saving a
    /// draft or a sent copy). Answers like every other call while the account has no credential
    /// (`Auth`) or the app's storage is not ready (`Transient`).
    pub(crate) async fn imap(&self) -> Result<Arc<ImapBackend>, BackendError> {
        self.current().await
    }

    fn ready(&self) -> Result<Arc<ImapBackend>, BackendError> {
        match &*self.lock() {
            State::Ready(b) => Ok(b.clone()),
            State::NeedsSignIn => Err(BackendError::Auth("sign-in needed".into())),
            State::NotReady { .. } => Err(BackendError::Transient(
                "secure storage is not ready".into(),
            )),
            State::Broken => Err(BackendError::Permanent("unusable server settings".into())),
        }
    }
}

impl MailBackend for AccountBackend {
    fn capabilities(&self) -> BackendFuture<'_, Capabilities> {
        Box::pin(async move { self.current().await?.capabilities().await })
    }

    fn list_folders(&self) -> BackendFuture<'_, Vec<RemoteFolder>> {
        Box::pin(async move { self.current().await?.list_folders().await })
    }

    fn folder_status<'a>(&'a self, folder: &'a str) -> BackendFuture<'a, FolderStatus> {
        Box::pin(async move { self.current().await?.folder_status(folder).await })
    }

    fn snapshot<'a>(&'a self, folder: &'a str) -> BackendFuture<'a, RemoteSnapshot> {
        Box::pin(async move { self.current().await?.snapshot(folder).await })
    }

    fn fetch_headers<'a>(
        &'a self,
        folder: &'a str,
        uids: &'a [Uid],
    ) -> BackendFuture<'a, Vec<RemoteHeader>> {
        Box::pin(async move { self.current().await?.fetch_headers(folder, uids).await })
    }

    fn find_by_message_id<'a>(
        &'a self,
        folder: &'a str,
        message_id: &'a str,
    ) -> BackendFuture<'a, Option<Uid>> {
        Box::pin(async move {
            self.current()
                .await?
                .find_by_message_id(folder, message_id)
                .await
        })
    }

    fn set_flag<'a>(
        &'a self,
        folder: &'a str,
        uids: &'a [Uid],
        change: FlagChange,
    ) -> BackendFuture<'a, ()> {
        Box::pin(async move { self.current().await?.set_flag(folder, uids, change).await })
    }

    fn move_to<'a>(
        &'a self,
        folder: &'a str,
        uids: &'a [Uid],
        dest: &'a str,
    ) -> BackendFuture<'a, MoveOutcome> {
        Box::pin(async move { self.current().await?.move_to(folder, uids, dest).await })
    }

    fn delete_forever<'a>(&'a self, folder: &'a str, uids: &'a [Uid]) -> BackendFuture<'a, ()> {
        Box::pin(async move { self.current().await?.delete_forever(folder, uids).await })
    }
}
