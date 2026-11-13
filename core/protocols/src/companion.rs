// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The Croft Companion's API for the apps (T34b): finding a Companion at an address, signing the
//! app in through the browser (the Companion's `/app/sign-in` page, a one-time code to the app's
//! redirect, redeemed with the app's PKCE verifier, T34a), and the app's own session: a 15-minute
//! access token renewed with a refresh token that the Companion replaces on every use.
//!
//! **The secret.** A session is kept by the app as an opaque blob ([`Session::secret`]), in the
//! platform keystore like an account's password. It holds the Companion's address and the tokens.
//! When the refresh token is replaced, the session hands the new blob to the app's saver; the old
//! one stops working (a refresh token used twice ends the session on the Companion).
//!
//! Tokens, codes and verifiers never reach a log or an error string.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::Arc;
use tokio::sync::{Mutex, OwnedMutexGuard};
use zeroize::{Zeroize, Zeroizing};

use crate::jmap::url::{is_loopback_name, Url};
use crate::oauth::fetch::{self, FetchError};
use crate::oauth::{random_token, Pkce};

/// The most an answer from the Companion may be.
const MAX_ANSWER: usize = 256 * 1024;
/// The access token is renewed when it has less than this left.
const RENEW_BEFORE_MS: i64 = 60_000;

/// Why a Companion call did not work. Fixed categories, never a token or an address.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompanionError {
    /// The address is not an `https` address of a host (with an optional port), or plain `http`
    /// to this machine.
    BadAddress,
    /// Something answers there, but it is not a Croft Companion.
    NotACompanion,
    /// No answer (offline, a wrong address, a certificate the device does not trust).
    Unreachable,
    /// The sign-in did not work (the code was spent, too late, or not for this app), or the person
    /// cancelled it.
    SignInFailed,
    /// The Companion ended this app's session (signed out from another device, or a refresh token
    /// came back twice): sign in again.
    SignedOut,
    /// Too many requests: try again in a while.
    RateLimited,
    /// The Companion refused or failed.
    Server,
}

impl std::fmt::Display for CompanionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::BadAddress => "not a Companion address",
            Self::NotACompanion => "not a Croft Companion",
            Self::Unreachable => "the Companion did not answer",
            Self::SignInFailed => "the sign-in did not work",
            Self::SignedOut => "signed out",
            Self::RateLimited => "too many requests",
            Self::Server => "the Companion refused",
        })
    }
}

impl std::error::Error for CompanionError {}

fn from_fetch(e: &FetchError) -> CompanionError {
    match e {
        FetchError::BadUrl => CompanionError::BadAddress,
        FetchError::Unreachable => CompanionError::Unreachable,
        _ => CompanionError::Server,
    }
}

/// The Companion's address as the app keeps it: `https://host[:port]` (no path; the web app and
/// API live at the root, T30), or `http://` to this machine in debug builds (tests). What the person typed may
/// leave out `https://` and end with `/`.
pub fn normalize(typed: &str) -> Result<String, CompanionError> {
    let t = typed.trim();
    if t.is_empty() || t.len() > 300 || t.chars().any(char::is_whitespace) {
        return Err(CompanionError::BadAddress);
    }
    let with_scheme = if t.starts_with("https://") || t.starts_with("http://") {
        t.to_owned()
    } else {
        format!("https://{t}")
    };
    let url = Url::parse(&with_scheme).ok_or(CompanionError::BadAddress)?;
    if url.path_and_query != "/" || with_scheme.contains('#') {
        return Err(CompanionError::BadAddress);
    }
    // Plain http only to this machine, and only in builds made for testing: in a release build
    // another app on the phone could be listening on a loopback port.
    if !url.secure && !(cfg!(debug_assertions) && is_loopback_name(&url.host)) {
        return Err(CompanionError::BadAddress);
    }
    let scheme = if url.secure { "https" } else { "http" };
    Ok(format!("{scheme}://{}", url.host_header()))
}

/// What a Companion says about itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Info {
    /// Its address, normalized.
    pub url: String,
    /// Its version (`/version`).
    pub version: String,
    /// The single sign-on provider's name, when it has one.
    pub sso_name: Option<String>,
}

