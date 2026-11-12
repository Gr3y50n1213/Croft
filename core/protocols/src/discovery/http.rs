// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The real [`HttpFetch`]: one HTTPS `GET` over `tokio-rustls`, written for exactly what
//! discovery needs and no more.
//!
//! Why not a general HTTP client: discovery reads two kinds of small, public files from the
//! person's own domain. A hand-sized client on the rustls and `ring` stack the IMAP adapter
//! already uses keeps the dependency tree small, makes every limit explicit, and cannot do
//! anything surprising (no redirects, no cookies, no proxies, no HTTP/2, no compression).
//!
//! The request carries only `Host`, a fixed `User-Agent: Croft`, an `Accept` list and
//! `Connection: close`. TLS is 1.2 or newer, the certificate must chain to the bundled or the
//! platform roots and match the host name, and there is no way to switch that off.

use super::net::{FetchError, FetchLimits, HttpFetch, HttpResponse, HttpsUrl, NetFuture, Resolver};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::OnceCell;
use tokio::time::timeout;
use tokio_rustls::rustls::crypto::ring;
use tokio_rustls::rustls::pki_types::{CertificateDer, ServerName};
use tokio_rustls::rustls::{self, ClientConfig, RootCertStore};
use tokio_rustls::TlsConnector;

/// Most bytes of status line and headers read.
const MAX_HEAD: usize = 16 * 1024;
/// Most addresses tried for one host.
const MAX_ADDRS: usize = 4;

/// HTTPS requests through a [`Resolver`] and rustls.
pub struct HttpsFetcher {
    resolver: Arc<dyn Resolver>,
    config: OnceCell<Arc<ClientConfig>>,
    extra_roots: Vec<CertificateDer<'static>>,
}

impl std::fmt::Debug for HttpsFetcher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("HttpsFetcher")
    }
}

impl HttpsFetcher {
    /// A fetcher that looks hosts up with `resolver` and trusts the bundled Mozilla roots plus
    /// the platform's.
    pub fn new(resolver: Arc<dyn Resolver>) -> Self {
        Self {
            resolver,
            config: OnceCell::new(),
            extra_roots: Vec::new(),
        }
    }

    /// Also trusts the given certificates. Only for tests that run a local HTTPS server.
    #[cfg(test)]
    pub(crate) fn with_extra_roots(
        resolver: Arc<dyn Resolver>,
        roots: Vec<CertificateDer<'static>>,
    ) -> Self {
        Self {
            resolver,
            config: OnceCell::new(),
            extra_roots: roots,
        }
    }

    async fn tls_config(&self) -> Result<Arc<ClientConfig>, FetchError> {
        self.config
            .get_or_try_init(|| async {
                let extra = self.extra_roots.clone();
                let store = tokio::task::spawn_blocking(move || {
                    let mut store = RootCertStore::empty();
                    store.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
                    let native = rustls_native_certs::load_native_certs();
                    let _ = store.add_parsable_certificates(native.certs);
                    let _ = store.add_parsable_certificates(extra);
                    store
                })
                .await
                .map_err(|_| FetchError::Tls)?;
                let provider = Arc::new(ring::default_provider());
                let config = ClientConfig::builder_with_provider(provider)
                    .with_protocol_versions(&[&rustls::version::TLS13, &rustls::version::TLS12])
                    .map_err(|_| FetchError::Tls)?
                    .with_root_certificates(store)
                    .with_no_client_auth();
                Ok(Arc::new(config))
            })
            .await
            .cloned()
    }

