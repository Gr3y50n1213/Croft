// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Sending JMAP requests and reading the answers (RFC 8620 sections 3 and 6).
//!
//! [`Client`] owns the HTTP side and the credential. It only ever talks to the origin of the
//! session URL: session fetches follow same-origin redirects (at most 3, the way
//! `/.well-known/jmap` hands out the session address) and refuse any other, and every request URL
//! is checked against that origin before the `Authorization` header is attached.

use super::config::JmapConfig;
use super::error::{from_method_error, from_status, malformed};
use super::http::{Http, HttpRequest, HttpResponse, SendError};
use super::session::Session;
use super::tls::Trust;
use super::url::Url;
use crate::imap::Credential;
use crate::types::BackendError;
use base64::Engine;
use serde::Deserialize;
use serde_json::{json, Value};
use zeroize::Zeroizing;

/// Most same-origin redirects followed when fetching the session.
const MAX_REDIRECTS: usize = 3;
/// Largest session resource accepted.
const MAX_SESSION_BYTES: usize = 1024 * 1024;
/// Largest answer to an upload (a small JSON object).
const MAX_UPLOAD_ANSWER: usize = 64 * 1024;

/// A method-level failure: either a classified error, or the server's error `type` for the caller
/// that wants to handle one specially (`cannotCalculateChanges` on `Email/changes`).
#[derive(Debug)]
pub(crate) enum ApiError {
    /// An error already classified.
    Failed(BackendError),
    /// The server answered this call with `["error", {"type": ...}, id]`.
    Method(String),
}

impl ApiError {
    /// The classified error: a method error is mapped by [`from_method_error`].
    pub fn into_backend(self) -> BackendError {
        match self {
            Self::Failed(e) => e,
            Self::Method(kind) => from_method_error(&kind),
        }
    }
}

impl From<BackendError> for ApiError {
    fn from(e: BackendError) -> Self {
        Self::Failed(e)
    }
}

/// The answers to one request, in order.
#[derive(Debug)]
pub(crate) struct Responses {
    items: Vec<Option<(String, Value)>>,
    /// The `sessionState` of the answer.
    pub session_state: Option<String>,
}

impl Responses {
    /// Reads a `{"methodResponses": [...], "sessionState": ...}` answer. The calls must come back
    /// in the order they were sent, one answer each.
    pub fn parse(body: &[u8], expected_calls: usize) -> Result<Self, BackendError> {
        #[derive(Deserialize)]
        struct Raw {
            #[serde(rename = "methodResponses")]
            method_responses: Vec<(String, Value, String)>,
            #[serde(default, rename = "sessionState")]
            session_state: Option<String>,
        }
        let raw: Raw = serde_json::from_slice(body).map_err(|_| malformed())?;
        if raw.method_responses.len() != expected_calls {
            return Err(malformed());
        }
        for (i, (_, _, id)) in raw.method_responses.iter().enumerate() {
            if *id != call_id(i) {
                return Err(malformed());
            }
        }
        Ok(Self {
            items: raw
                .method_responses
                .into_iter()
                .map(|(name, args, _)| Some((name, args)))
                .collect(),
            session_state: raw.session_state,
        })
    }

    /// Takes the answer to call `index`, which must be a `name` response.
    pub fn take(&mut self, index: usize, name: &str) -> Result<Value, ApiError> {
        let (got, args) = self
            .items
            .get_mut(index)
            .and_then(Option::take)
            .ok_or_else(malformed)?;
        if got == "error" {
            let kind = args
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or("unknown")
                .to_string();
            return Err(ApiError::Method(kind));
        }
        if got != name {
            return Err(malformed().into());
        }
        Ok(args)
    }
}

fn call_id(index: usize) -> String {
    format!("c{index}")
}

/// The HTTP side of one account, with its credential.
pub(crate) struct Client {
    http: Http,
    session_url: Url,
    authorization: Zeroizing<String>,
    max_response: usize,
}

