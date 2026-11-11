// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Connection settings for one JMAP account.

use super::url::Url;
use crate::imap::CertFingerprint;
use crate::types::BackendError;
use std::fmt;
use std::time::Duration;

/// Default for [`JmapConfig::deadline`].
pub const DEFAULT_DEADLINE: Duration = Duration::from_secs(120);
/// Default for [`JmapConfig::max_response_bytes`]: 32 MiB.
pub const DEFAULT_MAX_RESPONSE_BYTES: usize = 32 * 1024 * 1024;

/// Where and how to connect to a JMAP server.
#[derive(Clone, PartialEq, Eq)]
pub struct JmapConfig {
    /// The JMAP session resource, or `https://<domain>/.well-known/jmap` (which redirects to it),
    /// exactly as discovery found it. It must be `https`. Plain `http` is accepted only for
    /// `127.0.0.1`, `localhost` and `::1`, so a test server on the same machine can be used.
    ///
    /// This origin (scheme, host and port) is the only place the credential is ever sent: the
    /// `apiUrl` and `uploadUrl` in the session answer are used for their path only, and a
    /// redirect to another origin is refused. Show the host to the person at setup (it comes from
    /// the person's domain, but discovery does not check it against that domain).
    pub session_url: String,
    /// Login name for a password credential. Often the whole email address. Must not contain a
    /// colon (HTTP Basic cannot carry one).
    pub username: String,
    /// When set, the server is accepted only if its leaf certificate has exactly this SHA-256
    /// fingerprint (the handshake signature is still verified). When `None`, the certificate must
    /// chain to the webpki or platform roots and match the host name.
    pub pinned_cert: Option<CertFingerprint>,
    /// Longest wait for connecting, for the TLS handshake, or for the next bytes of an answer. A
    /// timeout is a `Transient` error.
    pub timeout: Duration,
    /// Longest one HTTP request may take from start to the end of the answer, however steadily
    /// the server keeps sending (an upload gets a little extra per megabyte). Default 120 s.
    pub deadline: Duration,
    /// Largest answer accepted for one request, in bytes. A bigger one is a `Permanent` error.
    /// Default 32 MiB.
    pub max_response_bytes: usize,
}

impl JmapConfig {
    /// Settings with the default 30 second timeout and limits and no pinned certificate.
    pub fn new(session_url: impl Into<String>, username: impl Into<String>) -> Self {
        Self {
            session_url: session_url.into(),
            username: username.into(),
            pinned_cert: None,
            timeout: Duration::from_secs(30),
            deadline: DEFAULT_DEADLINE,
            max_response_bytes: DEFAULT_MAX_RESPONSE_BYTES,
        }
    }

    /// Trusts exactly this certificate (see [`JmapConfig::pinned_cert`]).
    pub fn with_pinned_cert(mut self, fingerprint: CertFingerprint) -> Self {
        self.pinned_cert = Some(fingerprint);
        self
    }

    /// Changes the timeout.
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Changes the overall deadline for one request.
    pub fn with_deadline(mut self, deadline: Duration) -> Self {
        self.deadline = deadline;
        self
    }

    /// Changes the cap on the size of an answer.
    pub fn with_max_response_bytes(mut self, max: usize) -> Self {
        self.max_response_bytes = max;
        self
    }

    /// Checks the settings can be used and returns the parsed session URL: an `https` URL (or
    /// `http` for a loopback host), and a user name that HTTP Basic can carry.
    pub(crate) fn validate(&self) -> Result<Url, BackendError> {
        let url = Url::parse(&self.session_url)
            .ok_or_else(|| BackendError::Permanent("the JMAP address is not valid".into()))?;
        if !url.is_allowed() {
            return Err(BackendError::Permanent(
                "JMAP needs an https address (plain http is only allowed for localhost)".into(),
            ));
        }
        if self.username.contains(':') || self.username.contains(['\r', '\n']) {
            return Err(BackendError::Permanent(
                "the user name cannot be used with JMAP".into(),
            ));
        }
        Ok(url)
    }
}

impl fmt::Debug for JmapConfig {
    /// The user name is often an email address, so it is not printed. The address is printed
    /// without its path and query, which can carry tokens on some providers.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let origin = Url::parse(&self.session_url)
            .map(|u| {
                format!(
                    "{}://{}",
                    if u.secure { "https" } else { "http" },
                    u.host_header()
                )
            })
            .unwrap_or_else(|| "(invalid)".into());
        f.debug_struct("JmapConfig")
            .field("origin", &origin)
            .field("username", &"***")
            .field("pinned_cert", &self.pinned_cert)
            .field("timeout", &self.timeout)
            .field("deadline", &self.deadline)
            .field("max_response_bytes", &self.max_response_bytes)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plaintext_is_refused_for_real_hosts() {
        let ok = |u: &str| JmapConfig::new(u, "u").validate().is_ok();
        assert!(ok("https://mail.example.org/.well-known/jmap"));
        assert!(ok("http://127.0.0.1:8080/jmap/session"));
        assert!(ok("http://localhost/jmap/session"));
        assert!(!ok("http://mail.example.org/jmap/session"));
        assert!(!ok("http://192.168.1.5/jmap/session"));
        assert!(!ok("mail.example.org"));
        assert!(!ok(""));
    }

    #[test]
    fn user_names_basic_auth_cannot_carry_are_refused() {
        let bad = |u: &str| JmapConfig::new("https://x.org/", u).validate().is_err();
        assert!(bad("a:b"));
        assert!(bad("a\r\nb"));
        assert!(!bad("alice@example.org"));
    }

    #[test]
    fn debug_hides_the_user_and_the_path() {
        let cfg = JmapConfig::new(
            "https://mail.example.org/jmap/session?token=abc",
            "alice@x.org",
        );
        let shown = format!("{cfg:?}");
        assert!(!shown.contains("alice") && !shown.contains("token"));
        assert!(shown.contains("mail.example.org"));
    }
}
