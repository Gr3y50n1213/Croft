// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Phase two: sign in with the settings that were found.
//!
//! [`verify`] first checks the server's certificate, then connects with the IMAP adapter, signs in,
//! reads the server's capabilities and signs out. The password is sent only inside the TLS
//! connection the adapter sets up (discovery never produces an unencrypted choice), only to the
//! host named in the settings, and only after the certificate check passed.
//!
//! # The certificate check comes first
//!
//! [`check_certificate`] runs a TLS handshake with full validation and no credentials, and
//! returns a typed reason when the certificate is refused. Discovery never guesses a certificate
//! problem from the class or the text of a sign-in error: the adapter maps certificate failures to
//! `Auth` (so a queue pauses instead of failing), which is the same class as a wrong password.
//!
//! The story the sign-in screen (S04) follows:
//!
//! 1. `verify` with no pin. A server whose certificate does not chain to a trusted root
//!    (self-signed, or a private authority) fails with [`VerifyError::UntrustedCertificate`],
//!    which carries the certificate's SHA-256 fingerprint. Nothing secret was sent.
//! 2. The screen shows the fingerprint and asks once whether to trust it. This is offered **only**
//!    when the settings do not come from the provider table: a certificate problem on Gmail,
//!    Outlook, iCloud or Fastmail is [`VerifyError::ConnectionIntercepted`], which cannot be
//!    trusted.
//! 3. On yes, call `verify` again with [`VerifyOptions::pinned_cert`] set to that fingerprint,
//!    and store it with the account (the returned [`Verified::imap`] already carries it). If the
//!    certificate later changes, the check fails with [`VerifyError::CertificateChanged`].
//! 4. An expired certificate, one made for another name, or any other certificate fault is
//!    [`VerifyError::CertificateInvalid`]: there is nothing for the person to trust, so none is
//!    offered.
//!
//! # Why a self-signed certificate is offered whatever name it was made for
//!
//! A certificate that chains to nothing Croft trusts (`UnknownIssuer`) is offered for
//! trust-on-first-use even when it was made for another server name. The pinned connection checks
//! the exact SHA-256 fingerprint, which is stronger than a name check, and self-hosted servers
//! often use certificates whose name does not match how the person reaches them (an IP address,
//! a local name, a different alias). A certificate that chains to a trusted authority but is
//! expired or made for another name is refused without any offer: that is a fault the server's
//! owner has to fix.
//!
//! A provider-table domain never takes a pin: [`VerifyOptions::pinned_cert`] is ignored there.

