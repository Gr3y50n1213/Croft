// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Turns everything that can go wrong into [`BackendError`].
//!
//! The split matters: the queue retries `Transient`, fails `Permanent`, and pauses the account on
//! `Auth`. Messages here are fixed strings chosen by this module. Text sent by the server, the
//! user name, the password and message data are never copied into an error, because errors reach
//! logs, the UI and crash reports.

use crate::types::BackendError;
use async_imap::error::Error as ImapError;
use tokio_rustls::rustls;

/// A failure inside one operation, with what the connection layer should do about it.
#[derive(Debug)]
pub(crate) struct OpError {
    /// What the caller sees.
    pub error: BackendError,
    /// The connection can no longer be trusted (I/O error, timeout, protocol confusion): drop it.
    pub broken: bool,
    /// The connection died while idle in the pool rather than from something this operation
    /// did, so reconnecting once and repeating the operation is reasonable.
    pub reconnect_and_retry: bool,
}

impl OpError {
    /// A refusal that leaves the connection usable.
    pub fn permanent(msg: impl Into<String>) -> Self {
        Self {
            error: BackendError::Permanent(msg.into()),
            broken: false,
            reconnect_and_retry: false,
        }
    }

    /// A temporary failure that leaves the connection usable.
    pub fn transient(msg: impl Into<String>) -> Self {
        Self {
            error: BackendError::Transient(msg.into()),
            broken: false,
            reconnect_and_retry: false,
        }
    }

    /// The connection is gone.
    pub fn dropped(msg: impl Into<String>) -> Self {
        Self {
            error: BackendError::Transient(msg.into()),
            broken: true,
            reconnect_and_retry: true,
        }
    }

    /// A reply took longer than the timeout.
    pub fn timed_out(what: &str) -> Self {
        Self {
            error: BackendError::Transient(format!("timed out {what}")),
            broken: true,
            reconnect_and_retry: false,
        }
    }

    /// Sign-in failed.
    pub fn auth(msg: impl Into<String>) -> Self {
        Self {
            error: BackendError::Auth(msg.into()),
            broken: true,
            reconnect_and_retry: false,
        }
    }
}

impl From<BackendError> for OpError {
    fn from(error: BackendError) -> Self {
        let broken = !matches!(error, BackendError::Permanent(_));
        Self {
            error,
            broken,
            reconnect_and_retry: false,
        }
    }
}

/// Classifies a lower-level I/O error. TLS failures arrive as `io::Error` wrapping a rustls
/// error: certificate problems and incompatible servers will not fix themselves, so they are
/// `Permanent`; everything else (reset, EOF, refused) is a network problem and `Transient`.
pub(crate) fn from_io(e: &std::io::Error, doing: &str) -> OpError {
    if let Some(tls) = e
        .get_ref()
        .and_then(|inner| inner.downcast_ref::<rustls::Error>())
    {
        return from_tls(tls);
    }
    OpError::dropped(format!("network error while {doing}"))
}

/// Classifies a TLS error without echoing any certificate content.
pub(crate) fn from_tls(e: &rustls::Error) -> OpError {
    use rustls::{CertificateError as C, Error as E};
    let msg = match e {
        E::InvalidCertificate(C::Other(_)) => {
            "the server's certificate does not match the trusted fingerprint"
        }
        E::InvalidCertificate(C::Expired | C::ExpiredContext { .. }) => {
            "the server's certificate has expired"
        }
        E::InvalidCertificate(C::NotValidYet | C::NotValidYetContext { .. }) => {
            "the server's certificate is not valid yet (check the device clock)"
        }
        E::InvalidCertificate(C::NotValidForName | C::NotValidForNameContext { .. }) => {
            "the server's certificate is for a different host name"
        }
        E::InvalidCertificate(C::UnknownIssuer) => {
            "the server's certificate is not signed by a trusted authority"
        }
        E::InvalidCertificate(_) => "the server's certificate was not accepted",
        E::PeerIncompatible(_) => "the server does not offer TLS 1.2 or newer",
        E::AlertReceived(_) => "the server refused the secure connection",
        _ => "the secure connection could not be set up",
    };
    OpError {
        error: BackendError::Permanent(msg.into()),
        broken: true,
        reconnect_and_retry: false,
    }
}

