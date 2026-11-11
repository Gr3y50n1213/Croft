// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Connection settings and credentials for one IMAP account.

use crate::types::BackendError;
use sha2::{Digest, Sha256};
use std::fmt;
use std::time::Duration;
use zeroize::Zeroizing;

/// How the connection to the server is secured.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Security {
    /// Implicit TLS from the first byte (the IMAPS port, normally 993).
    Tls,
    /// A plain connection upgraded with `STARTTLS` before anything secret is sent (normally 143).
    /// If the server does not agree to upgrade, the connection fails: it never falls back to
    /// plaintext.
    StartTls,
    /// No encryption at all. Accepted only when the host is `127.0.0.1`, `localhost` or `::1`
    /// (and the name resolves to a loopback address), so a test server on the same machine can be
    /// used without a certificate. [`ImapBackend::new`](super::ImapBackend::new) refuses it for
    /// any other host. Product code must never offer this to a person.
    PlaintextForTestsOnly,
}

/// SHA-256 of a server's leaf certificate (the DER bytes), used to trust one specific
/// self-signed certificate. The caller stores it after the person has compared it with what the
/// server shows.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct CertFingerprint([u8; 32]);

impl CertFingerprint {
    /// Fingerprint of a certificate given as DER bytes.
    pub fn of_der(der: &[u8]) -> Self {
        let digest = Sha256::digest(der);
        let mut out = [0u8; 32];
        out.copy_from_slice(digest.as_slice());
        Self(out)
    }

    /// Wraps 32 raw digest bytes.
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// Reads 64 hex digits, with or without `:` separators, in any case.
    pub fn from_hex(text: &str) -> Result<Self, BackendError> {
        let digits: Vec<u8> = text
            .bytes()
            .filter(|b| !matches!(b, b':' | b' ' | b'-'))
            .collect();
        let bad = || BackendError::Permanent("certificate fingerprint is not valid hex".into());
        if digits.len() != 64 {
            return Err(bad());
        }
        let mut out = [0u8; 32];
        for (i, [h, l]) in digits.as_chunks::<2>().0.iter().enumerate() {
            let hi = hex_val(*h).ok_or_else(bad)?;
            let lo = hex_val(*l).ok_or_else(bad)?;
            out[i] = (hi << 4) | lo;
        }
        Ok(Self(out))
    }

    /// The 32 digest bytes.
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

impl fmt::Display for CertFingerprint {
    /// Upper-case hex pairs separated by colons, the way browsers and `openssl` show it.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, b) in self.0.iter().enumerate() {
            if i > 0 {
                f.write_str(":")?;
            }
            write!(f, "{b:02X}")?;
        }
        Ok(())
    }
}

impl fmt::Debug for CertFingerprint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "CertFingerprint({self})")
    }
}

/// Where and how to connect.
#[derive(Clone, PartialEq, Eq)]
pub struct ImapConfig {
    /// Server host name or IP address. Also the name the certificate is checked against.
    pub host: String,
    /// TCP port (993 for [`Security::Tls`], 143 for [`Security::StartTls`]).
    pub port: u16,
    /// How the connection is secured.
    pub security: Security,
    /// Login name for [`Credential::Password`]. Often the whole email address.
    pub username: String,
    /// When set, the server is accepted only if its leaf certificate has exactly this SHA-256
    /// fingerprint. Chain, name and expiry checks are then replaced by the pin (the handshake
    /// signature is still verified). When `None`, the certificate must chain to the webpki or
    /// platform roots and match `host`.
    pub pinned_cert: Option<CertFingerprint>,
    /// Longest wait for connecting, for one command's next reply, or for the TLS handshake. A
    /// timeout is a `Transient` error and drops the connection.
    pub timeout: Duration,
    /// Longest a single command may take from sending it to its final reply, however steadily the
    /// server keeps sending. [`timeout`](Self::timeout) only limits the silence between two
    /// replies, so a server that never stops talking would otherwise hold a command open forever.
    /// Running past it is a `Transient` error and drops the connection. Default 120 seconds.
    pub command_deadline: Duration,
    /// Most untagged replies accepted for one command. Past it the command fails as `Transient`
    /// and the connection is dropped. The default (2 million) is far above what a mailbox of
    /// realistic size produces for one `UID FETCH 1:* (FLAGS)`.
    pub max_untagged_replies: usize,
}

/// Default for [`ImapConfig::command_deadline`].
pub const DEFAULT_COMMAND_DEADLINE: Duration = Duration::from_secs(120);
/// Default for [`ImapConfig::max_untagged_replies`].
pub const DEFAULT_MAX_UNTAGGED_REPLIES: usize = 2_000_000;

impl ImapConfig {
    /// Settings with the default 30 second timeout, the default command limits and no pinned
    /// certificate.
    pub fn new(
        host: impl Into<String>,
        port: u16,
        security: Security,
        username: impl Into<String>,
    ) -> Self {
        Self {
            host: host.into(),
            port,
            security,
            username: username.into(),
            pinned_cert: None,
            timeout: Duration::from_secs(30),
            command_deadline: DEFAULT_COMMAND_DEADLINE,
            max_untagged_replies: DEFAULT_MAX_UNTAGGED_REPLIES,
        }
    }

