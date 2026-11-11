// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! A small HTTP/1.1 client for JMAP, written for exactly what the adapter needs.
//!
//! Why not `reqwest` or the `jmap-client` crate (which uses it): the adapter must apply the same
//! certificate rules as IMAP and SMTP (TLS 1.2 and newer, validation always on, a server pinned
//! only by the exact fingerprint of its certificate). `jmap-client` 0.4.3 offers one switch,
//! `danger_accept_invalid_certs`, and no way to give it a verifier, and it follows redirects and
//! pulls in its own TLS provider. A hand-sized client on the rustls and `ring` stack the other
//! adapters already use makes every limit explicit.
//!
//! What it does: one request per connection (`Connection: close`), no redirects, no cookies, no
//! proxies, no compression, no HTTP/2. The request carries `Host`, a fixed `User-Agent: Croft`,
//! `Accept`, `Content-Length`, and the `Authorization` header when one is given. The header is
//! built in a buffer that is wiped when dropped and is never printed or logged.
//!
//! Limits: the connection attempt, the TLS handshake and every wait for more bytes are bounded by
//! the timeout; the whole request by the deadline (plus a little per megabyte sent); the status
//! line and headers by 16 KiB; the body by the caller's maximum. A body is read only for a 2xx
//! answer, because an error answer's body is of no use.

use super::tls::{self, Trust};
use super::url::Url;
use crate::types::BackendError;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::OnceCell;
use tokio::time::timeout;
use tokio_rustls::rustls::pki_types::ServerName;
use tokio_rustls::rustls::{self, ClientConfig};
use tokio_rustls::TlsConnector;
use zeroize::Zeroizing;

/// Most bytes of status line and headers read.
const MAX_HEAD: usize = 16 * 1024;
/// Most addresses tried for one host.
const MAX_ADDRS: usize = 4;

/// What the server answered. The body is empty unless the status is 2xx.
#[derive(Debug)]
pub(crate) struct HttpResponse {
    /// Status code.
    pub status: u16,
    /// The `Location` header, for redirects.
    pub location: Option<String>,
    /// The body of a 2xx answer.
    pub body: Vec<u8>,
}

/// Why a request failed, and whether the server may nevertheless have acted on it.
#[derive(Debug)]
pub(crate) struct SendError {
    /// What the caller sees.
    pub error: BackendError,
    /// True when the whole request had been written before the failure, so the server may have
    /// processed it (a lost answer). False for a failure to connect, to set up TLS or to write the
    /// request: then nothing happened on the server and retrying is safe.
    pub request_may_have_arrived: bool,
}

impl From<BackendError> for SendError {
    fn from(error: BackendError) -> Self {
        Self {
            error,
            request_may_have_arrived: false,
        }
    }
}

/// One request.
pub(crate) struct HttpRequest<'a> {
    /// `GET` or `POST`.
    pub method: &'static str,
    /// Where to send it. Must be allowed (`https`, or `http` to loopback).
    pub url: &'a Url,
    /// The value of the `Authorization` header, if any.
    pub authorization: Option<&'a str>,
    /// `Content-Type` of the body.
    pub content_type: Option<&'a str>,
    /// The body (empty for `GET`).
    pub body: &'a [u8],
    /// Largest body accepted in the answer.
    pub max_response: usize,
}

/// The HTTP side of one account: how to trust the server and how long to wait.
pub(crate) struct Http {
    trust: Trust,
    tls: OnceCell<Arc<ClientConfig>>,
    timeout: Duration,
    deadline: Duration,
}

impl Http {
    /// A client that judges certificates by `trust`.
    pub fn new(trust: Trust, timeout: Duration, deadline: Duration) -> Self {
        Self {
            trust,
            tls: OnceCell::new(),
            timeout,
            deadline,
        }
    }

    async fn tls_config(&self) -> Result<Arc<ClientConfig>, BackendError> {
        self.tls
            .get_or_try_init(|| tls::client_config(&self.trust))
            .await
            .cloned()
    }

    /// Sends the request and reads the answer, within the deadline.
    pub async fn send(&self, req: &HttpRequest<'_>) -> Result<HttpResponse, SendError> {
        if !req.url.is_allowed() {
            return Err(BackendError::Permanent(
                "refusing to send to an address that is not https".into(),
            )
            .into());
        }
        let allowance = Duration::from_secs((req.body.len() / (256 * 1024)) as u64);
        let written = AtomicBool::new(false);
        let run = self.exchange(req, &written);
        match timeout(self.deadline.saturating_add(allowance), run).await {
            Ok(Ok(r)) => Ok(r),
            Ok(Err(error)) => Err(SendError {
                error,
                request_may_have_arrived: written.load(Ordering::Relaxed),
            }),
            Err(_) => Err(SendError {
                error: BackendError::Transient("the server took too long to answer".into()),
                request_may_have_arrived: written.load(Ordering::Relaxed),
            }),
        }
    }