    async fn fetch(&self, url: &HttpsUrl, limits: FetchLimits) -> Result<HttpResponse, FetchError> {
        use super::net::ResolveError;
        let addrs = self
            .resolver
            .lookup_ip(url.host())
            .await
            .map_err(|e| match e {
                ResolveError::NotFound => FetchError::NameNotFound,
                _ => FetchError::Dns,
            })?;
        if addrs.is_empty() {
            return Err(FetchError::NameNotFound);
        }
        // T26: the process's address policy (the Companion refuses its own network).
        let addrs: Vec<_> = addrs
            .into_iter()
            .filter(|ip| crate::netpolicy::allowed(*ip))
            .collect();
        if addrs.is_empty() {
            return Err(FetchError::Connect);
        }
        let per_address = (limits.timeout / 3).max(Duration::from_millis(100));
        let mut tcp = None;
        for ip in addrs.iter().take(MAX_ADDRS) {
            if let Ok(Ok(s)) = timeout(per_address, TcpStream::connect((*ip, url.port()))).await {
                tcp = Some(s);
                break;
            }
        }
        let tcp = tcp.ok_or(FetchError::Connect)?;
        let _ = tcp.set_nodelay(true);

        let name = ServerName::try_from(url.host().to_string()).map_err(|_| FetchError::Tls)?;
        let connector = TlsConnector::from(self.tls_config().await?);
        let mut stream = connector.connect(name, tcp).await.map_err(|e| {
            if e.get_ref().is_some_and(|i| i.is::<rustls::Error>()) {
                FetchError::Tls
            } else {
                FetchError::Connect
            }
        })?;

        let request = format!(
            "GET {} HTTP/1.1\r\nHost: {}\r\nUser-Agent: Croft\r\n\
             Accept: application/xml, text/xml, application/json;q=0.9, */*;q=0.1\r\n\
             Accept-Encoding: identity\r\nConnection: close\r\n\r\n",
            url.path_and_query(),
            url.host()
        );
        stream
            .write_all(request.as_bytes())
            .await
            .map_err(|_| FetchError::Connect)?;
        read_response(&mut stream, limits.max_body).await
    }
}

impl HttpFetch for HttpsFetcher {
    fn get<'a>(
        &'a self,
        url: &'a HttpsUrl,
        limits: FetchLimits,
    ) -> NetFuture<'a, Result<HttpResponse, FetchError>> {
        Box::pin(async move {
            match timeout(limits.timeout, self.fetch(url, limits)).await {
                Ok(r) => r,
                Err(_) => Err(FetchError::TimedOut),
            }
        })
    }
}

/// Reads more bytes onto `buf`. `Ok(0)` is the end of the stream; a TLS stream that ends without
/// a close notice counts as the end too, because a truncated body fails to parse anyway.
async fn fill<R: AsyncReadExt + Unpin>(
    reader: &mut R,
    buf: &mut Vec<u8>,
) -> Result<usize, FetchError> {
    let mut chunk = [0u8; 4096];
    match reader.read(&mut chunk).await {
        Ok(n) => {
            buf.extend_from_slice(&chunk[..n]);
            Ok(n)
        }
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => Ok(0),
        Err(_) => Err(FetchError::Connect),
    }
}