/// Checks that a Croft Companion answers at `typed` (normalized first): its version, and whether
/// it offers single sign-on.
pub async fn probe(typed: &str) -> Result<Info, CompanionError> {
    let url = normalize(typed)?;
    let v = fetch::companion("GET", &format!("{url}/version"), None, None, 64)
        .await
        .map_err(|e| from_fetch(&e))?;
    let version = std::str::from_utf8(&v.body)
        .map(str::trim)
        .unwrap_or_default()
        .to_owned();
    let looks_like_version = !version.is_empty()
        && version.len() <= 32
        && version
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'+'))
        && version.bytes().next().is_some_and(|b| b.is_ascii_digit());
    if v.status != 200 || !looks_like_version {
        return Err(CompanionError::NotACompanion);
    }
    let sso_name = match fetch::companion("GET", &format!("{url}/api/oidc"), None, None, 4096).await
    {
        Ok(a) if a.status == 200 => serde_json::from_slice::<Value>(&a.body)
            .ok()
            .filter(|v| v["enabled"] == true)
            .map(|v| {
                v["name"]
                    .as_str()
                    .unwrap_or("single sign-on")
                    .chars()
                    .take(64)
                    .collect()
            }),
        _ => None,
    };
    Ok(Info {
        url,
        version,
        sso_name,
    })
}

/// A sign-in in progress: open [`SignIn::browser_url`] in a Custom Tab (or the platform's
/// equivalent), then hand the address the browser came back to to [`SignIn::finish`].
pub struct SignIn {
    url: String,
    redirect: String,
    pkce: Pkce,
}

impl std::fmt::Debug for SignIn {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SignIn")
    }
}

/// Percent-encodes a query value (everything but the unreserved characters).
fn encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
            out.push(char::from(b));
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

impl SignIn {
    /// Starts a sign-in to the Companion at `url` (normalized) for the app whose redirect is
    /// `redirect` (`<applicationId>:/companion`).
    pub fn begin(url: &str, redirect: &str) -> Result<Self, CompanionError> {
        let url = normalize(url)?;
        let verifier = random_token().map_err(|_| CompanionError::Server)?;
        Ok(Self {
            url,
            redirect: redirect.to_owned(),
            pkce: Pkce::from_verifier(&verifier),
        })
    }

    /// The Companion's page to open in the browser.
    pub fn browser_url(&self) -> String {
        format!(
            "{}/app/sign-in?app_challenge={}&app_redirect={}",
            self.url,
            encode(&self.pkce.challenge),
            encode(&self.redirect)
        )
    }

    /// The Companion's address.
    pub fn url(&self) -> &str {
        &self.url
    }

    /// Finishes the sign-in with the address the browser came back to (`<redirect>?code=...`):
    /// the code is redeemed with this sign-in's verifier for the app's session. `device` names it
    /// in the person's list of devices. [`CompanionError::SignInFailed`] leaves the sign-in usable:
    /// a code that does not belong to it (another app firing the redirect) must not spend it.
    pub async fn finish(&self, callback: &str, device: &str) -> Result<Session, CompanionError> {
        let query = callback
            .strip_prefix(&self.redirect)
            .and_then(|q| q.strip_prefix('?'))
            .ok_or(CompanionError::SignInFailed)?;
        let code = query
            .split('&')
            .find_map(|kv| kv.strip_prefix("code="))
            .filter(|c| {
                !c.is_empty()
                    && c.len() <= 128
                    && c.bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
            })
            .ok_or(CompanionError::SignInFailed)?;
        let device: String = device
            .chars()
            .filter(|c| !c.is_control())
            .take(64)
            .collect();
        #[derive(Serialize)]
        struct Exchange<'a> {
            code: &'a str,
            verifier: &'a str,
            device: &'a str,
        }
        let body = Zeroizing::new(
            serde_json::to_string(&Exchange {
                code,
                verifier: &self.pkce.verifier,
                device: &device,
            })
            .map_err(|_| CompanionError::Server)?,
        );
        let a = fetch::companion(
            "POST",
            &format!("{}/api/sign-in/app/exchange", self.url),
            None,
            Some(body.as_bytes()),
            MAX_ANSWER,
        )
        .await
        .map_err(|e| from_fetch(&e))?;
        match a.status {
            200 => {}
            401 | 400 => return Err(CompanionError::SignInFailed),
            429 => return Err(CompanionError::RateLimited),
            _ => return Err(CompanionError::Server),
        }
        let tokens = Tokens::from_answer(&self.url, &a.body).ok_or(CompanionError::Server)?;
        Ok(Session::new(tokens))
    }
}