    async fn exchange(
        &self,
        req: &HttpRequest<'_>,
        written: &AtomicBool,
    ) -> Result<HttpResponse, BackendError> {
        let tcp = self.connect(req.url).await?;
        let _ = tcp.set_nodelay(true);
        let head = request_head(req);
        if req.url.secure {
            let name = ServerName::try_from(req.url.host.clone())
                .map_err(|_| BackendError::Permanent("the server name is not valid".into()))?;
            let connector = TlsConnector::from(self.tls_config().await?);
            let stream = match timeout(self.timeout, connector.connect(name, tcp)).await {
                Ok(Ok(s)) => s,
                Ok(Err(e)) => return Err(from_io(&e)),
                Err(_) => {
                    return Err(BackendError::Transient(
                        "the secure connection timed out".into(),
                    ))
                }
            };
            self.talk(stream, &head, req, written).await
        } else {
            self.talk(tcp, &head, req, written).await
        }
    }

    async fn connect(&self, url: &Url) -> Result<TcpStream, BackendError> {
        let unreachable = || BackendError::Transient("could not connect to the server".into());
        let addrs: Vec<SocketAddr> = match timeout(
            self.timeout,
            tokio::net::lookup_host((url.host.as_str(), url.port)),
        )
        .await
        {
            Ok(Ok(it)) => it.take(MAX_ADDRS).collect(),
            _ => return Err(unreachable()),
        };
        if addrs.is_empty() {
            return Err(unreachable());
        }
        // Plain `http` is for a test server on this machine: every address the name resolves to
        // must be loopback, or the credential could go out in the clear.
        if !url.secure && addrs.iter().any(|a| !a.ip().is_loopback()) {
            return Err(BackendError::Permanent(
                "plain http is only allowed for localhost".into(),
            ));
        }
        let per_address = (self.timeout / 3).max(Duration::from_millis(200));
        for addr in addrs {
            if let Ok(Ok(s)) = timeout(per_address, TcpStream::connect(addr)).await {
                return Ok(s);
            }
        }
        Err(unreachable())
    }

    async fn talk<S: AsyncRead + AsyncWrite + Unpin>(
        &self,
        mut stream: S,
        head: &Zeroizing<String>,
        req: &HttpRequest<'_>,
        written: &AtomicBool,
    ) -> Result<HttpResponse, BackendError> {
        let io_err = |_: std::io::Error| BackendError::Transient("network error".into());
        stream.write_all(head.as_bytes()).await.map_err(io_err)?;
        stream.write_all(req.body).await.map_err(io_err)?;
        stream.flush().await.map_err(io_err)?;
        written.store(true, Ordering::Relaxed);
        read_response(&mut stream, req.max_response, self.timeout).await
    }
}

/// The request line and headers, in a buffer that is wiped when dropped (it holds the
/// `Authorization` value).
fn request_head(req: &HttpRequest<'_>) -> Zeroizing<String> {
    let mut head = Zeroizing::new(String::with_capacity(256));
    head.push_str(req.method);
    head.push(' ');
    head.push_str(&req.url.path_and_query);
    head.push_str(" HTTP/1.1\r\nHost: ");
    head.push_str(&req.url.host_header());
    head.push_str("\r\nUser-Agent: Croft\r\nAccept: application/json\r\nAccept-Encoding: identity\r\nConnection: close\r\n");
    if let Some(auth) = req.authorization {
        head.push_str("Authorization: ");
        head.push_str(auth);
        head.push_str("\r\n");
    }
    if let Some(ct) = req.content_type {
        head.push_str("Content-Type: ");
        head.push_str(ct);
        head.push_str("\r\n");
    }
    if req.method == "POST" {
        head.push_str(&format!("Content-Length: {}\r\n", req.body.len()));
    }
    head.push_str("\r\n");
    head
}

/// Classifies an I/O error from the handshake: a certificate or protocol problem keeps its own
/// class, anything else is a network problem.
fn from_io(e: &std::io::Error) -> BackendError {
    if let Some(t) = e
        .get_ref()
        .and_then(|inner| inner.downcast_ref::<rustls::Error>())
    {
        return tls::from_tls(t);
    }
    BackendError::Transient("could not set up the secure connection".into())
}