    /// Implicit TLS on port 993.
    pub fn tls(host: impl Into<String>, username: impl Into<String>) -> Self {
        Self::new(host, 993, Security::Tls, username)
    }

    /// STARTTLS on port 143.
    pub fn starttls(host: impl Into<String>, username: impl Into<String>) -> Self {
        Self::new(host, 143, Security::StartTls, username)
    }

    /// Trusts exactly this certificate (see [`ImapConfig::pinned_cert`]).
    pub fn with_pinned_cert(mut self, fingerprint: CertFingerprint) -> Self {
        self.pinned_cert = Some(fingerprint);
        self
    }

    /// Changes the timeout.
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Changes the overall deadline for one command (see [`ImapConfig::command_deadline`]).
    pub fn with_command_deadline(mut self, deadline: Duration) -> Self {
        self.command_deadline = deadline;
        self
    }

    /// Changes the cap on untagged replies per command (see [`ImapConfig::max_untagged_replies`]).
    pub fn with_max_untagged_replies(mut self, max: usize) -> Self {
        self.max_untagged_replies = max;
        self
    }

    /// Checks the settings can be used: plaintext only for loopback hosts, a non-empty host.
    pub fn validate(&self) -> Result<(), BackendError> {
        if self.host.trim().is_empty() {
            return Err(BackendError::Permanent("no server host set".into()));
        }
        if self.security == Security::PlaintextForTestsOnly && !is_loopback_name(&self.host) {
            return Err(BackendError::Permanent(
                "unencrypted IMAP is only allowed for localhost".into(),
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

impl fmt::Debug for ImapConfig {
    /// The username is often an email address, so it is not printed.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ImapConfig")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("security", &self.security)
            .field("username", &"***")
            .field("pinned_cert", &self.pinned_cert)
            .field("timeout", &self.timeout)
            .field("command_deadline", &self.command_deadline)
            .field("max_untagged_replies", &self.max_untagged_replies)
            .finish()
    }
}

/// What proves who the person is. It comes from the platform keystore and is handed in by the
/// caller; Croft never stores it. The secret strings are wiped from memory when the value is
/// dropped ([`Zeroizing`]); build one with [`Credential::password`] or [`Credential::oauth2`].
pub enum Credential {
    /// A password, sent with `LOGIN` (only ever over TLS).
    Password(Zeroizing<String>),
    /// An OAuth 2 access token, sent with `AUTHENTICATE XOAUTH2`.
    OAuth2 {
        /// The account name the token belongs to.
        user: String,
        /// The current access token.
        access_token: Zeroizing<String>,
    },
}

impl Credential {
    /// A password credential.
    pub fn password(password: impl Into<String>) -> Self {
        Self::Password(Zeroizing::new(password.into()))
    }

    /// An OAuth 2 credential: the account name and the current access token.
    pub fn oauth2(user: impl Into<String>, access_token: impl Into<String>) -> Self {
        Self::OAuth2 {
            user: user.into(),
            access_token: Zeroizing::new(access_token.into()),
        }
    }
}

impl fmt::Debug for Credential {
    /// Prints `***` for everything secret, including the account name.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Password(_) => f.write_str("Credential::Password(***)"),
            Self::OAuth2 { .. } => f.write_str("Credential::OAuth2(***)"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_output_hides_secrets() {
        let cred = Credential::password("hunter2");
        assert!(!format!("{cred:?}").contains("hunter2"));
        let oauth = Credential::oauth2("alice@example.org", "ya29.secret");
        let shown = format!("{oauth:?}");
        assert!(!shown.contains("ya29") && !shown.contains("alice"));
        let cfg = ImapConfig::tls("imap.example.org", "alice@example.org");
        assert!(!format!("{cfg:?}").contains("alice"));
    }

    #[test]
    fn plaintext_is_refused_for_real_hosts() {
        for host in [
            "imap.example.org",
            "192.168.1.5",
            "localhost.example.org",
            "",
        ] {
            let cfg = ImapConfig::new(host, 143, Security::PlaintextForTestsOnly, "u");
            assert!(
                matches!(cfg.validate(), Err(BackendError::Permanent(_))),
                "{host}"
            );
        }
        for host in ["127.0.0.1", "localhost", "LOCALHOST", "::1", "[::1]"] {
            let cfg = ImapConfig::new(host, 143, Security::PlaintextForTestsOnly, "u");
            assert!(cfg.validate().is_ok(), "{host}");
        }
        assert!(ImapConfig::tls("imap.example.org", "u").validate().is_ok());
    }

    #[test]
    fn fingerprint_round_trips_through_hex() {
        let fp = CertFingerprint::of_der(b"certificate bytes");
        let shown = fp.to_string();
        assert_eq!(shown.len(), 32 * 3 - 1);
        assert_eq!(CertFingerprint::from_hex(&shown).ok(), Some(fp));
        assert_eq!(
            CertFingerprint::from_hex(&shown.to_lowercase()).ok(),
            Some(fp)
        );
        assert!(CertFingerprint::from_hex("abcd").is_err());
        assert!(CertFingerprint::from_hex(&"zz".repeat(32)).is_err());
    }

    #[test]
    fn sha256_of_known_input() {
        // SHA-256("abc") from FIPS 180-2.
        let fp = CertFingerprint::of_der(b"abc");
        assert_eq!(
            fp.to_string().replace(':', "").to_lowercase(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
}