/// The app's tokens with the Companion. Wiped when dropped.
#[derive(Serialize, Deserialize, Clone)]
struct Tokens {
    url: String,
    access_token: String,
    access_expires_ms: i64,
    refresh_token: String,
}

impl Drop for Tokens {
    fn drop(&mut self) {
        self.access_token.zeroize();
        self.refresh_token.zeroize();
    }
}

impl Tokens {
    fn from_answer(url: &str, body: &[u8]) -> Option<Self> {
        #[derive(Deserialize)]
        struct Answer {
            access_token: String,
            access_expires_ms: i64,
            refresh_token: String,
        }
        let mut a: Answer = serde_json::from_slice(body).ok()?;
        let t = Self {
            url: url.to_owned(),
            access_token: std::mem::take(&mut a.access_token),
            access_expires_ms: a.access_expires_ms,
            refresh_token: std::mem::take(&mut a.refresh_token),
        };
        Some(t)
    }
}

/// Called with the new secret each time the Companion replaces the refresh token: keep it in
/// place of the old one (the old one no longer works).
pub type SecretSaver = Box<dyn Fn(Zeroizing<Vec<u8>>) + Send + Sync>;

/// The app's session with its Companion. Calls are serialised, so a renewal is never made twice
/// with the same refresh token. Keep one per process: two sessions made from the same secret
/// would each spend the same refresh token, and the second spend ends the session (reuse).
pub struct Session {
    tokens: Arc<Mutex<Option<Tokens>>>,
    url: String,
    saver: Arc<std::sync::Mutex<Option<SecretSaver>>>,
    now: fn() -> i64,
}

impl std::fmt::Debug for Session {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Session")
    }
}

fn system_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}

/// One of the person's devices (`GET /api/sessions`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Device {
    /// The Companion's id for it.
    pub id: i64,
    /// Its name.
    pub name: String,
    /// When it signed in, Unix milliseconds.
    pub created_ms: i64,
    /// When it was last seen.
    pub last_seen_ms: i64,
    /// This app.
    pub current: bool,
}

/// Hands the new secret to the app's saver, if it set one.
fn save(saver: &std::sync::Mutex<Option<SecretSaver>>, t: &Tokens) {
    if let Ok(bytes) = serde_json::to_vec(t) {
        let bytes = Zeroizing::new(bytes);
        if let Some(saver) = saver
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .as_ref()
        {
            saver(bytes);
        }
    }
}

/// The renewal itself ([`Session::renew`] runs it as a task). Only a 401 means the session is over:
/// a 400 or a proxy's error page says nothing about the tokens.
async fn renew_in(
    url: &str,
    saver: &std::sync::Mutex<Option<SecretSaver>>,
    slot: &mut Option<Tokens>,
) -> Result<(), CompanionError> {
    #[derive(Serialize)]
    struct Refresh<'a> {
        refresh_token: &'a str,
    }
    let Some(t) = slot.as_ref() else {
        return Err(CompanionError::SignedOut);
    };
    let body = Zeroizing::new(
        serde_json::to_string(&Refresh {
            refresh_token: &t.refresh_token,
        })
        .map_err(|_| CompanionError::Server)?,
    );
    let a = fetch::companion(
        "POST",
        &format!("{url}/api/token/refresh"),
        None,
        Some(body.as_bytes()),
        MAX_ANSWER,
    )
    .await
    .map_err(|e| from_fetch(&e))?;
    match a.status {
        200 => {
            let new = Tokens::from_answer(url, &a.body).ok_or(CompanionError::Server)?;
            save(saver, &new);
            *slot = Some(new);
            Ok(())
        }
        401 => {
            *slot = None;
            Err(CompanionError::SignedOut)
        }
        429 => Err(CompanionError::RateLimited),
        _ => Err(CompanionError::Server),
    }
}

impl Session {
    fn new(tokens: Tokens) -> Self {
        Self {
            url: tokens.url.clone(),
            tokens: Arc::new(Mutex::new(Some(tokens))),
            saver: Arc::new(std::sync::Mutex::new(None)),
            now: system_now,
        }
    }

    /// A session from a secret the app kept ([`Session::secret`]); `None` when it is not one.
    pub fn from_secret(secret: &[u8]) -> Option<Self> {
        let t: Tokens = serde_json::from_slice(secret).ok()?;
        normalize(&t.url).ok().filter(|u| *u == t.url)?;
        Some(Self::new(t))
    }

