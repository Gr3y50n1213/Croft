// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! A fake OAuth token endpoint on `127.0.0.1` (T21): it exchanges known codes (checking the
//! PKCE verifier against the challenge the test registered), refreshes known refresh tokens,
//! rotates them when asked, answers `invalid_grant` for unknown or revoked ones, and can be made
//! slow. Every token here is made up.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

#[derive(Default)]
struct State {
    /// code -> (PKCE challenge, refresh token to give).
    codes: HashMap<String, (String, String)>,
    /// Refresh tokens that work.
    refresh: HashSet<String>,
    rotate: bool,
    slow: Option<Duration>,
    issued: u32,
    requests: Vec<String>,
}

/// The fake endpoint. Dropping it stops nothing (the task ends with the test's runtime).
#[derive(Clone)]
pub struct FakeTokenEndpoint {
    port: u16,
    st: Arc<Mutex<State>>,
}

impl FakeTokenEndpoint {
    /// Starts it.
    pub async fn start() -> Self {
        #[allow(clippy::expect_used)]
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.expect("bind");
        #[allow(clippy::expect_used)]
        let port = listener.local_addr().expect("addr").port();
        let st = Arc::new(Mutex::new(State::default()));
        let shared = st.clone();
        tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                let st = shared.clone();
                tokio::spawn(async move {
                    let _ = serve(stream, st).await;
                });
            }
        });
        Self { port, st }
    }

    fn st(&self) -> MutexGuard<'_, State> {
        self.st.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The token URL (`http://127.0.0.1:<port>/token`).
    pub fn url(&self) -> String {
        format!("http://127.0.0.1:{}/token", self.port)
    }

    /// `code` is valid once, for the verifier whose S256 challenge is `challenge` (as the
    /// authorisation URL carried it); it gives `refresh_token`.
    pub fn add_code(&self, code: &str, challenge: &str, refresh_token: &str) {
        self.st()
            .codes
            .insert(code.into(), (challenge.into(), refresh_token.into()));
    }

    /// `token` works as a refresh token.
    pub fn add_refresh_token(&self, token: &str) {
        self.st().refresh.insert(token.into());
    }

    /// Revokes a refresh token (the next refresh is `invalid_grant`).
    pub fn revoke(&self, token: &str) {
        self.st().refresh.remove(token);
    }

    /// Each refresh gives a new refresh token and revokes the old one.
    pub fn set_rotation(&self, on: bool) {
        self.st().rotate = on;
    }

    /// Answer only after `delay`.
    pub fn set_slow(&self, delay: Option<Duration>) {
        self.st().slow = delay;
    }

    /// The form bodies received, in order (tests check for a secret or a verifier).
    pub fn requests(&self) -> Vec<String> {
        self.st().requests.clone()
    }

    /// The access token the endpoint gave last (`access-<n>`), if any.
    pub fn last_access_token(&self) -> Option<String> {
        let n = self.st().issued;
        (n > 0).then(|| format!("access-{n}"))
    }
}

fn s256(verifier: &str) -> String {
    use base64::Engine as _;
    use sha2::Digest as _;
    base64::engine::general_purpose::URL_SAFE_NO_PAD
        .encode(sha2::Sha256::digest(verifier.as_bytes()))
}

fn form(body: &str) -> HashMap<String, String> {
    body.split('&')
        .filter_map(|p| p.split_once('='))
        .map(|(k, v)| (k.to_owned(), decode(v)))
        .collect()
}

fn decode(v: &str) -> String {
    let b = v.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let Ok(x) = u8::from_str_radix(&v[i + 1..i + 3], 16) {
                out.push(x);
                i += 3;
                continue;
            }
        }
        out.push(if b[i] == b'+' { b' ' } else { b[i] });
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

async fn serve(mut stream: tokio::net::TcpStream, st: Arc<Mutex<State>>) -> std::io::Result<()> {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    let (head_end, len) = loop {
        let n = stream.read(&mut chunk).await?;
        if n == 0 {
            return Ok(());
        }
        buf.extend_from_slice(&chunk[..n]);
        if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            let head = String::from_utf8_lossy(&buf[..pos]).to_ascii_lowercase();
            let len = head
                .lines()
                .find_map(|l| l.strip_prefix("content-length:"))
                .and_then(|v| v.trim().parse::<usize>().ok())
                .unwrap_or(0);
            break (pos + 4, len);
        }
    };
    while buf.len() < head_end + len {
        let n = stream.read(&mut chunk).await?;
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..n]);
    }
    let body = String::from_utf8_lossy(&buf[head_end..]).into_owned();
    let slow = st.lock().unwrap_or_else(PoisonError::into_inner).slow;
    if let Some(d) = slow {
        tokio::time::sleep(d).await;
    }
    let f = form(&body);
    let (status, json) = {
        let mut s = st.lock().unwrap_or_else(PoisonError::into_inner);
        s.requests.push(body.clone());
        match f.get("grant_type").map(String::as_str) {
            Some("authorization_code") => {
                let code = f.get("code").cloned().unwrap_or_default();
                match s.codes.remove(&code) {
                    Some((challenge, refresh))
                        if f.get("code_verifier").is_some_and(|v| s256(v) == challenge) =>
                    {
                        s.refresh.insert(refresh.clone());
                        s.issued += 1;
                        let n = s.issued;
                        (
                            200,
                            format!(
                                r#"{{"access_token":"access-{n}","expires_in":3600,"refresh_token":"{refresh}","token_type":"Bearer"}}"#
                            ),
                        )
                    }
                    _ => (400, r#"{"error":"invalid_grant"}"#.to_owned()),
                }
            }
            Some("refresh_token") => {
                let rt = f.get("refresh_token").cloned().unwrap_or_default();
                if s.refresh.contains(&rt) {
                    s.issued += 1;
                    let n = s.issued;
                    if s.rotate {
                        s.refresh.remove(&rt);
                        let next = format!("refresh-{n}");
                        s.refresh.insert(next.clone());
                        (
                            200,
                            format!(
                                r#"{{"access_token":"access-{n}","expires_in":3600,"refresh_token":"{next}"}}"#
                            ),
                        )
                    } else {
                        (
                            200,
                            format!(r#"{{"access_token":"access-{n}","expires_in":3600}}"#),
                        )
                    }
                } else {
                    (400, r#"{"error":"invalid_grant"}"#.to_owned())
                }
            }
            _ => (400, r#"{"error":"unsupported_grant_type"}"#.to_owned()),
        }
    };
    let reason = if status == 200 { "OK" } else { "Bad Request" };
    let reply = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{json}",
        json.len()
    );
    stream.write_all(reply.as_bytes()).await?;
    stream.shutdown().await
}
