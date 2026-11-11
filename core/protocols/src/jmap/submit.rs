// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Sending mail over JMAP (`EmailSubmission/set`, RFC 8621 section 7): [`JmapSubmitter`] has the
//! same shape as [`SmtpSender`](crate::SmtpSender), so the engine can pick either.
//!
//! One send is: read the session, read the Drafts and Sent mailboxes and the identities, upload
//! the message as a blob, then one request that imports it into Drafts (keyword `$draft`) and
//! submits it, with `onSuccessUpdateEmail` moving it to Sent and clearing `$draft` once the
//! server has accepted the submission. The server files Sent; nothing is appended.
//!
//! **Never sent twice.** Everything up to the last request is safe to repeat and fails as usual
//! (`Transient`, `Auth`, `Permanent`). Once the last request has been written in full, a lost
//! answer (a timeout, a dropped connection) leaves it unknown whether the server delivered the
//! message, so that failure is `Permanent` with [`MAYBE_SENT_REASON`] and is never retried by the
//! queue; the person checks Sent. An answer that says the submission failed is a plain failure: the
//! copy imported into Drafts is deleted again (best effort) so a retry does not leave a pile of
//! drafts behind.

use super::api::{ApiError, Client};
use super::config::JmapConfig;
use super::error::{from_set_error, malformed};
use super::session::{CAP_CORE, CAP_MAIL, CAP_SUBMISSION};
use crate::imap::Credential;
use crate::smtp::{SendReceipt, MAYBE_SENT_REASON};
use crate::types::BackendError;
use serde::Deserialize;
use serde_json::{json, Value};

const USING: &[&str] = &[CAP_CORE, CAP_MAIL, CAP_SUBMISSION];

/// Sends mail for one JMAP account. Holds the settings and the credential; fetches the session per
/// message.
pub struct JmapSubmitter {
    config: JmapConfig,
    client: Client,
}

impl std::fmt::Debug for JmapSubmitter {
    /// Prints the settings (origin only) and never the credential.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("JmapSubmitter")
            .field("config", &self.config)
            .field("credential", &"***")
            .finish()
    }
}

#[derive(Deserialize)]
struct RawMailbox {
    id: String,
    #[serde(default)]
    role: Option<String>,
}

#[derive(Deserialize)]
struct RawIdentity {
    id: String,
    #[serde(default)]
    email: String,
}

impl JmapSubmitter {
    /// Creates the sender. Nothing is sent yet. Fails with [`BackendError::Permanent`] when the
    /// settings are unusable (see [`JmapBackend::new`](super::JmapBackend::new)).
    pub fn new(config: JmapConfig, credential: Credential) -> Result<Self, BackendError> {
        let url = config.validate()?;
        let client = Client::new(&config, &credential, url)?;
        Ok(Self { config, client })
    }

    /// The settings this sender was made with.
    pub fn config(&self) -> &JmapConfig {
        &self.config
    }

