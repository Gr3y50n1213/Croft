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
/// How fast a download is assumed to arrive at least: one second of extra time per 16 KiB of the
/// size the server announced (the same allowance as the IMAP adapter's).
const DOWNLOAD_BYTES_PER_SECOND: u64 = 16 * 1024;

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
    /// Reads a `{"methodResponses": [...], "sessionState": ...}` answer. Every call must be
    /// answered, under its own call id. A call can also produce extra answers with the same id
    /// (the `Email/set` that `onSuccessUpdateEmail` implies, RFC 8621 section 7.5); the first
    /// answer for a call is the call's own, and the others are ignored.
    pub fn parse(body: &[u8], expected_calls: usize) -> Result<Self, BackendError> {
        #[derive(Deserialize)]
        struct Raw {
            #[serde(rename = "methodResponses")]
            method_responses: Vec<(String, Value, String)>,
            #[serde(default, rename = "sessionState")]
            session_state: Option<String>,
        }
        let raw: Raw = serde_json::from_slice(body).map_err(|_| malformed())?;
        let mut items: Vec<Option<(String, Value)>> = vec![None; expected_calls];
        for (name, args, id) in raw.method_responses {
            let index = (0..expected_calls)
                .find(|i| call_id(*i) == id)
                .ok_or_else(malformed)?;
            if items[index].is_none() {
                items[index] = Some((name, args));
            }
        }
        if items.iter().any(Option::is_none) {
            return Err(malformed());
        }
        Ok(Self {
            items,
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

/// The HTTP statuses that mean the server rejected the request without acting on it: bad request,
/// sign-in refused, forbidden, body too large. Anything else (above all 5xx and 408, which a
/// reverse proxy sends after the server may have finished the work) is ambiguous.
fn refused_unprocessed(status: u16) -> bool {
    matches!(status, 400 | 401 | 403 | 413)
}

fn call_id(index: usize) -> String {
    format!("c{index}")
}

/// The `Authorization` value for HTTP Basic: `Basic ` and the base64 of `user:password`.
///
/// Every buffer that holds the secret is allocated at its final size before the secret is written
/// and is wiped on drop: the `user:password` pair, and the header, which the base64 is encoded into
/// directly with `encode_slice` (not `encode`, which returns a fresh unwiped `String`, and not into
/// a buffer that may need to grow). Nothing here reallocates with the secret inside, so no old
/// copy is freed without being wiped.
fn basic_header(user: &str, password: &str) -> Result<Zeroizing<String>, BackendError> {
    let unusable = || BackendError::Permanent("the password cannot be used".into());
    let mut pair = Zeroizing::new(String::with_capacity(user.len() + 1 + password.len()));
    pair.push_str(user);
    pair.push(':');
    pair.push_str(password);
    const PREFIX: &[u8] = b"Basic ";
    let encoded_len = base64::encoded_len(pair.len(), true).ok_or_else(unusable)?;
    let mut header: Zeroizing<Vec<u8>> =
        Zeroizing::new(Vec::with_capacity(PREFIX.len() + encoded_len));
    header.extend_from_slice(PREFIX);
    header.resize(PREFIX.len() + encoded_len, 0);
    base64::engine::general_purpose::STANDARD
        .encode_slice(pair.as_bytes(), &mut header[PREFIX.len()..])
        .map_err(|_| unusable())?;
    // The bytes are ASCII; move them into a `String` without copying them.
    let bytes = std::mem::take(&mut *header);
    match String::from_utf8(bytes) {
        Ok(text) => Ok(Zeroizing::new(text)),
        Err(e) => {
            let mut bytes = e.into_bytes();
            zeroize::Zeroize::zeroize(&mut bytes);
            Err(unusable())
        }
    }
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
            Credential::Password(pw) => basic_header(&config.username, pw)?,
            Credential::OAuth2 { access_token, .. } => {
                if access_token.contains(['\r', '\n']) {
                    return Err(BackendError::Permanent(
                        "the access token cannot be used".into(),
                    ));
                }
                // Sized exactly before the token is written, so the buffer never grows (a
                // `String` that grows moves its contents and frees the old block unwiped).
                let mut header = Zeroizing::new(String::with_capacity(7 + access_token.len()));
                header.push_str("Bearer ");
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
        self.send_with_time(
            method,
            url,
            content_type,
            body,
            max_response,
            std::time::Duration::ZERO,
        )
        .await
    }

    async fn send_with_time(
        &self,
        method: &'static str,
        url: &Url,
        content_type: Option<&str>,
        body: &[u8],
        max_response: usize,
        extra_time: std::time::Duration,
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
                extra_time,
                error_body: false,
                extra_headers: &[],
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
    /// lost, garbled or proxy-generated answer (5xx, 408, a 2xx body that is not JMAP) must not be
    /// retried. Only a connection that never completed the request, or a 400, 401, 403 or 413
    /// answer, says nothing was done.
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
        // Whatever came back, a request that was written in full may have been processed. Only a
        // few 4xx answers say it was refused unread; a 5xx or 408 is typically a proxy giving up
        // after the server did the work, and a body that does not parse tells nothing either.
        if !(200..300).contains(&resp.status) {
            return Err(SendError {
                error: from_status(resp.status),
                request_may_have_arrived: !refused_unprocessed(resp.status),
            });
        }
        Responses::parse(&resp.body, n).map_err(|error| SendError {
            error,
            request_may_have_arrived: true,
        })
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

    /// Downloads a blob (RFC 8620 section 6.2) of at most `max` bytes. `expected` is the size the
    /// server announced for it, which buys the download extra time. A 404 is `Ok(None)` (the blob
    /// is gone); a bigger answer than `max` is refused while it is read.
    pub async fn download(
        &self,
        session: &Session,
        blob_id: &str,
        content_type: &str,
        name: &str,
        expected: u64,
        max: usize,
    ) -> Result<Option<Vec<u8>>, BackendError> {
        let url = session
            .download_url(blob_id, content_type, name)
            .ok_or_else(|| {
                BackendError::Permanent("the server offers no message downloads".into())
            })?;
        let extra = std::time::Duration::from_secs(expected / DOWNLOAD_BYTES_PER_SECOND);
        let resp = self
            .send_with_time("GET", &url, None, b"", max, extra)
            .await
            .map_err(|e| e.error)?;
        match resp.status {
            200..=299 => Ok(Some(resp.body)),
            404 => Ok(None),
            status => Err(from_status(status)),
        }
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
        // A missing answer, an unknown id and a wrong name are all malformed.
        assert!(Responses::parse(body, 3).is_err());
        let wrong_id = br#"{"methodResponses":[["Mailbox/get",{},"zzz"]]}"#;
        assert!(Responses::parse(wrong_id, 1).is_err());
        let mut r = Responses::parse(br#"{"methodResponses":[["Email/get",{},"c0"]]}"#, 1).unwrap();
        assert!(r.take(0, "Mailbox/get").is_err());
    }

    #[test]
    fn extra_answers_for_one_call_are_ignored() {
        // The Email/set that onSuccessUpdateEmail implies carries the call id of the submission.
        let body = br#"{"methodResponses":[["Email/import",{"a":1},"c0"],
            ["EmailSubmission/set",{"b":2},"c1"],["Email/set",{"c":3},"c1"]]}"#;
        let mut r = Responses::parse(body, 2).unwrap();
        assert_eq!(r.take(1, "EmailSubmission/set").unwrap()["b"], 2);
        assert_eq!(r.take(0, "Email/import").unwrap()["a"], 1);
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

    #[test]
    fn long_credentials_make_the_right_header_in_exactly_sized_buffers() {
        let cfg = JmapConfig::new("http://127.0.0.1:1/s", "alice");
        let url = cfg.validate().unwrap();
        // A 2 KB token.
        let token = "t".repeat(2048);
        let c = Client::new(
            &cfg,
            &Credential::oauth2("alice", token.clone()),
            url.clone(),
        )
        .unwrap();
        assert_eq!(c.authorization.as_str(), format!("Bearer {token}"));
        assert_eq!(
            c.authorization.capacity(),
            c.authorization.len(),
            "sized up front, never grown"
        );
        // A 2 KB password, in Basic form.
        let password = "p".repeat(2048);
        let c = Client::new(&cfg, &Credential::password(password.clone()), url).unwrap();
        let expected = format!(
            "Basic {}",
            base64::engine::general_purpose::STANDARD.encode(format!("alice:{password}"))
        );
        assert_eq!(c.authorization.as_str(), expected);
        assert_eq!(c.authorization.capacity(), c.authorization.len());
    }

    #[test]
    fn only_a_few_4xx_answers_prove_the_request_was_not_acted_on() {
        for status in [400, 401, 403, 413] {
            assert!(refused_unprocessed(status), "{status}");
        }
        for status in [404, 408, 429, 500, 502, 503, 504] {
            assert!(!refused_unprocessed(status), "{status}");
        }
    }
}
