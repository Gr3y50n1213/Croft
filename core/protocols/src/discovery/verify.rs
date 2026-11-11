// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Phase two: sign in with the settings that were found.
//!
//! [`verify`] connects with the IMAP adapter, signs in, reads the server's capabilities and signs
//! out. The password is sent only inside the TLS connection the adapter sets up (discovery never
//! produces an unencrypted choice), and only to the host named in the settings.
//!
//! The certificate story, which the sign-in screen (S04) follows:
//!
//! 1. `verify` with no pin. A server with a certificate that does not chain to a trusted root
//!    (self-signed, expired, wrong name) fails with [`VerifyError::UntrustedCertificate`], which
//!    carries the certificate's SHA-256 fingerprint. Nothing secret was sent: the connection was
//!    refused during the TLS handshake.
//! 2. The screen shows the fingerprint and asks once whether to trust it.
//! 3. On yes, call `verify` again with [`VerifyOptions::pinned_cert`] set to that fingerprint,
//!    and store it with the account (the returned [`Verified::imap`] already carries it). If the
//!    certificate later changes, the pinned connection fails with
//!    [`VerifyError::CertificateChanged`].

use super::{DiscoveredSettings, Provider};
use crate::imap::{probe_certificate, CertFingerprint, Credential, ImapBackend, ImapConfig};
use crate::types::{BackendError, Capabilities};
use std::time::Duration;

/// Options for [`verify`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VerifyOptions {
    /// Accept exactly this certificate (the person's one-time "trust this certificate" answer).
    pub pinned_cert: Option<CertFingerprint>,
    /// Longest wait for connecting, for the TLS handshake and for each reply. Default 15 seconds.
    pub timeout: Duration,
}

impl Default for VerifyOptions {
    fn default() -> Self {
        Self {
            pinned_cert: None,
            timeout: Duration::from_secs(15),
        }
    }
}

/// A successful sign-in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Verified {
    /// Settings that work, including the pinned certificate when one was used. Build the
    /// account's [`ImapBackend`] from this.
    pub imap: ImapConfig,
    /// What the server can do.
    pub capabilities: Capabilities,
}

/// Why signing in failed. Each variant has its own plain-language [`message`](Self::message).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VerifyError {
    /// The server rejected the address or password.
    WrongPassword,
    /// The server rejected the sign-in and belongs to a provider that does not accept ordinary
    /// passwords from apps (Gmail, Outlook, iCloud, Fastmail). The message explains that
    /// provider's way in.
    NeedsAppPasswordOrOAuth(Provider),
    /// The server's certificate is not trusted (self-signed, expired or for another name). The
    /// fingerprint is what the screen shows; see the module documentation.
    UntrustedCertificate {
        /// SHA-256 of the server's certificate.
        fingerprint: CertFingerprint,
    },
    /// A certificate was pinned and the server now presents a different one.
    CertificateChanged {
        /// SHA-256 of the certificate the server presented now.
        fingerprint: CertFingerprint,
    },
    /// The server could not be reached, or did not answer in time.
    CannotConnect,
    /// The server does not offer encryption Croft accepts (TLS 1.2 or newer, or STARTTLS).
    NoSecureConnection,
    /// The server answered but something else went wrong (it refused for another reason).
    ServerProblem,
    /// The domain offers JMAP and no IMAP, and Croft cannot sign in over JMAP yet.
    JmapOnly,
    /// The settings cannot be used (for example no host).
    BadSettings,
}

