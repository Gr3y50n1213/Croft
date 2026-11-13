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

/// What the server answered. The body is empty unless the status is 2xx. `Debug` leaves the
/// body out: it can hold tokens (an OAuth token answer) or mail.
pub(crate) struct HttpResponse {
    /// Status code.
    pub status: u16,
    /// The `Location` header, for redirects.
    pub location: Option<String>,
    /// The body of a 2xx answer.
    pub body: Vec<u8>,
}

impl std::fmt::Debug for HttpResponse {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HttpResponse")
            .field("status", &self.status)
            .field("body_len", &self.body.len())
            .finish_non_exhaustive()
    }
}

/// Why a request failed, and whether the server may nevertheless have acted on it.
#[derive(Debug)]
pub(crate) struct SendError {
    /// What the caller sees.
    pub error: BackendError,
    /// True when the server may have processed the request: the whole request had been written
    /// before the failure (a lost answer), or the caller found the answer itself ambiguous (a
    /// proxy's 5xx, an unreadable 2xx body). False for a failure to connect, to set up TLS or to
    /// write the request: then nothing happened on the server and retrying is safe.
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
    /// Extra time on top of the deadline, for an answer known to be big (a message download).
    pub extra_time: Duration,
    /// Also read the body of a 4xx answer (an OAuth token endpoint puts its error code there).
    pub error_body: bool,
    /// Stop after the status line and headers: the body is never read (T37: a one-click
    /// unsubscribe acts when the server answers; a large or cut-off page is not a failure).
    pub status_only: bool,
    /// More headers (T28: Web Push's `TTL`, `Urgency`, `Content-Encoding`). Names and values are
    /// the caller's own constants and encodings; a value with a line break is refused.
    pub extra_headers: &'a [(&'a str, &'a str)],
}

/// The HTTP side of one account: how to trust the server and how long to wait.
pub(crate) struct Http {
    trust: Trust,
    tls: OnceCell<Arc<ClientConfig>>,
    timeout: Duration,
    deadline: Duration,
    /// Skips the process's address policy: only for the Companion's OIDC provider, which its admin
    /// configures (no user chooses where it connects).
    admin_configured: bool,
    /// Refuses every internal address whatever the process's policy allows (T28: push endpoints
    /// never reach networks an admin opened for mail servers).
    internet_only: bool,
}

impl Http {
    /// A client that judges certificates by `trust`.
    pub fn new(trust: Trust, timeout: Duration, deadline: Duration) -> Self {
        Self {
            trust,
            tls: OnceCell::new(),
            timeout,
            deadline,
            admin_configured: false,
            internet_only: false,
        }
    }

    /// The same, reaching public addresses only (see `internet_only`).
    pub fn internet_only(mut self) -> Self {
        self.internet_only = true;
        self
    }

