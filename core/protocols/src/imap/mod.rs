// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The IMAP implementation of [`MailBackend`].
//!
//! One [`ImapBackend`] per account. It holds one authenticated session behind a mutex, connects
//! lazily, and reconnects when the connection has dropped (once per operation, since every
//! operation is safe to repeat). TLS is always on, with certificates validated against public
//! roots or a pinned fingerprint; see [`ImapConfig`] and [`Security`].
//!
//! Error classes: network failures, timeouts, a command that runs past its deadline or a server
//! that sends far too much, and servers that are busy (only a bracketed `[UNAVAILABLE]`,
//! `[INUSE]` or `[SERVERBUG]` code counts, never free text) are [`BackendError::Transient`]; a
//! rejected `LOGIN`/`AUTHENTICATE`, a server that hangs up while the answer is awaited, and a
//! certificate that cannot be verified are [`BackendError::Auth`] (the account pauses and the
//! queued operations are kept); a tagged `NO`, or a request the adapter refuses by design, is
//! [`BackendError::Permanent`]. Error text is fixed wording and never contains the password,
//! server text, message data or addresses. The details are in the `error` module.
//!
//! The adapter uses a patched copy of `async-imap` (`core/vendor/async-imap`, see its
//! `CROFT-PATCHES.md`): the stock crate logs whole commands, including the password.

// Build-time guard, for debug and release builds alike. Croft's patched async-imap (core/vendor)
// no longer logs commands or responses; this is the second lock. Nothing in this crate logs at
// `trace` level or secret content at any level, so if some other crate in the build turns on a
// finer level (`log/max_level_trace`, `log/release_max_level_trace`, ...), the build stops here
// and someone has to look at it. Cargo.toml keeps `log/max_level_debug`.
const _: () = assert!(
    (log::STATIC_MAX_LEVEL as usize) <= (log::LevelFilter::Debug as usize),
    "log's compile-time level is above Debug: some crate enabled log/max_level_trace or \
     log/release_max_level_trace. Croft allows at most Debug, so that nothing finer can ever \
     reach a logger (see core/vendor/async-imap/CROFT-PATCHES.md)."
);

mod config;
mod conn;
mod error;
mod stream;
mod text;
mod tls;

pub use config::{CertFingerprint, Credential, ImapConfig, Security};
pub use error::CertificateProblem;

use crate::backend::{BackendFuture, MailBackend};
use crate::types::*;
use conn::Conn;
use error::OpError;
use std::collections::HashMap;
use std::sync::{Arc, Mutex as StdMutex};
use tls::{Observed, Trust};
use tokio::sync::Mutex;

struct State {
    conn: Option<Conn>,
    /// Folder caches saved from a dropped connection, handed to the next one.
    saved_cache: HashMap<String, conn::FolderCache>,
}