    /// Submits one message. `envelope_from` and `recipients` are bare addresses (no display names,
    /// no angle brackets), exactly the ones the server should use: Bcc recipients belong in
    /// `recipients` and not in the headers. `mime` is the whole RFC 5322 message; bare line feeds
    /// are turned into CRLF.
    pub async fn send<R: AsRef<str>>(
        &self,
        envelope_from: &str,
        recipients: &[R],
        mime: &[u8],
    ) -> Result<SendReceipt, BackendError> {
        let bad = || BackendError::Permanent("an address is not valid".into());
        if recipients.is_empty() {
            return Err(BackendError::Permanent(
                "the message has no recipients".into(),
            ));
        }
        if !address_is_valid(envelope_from)
            || recipients.iter().any(|r| !address_is_valid(r.as_ref()))
        {
            return Err(bad());
        }
        let session = self.client.fetch_session().await?;
        if !session.has_submission {
            return Err(BackendError::Permanent(
                "the server cannot send mail over JMAP".into(),
            ));
        }
        let account = session.account_id.clone();

        // Where the message is staged and filed, and who sends it.
        let calls = vec![
            (
                "Mailbox/get",
                json!({"accountId": account, "ids": null, "properties": ["id", "role"]}),
            ),
            ("Identity/get", json!({"accountId": account, "ids": null})),
        ];
        let mut r = self.client.call(&session, USING, calls).await?;
        let mailboxes: Vec<RawMailbox> = list_of(r.take(0, "Mailbox/get"))?;
        let identities: Vec<RawIdentity> = list_of(r.take(1, "Identity/get"))?;
        let role = |name: &str| {
            mailboxes
                .iter()
                .find(|m| m.role.as_deref() == Some(name))
                .map(|m| m.id.clone())
        };
        let (drafts, sent) = (role("drafts"), role("sent"));
        let Some(staging) = drafts.clone().or_else(|| sent.clone()) else {
            return Err(BackendError::Permanent(
                "the account has no Drafts or Sent folder".into(),
            ));
        };
        let identity = pick_identity(&identities, envelope_from).ok_or_else(|| {
            BackendError::Permanent("the account has no identity to send from".into())
        })?;

        let body = normalize_line_endings(mime);
        let blob = self
            .client
            .upload(&session, "message/rfc822", &body)
            .await?;

        let staged_as_draft = drafts.is_some();
        let keywords = if staged_as_draft {
            json!({"$draft": true, "$seen": true})
        } else {
            json!({"$seen": true})
        };
        let mut submission = json!({
            "accountId": account,
            "create": {"s": {
                "identityId": identity,
                "emailId": "#m",
                "envelope": {
                    "mailFrom": {"email": envelope_from},
                    "rcptTo": recipients.iter().map(|r| json!({"email": r.as_ref()})).collect::<Vec<_>>(),
                }}},
        });
        if staged_as_draft {
            // On success: out of Drafts, into Sent (or, with no Sent folder, gone), and no longer a draft.
            match &sent {
                Some(sent_id) => {
                    submission["onSuccessUpdateEmail"] = json!({"#s": {
                        "keywords/$draft": null,
                        format!("mailboxIds/{}", super::mapping::pointer_escape(&staging)): null,
                        format!("mailboxIds/{}", super::mapping::pointer_escape(sent_id)): true,
                    }});
                }
                None => submission["onSuccessDestroyEmail"] = json!(["#s"]),
            }
        }
        let calls = vec![
            (
                "Email/import",
                json!({"accountId": account, "emails": {"m": {
                    "blobId": blob, "mailboxIds": {staging.clone(): true}, "keywords": keywords}}}),
            ),
            ("EmailSubmission/set", submission),
        ];
        // From here on a lost answer is ambiguous.
        let mut r = match self.client.call_tracked(&session, USING, calls).await {
            Ok(r) => r,
            Err(e) if e.request_may_have_arrived => {
                return Err(BackendError::Permanent(MAYBE_SENT_REASON.into()))
            }
            Err(e) => return Err(e.error),
        };
        let imported = r.take(0, "Email/import").map_err(ApiError::into_backend)?;
        let Some(email_id) = imported
            .get("created")
            .and_then(|c| c.get("m"))
            .and_then(|m| m.get("id"))
            .and_then(Value::as_str)
            .map(str::to_string)
        else {
            return Err(set_failure(&imported, "notCreated", "m"));
        };
        let outcome = match r.take(1, "EmailSubmission/set") {
            Ok(args) => {
                if args.get("created").and_then(|c| c.get("s")).is_some() {
                    Ok(())
                } else {
                    Err(set_failure(&args, "notCreated", "s"))
                }
            }
            Err(e) => Err(e.into_backend()),
        };
        match outcome {
            Ok(()) => Ok(SendReceipt {
                recipients: recipients.len(),
                bytes: mime.len(),
            }),
            Err(e) => {
                // Not sent: take the staged copy away again (best effort; the failure above is
                // what the caller needs to hear about).
                let cleanup = vec![(
                    "Email/set",
                    json!({"accountId": account, "destroy": [email_id]}),
                )];
                let _ = self.client.call(&session, USING, cleanup).await;
                Err(e)
            }
        }
    }
}

/// The error for a failed create: the per-object error type from `notCreated`, classified.
fn set_failure(args: &Value, key: &str, creation_id: &str) -> BackendError {
    let kind = args
        .get(key)
        .and_then(|n| n.get(creation_id))
        .and_then(|e| e.get("type"))
        .and_then(Value::as_str);
    match kind {
        Some(k) => from_set_error(k).unwrap_or_else(|| {
            BackendError::Permanent("the server could not find what it needed to send".into())
        }),
        None => malformed(),
    }
}

fn list_of<T: for<'de> Deserialize<'de>>(
    answer: Result<Value, ApiError>,
) -> Result<Vec<T>, BackendError> {
    let args = answer.map_err(ApiError::into_backend)?;
    serde_json::from_value(args.get("list").cloned().ok_or_else(malformed)?)
        .map_err(|_| malformed())
}