/// Reads more bytes onto `buf`. `Ok(0)` is the end of the stream; a TLS stream that ends without a
/// close notice counts as the end too, because a truncated body fails to parse anyway.
async fn fill<R: AsyncRead + Unpin>(
    reader: &mut R,
    buf: &mut Vec<u8>,
    idle: Duration,
) -> Result<usize, BackendError> {
    let mut chunk = [0u8; 8192];
    match timeout(idle, reader.read(&mut chunk)).await {
        Ok(Ok(n)) => {
            buf.extend_from_slice(&chunk[..n]);
            Ok(n)
        }
        Ok(Err(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => Ok(0),
        Ok(Err(_)) => Err(BackendError::Transient("network error".into())),
        Err(_) => Err(BackendError::Transient(
            "the server stopped answering".into(),
        )),
    }
}

fn unreadable() -> BackendError {
    BackendError::Transient("the server sent an answer that could not be read".into())
}

fn too_large() -> BackendError {
    BackendError::Permanent("the server's answer was larger than allowed".into())
}

/// Reads an HTTP/1.1 answer: headers, then (for 2xx) a body of at most `max_body` bytes.
pub(crate) async fn read_response<R: AsyncRead + Unpin>(
    reader: &mut R,
    max_body: usize,
    idle: Duration,
) -> Result<HttpResponse, BackendError> {
    let mut buf = Vec::with_capacity(4096);
    let (head_len, status, location, content_length, chunked) = loop {
        let mut headers = [httparse::EMPTY_HEADER; 64];
        let mut response = httparse::Response::new(&mut headers);
        match response.parse(&buf) {
            Ok(httparse::Status::Complete(n)) => {
                let status = response.code.ok_or_else(unreadable)?;
                let mut location = None;
                let mut content_length = None;
                let mut chunked = false;
                for h in response.headers.iter() {
                    if h.name.eq_ignore_ascii_case("location") {
                        location = std::str::from_utf8(h.value)
                            .ok()
                            .map(|v| v.trim().to_string());
                    } else if h.name.eq_ignore_ascii_case("content-length") {
                        content_length = std::str::from_utf8(h.value)
                            .ok()
                            .and_then(|v| v.trim().parse::<usize>().ok());
                    } else if h.name.eq_ignore_ascii_case("transfer-encoding") {
                        chunked = std::str::from_utf8(h.value)
                            .is_ok_and(|v| v.to_ascii_lowercase().contains("chunked"));
                    }
                }
                break (n, status, location, content_length, chunked);
            }
            Ok(httparse::Status::Partial) => {
                if buf.len() >= MAX_HEAD || fill(reader, &mut buf, idle).await? == 0 {
                    return Err(unreadable());
                }
            }
            Err(_) => return Err(unreadable()),
        }
    };
    if !(200..600).contains(&status) {
        return Err(unreadable());
    }
    if !(200..300).contains(&status) || status == 204 {
        return Ok(HttpResponse {
            status,
            location,
            body: Vec::new(),
        });
    }

    let mut raw = buf.split_off(head_len);
    let body = if chunked {
        let cap = max_body.saturating_mul(2).saturating_add(MAX_HEAD);
        loop {
            match decode_chunked(&raw, max_body)? {
                Some(body) => break body,
                None => {
                    if raw.len() > cap || fill(reader, &mut raw, idle).await? == 0 {
                        return Err(unreadable());
                    }
                }
            }
        }
    } else if let Some(len) = content_length {
        if len > max_body {
            return Err(too_large());
        }
        while raw.len() < len {
            if fill(reader, &mut raw, idle).await? == 0 {
                return Err(unreadable());
            }
        }
        raw.truncate(len);
        raw
    } else {
        while raw.len() <= max_body {
            if fill(reader, &mut raw, idle).await? == 0 {
                break;
            }
        }
        if raw.len() > max_body {
            return Err(too_large());
        }
        raw
    };
    Ok(HttpResponse {
        status,
        location,
        body,
    })
}

/// Decodes a chunked body from the bytes read so far. `Ok(Some(body))` when the final chunk has
/// arrived, `Ok(None)` when more bytes are needed, an error when the framing is wrong or the
/// decoded body would pass `max`. Trailers after the last chunk are ignored.
pub(crate) fn decode_chunked(raw: &[u8], max: usize) -> Result<Option<Vec<u8>>, BackendError> {
    let mut pos = 0usize;
    let mut out = Vec::new();
    loop {
        let rest = &raw[pos..];
        let Some(eol) = rest.windows(2).position(|w| w == b"\r\n") else {
            // A size line is a few hex digits and an optional extension; a long one is abuse.
            return if rest.len() > 128 {
                Err(unreadable())
            } else {
                Ok(None)
            };
        };
        let line = std::str::from_utf8(&rest[..eol]).map_err(|_| unreadable())?;
        let digits = line.split(';').next().unwrap_or("").trim();
        if digits.is_empty() || digits.len() > 8 {
            return Err(unreadable());
        }
        let size = usize::from_str_radix(digits, 16).map_err(|_| unreadable())?;
        pos += eol + 2;
        if size == 0 {
            return Ok(Some(out));
        }
        if out.len().saturating_add(size) > max {
            return Err(too_large());
        }
        let end = pos + size;
        if raw.len() < end + 2 {
            return Ok(None);
        }
        if &raw[end..end + 2] != b"\r\n" {
            return Err(unreadable());
        }
        out.extend_from_slice(&raw[pos..end]);
        pos = end + 2;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const IDLE: Duration = Duration::from_secs(2);

    async fn parse(bytes: &[u8], max: usize) -> Result<HttpResponse, BackendError> {
        let mut r = bytes;
        read_response(&mut r, max, IDLE).await
    }

    #[tokio::test]
    async fn reads_content_length_and_chunked_bodies() {
        let r = parse(b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nhello", 100)
            .await
            .unwrap();
        assert_eq!((r.status, r.body.as_slice()), (200, &b"hello"[..]));
        let r = parse(
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n3\r\nabc\r\n2;x=1\r\nde\r\n0\r\n\r\n",
            100,
        )
        .await
        .unwrap();
        assert_eq!(r.body, b"abcde");
        let r = parse(b"HTTP/1.0 200 OK\r\n\r\nuntil the end", 100)
            .await
            .unwrap();
        assert_eq!(r.body, b"until the end");
    }

    #[tokio::test]
    async fn error_answers_carry_no_body_and_redirects_keep_the_location() {
        let r = parse(
            b"HTTP/1.1 401 Unauthorized\r\nContent-Length: 3\r\n\r\nabc",
            100,
        )
        .await
        .unwrap();
        assert_eq!((r.status, r.body.len()), (401, 0));
        let r = parse(
            b"HTTP/1.1 307 Temporary Redirect\r\nLocation: /jmap/session\r\nContent-Length: 0\r\n\r\n",
            100,
        )
        .await
        .unwrap();
        assert_eq!(r.location.as_deref(), Some("/jmap/session"));
    }

    #[tokio::test]
    async fn oversized_and_broken_answers_are_refused() {
        let big = parse(b"HTTP/1.1 200 OK\r\nContent-Length: 500\r\n\r\n", 100).await;
        assert!(matches!(big, Err(BackendError::Permanent(_))));
        let big = parse(
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n64\r\nxx",
            10,
        )
        .await;
        assert!(matches!(big, Err(BackendError::Permanent(_))));
        let endless = parse(b"HTTP/1.1 200 OK\r\n\r\nxxxxxxxxxxxxxxxxxxxxxxxx", 8).await;
        assert!(matches!(endless, Err(BackendError::Permanent(_))));
        for bad in [
            &b"garbage\r\n\r\n"[..],
            b"HTTP/1.1 200 OK\r\nContent-Length: 10\r\n\r\nshort",
            b"HTTP/1.1 700 Weird\r\n\r\n",
            b"",
        ] {
            assert!(matches!(
                parse(bad, 100).await,
                Err(BackendError::Transient(_))
            ));
        }
    }

    #[test]
    fn the_request_names_only_what_it_needs() {
        let url = Url::parse("https://mail.example.org:8443/jmap/").unwrap();
        let req = HttpRequest {
            method: "POST",
            url: &url,
            authorization: Some("Basic c2VjcmV0"),
            content_type: Some("application/json"),
            body: b"{}",
            max_response: 10,
        };
        let head = request_head(&req);
        assert!(head.starts_with("POST /jmap/ HTTP/1.1\r\nHost: mail.example.org:8443\r\n"));
        assert!(head.contains("Authorization: Basic c2VjcmV0\r\n"));
        assert!(head.contains("Content-Length: 2\r\n"));
        assert!(head.ends_with("\r\n\r\n"));
    }

    #[tokio::test]
    async fn plain_http_to_a_real_host_is_never_attempted() {
        let http = Http::new(Trust::Roots, IDLE, IDLE);
        let url = Url::parse("http://mail.example.org/jmap/session").unwrap();
        let req = HttpRequest {
            method: "GET",
            url: &url,
            authorization: Some("Basic c2VjcmV0"),
            content_type: None,
            body: b"",
            max_response: 10,
        };
        let err = http.send(&req).await.unwrap_err();
        assert!(matches!(err.error, BackendError::Permanent(_)));
        assert!(!err.request_may_have_arrived);
    }
}