/// One operation, as data, so a single place can run it and retry it on a fresh connection.
enum Op<'a> {
    Capabilities,
    ListFolders,
    FolderStatus(&'a str),
    Snapshot(&'a str),
    FetchHeaders(&'a str, &'a [Uid]),
    FindByMessageId(&'a str, &'a str),
    SetFlag(&'a str, &'a [Uid], FlagChange),
    MoveTo(&'a str, &'a [Uid], &'a str),
    DeleteForever(&'a str, &'a [Uid]),
    Append(&'a str, &'a [&'a str], &'a str),
    CreateFolder(&'a str),
}

/// What an [`Op`] produced.
enum Out {
    Capabilities(Capabilities),
    Folders(Vec<RemoteFolder>),
    Status(FolderStatus),
    Snapshot(RemoteSnapshot),
    Headers(Vec<RemoteHeader>),
    Uid(Option<Uid>),
    Moved(MoveOutcome),
    Done,
}

impl Op<'_> {
    async fn perform(&self, c: &mut Conn) -> Result<Out, OpError> {
        Ok(match *self {
            Op::Capabilities => Out::Capabilities(c.caps),
            Op::ListFolders => Out::Folders(c.list_folders().await?),
            Op::FolderStatus(f) => Out::Status(c.folder_status(f).await?),
            Op::Snapshot(f) => Out::Snapshot(c.snapshot(f).await?),
            Op::FetchHeaders(f, uids) => Out::Headers(c.fetch_headers(f, uids).await?),
            Op::FindByMessageId(f, id) => Out::Uid(c.find_by_message_id(f, id).await?),
            Op::SetFlag(f, uids, change) => {
                c.set_flag(f, uids, change).await?;
                Out::Done
            }
            Op::MoveTo(f, uids, dest) => Out::Moved(c.move_to(f, uids, dest).await?),
            Op::DeleteForever(f, uids) => {
                c.delete_forever(f, uids).await?;
                Out::Done
            }
            Op::Append(f, flags, msg) => Out::Uid(c.append(f, flags, msg).await?),
            Op::CreateFolder(f) => {
                c.create_folder(f).await?;
                Out::Done
            }
        })
    }
}

fn unexpected() -> BackendError {
    BackendError::Permanent("internal error: unexpected result".into())
}

/// An IMAP account: settings, the credential, and one lazily opened, reconnecting session.
pub struct ImapBackend {
    config: ImapConfig,
    credential: Credential,
    state: Mutex<State>,
}

impl std::fmt::Debug for ImapBackend {
    /// Prints the settings (host, port, security) and never the credential.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ImapBackend")
            .field("config", &self.config)
            .field("credential", &"***")
            .finish()
    }
}

impl ImapBackend {
    /// Creates the adapter. Nothing is sent yet: the first operation connects. Fails with
    /// [`BackendError::Permanent`] when the settings are unusable, for example plaintext to a
    /// host that is not localhost.
    pub fn new(config: ImapConfig, credential: Credential) -> Result<Self, BackendError> {
        config.validate()?;
        Ok(Self {
            config,
            credential,
            state: Mutex::new(State {
                conn: None,
                saved_cache: HashMap::new(),
            }),
        })
    }

    /// The settings this adapter was made with.
    pub fn config(&self) -> &ImapConfig {
        &self.config
    }

    /// Runs one operation on the session, connecting first if needed. If a connection that was
    /// opened by an earlier call turns out to be dead, reconnects once and repeats the operation.
    async fn run(&self, op: Op<'_>) -> Result<Out, BackendError> {
        let mut guard = self.state.lock().await;
        let state = &mut *guard;
        let mut attempts = 0;
        loop {
            attempts += 1;
            let fresh = state.conn.is_none();
            if fresh {
                let mut conn = Conn::open(&self.config, &self.credential)
                    .await
                    .map_err(|e| e.error)?;
                conn.cache = std::mem::take(&mut state.saved_cache);
                state.conn = Some(conn);
            }
            let Some(conn) = state.conn.as_mut() else {
                return Err(BackendError::Transient("no connection".into()));
            };
            match op.perform(conn).await {
                Ok(v) => return Ok(v),
                Err(e) => {
                    if e.broken {
                        if let Some(dead) = state.conn.take() {
                            state.saved_cache = dead.cache;
                        }
                    }
                    if e.broken && e.reconnect_and_retry && !fresh && attempts < 2 {
                        continue;
                    }
                    return Err(e.error);
                }
            }
        }
    }

    /// Signs out and closes the connection. The adapter can be used again afterwards: the next
    /// call reconnects.
    pub async fn close(&self) {
        let mut state = self.state.lock().await;
        if let Some(mut conn) = state.conn.take() {
            conn.logout().await;
            state.saved_cache = conn.cache;
        }
    }

    /// Creates a folder (for example an Archive when the server has none). Succeeds if it
    /// already exists. Not part of [`MailBackend`] yet; see [`append_message`](Self::append_message).
    pub fn create_folder<'a>(&'a self, folder: &'a str) -> BackendFuture<'a, ()> {
        Box::pin(async move {
            match self.run(Op::CreateFolder(folder)).await? {
                Out::Done => Ok(()),
                _ => Err(unexpected()),
            }
        })
    }

    /// Adds a message to a folder, with the given flags (for example `\Seen`, `\Draft`), and
    /// returns the new UID when the server reports it (UIDPLUS `APPENDUID`).
    ///
    /// Not part of [`MailBackend`] yet: saving drafts and filing sent mail will need it later, and
    /// the integration tests use it to put messages on a server. `message` must be text
    /// (RFC 5322 with CRLF line ends).
    pub fn append_message<'a>(
        &'a self,
        folder: &'a str,
        flags: &'a [&'a str],
        message: &'a str,
    ) -> BackendFuture<'a, Option<Uid>> {
        Box::pin(async move {
            match self.run(Op::Append(folder, flags, message)).await? {
                Out::Uid(u) => Ok(u),
                _ => Err(unexpected()),
            }
        })
    }
}

