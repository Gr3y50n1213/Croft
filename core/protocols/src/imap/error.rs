// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Turns everything that can go wrong into [`BackendError`].
//!
//! The split matters: the queue retries `Transient`, fails `Permanent`, and pauses the account on
//! `Auth`. Messages here are fixed strings chosen by this module. Text sent by the server, the
//! user name, the password and message data are never copied into an error, because errors reach
//! logs, the UI and crash reports.
//!
//! Three rules here exist because a wrong guess costs the person something:
//!
//! - **A refusal of the password is never retried.** Retrying a wrong password locks the account
//!   at many providers. So only a bracketed response code (`[UNAVAILABLE]`, `[INUSE]`,
//!   `[SERVERBUG]`, RFC 5530) at the start of the server's answer counts as "temporary", never
//!   free text, and never when an authentication-failure code is present. A `BYE` or a dropped
//!   connection while waiting for the answer to `LOGIN` or `AUTHENTICATE` is [`Auth`](BackendError::Auth),
//!   because servers hang up on clients that fail too often.
//! - **A certificate that cannot be verified is [`Auth`](BackendError::Auth), not `Permanent`.**
//!   A hotel or captive-portal network that intercepts TLS makes the certificate fail until the
//!   person has signed in to the portal; `Permanent` would make the queue fail every pending
//!   operation for good. `Auth` pauses the account and keeps the operations, and the screen shows
//!   the message. There is no extra error variant for this: the person has to look at it either
//!   way (a wrong pin, a renewed certificate, a hostile network).

use super::config::CertFingerprint;
use super::tls;
use crate::types::BackendError;
use async_imap::error::Error as ImapError;
use async_imap::imap_proto::ResponseCode;
use tokio_rustls::rustls;

/// Why a server's certificate was refused, found by [`check_certificate`](super::check_certificate)
/// in a handshake that sends no credential.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CertificateProblem {
    /// The certificate does not chain to a trusted root: self-signed, or signed by a private
    /// authority. The fingerprint is the certificate's SHA-256, for the person to compare.
    UnknownIssuer {
        /// SHA-256 of the certificate the server presented.
        fingerprint: CertFingerprint,
    },
    /// The certificate has expired.
    Expired,
    /// The certificate is for a different host name.
    WrongName,
    /// A pinned fingerprint was set and the server presented a different certificate.
    PinMismatch {
        /// SHA-256 of the certificate the server presented now.
        fingerprint: CertFingerprint,
    },
    /// Any other certificate failure (not yet valid, bad signature, unsupported key, ...).
    Other,
    /// The server could not be reached, or did not answer in time. Nothing is known about its
    /// certificate.
    CannotConnect,
    /// The server does not offer TLS 1.2 or newer (or STARTTLS), or refused the handshake for a
    /// reason that is not about its certificate.
    NoSecureConnection,
}