/// Reads a response: headers, then (for 2xx) a body of at most `max_body` bytes.
pub(crate) async fn read_response<R: AsyncReadExt + Unpin>(
    reader: &mut R,
    max_body: usize,
) -> Result<HttpResponse, FetchError> {
    let mut buf = Vec::with_capacity(4096);
    let (head_len, status, location, content_length, chunked) = loop {
        let mut headers = [httparse::EMPTY_HEADER; 48];
        let mut response = httparse::Response::new(&mut headers);
        match response.parse(&buf) {
            Ok(httparse::Status::Complete(n)) => {
                let status = response.code.ok_or(FetchError::Protocol)?;
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
                if buf.len() >= MAX_HEAD || fill(reader, &mut buf).await? == 0 {
                    return Err(FetchError::Protocol);
                }
            }
            Err(_) => return Err(FetchError::Protocol),
        }
    };
    if !(200..600).contains(&status) {
        return Err(FetchError::Protocol);
    }
    if !(200..300).contains(&status) {
        // Redirects and errors: the body is of no use, so none is read.
        return Ok(HttpResponse {
            status,
            location,
            body: Vec::new(),
        });
    }

    let mut raw = buf.split_off(head_len);
    let body = if chunked {
        let cap = max_body.saturating_mul(4).saturating_add(MAX_HEAD);
        loop {
            match decode_chunked(&raw, max_body)? {
                Some(body) => break body,
                None => {
                    if raw.len() > cap || fill(reader, &mut raw).await? == 0 {
                        return Err(FetchError::Protocol);
                    }
                }
            }
        }
    } else if let Some(len) = content_length {
        if len > max_body {
            return Err(FetchError::TooLarge);
        }
        while raw.len() < len {
            if fill(reader, &mut raw).await? == 0 {
                return Err(FetchError::Protocol);
            }
        }
        raw.truncate(len);
        raw
    } else {
        while raw.len() <= max_body {
            if fill(reader, &mut raw).await? == 0 {
                break;
            }
        }
        if raw.len() > max_body {
            return Err(FetchError::TooLarge);
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
pub(crate) fn decode_chunked(raw: &[u8], max: usize) -> Result<Option<Vec<u8>>, FetchError> {
    let mut pos = 0usize;
    let mut out = Vec::new();
    loop {
        let rest = &raw[pos..];
        let Some(eol) = rest.windows(2).position(|w| w == b"\r\n") else {
            // A size line is a few hex digits and an optional extension; a long one is abuse.
            return if rest.len() > 128 {
                Err(FetchError::Protocol)
            } else {
                Ok(None)
            };
        };
        let line = std::str::from_utf8(&rest[..eol]).map_err(|_| FetchError::Protocol)?;
        let digits = line.split(';').next().unwrap_or("").trim();
        if digits.is_empty() || digits.len() > 8 {
            return Err(FetchError::Protocol);
        }
        let size = usize::from_str_radix(digits, 16).map_err(|_| FetchError::Protocol)?;
        pos += eol + 2;
        if size == 0 {
            return Ok(Some(out));
        }
        if out.len().saturating_add(size) > max {
            return Err(FetchError::TooLarge);
        }
        let end = pos + size;
        if raw.len() < end + 2 {
            return Ok(None);
        }
        if &raw[end..end + 2] != b"\r\n" {
            return Err(FetchError::Protocol);
        }
        out.extend_from_slice(&raw[pos..end]);
        pos = end + 2;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::discovery::testing::FakeResolver;
    use std::net::IpAddr;
    use tokio::net::TcpListener;
    use tokio_rustls::rustls::pki_types::{PrivateKeyDer, PrivatePkcs8KeyDer};
    use tokio_rustls::TlsAcceptor;

    const LIMITS: FetchLimits = FetchLimits {
        timeout: Duration::from_secs(5),
        max_body: 1024,
    };

    #[test]
    fn chunked_bodies_decode_and_wait_for_more() {
        let raw = b"5\r\nhello\r\n6;ext=1\r\n world\r\n0\r\n\r\n";
        assert_eq!(decode_chunked(raw, 100).unwrap().unwrap(), b"hello world");
        for cut in 0..raw.len() - 5 {
            assert_eq!(decode_chunked(&raw[..cut], 100).unwrap(), None, "cut {cut}");
        }
        assert_eq!(decode_chunked(b"0\r\n", 100).unwrap().unwrap(), b"");
    }

    #[test]
    fn bad_chunk_framing_is_an_error() {
        assert_eq!(
            decode_chunked(b"zz\r\nabc\r\n", 100),
            Err(FetchError::Protocol)
        );
        assert_eq!(
            decode_chunked(b"3\r\nabcXX", 100),
            Err(FetchError::Protocol)
        );
        assert_eq!(decode_chunked(b"\r\n", 100), Err(FetchError::Protocol));
        assert_eq!(
            decode_chunked(b"123456789\r\n", 100),
            Err(FetchError::Protocol)
        );
        assert_eq!(decode_chunked(&[b'a'; 200], 100), Err(FetchError::Protocol));
        assert_eq!(
            decode_chunked(b"ffffffff\r\n", 100),
            Err(FetchError::TooLarge)
        );
        assert_eq!(
            decode_chunked(b"4\r\nabcd\r\n4\r\nefgh\r\n", 6),
            Err(FetchError::TooLarge)
        );
    }

    async fn parse(bytes: &[u8], max: usize) -> Result<HttpResponse, FetchError> {
        let mut reader = bytes;
        read_response(&mut reader, max).await
    }

    #[tokio::test]
    async fn reads_content_length_chunked_and_close_delimited_bodies() {
        let ok = parse(
            b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nhelloEXTRA",
            100,
        )
        .await
        .unwrap();
        assert_eq!((ok.status, ok.body.as_slice()), (200, &b"hello"[..]));
        let ch = parse(
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n3\r\nabc\r\n0\r\n\r\n",
            100,
        )
        .await
        .unwrap();
        assert_eq!(ch.body, b"abc");
        let eof = parse(b"HTTP/1.0 200 OK\r\n\r\nuntil the end", 100)
            .await
            .unwrap();
        assert_eq!(eof.body, b"until the end");
    }

    #[tokio::test]
    async fn redirects_and_errors_carry_no_body() {
        let r = parse(b"HTTP/1.1 301 Moved\r\nLocation: https://example.org/x\r\nContent-Length: 3\r\n\r\nabc", 100)
            .await
            .unwrap();
        assert_eq!(r.status, 301);
        assert_eq!(r.location.as_deref(), Some("https://example.org/x"));
        assert!(r.body.is_empty());
        let nf = parse(b"HTTP/1.1 404 Not Found\r\n\r\n<html>", 100)
            .await
            .unwrap();
        assert_eq!(nf.status, 404);
        assert!(nf.body.is_empty());
    }

    #[tokio::test]
    async fn oversized_truncated_and_garbled_replies_are_errors() {
        assert_eq!(
            parse(b"HTTP/1.1 200 OK\r\nContent-Length: 101\r\n\r\n", 100).await,
            Err(FetchError::TooLarge)
        );
        let long = format!("HTTP/1.1 200 OK\r\n\r\n{}", "x".repeat(200));
        assert_eq!(parse(long.as_bytes(), 100).await, Err(FetchError::TooLarge));
        assert_eq!(
            parse(b"HTTP/1.1 200 OK\r\nContent-Length: 10\r\n\r\nshort", 100).await,
            Err(FetchError::Protocol)
        );
        assert_eq!(
            parse(b"HTTP/1.1 200 OK\r\nContent-Le", 100).await,
            Err(FetchError::Protocol)
        );
        assert_eq!(
            parse(b"not http at all\r\n\r\n", 100).await,
            Err(FetchError::Protocol)
        );
        assert_eq!(parse(b"", 100).await, Err(FetchError::Protocol));
        assert_eq!(
            parse(b"HTTP/1.1 101 Switching\r\n\r\n", 100).await,
            Err(FetchError::Protocol)
        );
        let endless_head = format!("HTTP/1.1 200 OK\r\nX: {}", "y".repeat(MAX_HEAD + 100));
        assert_eq!(
            parse(endless_head.as_bytes(), 100).await,
            Err(FetchError::Protocol)
        );
    }

    type Seen = Arc<std::sync::Mutex<Vec<String>>>;

    /// Starts a local HTTPS server with a fresh certificate for `host` that answers every
    /// connection with `reply` and records each request it received.
    async fn serve(
        host: &str,
        reply: Vec<u8>,
    ) -> (
        u16,
        CertificateDer<'static>,
        Seen,
        tokio::task::JoinHandle<()>,
    ) {
        let ck = rcgen::generate_simple_self_signed(vec![host.to_string()]).unwrap();
        let der = CertificateDer::from(ck.cert.der().to_vec());
        let key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(ck.signing_key.serialize_der()));
        let cfg = rustls::ServerConfig::builder_with_provider(Arc::new(ring::default_provider()))
            .with_protocol_versions(&[&rustls::version::TLS13, &rustls::version::TLS12])
            .unwrap()
            .with_no_client_auth()
            .with_single_cert(vec![der.clone()], key)
            .unwrap();
        let acceptor = TlsAcceptor::from(Arc::new(cfg));
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let seen: Seen = Arc::default();
        let log = seen.clone();
        let task = tokio::spawn(async move {
            while let Ok((tcp, _)) = listener.accept().await {
                let acceptor = acceptor.clone();
                let reply = reply.clone();
                let log = log.clone();
                tokio::spawn(async move {
                    let Ok(mut tls) = acceptor.accept(tcp).await else {
                        return;
                    };
                    let mut req = vec![0u8; 4096];
                    let n = tls.read(&mut req).await.unwrap_or(0);
                    log.lock()
                        .unwrap()
                        .push(String::from_utf8_lossy(&req[..n]).to_string());
                    let _ = tls.write_all(&reply).await;
                    let _ = tls.shutdown().await;
                });
            }
        });
        (port, der, seen, task)
    }

    fn resolver_for(host: &str) -> Arc<FakeResolver> {
        let r = FakeResolver::new();
        r.set_ips(host, vec![IpAddr::from([127, 0, 0, 1])]);
        Arc::new(r)
    }

    #[tokio::test]
    async fn fetches_over_tls_with_a_trusted_certificate() {
        let host = "autoconfig.fetch.test";
        let (port, der, _seen, _task) = serve(
            host,
            b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\nConnection: close\r\n\r\ndata".to_vec(),
        )
        .await;
        let fetcher = HttpsFetcher::with_extra_roots(resolver_for(host), vec![der]);
        let url = HttpsUrl::with_port(host, port, "/mail/config-v1.1.xml").unwrap();
        let resp = fetcher.get(&url, LIMITS).await.unwrap();
        assert_eq!(resp.status, 200);
        assert_eq!(resp.body, b"data");
    }

    #[tokio::test]
    async fn an_untrusted_certificate_is_a_tls_error_and_nothing_is_read() {
        let host = "autoconfig.fetch.test";
        let (port, _der, _seen, _task) = serve(
            host,
            b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\n\r\ndata".to_vec(),
        )
        .await;
        // No extra root: the self-signed certificate must be refused.
        let fetcher = HttpsFetcher::new(resolver_for(host));
        let url = HttpsUrl::with_port(host, port, "/").unwrap();
        assert_eq!(fetcher.get(&url, LIMITS).await, Err(FetchError::Tls));
    }

    #[tokio::test]
    async fn a_certificate_for_another_name_is_refused() {
        let (port, der, _seen, _task) = serve(
            "other.fetch.test",
            b"HTTP/1.1 200 OK\r\nContent-Length: 1\r\n\r\nx".to_vec(),
        )
        .await;
        let host = "autoconfig.fetch.test";
        let fetcher = HttpsFetcher::with_extra_roots(resolver_for(host), vec![der]);
        let url = HttpsUrl::with_port(host, port, "/").unwrap();
        assert_eq!(fetcher.get(&url, LIMITS).await, Err(FetchError::Tls));
    }

    #[tokio::test]
    async fn the_request_is_a_bare_get_for_the_path_and_nothing_personal() {
        let host = "autoconfig.fetch.test";
        let (port, der, seen, _task) = serve(
            host,
            b"HTTP/1.1 301 Moved\r\nLocation: https://fetch.test/x\r\n\r\n".to_vec(),
        )
        .await;
        let fetcher = HttpsFetcher::with_extra_roots(resolver_for(host), vec![der]);
        let url = HttpsUrl::with_port(host, port, "/mail/config-v1.1.xml").unwrap();
        let resp = fetcher.get(&url, LIMITS).await.unwrap();
        assert_eq!(resp.status, 301);
        assert_eq!(resp.location.as_deref(), Some("https://fetch.test/x"));
        let request = seen.lock().unwrap().remove(0);
        let lines: Vec<&str> = request.split("\r\n").filter(|l| !l.is_empty()).collect();
        assert_eq!(lines[0], "GET /mail/config-v1.1.xml HTTP/1.1");
        assert_eq!(lines[1], format!("Host: {host}"));
        let names: Vec<String> = lines[1..]
            .iter()
            .map(|l| l.split(':').next().unwrap().to_ascii_lowercase())
            .collect();
        // Only these headers: no cookies, no authorization, nothing that names the person.
        assert_eq!(
            names,
            [
                "host",
                "user-agent",
                "accept",
                "accept-encoding",
                "connection"
            ]
        );
    }

    #[tokio::test]
    async fn an_unknown_host_and_a_closed_port_are_distinct_errors() {
        let fetcher = HttpsFetcher::new(Arc::new(FakeResolver::new()));
        let url = HttpsUrl::new("nothing.fetch.test", "/").unwrap();
        assert_eq!(
            fetcher.get(&url, LIMITS).await,
            Err(FetchError::NameNotFound)
        );
        // A port with nothing listening.
        let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let closed = l.local_addr().unwrap().port();
        drop(l);
        let fetcher = HttpsFetcher::new(resolver_for("closed.fetch.test"));
        let url = HttpsUrl::with_port("closed.fetch.test", closed, "/").unwrap();
        assert_eq!(fetcher.get(&url, LIMITS).await, Err(FetchError::Connect));
    }

    #[tokio::test]
    async fn a_server_that_never_answers_times_out() {
        let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = l.local_addr().unwrap().port();
        let _hold = tokio::spawn(async move {
            let mut keep = Vec::new();
            while let Ok((s, _)) = l.accept().await {
                keep.push(s); // accept and say nothing, so the TLS handshake never starts
            }
        });
        let fetcher = HttpsFetcher::new(resolver_for("slow.fetch.test"));
        let url = HttpsUrl::with_port("slow.fetch.test", port, "/").unwrap();
        let limits = FetchLimits {
            timeout: Duration::from_millis(300),
            max_body: 100,
        };
        assert_eq!(fetcher.get(&url, limits).await, Err(FetchError::TimedOut));
    }
}