    /// Where the new secret goes when the refresh token is replaced.
    pub fn set_saver(&self, saver: SecretSaver) {
        *self
            .saver
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(saver);
    }

    /// The Companion's address.
    pub fn url(&self) -> &str {
        &self.url
    }

    /// The secret to keep in the keystore (as it is now; it changes when the refresh token is
    /// replaced). Empty once the session has ended.
    pub async fn secret(&self) -> Zeroizing<Vec<u8>> {
        let t = self.tokens.lock().await;
        Zeroizing::new(
            t.as_ref()
                .and_then(|t| serde_json::to_vec(t).ok())
                .unwrap_or_default(),
        )
    }

    /// Renews the access token with the refresh token (replaced; the new secret is saved).
    ///
    /// The renewal runs as its own task and holds the token lock until it is done, so that a
    /// caller that gives up half way (a screen closed, a call cancelled) cannot lose the new
    /// tokens after the Companion has already spent the old refresh token. Only the wait is
    /// cancelled. Needs a Tokio runtime.
    async fn renew(
        &self,
        slot: OwnedMutexGuard<Option<Tokens>>,
    ) -> Result<OwnedMutexGuard<Option<Tokens>>, CompanionError> {
        let url = self.url.clone();
        let saver = Arc::clone(&self.saver);
        let task = tokio::spawn(async move {
            let mut slot = slot;
            let r = renew_in(&url, &saver, &mut slot).await;
            (slot, r)
        });
        let (slot, r) = task.await.map_err(|_| CompanionError::Server)?;
        r.map(|()| slot)
    }

    /// A call to the Companion's API (`path` from the root), renewing the access token first when
    /// it is due, and once more when it is refused. The answer's JSON.
    pub async fn call(
        &self,
        method: &'static str,
        path: &str,
        body: Option<Value>,
    ) -> Result<Value, CompanionError> {
        let mut slot = Arc::clone(&self.tokens).lock_owned().await;
        let due = slot
            .as_ref()
            .is_none_or(|t| t.access_expires_ms.saturating_sub(RENEW_BEFORE_MS) <= (self.now)());
        if due {
            slot = self.renew(slot).await?;
        }
        let body = body.map(|b| Zeroizing::new(b.to_string()));
        for attempt in 0..2 {
            let access = slot
                .as_ref()
                .map(|t| Zeroizing::new(t.access_token.clone()))
                .ok_or(CompanionError::SignedOut)?;
            let a = fetch::companion(
                method,
                &format!("{}{path}", self.url),
                Some(&access),
                body.as_ref().map(|b| b.as_bytes()),
                MAX_ANSWER,
            )
            .await
            .map_err(|e| from_fetch(&e))?;
            match a.status {
                200..=299 => {
                    return Ok(serde_json::from_slice(&a.body).unwrap_or(Value::Null));
                }
                401 if attempt == 0 => slot = self.renew(slot).await?,
                401 => return Err(CompanionError::SignedOut),
                429 => return Err(CompanionError::RateLimited),
                _ => return Err(CompanionError::Server),
            }
        }
        Err(CompanionError::Server)
    }

    /// Who is signed in: the person's name.
    pub async fn user_name(&self) -> Result<String, CompanionError> {
        let me = self.call("GET", "/api/me", None).await?;
        me["name"]
            .as_str()
            .map(|n| n.chars().filter(|c| !c.is_control()).take(64).collect())
            .ok_or(CompanionError::Server)
    }

    /// The person's signed-in devices.
    pub async fn devices(&self) -> Result<Vec<Device>, CompanionError> {
        let v = self.call("GET", "/api/sessions", None).await?;
        let list = v["sessions"].as_array().ok_or(CompanionError::Server)?;
        Ok(list
            .iter()
            .filter_map(|d| {
                Some(Device {
                    id: d["id"].as_i64()?,
                    name: d["device"]
                        .as_str()
                        .unwrap_or("")
                        .chars()
                        .take(64)
                        .collect(),
                    created_ms: d["created_ms"].as_i64().unwrap_or(0),
                    last_seen_ms: d["last_seen_ms"].as_i64().unwrap_or(0),
                    current: d["current"] == true,
                })
            })
            .collect())
    }