impl CertificateProblem {
    /// Classifies a refused handshake. `fingerprint` is what the verifier saw, if it got that far.
    pub(crate) fn from_tls(e: &rustls::Error, fingerprint: Option<CertFingerprint>) -> Self {
        use rustls::{CertificateError as C, Error as E};
        match (e, fingerprint) {
            (e, Some(fingerprint)) if tls::is_pin_mismatch(e) => Self::PinMismatch { fingerprint },
            // "Does not chain to a trusted root". Besides `UnknownIssuer`, a self-signed certificate
            // whose name equals the name of some installed root fails when the verifier tries
            // that root's key on it (`BadSignature`, or an algorithm the root's key cannot
            // check). Such a certificate is just as unknown, so the person may still be asked.
            (
                E::InvalidCertificate(
                    C::UnknownIssuer
                    | C::BadSignature
                    | C::UnsupportedSignatureAlgorithmContext { .. }
                    | C::UnsupportedSignatureAlgorithmForPublicKeyContext { .. },
                ),
                Some(fingerprint),
            ) => Self::UnknownIssuer { fingerprint },
            (E::InvalidCertificate(C::Expired | C::ExpiredContext { .. }), _) => Self::Expired,
            (E::InvalidCertificate(C::NotValidForName | C::NotValidForNameContext { .. }), _) => {
                Self::WrongName
            }
            _ => Self::Other,
        }
    }
}

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

    /// A command ran past its overall deadline, however steadily the server kept talking.
    pub fn too_slow(doing: &str) -> Self {
        Self {
            error: BackendError::Transient(format!(
                "the server took too long to finish while trying to {doing}"
            )),
            broken: true,
            reconnect_and_retry: false,
        }
    }

    /// The server sent more replies to one command than any real server would.
    pub fn flooded(doing: &str) -> Self {
        Self {
            error: BackendError::Transient(format!(
                "the server sent far more data than expected while trying to {doing}"
            )),
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
///
/// Certificate and pin failures are `Auth` (see the module notes): the connection is not made, the
/// operations are kept, and the person is told the certificate could not be verified. Servers that
/// cannot speak TLS 1.2 or refuse the handshake are `Permanent`, since waiting does not change
/// that.
pub(crate) fn from_tls(e: &rustls::Error) -> OpError {
    use rustls::{CertificateError as C, Error as E};
    let cert = |why: &str| OpError {
        error: BackendError::Auth(format!(
            "the server's certificate could not be verified: {why}"
        )),
        broken: true,
        reconnect_and_retry: false,
    };
    let other = |msg: &str| OpError {
        error: BackendError::Permanent(msg.into()),
        broken: true,
        reconnect_and_retry: false,
    };
    match e {
        E::InvalidCertificate(C::Other(_)) => cert("it does not match the trusted fingerprint"),
        E::InvalidCertificate(C::Expired | C::ExpiredContext { .. }) => cert("it has expired"),
        E::InvalidCertificate(C::NotValidYet | C::NotValidYetContext { .. }) => {
            cert("it is not valid yet (check the device clock)")
        }
        E::InvalidCertificate(C::NotValidForName | C::NotValidForNameContext { .. }) => {
            cert("it is for a different host name")
        }
        E::InvalidCertificate(C::UnknownIssuer) => cert("it is not signed by a trusted authority"),
        E::InvalidCertificate(_) => cert("it was not accepted"),
        E::PeerIncompatible(_) => other("the server does not offer TLS 1.2 or newer"),
        E::AlertReceived(_) => other("the server refused the secure connection"),
        _ => other("the secure connection could not be set up"),
    }
}

/// The server hung up, or said `BYE`, while Croft waited for the answer to `LOGIN` or
/// `AUTHENTICATE`. The credentials had been sent, and servers that drop the connection after a
/// failed attempt do exactly this, so it is `Auth`: retrying would repeat a wrong password.
pub(crate) fn sign_in_hangup() -> OpError {
    OpError::auth(
        "the server ended the connection while signing in, which usually means the sign-in \
         details are wrong",
    )
}

/// An I/O error while waiting for the answer to the sign-in command. TLS errors keep their own
/// classes, a connection that was reset or closed is [`sign_in_hangup`], and anything else (a
/// timeout from the operating system, an unparsable reply) is a network problem.
pub(crate) fn from_io_during_sign_in(e: &std::io::Error) -> OpError {
    use std::io::ErrorKind as K;
    if e.get_ref()
        .is_some_and(|inner| inner.downcast_ref::<rustls::Error>().is_some())
    {
        return from_io(e, "signing in");
    }
    match e.kind() {
        K::ConnectionReset | K::ConnectionAborted | K::UnexpectedEof | K::BrokenPipe => {
            sign_in_hangup()
        }
        _ => from_io(e, "signing in"),
    }
}

/// The server's tagged `NO` or `BAD` to the sign-in command. `detail` is built by
/// [`detail_text`]; it is only inspected, never copied. Every refusal is `Auth` (wrong password,
/// expired token, mechanism not offered) except when the server leads with a bracketed
/// `[UNAVAILABLE]`, `[INUSE]` or `[SERVERBUG]` and gives no authentication-failure code.
pub(crate) fn sign_in_refusal(detail: &str) -> OpError {
    if is_temporary(detail) {
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

/// The name of a response code the parser understood, in the spelling RFC 3501 uses.
fn parsed_code_name(code: &ResponseCode<'_>) -> Option<&'static str> {
    match code {
        ResponseCode::Alert => Some("ALERT"),
        ResponseCode::ReadOnly => Some("READ-ONLY"),
        ResponseCode::ReadWrite => Some("READ-WRITE"),
        ResponseCode::TryCreate => Some("TRYCREATE"),
        _ => None,
    }
}

/// Puts a tagged reply's response code and text into the one string the classifiers read:
/// `[CODE] text`. `async-imap`'s parser knows only some codes; for the rest (`UNAVAILABLE`,
/// `AUTHENTICATIONFAILED`, `NONEXISTENT`, ...) the code is still at the start of `information`.
/// The string is inspected and dropped; it never reaches an error message.
pub(crate) fn detail_text(code: Option<&ResponseCode<'_>>, information: Option<&str>) -> String {
    let text = information.unwrap_or("");
    match code.and_then(parsed_code_name) {
        Some(name) => format!("[{name}] {text}"),
        None => text.to_string(),
    }
}

/// A tagged `NO`: the server refused this request. `detail` is the server's response code and
/// text (see [`detail_text`]); it is only inspected here, never copied into the message.
pub(crate) fn refusal(detail: &str, doing: &str) -> OpError {
    let up = detail.to_ascii_uppercase();
    if is_temporary(detail) {
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
    } else if up.contains("ALREADYEXISTS") || up.contains("ALREADY EXISTS") {
        "that folder already exists"
    } else {
        "the server refused"
    };
    OpError::permanent(format!("{reason} while trying to {doing}"))
}

/// Every `[WORD ...]` token in `detail`, upper-cased, first word only. The first one is the real
/// response code when the server follows RFC 3501 (`resp-text` starts with the code).
fn bracketed_codes(detail: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = detail;
    while let Some(open) = rest.find('[') {
        let after = &rest[open + 1..];
        let Some(close) = after.find(']') else {
            break;
        };
        let word = after[..close].split_whitespace().next().unwrap_or("");
        out.push(word.to_ascii_uppercase());
        rest = &after[close + 1..];
    }
    out
}

/// True only when the answer *starts with* a bracketed `[UNAVAILABLE]`, `[INUSE]` or
/// `[SERVERBUG]` (RFC 5530) and no `[AUTHENTICATIONFAILED]`, `[AUTHORIZATIONFAILED]` or
/// `[EXPIRED]` appears anywhere in it. Free text never counts: a server (or someone in the way)
/// can put the word "unavailable" into any sentence, including "Authentication failed: account
/// unavailable", and a wrong password must never be retried.
fn is_temporary(detail: &str) -> bool {
    if !detail.trim_start().starts_with('[') {
        return false;
    }
    let codes = bracketed_codes(detail);
    let auth_failure = codes.iter().any(|c| {
        matches!(
            c.as_str(),
            "AUTHENTICATIONFAILED" | "AUTHORIZATIONFAILED" | "EXPIRED"
        )
    });
    let first_is_temporary = codes
        .first()
        .is_some_and(|c| matches!(c.as_str(), "UNAVAILABLE" | "INUSE" | "SERVERBUG"));
    first_is_temporary && !auth_failure
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
        let e = sign_in_refusal("[AUTHENTICATIONFAILED] bad password hunter2");
        match e.error {
            BackendError::Auth(m) => assert!(!m.contains("hunter2")),
            other => panic!("{other:?}"),
        }
        let busy = sign_in_refusal("[UNAVAILABLE] Temporary failure");
        assert!(matches!(busy.error, BackendError::Transient(_)));
        // Finding 2: a connection lost after the credentials went out is a refusal, not a
        // network blip. (Before, this was `Transient`, and the queue kept retrying.)
        let lost = sign_in_hangup();
        assert!(matches!(lost.error, BackendError::Auth(_)));
    }

    #[test]
    fn only_a_leading_bracketed_code_makes_a_refusal_temporary() {
        for temporary in [
            "[UNAVAILABLE] Try later",
            "[unavailable] lower case",
            "  [INUSE] Mailbox busy",
            "[SERVERBUG] oops",
            "[UNAVAILABLE]",
        ] {
            assert!(is_temporary(temporary), "{temporary}");
            assert!(
                matches!(sign_in_refusal(temporary).error, BackendError::Transient(_)),
                "{temporary}"
            );
        }
        for not_temporary in [
            // The reviewer's case: an auth failure whose free text says "unavailable".
            "[AUTHENTICATIONFAILED] Authentication failed: account unavailable",
            // Free text, no code.
            "Authentication failed: service unavailable, try again",
            "Server bug: INUSE",
            // The word in the middle of a sentence is not the response code.
            "Login failed (see [UNAVAILABLE])",
            // A temporary code next to an authentication-failure code.
            "[UNAVAILABLE] [AUTHENTICATIONFAILED] nope",
            "[UNAVAILABLE] but also [authorizationfailed]",
            "[EXPIRED] [UNAVAILABLE]",
            "[UNAVAILABLEISH] close but no",
            "",
        ] {
            assert!(!is_temporary(not_temporary), "{not_temporary}");
            assert!(
                matches!(sign_in_refusal(not_temporary).error, BackendError::Auth(_)),
                "{not_temporary}"
            );
        }
        // The same rule for ordinary commands: free text is never "busy".
        let e = refusal("Quota check unavailable for this mailbox", "move");
        assert!(matches!(e.error, BackendError::Permanent(_)));
    }

    #[test]
    fn detail_text_puts_parsed_codes_in_front() {
        assert_eq!(
            detail_text(Some(&ResponseCode::TryCreate), Some("No such folder")),
            "[TRYCREATE] No such folder"
        );
        assert_eq!(
            detail_text(None, Some("[UNAVAILABLE] later")),
            "[UNAVAILABLE] later"
        );
        assert_eq!(detail_text(None, None), "");
    }

    #[test]
    fn a_hangup_while_signing_in_is_auth() {
        assert!(matches!(sign_in_hangup().error, BackendError::Auth(_)));
        for kind in [
            std::io::ErrorKind::ConnectionReset,
            std::io::ErrorKind::UnexpectedEof,
            std::io::ErrorKind::BrokenPipe,
        ] {
            let e = from_io_during_sign_in(&std::io::Error::new(kind, "gone"));
            assert!(matches!(e.error, BackendError::Auth(_)), "{kind:?}");
        }
        // Not a hangup: the operating system gave up waiting.
        let e = from_io_during_sign_in(&std::io::Error::new(std::io::ErrorKind::TimedOut, "slow"));
        assert!(matches!(e.error, BackendError::Transient(_)));
    }

    #[test]
    fn io_errors_are_transient() {
        let io = std::io::Error::new(std::io::ErrorKind::ConnectionReset, "reset by peer");
        let e = from_io(&io, "listing folders");
        assert!(matches!(e.error, BackendError::Transient(_)));
        assert!(e.broken && e.reconnect_and_retry);
    }

    #[test]
    fn certificate_problems_pause_the_account_instead_of_failing_operations() {
        use rustls::CertificateError as C;
        // Finding 5: a hotel or captive-portal network makes every certificate fail until the
        // person signs in to it. `Permanent` would fail every queued operation for good; `Auth`
        // pauses the account and keeps them.
        for problem in [
            C::UnknownIssuer,
            C::Expired,
            C::NotValidForName,
            C::NotValidYet,
            C::BadSignature,
        ] {
            let e = from_tls(&rustls::Error::InvalidCertificate(problem));
            match e.error {
                BackendError::Auth(m) => assert!(m.contains("could not be verified"), "{m}"),
                other => panic!("{other:?}"),
            }
            assert!(e.broken);
        }
        // A server that cannot do TLS 1.2 is a different matter: waiting will not help.
        let e = from_tls(&rustls::Error::PeerIncompatible(
            rustls::PeerIncompatible::Tls12NotOffered,
        ));
        assert!(matches!(e.error, BackendError::Permanent(_)));
    }

    #[test]
    fn certificate_problems_are_typed_without_reading_text() {
        use rustls::CertificateError as C;
        let fp = CertFingerprint::from_bytes([9; 32]);
        let typed =
            |c: C| CertificateProblem::from_tls(&rustls::Error::InvalidCertificate(c), Some(fp));
        assert_eq!(
            typed(C::UnknownIssuer),
            CertificateProblem::UnknownIssuer { fingerprint: fp }
        );
        // A self-signed certificate that shares its name with an installed root fails against
        // that root's key: still "does not chain to a trusted root".
        assert_eq!(
            typed(C::BadSignature),
            CertificateProblem::UnknownIssuer { fingerprint: fp }
        );
        assert_eq!(
            typed(C::UnsupportedSignatureAlgorithmForPublicKeyContext {
                signature_algorithm_id: vec![1],
                public_key_algorithm_id: vec![2],
            }),
            CertificateProblem::UnknownIssuer { fingerprint: fp }
        );
        assert_eq!(typed(C::Expired), CertificateProblem::Expired);
        assert_eq!(typed(C::NotValidForName), CertificateProblem::WrongName);
        assert_eq!(typed(C::NotValidYet), CertificateProblem::Other);
        assert_eq!(typed(C::Revoked), CertificateProblem::Other);
        // Without a fingerprint there is nothing to show, so nothing to trust.
        assert_eq!(
            CertificateProblem::from_tls(
                &rustls::Error::InvalidCertificate(C::UnknownIssuer),
                None
            ),
            CertificateProblem::Other
        );
    }
}
