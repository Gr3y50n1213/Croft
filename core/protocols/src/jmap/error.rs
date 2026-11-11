// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Turns what a JMAP server says into [`BackendError`].
//!
//! The queue retries `Transient`, fails `Permanent`, and pauses the account on `Auth`. Every
//! message here is a fixed string chosen by this module: server text (the `description` of an
//! error, a problem-details body), the user name, tokens, addresses and message content are never
//! copied into an error, because errors reach logs, the UI and crash reports.
//!
//! | What happened | Class |
//! | --- | --- |
//! | HTTP 401 or 403 | `Auth` |
//! | HTTP 408, 429, 5xx | `Transient` |
//! | other HTTP 4xx | `Permanent` |
//! | no connection, timeout, a connection that drops | `Transient` |
//! | certificate or pin problem | `Auth` (same as IMAP: a captive portal must not fail the queue) |
//! | method errors `serverFail`, `serverUnavailable`, `serverPartialFail`, `rateLimit` | `Transient` |
//! | any other method error (`forbidden`, `invalidArguments`, `accountNotFound`, ...) | `Permanent` |
//! | per-object `notFound` | not an error (the object is gone, which is what was wanted) |
//! | per-object `rateLimit`, `serverFail`, `serverUnavailable` | `Transient` |
//! | any other per-object error (`forbidden`, `invalidProperties`, `invalidPatch`, ...) | `Permanent` |

use crate::types::BackendError;

/// The class for an HTTP status that is not a success.
pub(crate) fn from_status(status: u16) -> BackendError {
    match status {
        401 | 403 => BackendError::Auth("the server rejected the sign-in".into()),
        408 | 429 => BackendError::Transient("the server asked to try again later".into()),
        500..=599 => {
            BackendError::Transient("the server had a problem and could not answer".into())
        }
        404 | 410 => {
            BackendError::Permanent("the server has no JMAP service at this address".into())
        }
        _ => BackendError::Permanent("the server did not accept the request".into()),
    }
}

/// The class for a method-level error type (the `type` of an `["error", {...}, id]` response).
pub(crate) fn from_method_error(kind: &str) -> BackendError {
    match kind {
        "serverFail" | "serverUnavailable" | "serverPartialFail" | "rateLimit" => {
            BackendError::Transient("the server could not finish the request right now".into())
        }
        _ => BackendError::Permanent("the server refused the request".into()),
    }
}

/// What a per-object error in `notUpdated`, `notDestroyed` or `notCreated` means. `None` for
/// `notFound`: the object is already gone, which is the outcome that was asked for.
pub(crate) fn from_set_error(kind: &str) -> Option<BackendError> {
    match kind {
        "notFound" => None,
        "rateLimit" | "serverFail" | "serverUnavailable" => Some(BackendError::Transient(
            "the server could not change the message right now".into(),
        )),
        _ => Some(BackendError::Permanent(
            "the server refused to change the message".into(),
        )),
    }
}

/// An answer that is not what the protocol says it must be.
pub(crate) fn malformed() -> BackendError {
    BackendError::Permanent("the server's answer was not valid JMAP".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn http_statuses_map_to_the_documented_classes() {
        for (status, auth, transient) in [
            (401, true, false),
            (403, true, false),
            (408, false, true),
            (429, false, true),
            (500, false, true),
            (503, false, true),
            (404, false, false),
            (400, false, false),
        ] {
            let e = from_status(status);
            assert_eq!(matches!(e, BackendError::Auth(_)), auth, "{status}");
            assert_eq!(e.is_retryable(), transient, "{status}");
        }
    }

    #[test]
    fn method_and_object_errors_map_to_the_documented_classes() {
        for kind in [
            "serverFail",
            "serverUnavailable",
            "rateLimit",
            "serverPartialFail",
        ] {
            assert!(from_method_error(kind).is_retryable(), "{kind}");
        }
        for kind in [
            "forbidden",
            "invalidArguments",
            "accountNotFound",
            "unknownMethod",
        ] {
            assert!(
                matches!(from_method_error(kind), BackendError::Permanent(_)),
                "{kind}"
            );
        }
        assert!(from_set_error("notFound").is_none());
        assert!(from_set_error("rateLimit").unwrap().is_retryable());
        for kind in [
            "forbidden",
            "invalidProperties",
            "invalidPatch",
            "overQuota",
        ] {
            assert!(
                matches!(from_set_error(kind), Some(BackendError::Permanent(_))),
                "{kind}"
            );
        }
    }

    #[test]
    fn messages_never_contain_server_text() {
        // The functions take only the error type, and the type itself is not echoed either.
        let shown = format!("{:?}", from_method_error("secret@example.org says no"));
        assert!(!shown.contains("secret"));
        let shown = format!("{:?}", from_set_error("alice@example.org").unwrap());
        assert!(!shown.contains("alice"));
    }
}