    /// Signs another device out.
    pub async fn sign_out_device(&self, id: i64) -> Result<(), CompanionError> {
        self.call("DELETE", &format!("/api/sessions/{id}"), None)
            .await
            .map(|_| ())
    }

    /// Signs this app out: the Companion ends its session (and its push subscriptions); the
    /// secret is of no more use. Done here even when the Companion cannot be reached.
    pub async fn sign_out(&self) -> Result<(), CompanionError> {
        let result = self.call("POST", "/api/sign-out", None).await.map(|_| ());
        *self.tokens.lock().await = None;
        result
    }

    /// The Companion's push key (VAPID, base64url): for UnifiedPush and Web Push subscriptions.
    pub async fn push_key(&self) -> Result<String, CompanionError> {
        let v = self.call("GET", "/api/push/vapid", None).await?;
        v["key"]
            .as_str()
            .map(str::to_owned)
            .ok_or(CompanionError::Server)
    }

    /// Gives a push subscription (an endpoint and its keys, base64url) to the Companion for this
    /// device: it pings it, content-free, when mail comes. Its id.
    pub async fn add_push(
        &self,
        endpoint: &str,
        p256dh: &str,
        auth: &str,
    ) -> Result<i64, CompanionError> {
        let v = self
            .call(
                "POST",
                "/api/push/subscriptions",
                Some(json!({ "endpoint": endpoint, "keys": { "p256dh": p256dh, "auth": auth } })),
            )
            .await?;
        v["id"].as_i64().ok_or(CompanionError::Server)
    }

    /// Removes a push subscription.
    pub async fn remove_push(&self, id: i64) -> Result<(), CompanionError> {
        self.call("DELETE", &format!("/api/push/subscriptions/{id}"), None)
            .await
            .map(|_| ())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn addresses_are_normalized_or_refused() {
        for (typed, want) in [
            ("companion.example.org", "https://companion.example.org"),
            (
                " https://Companion.Example.org/ ",
                "https://companion.example.org",
            ),
            (
                "https://mail.example.org:8443",
                "https://mail.example.org:8443",
            ),
            ("http://127.0.0.1:9000", "http://127.0.0.1:9000"),
            ("http://localhost:9000/", "http://localhost:9000"),
        ] {
            assert_eq!(normalize(typed).as_deref(), Ok(want), "{typed}");
        }
        for bad in [
            "",
            "http://companion.example.org",
            "https://companion.example.org/croft",
            "https://companion.example.org/?x=1",
            "https://user@companion.example.org",
            "https://companion example.org",
            "ftp://companion.example.org",
            "https://companion.example.org#x",
        ] {
            assert_eq!(normalize(bad), Err(CompanionError::BadAddress), "{bad}");
        }
    }

    #[test]
    fn the_browser_page_carries_the_challenge_and_redirect_only() {
        let s = SignIn::begin("companion.example.org", "com.example.croft:/companion").unwrap();
        let url = s.browser_url();
        assert!(url.starts_with("https://companion.example.org/app/sign-in?app_challenge="));
        assert!(
            url.ends_with("&app_redirect=com.example.croft%3A%2Fcompanion"),
            "{url}"
        );
        assert!(!url.contains(s.pkce.verifier.as_str()));
    }

    #[test]
    fn a_secret_reads_back_and_a_bad_one_is_refused() {
        let t = Tokens {
            url: "https://companion.example.org".into(),
            access_token: "made-up-access".into(),
            access_expires_ms: 1,
            refresh_token: "made-up-refresh".into(),
        };
        let bytes = serde_json::to_vec(&t).unwrap();
        let s = Session::from_secret(&bytes).unwrap();
        assert_eq!(s.url(), "https://companion.example.org");
        assert!(Session::from_secret(b"nonsense").is_none());
        // An address that is not a Companion's own form is refused.
        let mut bad = t.clone();
        bad.url = "https://companion.example.org/elsewhere".into();
        assert!(Session::from_secret(&serde_json::to_vec(&bad).unwrap()).is_none());
    }

    #[tokio::test]
    async fn a_callback_without_a_code_or_for_another_redirect_fails_before_any_request() {
        for cb in [
            "com.example.croft:/companion?error=provider",
            "com.evil.app:/companion?code=abc",
            "com.example.croft:/companion",
            "com.example.croft:/companion?code=",
            "com.example.croft:/companion?code=a%20b",
        ] {
            let s = SignIn::begin("http://127.0.0.1:9", "com.example.croft:/companion").unwrap();
            assert_eq!(
                s.finish(cb, "Test phone").await.unwrap_err(),
                CompanionError::SignInFailed,
                "{cb}"
            );
        }
    }

    /// A tiny Companion that answers one refresh slowly, then any call with 200. Counts refreshes.
    async fn slow_companion(
        delay: std::time::Duration,
        status: u16,
    ) -> (String, Arc<std::sync::atomic::AtomicUsize>) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let refreshes = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let count = Arc::clone(&refreshes);
        tokio::spawn(async move {
            loop {
                let Ok((mut sock, _)) = listener.accept().await else {
                    return;
                };
                let count = Arc::clone(&count);
                tokio::spawn(async move {
                    let mut buf = vec![0u8; 8192];
                    let n = sock.read(&mut buf).await.unwrap_or(0);
                    let head = String::from_utf8_lossy(&buf[..n]).into_owned();
                    let body = if head.starts_with("POST /api/token/refresh") {
                        count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                        tokio::time::sleep(delay).await;
                        r#"{"access_token":"made-up-access-2","access_expires_ms":9999999999999,"refresh_token":"made-up-refresh-2"}"#
                    } else {
                        r#"{"name":"Test person"}"#
                    };
                    let status = if head.starts_with("POST /api/token/refresh") {
                        status
                    } else {
                        200
                    };
                    let answer = format!(
                        "HTTP/1.1 {status} X\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                        body.len()
                    );
                    let _ = sock.write_all(answer.as_bytes()).await;
                });
            }
        });
        (url, refreshes)
    }