impl Client {
    /// Builds the client from validated settings. The credential is turned into the
    /// `Authorization` header value once and is not kept in any other form.
    pub fn new(
        config: &JmapConfig,
        credential: &Credential,
        session_url: Url,
    ) -> Result<Self, BackendError> {
        let authorization = match credential {
            Credential::Password(pw) => {
                let mut pair = Zeroizing::new(String::new());
                pair.push_str(&config.username);
                pair.push(':');
                pair.push_str(pw);
                let encoded = base64::engine::general_purpose::STANDARD.encode(pair.as_bytes());
                let mut header = Zeroizing::new(String::from("Basic "));
                header.push_str(&encoded);
                header
            }
            Credential::OAuth2 { access_token, .. } => {
                if access_token.contains(['\r', '\n']) {
                    return Err(BackendError::Permanent(
                        "the access token cannot be used".into(),
                    ));
                }
                let mut header = Zeroizing::new(String::from("Bearer "));
                header.push_str(access_token);
                header
            }
        };
        let trust = match config.pinned_cert {
            Some(pin) => Trust::Pinned(pin),
            None => Trust::Roots,
        };
        Ok(Self {
            http: Http::new(trust, config.timeout, config.deadline),
            session_url,
            authorization,
            max_response: config.max_response_bytes,
        })
    }

    async fn send(
        &self,
        method: &'static str,
        url: &Url,
        content_type: Option<&str>,
        body: &[u8],
        max_response: usize,
    ) -> Result<HttpResponse, SendError> {
        if !url.same_origin(&self.session_url) {
            return Err(BackendError::Permanent(
                "refusing to send the sign-in to another server".into(),
            )
            .into());
        }
        self.http
            .send(&HttpRequest {
                method,
                url,
                authorization: Some(&self.authorization),
                content_type,
                body,
                max_response,
            })
            .await
    }

    /// Fetches the session resource, following same-origin redirects.
    pub async fn fetch_session(&self) -> Result<Session, BackendError> {
        let mut url = self.session_url.clone();
        for _ in 0..=MAX_REDIRECTS {
            let resp = self
                .send("GET", &url, None, b"", MAX_SESSION_BYTES)
                .await
                .map_err(|e| e.error)?;
            match resp.status {
                200..=299 => return Session::parse(&resp.body, &url),
                301 | 302 | 303 | 307 | 308 => {
                    let next = resp
                        .location
                        .as_deref()
                        .and_then(|l| url.resolve(l))
                        .ok_or_else(|| {
                            BackendError::Permanent("the server sent a bad redirect".into())
                        })?;
                    if !next.same_origin(&self.session_url) {
                        return Err(BackendError::Permanent(
                            "the server redirected the sign-in to another server".into(),
                        ));
                    }
                    url = next;
                }
                status => return Err(from_status(status)),
            }
        }
        Err(BackendError::Permanent(
            "the server redirected too many times".into(),
        ))
    }