use super::{DiscoveredSettings, Provider};
use crate::imap::{
    check_certificate, CertFingerprint, CertificateProblem, Credential, ImapBackend, ImapConfig,
};
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
    /// The server's certificate does not chain to a trusted authority (self-signed, or made by a
    /// private authority) and the settings are not a well-known provider's. The fingerprint is
    /// what the screen shows next to the offer to trust it once; see the module documentation.
    UntrustedCertificate {
        /// SHA-256 of the server's certificate.
        fingerprint: CertFingerprint,
    },
    /// A certificate was pinned and the server now presents a different one.
    CertificateChanged {
        /// SHA-256 of the certificate the server presented now.
        fingerprint: CertFingerprint,
    },
    /// A well-known provider's server (Gmail, Outlook, iCloud, Fastmail) presented a certificate
    /// that cannot be verified. Real providers always have valid certificates, so the connection
    /// is being intercepted (a hostile or captive network) or something is misconfigured. This
    /// can never be trusted, and the password was not sent.
    ConnectionIntercepted,
    /// The certificate is expired, made for another server name, not valid yet or faulty in
    /// another way. Nothing to trust: the server's owner has to fix it. The password was not sent.
    CertificateInvalid,
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
                "This server's security certificate was not issued by an authority Croft knows. It may be self-signed. That is expected only if you, or your organisation, set up this server with its own certificate. If you did not, do not trust it: someone may be intercepting the connection. If you did, compare the fingerprint with the one your server shows, and trust it only if they match. Your password has not been sent."
            }
            Self::CertificateChanged { .. } => {
                "This server's security certificate is not the one you trusted before. It may have been renewed, or someone may be intercepting the connection. Check with the server's owner before trusting the new one."
            }
            Self::ConnectionIntercepted => {
                "Croft could not confirm that it is talking to your email provider. The connection is being intercepted, or the provider's server is misconfigured. Your password was not sent. Try another network, and do not trust this connection."
            }
            Self::CertificateInvalid => {
                "This server's security certificate is not valid: it may have expired, be made for a different server name, or your device's clock may be wrong. Croft will not connect, and your password was not sent. Check the server address and the date on your device, or ask the server's owner."
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

/// Checks the certificate, then signs in with the IMAP settings in `settings` and the given
/// credential. See the module documentation for the certificate flow.
///
/// The credential is not used until the certificate check has passed. No email address, password
/// or server text appears in the error.
pub async fn verify(
    settings: &DiscoveredSettings,
    credential: Credential,
    options: VerifyOptions,
) -> Result<Verified, VerifyError> {
    let Some(server) = &settings.imap else {
        return Err(VerifyError::JmapOnly);
    };
    let settings = &*with_host_provider(settings);
    let mut config = server.to_config().with_timeout(options.timeout);
    // A provider-table domain never takes a pin: real providers have valid certificates, so a
    // pin could only ever pin an interceptor's certificate. The caller's pin is ignored for the
    // check and for the sign-in, and a certificate problem stays `ConnectionIntercepted`.
    if let Some(pin) = pin_for(settings, options.pinned_cert) {
        config = config.with_pinned_cert(pin);
    }
    let backend =
        ImapBackend::new(config.clone(), credential).map_err(|_| VerifyError::BadSettings)?;

    if let Err(problem) = check_certificate(&config).await {
        backend.close().await;
        return Err(explain_certificate(problem, settings, &config));
    }

    use crate::backend::MailBackend;
    let result = backend.capabilities().await;
    backend.close().await;
    match result {
        Ok(capabilities) => Ok(Verified {
            imap: config,
            capabilities,
        }),
        Err(BackendError::Auth(_)) => {
            // The certificate passed a moment ago, so this is normally a refused sign-in. A
            // certificate that changed in between is also `Auth` in the adapter, so look again
            // (no credential is sent) rather than call it a wrong password.
            // Only an actual certificate problem replaces the answer. If the server cannot be
            // reached now (it dropped us after the refusal), the refusal still stands.
            if let Err(problem) = check_certificate(&config).await {
                if !matches!(
                    problem,
                    CertificateProblem::CannotConnect | CertificateProblem::NoSecureConnection
                ) {
                    return Err(explain_certificate(problem, settings, &config));
                }
            }
            Err(match settings.provider {
                Some(provider) => VerifyError::NeedsAppPasswordOrOAuth(provider),
                None => VerifyError::WrongPassword,
            })
        }
        Err(BackendError::Transient(_)) => Err(VerifyError::CannotConnect),
        Err(BackendError::Permanent(_)) => Err(VerifyError::ServerProblem),
    }
}

/// The settings with `provider` filled in from the IMAP host when it is a listed provider's own
/// server. A provider's server is that provider's whatever the `provider` field says: the caller may
/// have changed the port or typed the host by hand, and that must not remove the protection against
/// trusting an interceptor's certificate.
fn with_host_provider(settings: &DiscoveredSettings) -> std::borrow::Cow<'_, DiscoveredSettings> {
    let from_host = settings
        .imap
        .as_ref()
        .and_then(|server| super::provider::provider_for_host(&server.host));
    match (settings.provider, from_host) {
        (None, Some(provider)) => std::borrow::Cow::Owned(DiscoveredSettings {
            provider: Some(provider),
            ..settings.clone()
        }),
        _ => std::borrow::Cow::Borrowed(settings),
    }
}

/// The pin the connection may use: the caller's, unless the settings are a provider's (a pin could
/// only ever pin an interceptor's certificate there).
fn pin_for(settings: &DiscoveredSettings, pin: Option<CertFingerprint>) -> Option<CertFingerprint> {
    if settings.provider.is_none() {
        pin
    } else {
        None
    }
}

