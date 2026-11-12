// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The loopback redirect (RFC 8252 section 7.3): the core listens on `127.0.0.1` on a port the
//! system picks, the provider's page sends the browser to `http://127.0.0.1:<port>/?code=...`, and
//! the core reads that one request. The code never passes through another app or an Android
//! intent. Google allows only this form for the client type a person can register without the
//! app's signing key (a "Desktop app" client); Microsoft accepts it for "Mobile and desktop
//! applications".
//!
//! Only `127.0.0.1` is bound, so nothing off the device can connect. Each connection gets a tiny
//! fixed page; a request without the flow's `state` (a favicon, another tab) is answered and
//! ignored, so only the real redirect ends the wait.

use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use zeroize::Zeroizing;

use super::OAuthError;

/// Most bytes of one request read (the redirect is one line of a few hundred bytes).
const MAX_REQUEST: usize = 8 * 1024;
/// Each connection must send its request within this.
const READ_TIMEOUT: Duration = Duration::from_secs(10);

/// A listener on `127.0.0.1` for one flow's redirect.
pub struct Loopback {
    listener: TcpListener,
    port: u16,
}

impl std::fmt::Debug for Loopback {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Loopback")
            .field("port", &self.port)
            .finish()
    }
}

impl Loopback {
    /// Binds `127.0.0.1` on a free port.
    pub async fn bind() -> Result<Self, OAuthError> {
        let listener = TcpListener::bind(("127.0.0.1", 0))
            .await
            .map_err(|_| OAuthError::Refused("loopback".into()))?;
        let port = listener
            .local_addr()
            .map_err(|_| OAuthError::Refused("loopback".into()))?
            .port();
        Ok(Self { listener, port })
    }

    /// The port.
    pub fn port(&self) -> u16 {
        self.port
    }

    /// The redirect URI for this listener, as the provider must see it: `http://127.0.0.1:<port>`.
    pub fn redirect_uri(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    /// Waits for the request that carries `state` and returns it as a full URL (redirect URI
    /// plus the path and query), for [`super::parse_redirect`]. `page` is what the browser shows
    /// for it; other requests get an empty 404. Connections are read side by side (at most
    /// [`MAX_OPEN`] at once), so a connection that sends nothing cannot hold the real redirect up.
    pub async fn wait(&self, state: &str, page: &str) -> Result<Zeroizing<String>, OAuthError> {
        let mut open = tokio::task::JoinSet::new();
        loop {
            tokio::select! {
                accepted = self.listener.accept() => {
                    let Ok((stream, peer)) = accepted else {
                        // A connection reset before it was accepted, say: keep listening.
                        tokio::time::sleep(Duration::from_millis(50)).await;
                        continue;
                    };
                    if !peer.ip().is_loopback() || open.len() >= MAX_OPEN {
                        continue;
                    }
                    let (state, page) = (state.to_owned(), page.to_owned());
                    open.spawn(async move { answer(stream, &state, &page).await });
                }
                Some(done) = open.join_next(), if !open.is_empty() => {
                    if let Ok(Some(target)) = done {
                        return Ok(Zeroizing::new(format!("{}{}", self.redirect_uri(), target.as_str())));
                    }
                }
            }
        }
    }
}

/// Most connections read at once.
const MAX_OPEN: usize = 16;

/// Reads one connection's request and answers it: the page when it carries `state` (the
/// target is returned), an empty 404 otherwise.
async fn answer(
    mut stream: tokio::net::TcpStream,
    state: &str,
    page: &str,
) -> Option<Zeroizing<String>> {
    let target = tokio::time::timeout(READ_TIMEOUT, read_target(&mut stream))
        .await
        .ok()
        .flatten()?;
    let ours = target
        .split_once('?')
        .is_some_and(|(_, q)| q.split('&').any(|p| p == format!("state={state}")));
    let reply = if ours {
        format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nCache-Control: no-store\r\nReferrer-Policy: no-referrer\r\nConnection: close\r\n\r\n{}",
            page.len(),
            page
        )
    } else {
        "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_owned()
    };
    let _ = stream.write_all(reply.as_bytes()).await;
    let _ = stream.shutdown().await;
    ours.then_some(target)
}

