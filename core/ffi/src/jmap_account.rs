// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! JMAP accounts (T22): the account's persistent id map, its sender, and the sign-in check.
//!
//! A JMAP account reads and sends through JMAP only (`JmapBackend`, `JmapSubmitter`, which files
//! the sent copy itself). Its UIDs come from the id map in SQLite (`SqliteJmapIdStore`, T09) so
//! they survive a restart. Drafts stay on the device for now (`DraftSyncOutcome::NotApplicable`):
//! JMAP draft copies are a later task.

use std::sync::{Arc, Mutex, PoisonError};

use croft_protocols::jmap::{IdStoreError, JmapIdStore};
use croft_protocols::{
    check_jmap_certificate, BackendError, BackendFuture, CertFingerprint, Credential, JmapBackend,
    JmapConfig, JmapSubmitter, MailBackend, MailSubmitter, MemoryIdStore, Uid,
};
use croft_storage::jmap_ids::SqliteJmapIdStore;
use croft_storage::Db;

use crate::credentials::{ask, AccountBackend, Asked, CredentialProvider, FfiCredential};
use crate::error::VerifyFailure;

/// One account's id map over the database connection the core keeps for JMAP id maps (shared by
/// every JMAP account; each call borrows it for the length of one transaction).
pub(crate) struct SharedIdStore {
    db: Arc<Mutex<Db>>,
    account: i64,
}

impl SharedIdStore {
    pub(crate) fn new(db: Arc<Mutex<Db>>, account: i64) -> Self {
        Self { db, account }
    }

    fn with<T>(
        &self,
        f: impl FnOnce(&SqliteJmapIdStore<&mut Db>) -> Result<T, IdStoreError>,
    ) -> Result<T, IdStoreError> {
        let mut db = self.db.lock().unwrap_or_else(PoisonError::into_inner);
        let store = SqliteJmapIdStore::new(&mut *db, self.account);
        f(&store)
    }
}

impl JmapIdStore for SharedIdStore {
    fn uid_validity(&self, mailbox: &str) -> Result<u32, IdStoreError> {
        self.with(|s| s.uid_validity(mailbox))
    }

    fn reset_mailbox(&self, mailbox: &str) -> Result<u32, IdStoreError> {
        self.with(|s| s.reset_mailbox(mailbox))
    }

    fn forget_mailbox(&self, mailbox: &str) -> Result<(), IdStoreError> {
        self.with(|s| s.forget_mailbox(mailbox))
    }

    fn next_uid(&self, mailbox: &str) -> Result<Uid, IdStoreError> {
        self.with(|s| s.next_uid(mailbox))
    }

    fn assign(&self, mailbox: &str, email_ids: &[String]) -> Result<Vec<Uid>, IdStoreError> {
        self.with(|s| s.assign(mailbox, email_ids))
    }

    fn email_ids(
        &self,
        mailbox: &str,
        uids: &[Uid],
    ) -> Result<std::collections::BTreeMap<Uid, String>, IdStoreError> {
        self.with(|s| s.email_ids(mailbox, uids))
    }

    fn entries(
        &self,
        mailbox: &str,
    ) -> Result<std::collections::BTreeMap<Uid, String>, IdStoreError> {
        self.with(|s| s.entries(mailbox))
    }

    fn remove(&self, mailbox: &str, email_ids: &[String]) -> Result<(), IdStoreError> {
        self.with(|s| s.remove(mailbox, email_ids))
    }

    fn email_state(&self) -> Result<Option<String>, IdStoreError> {
        self.with(|s| s.email_state())
    }

    fn set_email_state(&self, state: Option<&str>) -> Result<(), IdStoreError> {
        self.with(|s| s.set_email_state(state))
    }
}

/// Sends a JMAP account's mail with EmailSubmission. The credential is asked of the app at each
/// send (as for SMTP) and dropped after. The server files the sent copy itself.
pub(crate) struct JmapAccountSubmitter {
    account: i64,
    config: JmapConfig,
    provider: Arc<dyn CredentialProvider>,
}

impl JmapAccountSubmitter {
    pub(crate) fn new(
        account: i64,
        config: JmapConfig,
        provider: Arc<dyn CredentialProvider>,
    ) -> Self {
        Self {
            account,
            config,
            provider,
        }
    }
}