/// Maps a library error during sign-in. Every refusal is `Auth` (wrong password, expired token,
/// mechanism not offered) except a server that says it is temporarily unavailable.
pub(crate) fn from_auth(e: &ImapError) -> OpError {
    match e {
        ImapError::No(info) | ImapError::Bad(info) => {
            if looks_temporary(info) {
                OpError {
                    error: BackendError::Transient(
                        "the server could not check the sign-in right now".into(),
                    ),
                    broken: true,
                    reconnect_and_retry: false,
                }
            } else {
                OpError::auth("the server rejected the sign-in")
            }
        }
        ImapError::Validate(_) => OpError::auth("the sign-in details contain invalid characters"),
        ImapError::Io(io) => from_io(io, "signing in"),
        ImapError::ConnectionLost => OpError::dropped("connection lost while signing in"),
        _ => OpError::dropped("unexpected server reply while signing in"),
    }
}

/// Maps a library error from an ordinary command.
pub(crate) fn from_command(e: &ImapError, doing: &str) -> OpError {
    match e {
        ImapError::Io(io) => from_io(io, doing),
        ImapError::ConnectionLost => OpError::dropped(format!("connection lost while {doing}")),
        ImapError::No(info) => refusal(info, doing),
        ImapError::Bad(_) => {
            OpError::permanent(format!("the server did not accept the request to {doing}"))
        }
        ImapError::Validate(_) => {
            OpError::permanent("a folder name contains characters IMAP cannot carry")
        }
        ImapError::Parse(_) => OpError {
            error: BackendError::Transient(format!("unexpected server reply while {doing}")),
            broken: true,
            reconnect_and_retry: false,
        },
        _ => OpError::permanent(format!("the server could not {doing}")),
    }
}

/// A tagged `NO`: the server refused this request. `detail` is the server's response code and
/// text; it is only inspected here, never copied into the message.
pub(crate) fn refusal(detail: &str, doing: &str) -> OpError {
    let up = detail.to_ascii_uppercase();
    if looks_temporary(detail) {
        return OpError::transient(format!("the server is busy and could not {doing}"));
    }
    let reason = if up.contains("OVERQUOTA") {
        "the mailbox is over its quota"
    } else if up.contains("NONEXISTENT") || up.contains("TRYCREATE") {
        "the folder does not exist"
    } else if up.contains("READ-ONLY") || up.contains("READONLY") {
        "the folder is read-only"
    } else if up.contains("NOPERM") || up.contains("AUTHORIZATIONFAILED") {
        "no permission"
    } else if up.contains("ALREADYEXISTS") {
        "that folder already exists"
    } else {
        "the server refused"
    };
    OpError::permanent(format!("{reason} while trying to {doing}"))
}

/// Response codes that mean "try again later" (RFC 5530 UNAVAILABLE, INUSE, LIMIT, SERVERBUG).
fn looks_temporary(detail: &str) -> bool {
    let up = detail.to_ascii_uppercase();
    up.contains("UNAVAILABLE") || up.contains("INUSE") || up.contains("SERVERBUG")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refusals_are_permanent_and_leak_nothing() {
        let e = refusal("Mailbox doesn't exist: secret-folder [NONEXISTENT]", "move");
        match e.error {
            BackendError::Permanent(m) => {
                assert!(m.contains("does not exist"));
                assert!(!m.contains("secret"));
            }
            other => panic!("{other:?}"),
        }
        assert!(!e.broken);
    }

    #[test]
    fn unavailable_is_transient() {
        let e = refusal("[UNAVAILABLE] try later", "move");
        assert!(matches!(e.error, BackendError::Transient(_)));
    }

    #[test]
    fn auth_failures_are_auth_not_transient() {
        let e = from_auth(&ImapError::No(
            "[AUTHENTICATIONFAILED] bad password hunter2".into(),
        ));
        match e.error {
            BackendError::Auth(m) => assert!(!m.contains("hunter2")),
            other => panic!("{other:?}"),
        }
        let busy = from_auth(&ImapError::No("[UNAVAILABLE] Temporary failure".into()));
        assert!(matches!(busy.error, BackendError::Transient(_)));
        let lost = from_auth(&ImapError::ConnectionLost);
        assert!(matches!(lost.error, BackendError::Transient(_)));
    }

    #[test]
    fn io_errors_are_transient() {
        let io = std::io::Error::new(std::io::ErrorKind::ConnectionReset, "reset by peer");
        let e = from_io(&io, "listing folders");
        assert!(matches!(e.error, BackendError::Transient(_)));
        assert!(e.broken && e.reconnect_and_retry);
    }

    #[test]
    fn certificate_problems_are_permanent() {
        let e = from_tls(&rustls::Error::InvalidCertificate(
            rustls::CertificateError::UnknownIssuer,
        ));
        assert!(matches!(e.error, BackendError::Permanent(_)));
    }
}
