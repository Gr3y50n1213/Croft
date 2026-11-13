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
use tokio::sync::Mutex;
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
/// API live at the root, T30), or `http://` to this machine (tests). What the person typed may
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
    if !url.secure && !is_loopback_name(&url.host) {
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
    let v = fetch::get(&format!("{url}/version"), 64)
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
    let sso_name = match fetch::get(&format!("{url}/api/oidc"), 4096).await {
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
    /// in the person's list of devices.
    pub async fn finish(self, callback: &str, device: &str) -> Result<Session, CompanionError> {
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
        let body = Zeroizing::new(
            json!({ "code": code, "verifier": self.pkce.verifier.as_str(), "device": device })
                .to_string(),
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
/// with the same refresh token.
pub struct Session {
    tokens: Mutex<Option<Tokens>>,
    url: String,
    saver: std::sync::Mutex<Option<SecretSaver>>,
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

impl Session {
    fn new(tokens: Tokens) -> Self {
        Self {
            url: tokens.url.clone(),
            tokens: Mutex::new(Some(tokens)),
            saver: std::sync::Mutex::new(None),
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

    fn save(&self, t: &Tokens) {
        if let Ok(bytes) = serde_json::to_vec(t) {
            if let Some(saver) = self
                .saver
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .as_ref()
            {
                saver(Zeroizing::new(bytes));
            }
        }
    }

    /// Renews the access token with the refresh token (replaced; the new secret is saved).
    async fn renew(&self, slot: &mut Option<Tokens>) -> Result<(), CompanionError> {
        let Some(t) = slot.as_ref() else {
            return Err(CompanionError::SignedOut);
        };
        let body = Zeroizing::new(json!({ "refresh_token": t.refresh_token }).to_string());
        let a = fetch::companion(
            "POST",
            &format!("{}/api/token/refresh", self.url),
            None,
            Some(body.as_bytes()),
            MAX_ANSWER,
        )
        .await
        .map_err(|e| from_fetch(&e))?;
        match a.status {
            200 => {
                let new = Tokens::from_answer(&self.url, &a.body).ok_or(CompanionError::Server)?;
                self.save(&new);
                *slot = Some(new);
                Ok(())
            }
            400 | 401 => {
                *slot = None;
                Err(CompanionError::SignedOut)
            }
            429 => Err(CompanionError::RateLimited),
            _ => Err(CompanionError::Server),
        }
    }

    /// A call to the Companion's API (`path` from the root), renewing the access token first when
    /// it is due, and once more when it is refused. The answer's JSON.
    pub async fn call(
        &self,
        method: &'static str,
        path: &str,
        body: Option<Value>,
    ) -> Result<Value, CompanionError> {
        let mut slot = self.tokens.lock().await;
        let due = slot
            .as_ref()
            .is_none_or(|t| t.access_expires_ms - RENEW_BEFORE_MS <= (self.now)());
        if due {
            self.renew(&mut slot).await?;
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
                401 if attempt == 0 => self.renew(&mut slot).await?,
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
            .map(str::to_owned)
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
}