impl VerifyError {
    /// A plain-language explanation for the person. Fixed text: it contains no address, host,
    /// fingerprint or server wording. The fingerprint is shown by the screen next to it.
    pub fn message(&self) -> &'static str {
        match self {
            Self::WrongPassword => {
                "The server did not accept that email address and password. Check them and try again."
            }
            Self::NeedsAppPasswordOrOAuth(Provider::Gmail) => {
                "Google did not accept that sign-in. Gmail does not allow your normal password in apps like Croft. Turn on 2-Step Verification, create an app password in your Google account, and use that here (or sign in with your own Google client)."
            }
            Self::NeedsAppPasswordOrOAuth(Provider::Outlook) => {
                "Microsoft did not accept that sign-in. Outlook.com and Microsoft 365 do not allow a normal password in apps like Croft: sign in with Microsoft instead."
            }
            Self::NeedsAppPasswordOrOAuth(Provider::ICloud) => {
                "Apple did not accept that sign-in. iCloud needs an app-specific password, not your Apple ID password. Create one in your Apple ID account settings and use it here."
            }
            Self::NeedsAppPasswordOrOAuth(Provider::Fastmail) => {
                "Fastmail did not accept that sign-in. Fastmail needs an app password, not your account password. Create one in your Fastmail settings and use it here."
            }
            Self::UntrustedCertificate { .. } => {
                "This server's security certificate cannot be checked automatically. It may be self-signed, which is common on servers people run themselves. Compare the fingerprint with the one your server shows, and trust it only if they match."
            }
            Self::CertificateChanged { .. } => {
                "This server's security certificate is not the one you trusted before. It may have been renewed, or someone may be intercepting the connection. Check with the server's owner before trusting the new one."
            }
            Self::CannotConnect => {
                "Croft could not connect to the server. Check your internet connection and the server address, then try again."
            }
            Self::NoSecureConnection => {
                "The server does not offer a secure connection that Croft accepts. Croft never sends a password without encryption."
            }
            Self::ServerProblem => {
                "The server answered but would not let Croft in. Try again later or ask the server's owner."
            }
            Self::JmapOnly => {
                "This server offers JMAP, which Croft cannot sign in with yet. If it also offers IMAP, enter those settings yourself."
            }
            Self::BadSettings => {
                "The server settings are not usable. Check the host and port and try again."
            }
        }
    }
}

impl std::fmt::Display for VerifyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.message())
    }
}

impl std::error::Error for VerifyError {}

/// Signs in with the IMAP settings in `settings` and the given credential. See the module
/// documentation for the certificate flow.
///
/// No email address, password or server text appears in the error.
pub async fn verify(
    settings: &DiscoveredSettings,
    credential: Credential,
    options: VerifyOptions,
) -> Result<Verified, VerifyError> {
    let Some(server) = &settings.imap else {
        return Err(VerifyError::JmapOnly);
    };
    let mut config = server.to_config().with_timeout(options.timeout);
    if let Some(pin) = options.pinned_cert {
        config = config.with_pinned_cert(pin);
    }
    let backend =
        ImapBackend::new(config.clone(), credential).map_err(|_| VerifyError::BadSettings)?;
    use crate::backend::MailBackend;
    let result = backend.capabilities().await;
    backend.close().await;
    match result {
        Ok(capabilities) => Ok(Verified {
            imap: config,
            capabilities,
        }),
        Err(BackendError::Auth(_)) => Err(match settings.provider {
            Some(provider) => VerifyError::NeedsAppPasswordOrOAuth(provider),
            None => VerifyError::WrongPassword,
        }),
        Err(BackendError::Transient(_)) => Err(VerifyError::CannotConnect),
        Err(BackendError::Permanent(_)) => Err(explain_permanent(&config).await),
    }
}

/// A permanent failure before sign-in is a problem with the secure connection. Ask what the
/// server's certificate is (the probe sends no command and no credential): if the handshake
/// completes when nothing is trusted, the certificate is the problem and its fingerprint is the
/// answer; if it does not, the server offers no acceptable encryption.
async fn explain_permanent(config: &ImapConfig) -> VerifyError {
    match probe_certificate(config).await {
        Ok(fingerprint) if config.pinned_cert.is_some() => {
            VerifyError::CertificateChanged { fingerprint }
        }
        Ok(fingerprint) => VerifyError::UntrustedCertificate { fingerprint },
        Err(BackendError::Transient(_)) => VerifyError::CannotConnect,
        Err(_) => VerifyError::NoSecureConnection,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_message_is_distinct_and_leaks_nothing() {
        let fp = CertFingerprint::from_bytes([7; 32]);
        let all = [
            VerifyError::WrongPassword,
            VerifyError::NeedsAppPasswordOrOAuth(Provider::Gmail),
            VerifyError::NeedsAppPasswordOrOAuth(Provider::Outlook),
            VerifyError::NeedsAppPasswordOrOAuth(Provider::ICloud),
            VerifyError::NeedsAppPasswordOrOAuth(Provider::Fastmail),
            VerifyError::UntrustedCertificate { fingerprint: fp },
            VerifyError::CertificateChanged { fingerprint: fp },
            VerifyError::CannotConnect,
            VerifyError::NoSecureConnection,
            VerifyError::ServerProblem,
            VerifyError::JmapOnly,
            VerifyError::BadSettings,
        ];
        let mut seen = std::collections::HashSet::new();
        for e in &all {
            let m = e.message();
            assert!(seen.insert(m), "duplicate message: {m}");
            assert!(!m.contains("07:07") && !m.contains('{') && !m.contains('@'));
            assert_eq!(e.to_string(), m);
        }
    }
}