impl MailBackend for ImapBackend {
    fn capabilities(&self) -> BackendFuture<'_, Capabilities> {
        Box::pin(async move {
            match self.run(Op::Capabilities).await? {
                Out::Capabilities(c) => Ok(c),
                _ => Err(unexpected()),
            }
        })
    }

    fn list_folders(&self) -> BackendFuture<'_, Vec<RemoteFolder>> {
        Box::pin(async move {
            match self.run(Op::ListFolders).await? {
                Out::Folders(v) => Ok(v),
                _ => Err(unexpected()),
            }
        })
    }

    fn folder_status<'a>(&'a self, folder: &'a str) -> BackendFuture<'a, FolderStatus> {
        Box::pin(async move {
            match self.run(Op::FolderStatus(folder)).await? {
                Out::Status(v) => Ok(v),
                _ => Err(unexpected()),
            }
        })
    }

    fn snapshot<'a>(&'a self, folder: &'a str) -> BackendFuture<'a, RemoteSnapshot> {
        Box::pin(async move {
            match self.run(Op::Snapshot(folder)).await? {
                Out::Snapshot(v) => Ok(v),
                _ => Err(unexpected()),
            }
        })
    }

    fn fetch_headers<'a>(
        &'a self,
        folder: &'a str,
        uids: &'a [Uid],
    ) -> BackendFuture<'a, Vec<RemoteHeader>> {
        Box::pin(async move {
            match self.run(Op::FetchHeaders(folder, uids)).await? {
                Out::Headers(v) => Ok(v),
                _ => Err(unexpected()),
            }
        })
    }

    fn find_by_message_id<'a>(
        &'a self,
        folder: &'a str,
        message_id: &'a str,
    ) -> BackendFuture<'a, Option<Uid>> {
        Box::pin(async move {
            match self.run(Op::FindByMessageId(folder, message_id)).await? {
                Out::Uid(v) => Ok(v),
                _ => Err(unexpected()),
            }
        })
    }

    fn set_flag<'a>(
        &'a self,
        folder: &'a str,
        uids: &'a [Uid],
        change: FlagChange,
    ) -> BackendFuture<'a, ()> {
        Box::pin(async move {
            match self.run(Op::SetFlag(folder, uids, change)).await? {
                Out::Done => Ok(()),
                _ => Err(unexpected()),
            }
        })
    }

    /// Moves messages with `UID MOVE`, or, without `MOVE`, with `UID COPY`, `\\Deleted` and expunge.
    ///
    /// On a server without `MOVE`, a retry after a dropped connection can leave duplicates in
    /// the destination (a chunk that was copied but not yet expunged is copied again). No message
    /// is ever lost: nothing is deleted from the source before it was copied. Without `UIDPLUS`
    /// the move refuses (`Permanent`) when another message in the folder is marked deleted,
    /// checked again before each chunk of 500 UIDs.
    fn move_to<'a>(
        &'a self,
        folder: &'a str,
        uids: &'a [Uid],
        dest: &'a str,
    ) -> BackendFuture<'a, MoveOutcome> {
        Box::pin(async move {
            match self.run(Op::MoveTo(folder, uids, dest)).await? {
                Out::Moved(v) => Ok(v),
                _ => Err(unexpected()),
            }
        })
    }

    /// Deletes messages for good. Without `UIDPLUS` it refuses (`Permanent`) when another message
    /// in the folder is marked deleted, checked again before each chunk of 500 UIDs, because the
    /// server's plain `EXPUNGE` would remove that message too.
    fn delete_forever<'a>(&'a self, folder: &'a str, uids: &'a [Uid]) -> BackendFuture<'a, ()> {
        Box::pin(async move {
            match self.run(Op::DeleteForever(folder, uids)).await? {
                Out::Done => Ok(()),
                _ => Err(unexpected()),
            }
        })
    }
}

