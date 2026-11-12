// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Plain HTTPS requests for an OpenID Connect relying party (the Companion, T25): fetching a
//! discovery document or a key set, and posting a form to a token endpoint. It is the same small
//! client as JMAP and the OAuth token client (certificate validation always on with the system's
//! and Mozilla's roots, TLS 1.2 and newer, no redirects followed, no cookies, no proxies, every
//! wait and the whole request bounded, the answer's size capped), offered to code outside this
//! crate. Plain `http` is accepted only to a loopback address (tests); the caller decides whether
//! it allows even that. It is for addresses the server's admin configured, so the process's address
//! policy (`crate::netpolicy`, for addresses users choose) does not restrict it.

use std::time::Duration;

use zeroize::Zeroizing;

use crate::jmap::http::{Http, HttpRequest};
use crate::jmap::tls::Trust;
use crate::jmap::url::{is_loopback_name, Url};
use crate::types::BackendError;

/// Each wait for the server.
const TIMEOUT: Duration = Duration::from_secs(10);
/// The whole request.
const DEADLINE: Duration = Duration::from_secs(20);

/// Why a request failed. Carries no part of the request or the answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FetchError {
    /// Not an absolute `https://` URL (or `http://` to a loopback address).
    BadUrl,
    /// The server could not be reached, or did not answer in time: try later.
    Unreachable,
    /// The connection was refused for good (a certificate that does not validate, a broken answer).
    Refused,
    /// The server answered with a redirect, which is never followed.
    Redirect,
}

impl std::fmt::Display for FetchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::BadUrl => "not an https address",
            Self::Unreachable => "the server could not be reached",
            Self::Refused => "the connection was refused",
            Self::Redirect => "the server answered with a redirect",
        })
    }
}

impl std::error::Error for FetchError {}

/// An answer: its status and body (the body of a 2xx or 4xx answer; wiped when dropped, since a
/// token endpoint's answer holds tokens).
pub struct Answer {
    /// HTTP status.
    pub status: u16,
    /// The body.
    pub body: Zeroizing<Vec<u8>>,
}

impl std::fmt::Debug for Answer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Answer")
            .field("status", &self.status)
            .field("body_len", &self.body.len())
            .finish()
    }
}

/// Whether `url` names a loopback address over plain `http` (what [`get`] and [`post_form`]
/// accept besides `https`).
pub fn is_loopback_http(url: &str) -> bool {
    Url::parse(url).is_some_and(|u| !u.secure && is_loopback_name(&u.host))
}

async fn send(
    method: &'static str,
    url: &str,
    authorization: Option<&str>,
    content_type: Option<&str>,
    body: &[u8],
    max_answer: usize,
) -> Result<Answer, FetchError> {
    let url = Url::parse(url).ok_or(FetchError::BadUrl)?;
    if !url.is_allowed() {
        return Err(FetchError::BadUrl);
    }
    // The OIDC provider is the admin's own choice, so the address policy (for what users choose)
    // does not apply: a provider on the same LAN or Docker network keeps working.
    let http = Http::new(Trust::Roots, TIMEOUT, DEADLINE).admin_configured();
    let req = HttpRequest {
        method,
        url: &url,
        authorization,
        content_type,
        body,
        max_response: max_answer,
        extra_time: Duration::ZERO,
        error_body: true,
    };
    let answer = http.send(&req).await.map_err(|e| match e.error {
        BackendError::Transient(_) => FetchError::Unreachable,
        _ => FetchError::Refused,
    })?;
    if (300..400).contains(&answer.status) {
        return Err(FetchError::Redirect);
    }
    Ok(Answer {
        status: answer.status,
        body: Zeroizing::new(answer.body),
    })
}

/// `GET url`, reading at most `max_answer` bytes of the answer.
pub async fn get(url: &str, max_answer: usize) -> Result<Answer, FetchError> {
    send("GET", url, None, None, &[], max_answer).await
}

/// `POST url` with a form body (`application/x-www-form-urlencoded`) and, when given, an
/// `Authorization` header (HTTP Basic client authentication), reading at most `max_answer` bytes
/// of the answer. The header is never logged.
pub async fn post_form(
    url: &str,
    form: &str,
    authorization: Option<&str>,
    max_answer: usize,
) -> Result<Answer, FetchError> {
    send(
        "POST",
        url,
        authorization,
        Some("application/x-www-form-urlencoded"),
        form.as_bytes(),
        max_answer,
    )
    .await
}

/// Encodes a form body (`application/x-www-form-urlencoded`), wiped when dropped.
pub fn form(pairs: &[(&str, &str)]) -> Zeroizing<String> {
    let enc = |s: &str| -> String {
        s.bytes()
            .map(|b| {
                if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
                    char::from(b).to_string()
                } else {
                    format!("%{b:02X}")
                }
            })
            .collect()
    };
    Zeroizing::new(
        pairs
            .iter()
            .map(|(k, v)| format!("{}={}", enc(k), enc(v)))
            .collect::<Vec<_>>()
            .join("&"),
    )
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[test]
    fn only_https_or_loopback_http_is_accepted() {
        assert!(is_loopback_http("http://127.0.0.1:8080/x"));
        assert!(!is_loopback_http("https://127.0.0.1/x"));
        assert!(!is_loopback_http("http://example.org/x"));
        assert_eq!(
            form(&[("a b", "c&d=é"), ("x", "")]).as_str(),
            "a%20b=c%26d%3D%C3%A9&x="
        );
    }

    #[tokio::test]
    async fn plain_http_elsewhere_is_refused_and_redirects_are_not_followed() {
        assert_eq!(
            get("http://example.org/", 1000).await.unwrap_err(),
            FetchError::BadUrl
        );
        assert_eq!(get("ftp://x/", 1000).await.unwrap_err(), FetchError::BadUrl);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            for answer in [
                "HTTP/1.1 302 Found\r\nLocation: https://evil.example/\r\nContent-Length: 0\r\n\r\n",
                "HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n{}",
            ] {
                let (mut s, _) = listener.accept().await.unwrap();
                let mut buf = [0u8; 2048];
                let _ = s.read(&mut buf).await.unwrap();
                s.write_all(answer.as_bytes()).await.unwrap();
            }
        });
        let url = format!("http://127.0.0.1:{port}/x");
        assert_eq!(get(&url, 1000).await.unwrap_err(), FetchError::Redirect);
        let ok = get(&url, 1000).await.unwrap();
        assert_eq!((ok.status, ok.body.as_slice()), (200, &b"{}"[..]));
    }
}
