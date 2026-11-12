// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Sending for a real account: [`AccountSubmitter`], the engine's [`MailSubmitter`] for an IMAP
//! account with an SMTP server.
//!
//! **The credential.** The password (or OAuth token) is not kept here. Each send asks the app's
//! [`CredentialProvider`] for it, builds a [`SmtpSender`] that holds it in the protocol layer's
//! wiping string, and drops the sender (wiping the secret) when the send is over. An OAuth token
//! is therefore as fresh as the app's provider makes it, and a password the person changed in the
//! meantime is used at once. No credential from the provider is `Auth` ("sign in needed": the
//! engine pauses the account's queue and its Outbox until `signed_in`); a keystore that is busy is
//! `Transient` (tried again later).
//!
//! **The certificate.** The pin, if any, is decided by `StoredSettings::smtp_config` (the SMTP pin
//! rule in `settings.rs`), not here.
//!
//! **The copy in Sent** goes over the account's IMAP adapter ([`file_copy_in_sent`]): looked up by
//! Message-ID first, so a retry never files a second copy. Gmail saves its own copy, so for it
//! nothing is appended (`files_sent_itself`).

use std::sync::Arc;

use croft_protocols::submit::file_copy_in_sent;
use croft_protocols::{BackendError, BackendFuture, MailSubmitter, SmtpConfig, SmtpSender};

use crate::credentials::{ask, AccountBackend, Asked, CredentialProvider};

/// Sends one account's mail over SMTP and files the copy over IMAP.
pub(crate) struct AccountSubmitter {
    account: i64,
    config: SmtpConfig,
    files_sent_itself: bool,
    provider: Arc<dyn CredentialProvider>,
    backend: Arc<AccountBackend>,
}

impl AccountSubmitter {
    /// `config` is the account's SMTP settings (pin rule applied by the caller).
    pub(crate) fn new(
        account: i64,
        config: SmtpConfig,
        files_sent_itself: bool,
        provider: Arc<dyn CredentialProvider>,
        backend: Arc<AccountBackend>,
    ) -> Self {
        Self {
            account,
            config,
            files_sent_itself,
            provider,
            backend,
        }
    }
}

impl MailSubmitter for AccountSubmitter {
    fn send<'a>(
        &'a self,
        envelope_from: &'a str,
        recipients: &'a [String],
        mime: &'a [u8],
    ) -> BackendFuture<'a, ()> {
        Box::pin(async move {
            let provider = self.provider.clone();
            let account = self.account;
            // The app's callback may take a moment: not on a runtime worker. If this future is
            // dropped while the callback runs, nothing has been sent yet.
            let asked = tokio::task::spawn_blocking(move || ask(&*provider, account))
                .await
                .unwrap_or(Asked::NotReady);
            let credential = match asked {
                Asked::Credential(c) => c,
                // The account's token session (shared with IMAP); a refused token is refreshed
                // once and the send tried once more.
                Asked::OAuth(_) => {
                    let first = match self.backend.oauth_credential(false).await {
                        Some(r) => r?,
                        None => return Err(BackendError::Auth("sign-in needed".into())),
                    };
                    let sent = SmtpSender::new(self.config.clone(), first)?
                        .send(envelope_from, recipients, mime)
                        .await;
                    return match sent {
                        Err(BackendError::Auth(_)) => {
                            let again = match self.backend.oauth_credential(true).await {
                                Some(r) => r?,
                                None => return Err(BackendError::Auth("sign-in needed".into())),
                            };
                            SmtpSender::new(self.config.clone(), again)?
                                .send(envelope_from, recipients, mime)
                                .await
                                .map(|_| ())
                        }
                        other => other.map(|_| ()),
                    };
                }
                Asked::NoCredential => return Err(BackendError::Auth("sign-in needed".into())),
                Asked::NotReady => {
                    return Err(BackendError::Transient(
                        "secure storage is not ready".into(),
                    ))
                }
            };
            let sender = SmtpSender::new(self.config.clone(), credential)?;
            sender
                .send(envelope_from, recipients, mime)
                .await
                .map(|_| ())
        })
    }

    fn files_sent_itself(&self) -> bool {
        self.files_sent_itself
    }

    fn file_in_sent<'a>(
        &'a self,
        message_id: Option<&'a str>,
        mime: &'a [u8],
    ) -> BackendFuture<'a, ()> {
        Box::pin(async move {
            let imap = self.backend.imap().await?;
            file_copy_in_sent(&imap, message_id, mime).await
        })
    }
}