    /// The same for an address the server's admin configured, which the address policy (meant for
    /// what users choose) does not restrict.
    pub fn admin_configured(mut self) -> Self {
        self.admin_configured = true;
        self
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
        let breaks = |t: &str| t.contains(['\r', '\n']);
        if req
            .extra_headers
            .iter()
            .any(|(n, v)| breaks(n) || breaks(v) || n.is_empty())
        {
            return Err(BackendError::Permanent("a header is malformed".into()).into());
        }
        let allowance = Duration::from_secs((req.body.len() / (256 * 1024)) as u64)
            .saturating_add(req.extra_time);
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
        // T26: the process's address policy (the Companion refuses its own network).
        let addrs = if self.internet_only {
            addrs
                .into_iter()
                .filter(|a| {
                    !crate::netpolicy::is_internal(a.ip()) && crate::netpolicy::allowed(a.ip())
                })
                .collect()
        } else if self.admin_configured {
            addrs
        } else {
            crate::netpolicy::filter(addrs)
        };
        if addrs.is_empty() {
            return Err(BackendError::Permanent(crate::netpolicy::REFUSED.into()));
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
        let max = if req.status_only { 0 } else { req.max_response };
        read_response(
            &mut stream,
            max,
            self.timeout,
            req.error_body && !req.status_only,
            req.status_only,
        )
        .await
    }
}

/// The request line and headers, in a buffer that is wiped when dropped (it holds the
/// `Authorization` value).
///
/// The buffer is allocated at its final size before anything is written to it. A `String` that
/// outgrows its capacity moves to a new block and frees the old one without wiping it, and the old
/// block would hold the credential, so the size is worked out first ([`head_capacity`]).
fn request_head(req: &HttpRequest<'_>) -> Zeroizing<String> {
    let host = req.url.host_header();
    let mut head = Zeroizing::new(String::with_capacity(head_capacity(req, &host)));
    head.push_str(req.method);
    head.push(' ');
    head.push_str(&req.url.path_and_query);
    head.push_str(" HTTP/1.1\r\nHost: ");
    head.push_str(&host);
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
    for (name, value) in req.extra_headers {
        head.push_str(name);
        head.push_str(": ");
        head.push_str(value);
        head.push_str("\r\n");
    }
    // Every request that carries a body says how long it is (T38a: WebDAV's `PROPFIND`, `REPORT`
    // and `PUT` too, not only `POST`).
    if req.method == "POST" || req.method == "PUT" || !req.body.is_empty() {
        // Written piecewise: `format!` would build the line in a separate, unwiped string.
        head.push_str("Content-Length: ");
        head.push_str(&req.body.len().to_string());
        head.push_str("\r\n");
    }
    head.push_str("\r\n");
    head
}

/// An upper bound for the size of [`request_head`], never below the real size.
fn head_capacity(req: &HttpRequest<'_>, host: &str) -> usize {
    // Fixed text: request line pieces, header names, separators and the blank line.
    const FIXED: usize = 256;
    // "Content-Length: " + at most 20 digits + CRLF.
    const LENGTH_LINE: usize = 40;
    FIXED
        + LENGTH_LINE
        + req.method.len()
        + req.url.path_and_query.len()
        + host.len()
        + req.authorization.map_or(0, str::len)
        + req.content_type.map_or(0, str::len)
        + req
            .extra_headers
            .iter()
            .map(|(n, v)| n.len() + v.len() + 4)
            .sum::<usize>()
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

/// Reads an HTTP/1.1 answer: headers, then (for 2xx, and for 4xx when `error_body`) a body of
/// at most `max_body` bytes.
pub(crate) async fn read_response<R: AsyncRead + Unpin>(
    reader: &mut R,
    max_body: usize,
    idle: Duration,
    error_body: bool,
    status_only: bool,
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
    let wanted = (200..300).contains(&status) || (error_body && (400..500).contains(&status));
    if status_only || !wanted || status == 204 {
        return Ok(HttpResponse {
            status,
            location,
            body: Vec::new(),
        });
    }

    let mut raw = buf.split_off(head_len);
    let body = if chunked {
        let cap = max_body.saturating_mul(2).saturating_add(MAX_HEAD);
        let mut decoder = ChunkedBody::new(max_body);
        loop {
            match decoder.feed(&raw)? {
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

/// Decodes a chunked body as bytes arrive. It keeps its place and the output between calls, so
/// each byte is looked at once however many reads the body is split into (decoding again from the
/// start after every read would take time quadratic in the body size).
struct ChunkedBody {
    /// Start of the first chunk not yet fully decoded, in the raw bytes.
    pos: usize,
    /// The decoded body so far.
    out: Vec<u8>,
    max: usize,
}

impl ChunkedBody {
    fn new(max: usize) -> Self {
        Self {
            pos: 0,
            out: Vec::new(),
            max,
        }
    }

    /// Decodes what `raw` (everything read so far, from the first byte of the body) holds beyond
    /// the last call. `Ok(Some(body))` when the final chunk has arrived, `Ok(None)` when more
    /// bytes are needed, an error when the framing is wrong or the decoded body would pass the
    /// maximum. Trailers after the last chunk are ignored.
    fn feed(&mut self, raw: &[u8]) -> Result<Option<Vec<u8>>, BackendError> {
        loop {
            let rest = &raw[self.pos.min(raw.len())..];
            // A size line is a few hex digits and an optional extension; a long one is abuse. Only
            // the first bytes are searched, so waiting for a big chunk costs nothing.
            let window = &rest[..rest.len().min(130)];
            let Some(eol) = window.windows(2).position(|w| w == b"\r\n") else {
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
            let start = eol + 2;
            if size == 0 {
                return Ok(Some(std::mem::take(&mut self.out)));
            }
            if self.out.len().saturating_add(size) > self.max {
                return Err(too_large());
            }
            let end = start + size;
            if rest.len() < end + 2 {
                // The chunk is not all here yet; start from its size line on the next call.
                return Ok(None);
            }
            if &rest[end..end + 2] != b"\r\n" {
                return Err(unreadable());
            }
            self.out.extend_from_slice(&rest[start..end]);
            self.pos += end + 2;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const IDLE: Duration = Duration::from_secs(2);

    async fn parse(bytes: &[u8], max: usize) -> Result<HttpResponse, BackendError> {
        let mut r = bytes;
        read_response(&mut r, max, IDLE, false, false).await
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

    /// A chunked body of `total` bytes in chunks of `chunk` bytes, ending with the last chunk.
    fn chunked(total: usize, chunk: usize) -> Vec<u8> {
        let mut out = Vec::with_capacity(total + total / chunk * 8 + 16);
        let mut left = total;
        while left > 0 {
            let n = left.min(chunk);
            out.extend_from_slice(format!("{n:x}\r\n").as_bytes());
            out.extend(std::iter::repeat_n(b'x', n));
            out.extend_from_slice(b"\r\n");
            left -= n;
        }
        out.extend_from_slice(b"0\r\n\r\n");
        out
    }

    #[test]
    fn a_big_chunked_body_in_small_chunks_decodes_in_linear_time() {
        const TOTAL: usize = 16 * 1024 * 1024;
        let wire = chunked(TOTAL, 1024);
        // Fed in reads the size `fill` uses, the way `read_response` does.
        let started = std::time::Instant::now();
        let mut raw: Vec<u8> = Vec::new();
        let mut decoder = ChunkedBody::new(TOTAL);
        let mut body = None;
        for piece in wire.chunks(8192) {
            raw.extend_from_slice(piece);
            if let Some(done) = decoder.feed(&raw).unwrap() {
                body = Some(done);
                break;
            }
        }
        let body = body.expect("the body ended");
        assert_eq!(body.len(), TOTAL);
        assert!(body.iter().all(|b| *b == b'x'));
        assert!(
            started.elapsed() < Duration::from_secs(1),
            "took {:?}",
            started.elapsed()
        );
    }

    #[tokio::test]
    async fn chunked_bodies_split_anywhere_still_decode_and_keep_their_limits() {
        // Every possible split point of a small body gives the same answer.
        let wire = chunked(40, 7);
        // (The final `0\r\n` is enough; the empty trailer line after it is not waited for.)
        for cut in 0..wire.len() - 2 {
            let mut raw = wire[..cut].to_vec();
            let mut d = ChunkedBody::new(100);
            assert_eq!(d.feed(&raw).unwrap(), None, "cut {cut}");
            raw.extend_from_slice(&wire[cut..]);
            assert_eq!(d.feed(&raw).unwrap().unwrap(), vec![b'x'; 40], "cut {cut}");
        }
        // The limit counts decoded bytes, and a bad frame is refused.
        let mut d = ChunkedBody::new(30);
        assert!(matches!(
            d.feed(&chunked(40, 7)),
            Err(BackendError::Permanent(_))
        ));
        let mut d = ChunkedBody::new(30);
        assert!(matches!(
            d.feed(b"3\r\nabcXX"),
            Err(BackendError::Transient(_))
        ));
        let mut d = ChunkedBody::new(30);
        assert!(d.feed(&[b'a'; 200]).is_err());
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
            extra_time: Duration::ZERO,
            error_body: false,
            status_only: false,
            extra_headers: &[("TTL", "3600"), ("Urgency", "normal")],
        };
        let head = request_head(&req);
        assert!(head.contains("TTL: 3600\r\nUrgency: normal\r\n"));
        assert!(head.len() <= head_capacity(&req, &req.url.host_header()));
        assert!(head.starts_with("POST /jmap/ HTTP/1.1\r\nHost: mail.example.org:8443\r\n"));
        assert!(head.contains("Authorization: Basic c2VjcmV0\r\n"));
        assert!(head.contains("Content-Length: 2\r\n"));
        assert!(head.ends_with("\r\n\r\n"));
    }

    #[test]
    fn a_long_credential_is_written_without_the_head_buffer_growing() {
        let url = Url::parse("https://mail.example.org/jmap/").unwrap();
        let auth = format!("Bearer {}", "t".repeat(2048));
        let req = HttpRequest {
            method: "POST",
            url: &url,
            authorization: Some(&auth),
            content_type: Some("application/json"),
            body: &[0u8; 123_456],
            max_response: 10,
            extra_time: Duration::ZERO,
            error_body: false,
            status_only: false,
            extra_headers: &[],
        };
        let head = request_head(&req);
        assert!(head.contains(&format!("\r\nAuthorization: {auth}\r\n")));
        assert!(head.contains("Content-Length: 123456\r\n"));
        assert!(head.ends_with("\r\n\r\n"));
        // Allocated once, at the bound, and the bound holds: it never had to grow.
        assert_eq!(head.capacity(), head_capacity(&req, &url.host_header()));
        assert!(head.len() <= head.capacity());
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
            extra_time: Duration::ZERO,
            error_body: false,
            status_only: false,
            extra_headers: &[],
        };
        let err = http.send(&req).await.unwrap_err();
        assert!(matches!(err.error, BackendError::Permanent(_)));
        assert!(!err.request_may_have_arrived);
    }
}