impl MailSubmitter for JmapAccountSubmitter {
    fn send<'a>(
        &'a self,
        envelope_from: &'a str,
        recipients: &'a [String],
        mime: &'a [u8],
    ) -> BackendFuture<'a, ()> {
        Box::pin(async move {
            let provider = self.provider.clone();
            let account = self.account;
            let asked = tokio::task::spawn_blocking(move || ask(&*provider, account))
                .await
                .unwrap_or(Asked::NotReady);
            let credential = match asked {
                Asked::Credential(c) => c,
                Asked::NoCredential | Asked::OAuth(_) => {
                    return Err(BackendError::Auth("sign-in needed".into()))
                }
                Asked::NotReady => {
                    return Err(BackendError::Transient(
                        "secure storage is not ready".into(),
                    ))
                }
            };
            JmapSubmitter::new(self.config.clone(), credential)?
                .send(envelope_from, recipients, mime)
                .await
                .map(|_| ())
        })
    }

    fn files_sent_itself(&self) -> bool {
        true
    }

    fn file_in_sent<'a>(
        &'a self,
        _message_id: Option<&'a str>,
        _mime: &'a [u8],
    ) -> BackendFuture<'a, ()> {
        // Never called: the submission files the copy (`files_sent_itself`).
        Box::pin(async { Ok(()) })
    }
}

/// Builds a JMAP account's backend: no credential yet (the caller sets it).
pub(crate) fn backend(
    config: JmapConfig,
    ids_db: &Arc<Mutex<Db>>,
    provider: Arc<dyn CredentialProvider>,
    account: i64,
) -> AccountBackend {
    AccountBackend::new_jmap(
        config,
        Arc::new(SharedIdStore::new(ids_db.clone(), account)),
        provider,
        account,
    )
}

/// Checks a JMAP account's settings and password (nothing is stored; the id map is a throwaway
/// one). **The certificate is checked first, without the credential**, exactly as for IMAP: the
/// password is only ever sent to a server whose certificate chains to a trusted root or is the
/// one pinned (`pin`: the person confirmed it, or the stored account's). A self-signed one that is
/// not pinned yet is the trust question (`UntrustedCertificate`, or `CertificateChanged` when the
/// stored account of this address used to reach the server without a pin); one that differs from
/// the pin is `CertificateChanged`; an expired one or one for another name is
/// `CertificateInvalid`. Returns the pin the check used.
pub(crate) async fn verify(
    config: JmapConfig,
    credential: &FfiCredential,
    pin: Option<CertFingerprint>,
    had_no_pin: bool,
) -> Result<Option<String>, VerifyFailure> {
    use croft_protocols::CertificateProblem as P;
    let config = match pin {
        Some(p) => config.with_pinned_cert(p),
        None => config,
    };
    match check_jmap_certificate(&config).await {
        Ok(()) => {}
        Err(P::UnknownIssuer { fingerprint }) if pin.is_none() => {
            let fingerprint = fingerprint.to_string();
            return Err(if had_no_pin {
                VerifyFailure::CertificateChanged { fingerprint }
            } else {
                VerifyFailure::UntrustedCertificate { fingerprint }
            });
        }
        Err(P::PinMismatch { fingerprint }) => {
            return Err(VerifyFailure::CertificateChanged {
                fingerprint: fingerprint.to_string(),
            })
        }
        Err(P::CannotConnect) => return Err(VerifyFailure::CannotConnect),
        Err(P::NoSecureConnection) => return Err(VerifyFailure::NoSecureConnection),
        Err(_) => return Err(VerifyFailure::CertificateInvalid),
    }
    // A JMAP account signs in with a password only (OAuth is for Gmail and Outlook over IMAP).
    let c: Credential = match credential.to_asked() {
        Asked::Credential(c @ Credential::Password(_)) => c,
        _ => return Err(VerifyFailure::WrongPassword),
    };
    let backend = JmapBackend::new(config, c, Arc::new(MemoryIdStore::new()))
        .map_err(|_| VerifyFailure::BadSettings)?;
    match backend.list_folders().await {
        Ok(_) => Ok(pin.map(|p| p.to_string())),
        Err(BackendError::Auth(_)) => Err(VerifyFailure::WrongPassword),
        Err(e) => Err(failure_of(&e)),
    }
}

fn failure_of(e: &BackendError) -> VerifyFailure {
    match e {
        BackendError::Auth(_) => VerifyFailure::WrongPassword,
        BackendError::Transient(_) => VerifyFailure::CannotConnect,
        BackendError::Permanent(_) => VerifyFailure::ServerProblem,
    }
}