    /// Sends one JMAP request with `calls` (method name and arguments) and returns the answers.
    /// `using` lists the capabilities the calls need.
    pub async fn call(
        &self,
        session: &Session,
        using: &[&str],
        calls: Vec<(&'static str, Value)>,
    ) -> Result<Responses, BackendError> {
        self.call_tracked(session, using, calls)
            .await
            .map_err(|e| e.error)
    }

    /// Like [`call`](Self::call), and the error says whether the server may have acted on the
    /// request anyway (see [`SendError::request_may_have_arrived`]). Sending mail needs that: a
    /// lost answer must not be retried.
    pub async fn call_tracked(
        &self,
        session: &Session,
        using: &[&str],
        calls: Vec<(&'static str, Value)>,
    ) -> Result<Responses, SendError> {
        if calls.len() > session.max_calls {
            return Err(BackendError::Permanent("too many calls for one request".into()).into());
        }
        let n = calls.len();
        let method_calls: Vec<Value> = calls
            .into_iter()
            .enumerate()
            .map(|(i, (name, args))| json!([name, args, call_id(i)]))
            .collect();
        let body = serde_json::to_vec(&json!({"using": using, "methodCalls": method_calls}))
            .map_err(|_| malformed())?;
        let resp = self
            .send(
                "POST",
                &session.api_url,
                Some("application/json"),
                &body,
                self.max_response,
            )
            .await?;
        // The server answered, so it did (or refused to) act: nothing below is ambiguous.
        if !(200..300).contains(&resp.status) {
            return Err(from_status(resp.status).into());
        }
        Ok(Responses::parse(&resp.body, n)?)
    }

    /// Uploads a blob and returns its id (RFC 8620 section 6.1).
    pub async fn upload(
        &self,
        session: &Session,
        content_type: &str,
        bytes: &[u8],
    ) -> Result<String, BackendError> {
        let url = session
            .upload_url()
            .ok_or_else(|| BackendError::Permanent("the server does not accept uploads".into()))?;
        if session
            .max_upload
            .is_some_and(|max| bytes.len() as u64 > max)
        {
            return Err(BackendError::Permanent(
                "the message is larger than the server accepts".into(),
            ));
        }
        let resp = self
            .send("POST", url, Some(content_type), bytes, MAX_UPLOAD_ANSWER)
            .await
            .map_err(|e| e.error)?;
        if !(200..300).contains(&resp.status) {
            return Err(from_status(resp.status));
        }
        #[derive(Deserialize)]
        struct Uploaded {
            #[serde(rename = "blobId")]
            blob_id: String,
        }
        let up: Uploaded = serde_json::from_slice(&resp.body).map_err(|_| malformed())?;
        Ok(up.blob_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn answers_must_match_the_calls_sent() {
        let body = br#"{"methodResponses":[["Mailbox/get",{"list":[]},"c0"],
            ["error",{"type":"cannotCalculateChanges","description":"secret@example.org"},"c1"]],
            "sessionState":"s9"}"#;
        let mut r = Responses::parse(body, 2).unwrap();
        assert_eq!(r.session_state.as_deref(), Some("s9"));
        assert!(r.take(0, "Mailbox/get").is_ok());
        match r.take(1, "Email/changes") {
            Err(ApiError::Method(kind)) => assert_eq!(kind, "cannotCalculateChanges"),
            other => panic!("{other:?}"),
        }
        // Wrong count, wrong ids and wrong names are all malformed.
        assert!(Responses::parse(body, 3).is_err());
        let wrong_id = br#"{"methodResponses":[["Mailbox/get",{},"zzz"]]}"#;
        assert!(Responses::parse(wrong_id, 1).is_err());
        let mut r = Responses::parse(br#"{"methodResponses":[["Email/get",{},"c0"]]}"#, 1).unwrap();
        assert!(r.take(0, "Mailbox/get").is_err());
    }

    #[test]
    fn a_method_error_never_carries_server_text_into_the_class() {
        let e = ApiError::Method("serverFail".into()).into_backend();
        assert!(e.is_retryable());
        let e = ApiError::Method("forbidden".into()).into_backend();
        assert!(matches!(e, BackendError::Permanent(_)));
    }

    #[test]
    fn credentials_become_basic_or_bearer_headers() {
        let cfg = JmapConfig::new("http://127.0.0.1:1/s", "alice");
        let url = cfg.validate().unwrap();
        let c = Client::new(&cfg, &Credential::password("pw"), url.clone()).unwrap();
        // base64("alice:pw")
        assert_eq!(c.authorization.as_str(), "Basic YWxpY2U6cHc=");
        let c = Client::new(&cfg, &Credential::oauth2("alice", "tok"), url.clone()).unwrap();
        assert_eq!(c.authorization.as_str(), "Bearer tok");
        let bad = Client::new(&cfg, &Credential::oauth2("alice", "tok\r\nX: y"), url);
        assert!(bad.is_err());
    }
}