    fn due_session(url: &str) -> Session {
        let secret = format!(
            r#"{{"url":"{url}","access_token":"made-up-access-1","access_expires_ms":0,"refresh_token":"made-up-refresh-1"}}"#
        );
        Session::from_secret(secret.as_bytes()).unwrap()
    }

    #[tokio::test]
    async fn a_call_given_up_during_a_renewal_still_keeps_the_new_tokens() {
        let (url, refreshes) = slow_companion(std::time::Duration::from_millis(300), 200).await;
        let session = due_session(&url);
        let saved = Arc::new(std::sync::Mutex::new(Vec::<Vec<u8>>::new()));
        let into = Arc::clone(&saved);
        session.set_saver(Box::new(move |s| into.lock().unwrap().push(s.to_vec())));

        // The screen closes while the refresh is on its way: the call is dropped.
        let gave_up =
            tokio::time::timeout(std::time::Duration::from_millis(50), session.user_name()).await;
        assert!(gave_up.is_err());

        // The renewal finishes on its own and its tokens are saved and used.
        assert_eq!(session.user_name().await.unwrap(), "Test person");
        assert_eq!(
            refreshes.load(std::sync::atomic::Ordering::SeqCst),
            1,
            "renewed once"
        );
        let saved = saved.lock().unwrap().clone();
        assert_eq!(saved.len(), 1);
        assert!(String::from_utf8_lossy(&saved[0]).contains("made-up-refresh-2"));
        assert!(String::from_utf8_lossy(&session.secret().await).contains("made-up-refresh-2"));
    }

    #[tokio::test]
    async fn only_a_401_ends_the_session() {
        let (url, _) = slow_companion(std::time::Duration::ZERO, 400).await;
        let session = due_session(&url);
        assert_eq!(
            session.user_name().await.unwrap_err(),
            CompanionError::Server
        );
        assert!(!session.secret().await.is_empty(), "a 400 keeps the tokens");

        let (url, _) = slow_companion(std::time::Duration::ZERO, 401).await;
        let session = due_session(&url);
        assert_eq!(
            session.user_name().await.unwrap_err(),
            CompanionError::SignedOut
        );
        assert!(session.secret().await.is_empty(), "a 401 ends it");
    }

    #[tokio::test]
    async fn a_hostile_expiry_does_not_overflow() {
        let (url, refreshes) = slow_companion(std::time::Duration::ZERO, 200).await;
        let secret = format!(
            r#"{{"url":"{url}","access_token":"a","access_expires_ms":{},"refresh_token":"r"}}"#,
            i64::MIN
        );
        let s = Session::from_secret(secret.as_bytes()).unwrap();
        assert_eq!(s.user_name().await.unwrap(), "Test person");
        assert_eq!(refreshes.load(std::sync::atomic::Ordering::SeqCst), 1);
    }
}