/// Turns a typed certificate problem into what the person is told, and whether trusting is
/// offered: only for an unknown issuer, and never for a provider-table domain.
fn explain_certificate(
    problem: CertificateProblem,
    settings: &DiscoveredSettings,
    config: &ImapConfig,
) -> VerifyError {
    match problem {
        CertificateProblem::CannotConnect => VerifyError::CannotConnect,
        // On a provider domain a missing secure connection is a downgrade attempt, not a quirk.
        CertificateProblem::NoSecureConnection if settings.provider.is_some() => {
            VerifyError::ConnectionIntercepted
        }
        CertificateProblem::NoSecureConnection => VerifyError::NoSecureConnection,
        _ if settings.provider.is_some() => VerifyError::ConnectionIntercepted,
        CertificateProblem::UnknownIssuer { fingerprint } => {
            VerifyError::UntrustedCertificate { fingerprint }
        }
        CertificateProblem::PinMismatch { fingerprint }
            if config.pinned_cert != Some(fingerprint) =>
        {
            VerifyError::CertificateChanged { fingerprint }
        }
        CertificateProblem::PinMismatch { .. }
        | CertificateProblem::Expired
        | CertificateProblem::WrongName
        | CertificateProblem::Other => VerifyError::CertificateInvalid,
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
            VerifyError::ConnectionIntercepted,
            VerifyError::CertificateInvalid,
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

    fn found(provider: Option<Provider>) -> DiscoveredSettings {
        DiscoveredSettings {
            imap: None,
            smtp: None,
            jmap: None,
            source: super::super::Source::Srv,
            provider,
            auth_hint: super::super::AuthHint::Password,
        }
    }

    fn on_host(host: &str, provider: Option<Provider>) -> DiscoveredSettings {
        DiscoveredSettings {
            imap: Some(super::super::ImapServer {
                host: host.into(),
                port: 143,
                security: crate::imap::Security::StartTls,
                username: "u".into(),
            }),
            ..found(provider)
        }
    }

    #[test]
    fn a_providers_own_host_is_the_providers_even_when_the_provider_field_is_empty_and_never_takes_a_pin(
    ) {
        let pin = CertFingerprint::from_bytes([2; 32]);
        let fp = CertFingerprint::from_bytes([1; 32]);
        // The settings were edited by hand (another port, another security): `provider` was dropped.
        for (host, provider) in [
            ("imap.gmail.com", Provider::Gmail),
            ("outlook.office365.com", Provider::Outlook),
            ("IMAP.MAIL.ME.COM", Provider::ICloud),
            ("imap.fastmail.com", Provider::Fastmail),
        ] {
            let edited = on_host(host, None);
            let effective = with_host_provider(&edited);
            assert_eq!(effective.provider, Some(provider), "{host}");
            assert_eq!(pin_for(&effective, Some(pin)), None, "{host}: no pin");
            // An unknown issuer is interception, with no offer to trust it.
            assert_eq!(
                explain_certificate(
                    CertificateProblem::UnknownIssuer { fingerprint: fp },
                    &effective,
                    &ImapConfig::tls(host, "u")
                ),
                VerifyError::ConnectionIntercepted,
                "{host}"
            );
            // The same settings with the field kept give the same answer.
            assert_eq!(
                with_host_provider(&on_host(host, Some(provider))).provider,
                Some(provider)
            );
        }
        // A server of the person's own is untouched and may take a pin.
        let own = on_host("mail.example.org", None);
        let effective = with_host_provider(&own);
        assert_eq!(effective.provider, None);
        assert_eq!(pin_for(&effective, Some(pin)), Some(pin));
        // A well-known provider field with another host stays as it is.
        assert_eq!(
            with_host_provider(&on_host("mail.example.org", Some(Provider::Gmail))).provider,
            Some(Provider::Gmail)
        );
    }

    #[test]
    fn certificate_problems_map_to_one_error_each_and_trust_is_never_offered_to_providers() {
        use CertificateProblem as P;
        let fp = CertFingerprint::from_bytes([1; 32]);
        let pin = CertFingerprint::from_bytes([2; 32]);
        let plain = ImapConfig::tls("mail.example.org", "u");
        let pinned = plain.clone().with_pinned_cert(pin);
        let own = found(None);
        let table = found(Some(Provider::Fastmail));
        let ex = |p, s: &DiscoveredSettings, c: &ImapConfig| explain_certificate(p, s, c);

        assert_eq!(
            ex(P::UnknownIssuer { fingerprint: fp }, &own, &plain),
            VerifyError::UntrustedCertificate { fingerprint: fp }
        );
        assert_eq!(
            ex(P::PinMismatch { fingerprint: fp }, &own, &pinned),
            VerifyError::CertificateChanged { fingerprint: fp }
        );
        // "Changed" only when the fingerprint really differs from the pin.
        assert_eq!(
            ex(P::PinMismatch { fingerprint: pin }, &own, &pinned),
            VerifyError::CertificateInvalid
        );
        for p in [P::Expired, P::WrongName, P::Other] {
            assert_eq!(ex(p, &own, &plain), VerifyError::CertificateInvalid);
        }
        assert_eq!(
            ex(P::CannotConnect, &own, &plain),
            VerifyError::CannotConnect
        );
        assert_eq!(
            ex(P::NoSecureConnection, &own, &plain),
            VerifyError::NoSecureConnection
        );
        // On a provider domain, no secure connection is interception, not a plain failure.
        assert_eq!(
            ex(P::NoSecureConnection, &table, &plain),
            VerifyError::ConnectionIntercepted
        );
        for p in [
            P::UnknownIssuer { fingerprint: fp },
            P::Expired,
            P::WrongName,
            P::PinMismatch { fingerprint: fp },
            P::Other,
        ] {
            assert_eq!(ex(p, &table, &pinned), VerifyError::ConnectionIntercepted);
        }
        // A provider that cannot be reached is still just unreachable.
        assert_eq!(
            ex(P::CannotConnect, &table, &plain),
            VerifyError::CannotConnect
        );
    }
}