/// The target of a `GET` request line (`/?code=...`), or `None` for anything else.
async fn read_target(stream: &mut tokio::net::TcpStream) -> Option<Zeroizing<String>> {
    let mut buf = Zeroizing::new(Vec::with_capacity(1024));
    let mut chunk = [0u8; 1024];
    while !buf.windows(2).any(|w| w == b"\r\n") {
        if buf.len() >= MAX_REQUEST {
            return None;
        }
        let n = stream.read(&mut chunk).await.ok()?;
        if n == 0 {
            return None;
        }
        buf.extend_from_slice(&chunk[..n]);
    }
    let line_end = buf.windows(2).position(|w| w == b"\r\n")?;
    let line = std::str::from_utf8(&buf[..line_end]).ok()?;
    let mut parts = line.split(' ');
    if parts.next()? != "GET" {
        return None;
    }
    let target = parts.next()?;
    if !target.starts_with('/') || !parts.next()?.starts_with("HTTP/1.") {
        return None;
    }
    Some(Zeroizing::new(target.to_owned()))
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use tokio::io::AsyncWriteExt;

    async fn get(port: u16, target: &str) -> String {
        let mut s = tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .unwrap();
        s.write_all(format!("GET {target} HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n").as_bytes())
            .await
            .unwrap();
        let mut out = String::new();
        s.read_to_string(&mut out).await.unwrap();
        out
    }

    #[tokio::test]
    async fn only_the_request_with_the_flows_state_ends_the_wait() {
        let lb = Loopback::bind().await.unwrap();
        let port = lb.port();
        assert_eq!(lb.redirect_uri(), format!("http://127.0.0.1:{port}"));
        let browser = tokio::spawn(async move {
            let favicon = get(port, "/favicon.ico").await;
            let other = get(port, "/?state=old&code=x").await;
            let real = get(port, "/?state=st4te&code=made-up").await;
            (favicon, other, real)
        });
        let url = lb.wait("st4te", "<p>Done</p>").await.unwrap();
        assert_eq!(
            &*url,
            &format!("http://127.0.0.1:{port}/?state=st4te&code=made-up")
        );
        let (favicon, other, real) = browser.await.unwrap();
        assert!(favicon.starts_with("HTTP/1.1 404"));
        assert!(other.starts_with("HTTP/1.1 404"));
        assert!(real.starts_with("HTTP/1.1 200") && real.ends_with("<p>Done</p>"));
        assert_eq!(
            super::super::parse_redirect(&url, &lb.redirect_uri(), "st4te")
                .unwrap()
                .as_str(),
            "made-up"
        );
    }

    #[tokio::test]
    async fn a_connection_that_sends_nothing_does_not_hold_up_the_real_redirect() {
        let lb = Loopback::bind().await.unwrap();
        let port = lb.port();
        // Another app opens connections and stays silent.
        let mut silent = Vec::new();
        for _ in 0..3 {
            silent.push(
                tokio::net::TcpStream::connect(("127.0.0.1", port))
                    .await
                    .unwrap(),
            );
        }
        let browser = tokio::spawn(async move { get(port, "/?state=s&code=c").await });
        let started = std::time::Instant::now();
        let url = lb.wait("s", "ok").await.unwrap();
        assert!(url.ends_with("/?state=s&code=c"));
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "{:?}",
            started.elapsed()
        );
        browser.await.unwrap();
        drop(silent);
    }

    #[tokio::test]
    async fn garbage_and_silent_connections_do_not_end_the_wait() {
        let lb = Loopback::bind().await.unwrap();
        let port = lb.port();
        let browser = tokio::spawn(async move {
            let mut s = tokio::net::TcpStream::connect(("127.0.0.1", port))
                .await
                .unwrap();
            s.write_all(b"POST / HTTP/1.1\r\n\r\n").await.unwrap();
            drop(s);
            let mut s = tokio::net::TcpStream::connect(("127.0.0.1", port))
                .await
                .unwrap();
            s.write_all(&[b'a'; 9000]).await.unwrap();
            drop(s);
            get(port, "/?code=c&state=s").await
        });
        let url = lb.wait("s", "ok").await.unwrap();
        assert!(url.ends_with("/?code=c&state=s"));
        browser.await.unwrap();
    }
}