/// The identity for the envelope sender: an exact match, then a wildcard (`*@domain`), then the
/// first one (the server refuses a sender the person may not use, with `forbiddenFrom`).
fn pick_identity(identities: &[RawIdentity], from: &str) -> Option<String> {
    let lower = from.to_ascii_lowercase();
    let domain = lower.rsplit_once('@').map(|(_, d)| d.to_string());
    identities
        .iter()
        .find(|i| i.email.to_ascii_lowercase() == lower)
        .or_else(|| {
            identities.iter().find(|i| {
                i.email.starts_with("*@")
                    && domain.as_deref() == Some(&i.email.to_ascii_lowercase()[2..])
            })
        })
        .or_else(|| identities.first())
        .map(|i| i.id.clone())
}

/// A bare address: something before and after one `@`, no spaces, controls or angle brackets.
fn address_is_valid(addr: &str) -> bool {
    let Some((local, domain)) = addr.split_once('@') else {
        return false;
    };
    !local.is_empty()
        && !domain.is_empty()
        && !domain.contains('@')
        && addr.len() <= 320
        && !addr
            .chars()
            .any(|c| c.is_control() || c.is_whitespace() || matches!(c, '<' | '>' | ',' | ';'))
}

/// Every line ends in CRLF: a bare LF (or a bare CR) becomes CRLF.
fn normalize_line_endings(mime: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(mime.len() + mime.len() / 40);
    let mut i = 0;
    while i < mime.len() {
        match mime[i] {
            b'\r' if mime.get(i + 1) == Some(&b'\n') => {
                out.extend_from_slice(b"\r\n");
                i += 2;
            }
            b'\r' | b'\n' => {
                out.extend_from_slice(b"\r\n");
                i += 1;
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ident(id: &str, email: &str) -> RawIdentity {
        RawIdentity {
            id: id.into(),
            email: email.into(),
        }
    }

    #[test]
    fn identities_are_picked_by_address_then_wildcard_then_first() {
        let all = [
            ident("i1", "first@example.org"),
            ident("i2", "Alice@Example.org"),
            ident("i3", "*@wild.org"),
        ];
        assert_eq!(
            pick_identity(&all, "alice@example.org").as_deref(),
            Some("i2")
        );
        assert_eq!(pick_identity(&all, "bob@wild.org").as_deref(), Some("i3"));
        assert_eq!(pick_identity(&all, "zed@other.org").as_deref(), Some("i1"));
        assert_eq!(pick_identity(&[], "a@b.c"), None);
    }

    #[test]
    fn addresses_are_checked_before_anything_is_sent() {
        assert!(address_is_valid("a@b.c"));
        for bad in [
            "",
            "a",
            "@b.c",
            "a@",
            "a@b@c",
            "a b@c.d",
            "a@b.c\r\nRCPT",
            "<a@b.c>",
            "a,b@c.d",
        ] {
            assert!(!address_is_valid(bad), "{bad:?}");
        }
    }

    #[test]
    fn line_endings_become_crlf() {
        assert_eq!(normalize_line_endings(b"a\nb\r\nc\rd"), b"a\r\nb\r\nc\r\nd");
        assert_eq!(normalize_line_endings(b""), b"");
    }

    #[test]
    fn a_failed_create_is_classified_without_server_text() {
        let args = json!({"notCreated": {"s": {"type": "forbiddenFrom", "description": "alice@example.org"}}});
        let e = set_failure(&args, "notCreated", "s");
        assert!(matches!(&e, BackendError::Permanent(m) if !m.contains("alice")));
        let args = json!({"notCreated": {"s": {"type": "rateLimit"}}});
        assert!(set_failure(&args, "notCreated", "s").is_retryable());
        assert!(matches!(
            set_failure(&json!({}), "notCreated", "s"),
            BackendError::Permanent(_)
        ));
    }

    #[tokio::test]
    async fn bad_envelopes_are_refused_before_connecting() {
        let s = JmapSubmitter::new(
            JmapConfig::new("http://127.0.0.1:1/jmap/session", "u"),
            Credential::password("pw"),
        )
        .unwrap();
        let none: [&str; 0] = [];
        assert!(s.send("a@b.c", &none, b"x").await.is_err());
        assert!(s.send("a@b.c", &["x@y.z\r\nDATA"], b"x").await.is_err());
        assert!(s.send("bad", &["x@y.z"], b"x").await.is_err());
        let shown = format!("{s:?}");
        assert!(!shown.contains("pw"));
    }
}
