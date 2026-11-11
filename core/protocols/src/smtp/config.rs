// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Connection settings for one SMTP submission server.

use crate::imap::CertFingerprint;
use crate::types::BackendError;
use std::fmt;
use std::time::Duration;

/// How the connection to the submission server is secured.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SmtpSecurity {
    /// Implicit TLS from the first byte (SMTPS, normally port 465).
    Tls,
    /// A plain connection upgraded with `STARTTLS` before anything secret is sent (submission,
    /// normally port 587). If the server does not offer `STARTTLS`, nothing is sent: there is no
    /// fallback to plaintext.
    StartTls,
    /// No encryption at all. Accepted only when the host is `127.0.0.1`, `localhost` or `::1`
    /// (and the name resolves to a loopback address), so a test server on the same machine can be
    /// used without a certificate. [`SmtpSender::new`](super::SmtpSender::new) refuses it for any
    /// other host. Product code must never offer this to a person.
    PlaintextForTestsOnly,
}

/// Where and how to connect for sending.
#[derive(Clone, PartialEq, Eq)]
pub struct SmtpConfig {
    /// Server host name or IP address. Also the name the certificate is checked against.
    pub host: String,
    /// TCP port (465 for [`SmtpSecurity::Tls`], 587 for [`SmtpSecurity::StartTls`]).
    pub port: u16,
    /// How the connection is secured.
    pub security: SmtpSecurity,
    /// Login name for a password [`Credential`](crate::Credential). Often the whole email
    /// address. (An OAuth credential carries its own account name.)
    pub username: String,
    /// When set, the server is accepted only if its leaf certificate has exactly this SHA-256
    /// fingerprint (the handshake signature is still verified). When `None`, the certificate must
    /// chain to the bundled or platform roots and match `host`. Same rule as for IMAP.
    pub pinned_cert: Option<CertFingerprint>,
    /// Longest wait for connecting, for the TLS handshake, or for the server's next reply to a
    /// command. A timeout before the message body has been sent is a `Transient` error.
    pub timeout: Duration,
    /// Longest the whole send may take, however steadily the server keeps talking. One second per
    /// 16 KiB of message is added, so a large attachment on a slow link is not cut off. Also bounds
    /// the wait for the final reply after the message body, which servers may take a while over
    /// (spam and virus scans). Default 120 seconds.
    pub deadline: Duration,
    /// The name given in `EHLO`. Default `localhost`: Croft does not tell the server the device's
    /// host name.
    pub ehlo_name: String,
}

/// Default for [`SmtpConfig::deadline`].
pub const DEFAULT_DEADLINE: Duration = Duration::from_secs(120);

impl SmtpConfig {
    /// Settings with a 30 second timeout, the default deadline and no pinned certificate.
    pub fn new(
        host: impl Into<String>,
        port: u16,
        security: SmtpSecurity,
        username: impl Into<String>,
    ) -> Self {
        Self {
            host: host.into(),
            port,
            security,
            username: username.into(),
            pinned_cert: None,
            timeout: Duration::from_secs(30),
            deadline: DEFAULT_DEADLINE,
            ehlo_name: "localhost".into(),
        }
    }

    /// Implicit TLS on port 465.
    pub fn tls(host: impl Into<String>, username: impl Into<String>) -> Self {
        Self::new(host, 465, SmtpSecurity::Tls, username)
    }

    /// STARTTLS on port 587.
    pub fn starttls(host: impl Into<String>, username: impl Into<String>) -> Self {
        Self::new(host, 587, SmtpSecurity::StartTls, username)
    }

    /// Trusts exactly this certificate (see [`SmtpConfig::pinned_cert`]).
    pub fn with_pinned_cert(mut self, fingerprint: CertFingerprint) -> Self {
        self.pinned_cert = Some(fingerprint);
        self
    }

    /// Changes the per-step timeout.
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Changes the overall deadline (see [`SmtpConfig::deadline`]).
    pub fn with_deadline(mut self, deadline: Duration) -> Self {
        self.deadline = deadline;
        self
    }

    /// Checks the settings can be used: a non-empty host, plaintext only for loopback hosts, and
    /// an `EHLO` name that cannot break the command line.
    pub fn validate(&self) -> Result<(), BackendError> {
        if self.host.trim().is_empty() {
            return Err(BackendError::Permanent("no server host set".into()));
        }
        if self.security == SmtpSecurity::PlaintextForTestsOnly && !is_loopback_name(&self.host) {
            return Err(BackendError::Permanent(
                "unencrypted SMTP is only allowed for localhost".into(),
            ));
        }
        if self.ehlo_name.is_empty()
            || !self
                .ehlo_name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'[' | b']' | b':'))
        {
            return Err(BackendError::Permanent(
                "the EHLO name is not a valid host name".into(),
            ));
        }
        Ok(())
    }
}

/// True for the three names plaintext is allowed for. (Resolution is checked again at connect.)
pub(crate) fn is_loopback_name(host: &str) -> bool {
    let h = host.trim().trim_start_matches('[').trim_end_matches(']');
    h == "127.0.0.1" || h == "::1" || h.eq_ignore_ascii_case("localhost")
}

impl fmt::Debug for SmtpConfig {
    /// The username is often an email address, so it is not printed.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SmtpConfig")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("security", &self.security)
            .field("username", &"***")
            .field("pinned_cert", &self.pinned_cert)
            .field("timeout", &self.timeout)
            .field("deadline", &self.deadline)
            .field("ehlo_name", &self.ehlo_name)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_hides_the_username() {
        let cfg = SmtpConfig::tls("smtp.example.org", "alice@example.org");
        assert!(!format!("{cfg:?}").contains("alice"));
    }

    #[test]
    fn plaintext_is_refused_for_real_hosts() {
        for host in [
            "smtp.example.org",
            "192.168.1.5",
            "localhost.example.org",
            "",
        ] {
            let cfg = SmtpConfig::new(host, 25, SmtpSecurity::PlaintextForTestsOnly, "u");
            assert!(
                matches!(cfg.validate(), Err(BackendError::Permanent(_))),
                "{host}"
            );
        }
        for host in ["127.0.0.1", "localhost", "LOCALHOST", "::1", "[::1]"] {
            let cfg = SmtpConfig::new(host, 25, SmtpSecurity::PlaintextForTestsOnly, "u");
            assert!(cfg.validate().is_ok(), "{host}");
        }
        assert!(SmtpConfig::tls("smtp.example.org", "u").validate().is_ok());
    }

    #[test]
    fn ehlo_name_cannot_carry_a_command() {
        let mut cfg = SmtpConfig::tls("smtp.example.org", "u");
        cfg.ehlo_name = "x\r\nMAIL FROM:<a@b>".into();
        assert!(cfg.validate().is_err());
        cfg.ehlo_name = "[127.0.0.1]".into();
        assert!(cfg.validate().is_ok());
    }

    #[test]
    fn default_ports() {
        assert_eq!(SmtpConfig::tls("h", "u").port, 465);
        assert_eq!(SmtpConfig::starttls("h", "u").port, 587);
    }
}