/// Connects to a server, completes the TLS handshake **without trusting the certificate**, reads
/// the leaf certificate's SHA-256 fingerprint, and disconnects. No IMAP command and no credential
/// is ever sent.
///
/// This is for the account-setup screen: when a server's certificate does not validate, the
/// person is shown this fingerprint to compare with the one the server's owner published, and may
/// then pin it with [`ImapConfig::with_pinned_cert`]. Calling it trusts nothing by itself. Any
/// `pinned_cert` already in `config` is ignored. Returns `Permanent` for
/// [`Security::PlaintextForTestsOnly`], which has no certificate.
pub async fn probe_certificate(config: &ImapConfig) -> Result<CertFingerprint, BackendError> {
    if config.security == Security::PlaintextForTestsOnly {
        return Err(BackendError::Permanent(
            "an unencrypted connection has no certificate".into(),
        ));
    }
    let slot = Arc::new(StdMutex::new(None));
    let client = conn::connect_client(config, &Trust::Capture(slot.clone()))
        .await
        .map_err(|e| e.error)?;
    drop(client);
    let seen = slot.lock().ok().and_then(|g| *g);
    seen.ok_or_else(|| BackendError::Transient("the server sent no certificate".into()))
}

/// Checks the server's certificate the way a real connection would, **before any credential
/// exists**: connects, completes the TLS handshake with full validation (the public roots, or
/// `config.pinned_cert` when one is set), disconnects, and says why the certificate was refused,
/// if it was. No sign-in command is ever sent (for STARTTLS the only command is `STARTTLS`).
///
/// This is the typed way to find out that a certificate is the problem: callers must not guess it
/// from error text or from the class of a later sign-in error (see [`CertificateProblem`]). The
/// fingerprint in [`CertificateProblem::UnknownIssuer`] and [`CertificateProblem::PinMismatch`]
/// belongs to the certificate that was judged in this same handshake. A certificate that fails
/// here fails the same way in [`ImapBackend`], which maps it to [`BackendError::Auth`].
pub async fn check_certificate(config: &ImapConfig) -> Result<(), CertificateProblem> {
    if config.security == Security::PlaintextForTestsOnly {
        return Err(CertificateProblem::NoSecureConnection);
    }
    let slot = Arc::new(StdMutex::new(Observed::default()));
    let trust = Trust::Observe {
        pin: config.pinned_cert,
        slot: slot.clone(),
    };
    let connected = conn::connect_client(config, &trust).await;
    let observed = match slot.lock() {
        Ok(mut guard) => std::mem::take(&mut *guard),
        Err(_) => Observed::default(),
    };
    match connected {
        Ok(client) => {
            drop(client);
            Ok(())
        }
        Err(e) => Err(match observed {
            Observed {
                error: Some(error),
                fingerprint,
            } => CertificateProblem::from_tls(&error, fingerprint),
            _ => match e.error {
                BackendError::Transient(_) => CertificateProblem::CannotConnect,
                _ => CertificateProblem::NoSecureConnection,
            },
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_log_level_is_capped_at_debug_in_every_build() {
        // The same rule as the `const _` assertion above, kept as a test so a failure shows up in
        // the test report with an explanation as well as in the compiler output.
        assert!(log::STATIC_MAX_LEVEL <= log::LevelFilter::Debug);
    }

    #[test]
    fn plaintext_to_a_real_host_is_refused_at_construction() {
        let cfg = ImapConfig::new(
            "mail.example.org",
            143,
            Security::PlaintextForTestsOnly,
            "u",
        );
        let err = ImapBackend::new(cfg, Credential::password("pw")).err();
        assert!(matches!(err, Some(BackendError::Permanent(_))));
    }

    #[test]
    fn debug_of_the_backend_hides_the_credential() {
        let cfg = ImapConfig::tls("mail.example.org", "alice@example.org");
        let b = ImapBackend::new(cfg, Credential::password("hunter2")).expect("valid settings");
        let shown = format!("{b:?}");
        assert!(!shown.contains("hunter2") && !shown.contains("alice"));
    }
}
