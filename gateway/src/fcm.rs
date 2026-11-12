// This Source Code Form is subject to the terms of the GNU Affero General Public License,
// version 3 or later. See LICENSE-AGPL-3.0 at the repository root.

//! Sending to FCM (HTTP v1 API) as the app's publisher: a Google service account signs a JWT
//! (RS256), which Google's token endpoint exchanges for an access token. One renewal runs at a time,
//! from 5 minutes before the token expires; meanwhile the still-valid token is used. Requests go
//! through the core's HTTPS client, to Google's hosts only (plain http on this machine in tests).

use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use base64::Engine as _;
use croft_protocols::oauth::fetch;
use std::sync::Arc;

use ring::rand::SystemRandom;
use ring::signature::{RsaKeyPair, RSA_PKCS1_SHA256};
use serde::Deserialize;
use zeroize::Zeroizing;

/// What the access token may do.
pub const SCOPE: &str = "https://www.googleapis.com/auth/firebase.messaging";
/// FCM's address.
pub const FCM_BASE: &str = "https://fcm.googleapis.com";
/// Google's token endpoint (the one a service account's `token_uri` must name).
pub const GOOGLE_TOKEN_URI: &str = "https://oauth2.googleapis.com/token";
/// How long a token request may take before sending gives up on it.
const TOKEN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);
/// After a failed token request, how long before another is tried.
const TOKEN_BACKOFF_MS: i64 = 5_000;
/// The JWT's `iat` is this far in the past, for a gateway clock that runs a little fast.
const CLOCK_SKEW_S: i64 = 60;
/// Requests to FCM at once; more wait (up to [`SLOT_WAIT`]).
const MAX_CALLS: usize = 64;
/// How long a message waits for a free slot before it is answered "busy".
const SLOT_WAIT: std::time::Duration = std::time::Duration::from_secs(5);
/// How long a signed JWT is valid (Google's maximum).
const JWT_LIFETIME_S: i64 = 3600;
/// An access token is renewed this long before it expires.
const RENEW_BEFORE_MS: i64 = 5 * 60 * 1000;
/// The largest answer read from Google.
const MAX_ANSWER: usize = 16 * 1024;

/// Why the service account could not be used. Carries nothing from the file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccountError {
    /// Not a service account JSON file.
    Malformed,
    /// The private key is not an RSA key in PKCS#8 PEM.
    Key,
    /// The project id or the token endpoint is not usable.
    Settings,
}

impl std::fmt::Display for AccountError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Malformed => "the service account file is not a Google service account key",
            Self::Key => "the service account's private key is not an RSA key in PKCS#8 PEM",
            Self::Settings => "the service account's project id or token endpoint is not usable",
        })
    }
}

impl std::error::Error for AccountError {}

#[derive(Deserialize)]
struct AccountFile {
    client_email: String,
    private_key: Zeroizing<String>,
    token_uri: String,
    project_id: String,
}

/// A Google service account allowed to send through FCM.
pub struct ServiceAccount {
    client_email: String,
    key: RsaKeyPair,
    token_uri: String,
    project_id: String,
}

impl std::fmt::Debug for ServiceAccount {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ServiceAccount")
            .field("project_id", &self.project_id)
            .finish_non_exhaustive()
    }
}

impl ServiceAccount {
    /// The account from its JSON key file.
    pub fn from_json(bytes: &[u8]) -> Result<Self, AccountError> {
        let file: AccountFile =
            serde_json::from_slice(bytes).map_err(|_| AccountError::Malformed)?;
        let der = pem_body(&file.private_key).ok_or(AccountError::Key)?;
        let key = RsaKeyPair::from_pkcs8(&der).map_err(|_| AccountError::Key)?;
        let project_ok = !file.project_id.is_empty()
            && file.project_id.len() <= 64
            && file
                .project_id
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
        let email_ok = !file.client_email.is_empty()
            && file.client_email.len() <= 256
            && file.client_email.bytes().all(|b| b.is_ascii_graphic());
        // Only Google's token endpoint (or a fake one on this machine in tests).
        let uri_ok = file.token_uri == GOOGLE_TOKEN_URI || fetch::is_loopback_http(&file.token_uri);
        if !project_ok || !email_ok || !uri_ok {
            return Err(AccountError::Settings);
        }
        Ok(Self {
            client_email: file.client_email,
            key,
            token_uri: file.token_uri,
            project_id: file.project_id,
        })
    }

    /// The project the app's FCM belongs to.
    pub fn project_id(&self) -> &str {
        &self.project_id
    }

    /// A signed JWT asking for an access token at `now_s`.
    fn assertion(&self, now_s: i64) -> Option<Zeroizing<String>> {
        let header = URL_SAFE_NO_PAD.encode(br#"{"alg":"RS256","typ":"JWT"}"#);
        let iat = now_s - CLOCK_SKEW_S;
        let claims = serde_json::json!({
            "iss": self.client_email,
            "scope": SCOPE,
            "aud": self.token_uri,
            "iat": iat,
            "exp": iat + JWT_LIFETIME_S,
        });
        let claims = URL_SAFE_NO_PAD.encode(serde_json::to_vec(&claims).ok()?);
        let modulus = self.key.public().modulus_len();
        // The whole assertion is a bearer grant for an hour: built in one wiped buffer.
        let mut out = Zeroizing::new(String::with_capacity(
            header.len() + claims.len() + 2 + modulus.div_ceil(3) * 4,
        ));
        out.push_str(&header);
        out.push('.');
        out.push_str(&claims);
        let mut signature = Zeroizing::new(vec![0u8; modulus]);
        self.key
            .sign(
                &RSA_PKCS1_SHA256,
                &SystemRandom::new(),
                out.as_bytes(),
                &mut signature,
            )
            .ok()?;
        out.push('.');
        let mut encoded = Zeroizing::new(vec![0u8; modulus.div_ceil(3) * 4]);
        let n = URL_SAFE_NO_PAD
            .encode_slice(&signature[..], &mut encoded[..])
            .ok()?;
        out.push_str(std::str::from_utf8(&encoded[..n]).ok()?);
        Some(out)
    }
}

/// The DER inside a PEM block (`-----BEGIN PRIVATE KEY-----` ... `-----END PRIVATE KEY-----`).
fn pem_body(pem: &str) -> Option<Zeroizing<Vec<u8>>> {
    let start = pem.find("-----BEGIN PRIVATE KEY-----")? + "-----BEGIN PRIVATE KEY-----".len();
    let end = pem[start..].find("-----END PRIVATE KEY-----")? + start;
    let mut body = Zeroizing::new(String::with_capacity(end - start));
    body.extend(pem[start..end].chars().filter(|c| !c.is_ascii_whitespace()));
    STANDARD.decode(body.as_bytes()).ok().map(Zeroizing::new)
}

/// What became of a message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Delivery {
    /// FCM took it.
    Sent,
    /// The token is no more, or belongs to another app: the endpoint will never work again.
    Gone,
    /// FCM could not be reached, refused for another reason, or no access token was had.
    Failed,
    /// Too many messages to FCM at once: try later.
    Busy,
}

/// One message for a device.
#[derive(Clone, Copy)]
pub struct Message<'a> {
    /// The device's FCM registration token.
    pub token: &'a str,
    /// The Web Push message's body, as the Companion sent it (encrypted for the device).
    pub body: &'a [u8],
    /// `Urgency: high`.
    pub high: bool,
    /// Seconds FCM keeps it for a device that is offline.
    pub ttl_s: u32,
}

impl std::fmt::Debug for Message<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Never the token or the body.
        f.debug_struct("Message")
            .field("high", &self.high)
            .field("ttl_s", &self.ttl_s)
            .finish_non_exhaustive()
    }
}

#[derive(Clone)]
struct Cached {
    token: Arc<Zeroizing<String>>,
    /// From here on a renewal is started.
    renew_at: i64,
    /// From here on the token is not used.
    expires_at: i64,
}

#[derive(Default)]
struct TokenState {
    cached: Option<Cached>,
    /// No token request before this (one just failed).
    retry_at: i64,
}

/// FCM, for one service account.
pub struct Fcm {
    account: ServiceAccount,
    base: String,
    state: std::sync::Mutex<TokenState>,
    /// One token request at a time.
    renewing: tokio::sync::Mutex<()>,
    calls: tokio::sync::Semaphore,
}

impl std::fmt::Debug for Fcm {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Fcm")
    }
}

impl Fcm {
    /// FCM at `base` (normally [`FCM_BASE`]; no trailing slash).
    pub fn new(account: ServiceAccount, base: &str) -> Self {
        Self {
            account,
            base: base.trim_end_matches('/').to_owned(),
            state: std::sync::Mutex::new(TokenState::default()),
            renewing: tokio::sync::Mutex::new(()),
            calls: tokio::sync::Semaphore::new(MAX_CALLS),
        }
    }

    fn state(&self) -> std::sync::MutexGuard<'_, TokenState> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// The kept token, if it is still valid and is not `rejected` (a token Google just refused).
    fn usable(&self, now: i64, rejected: Option<&Arc<Zeroizing<String>>>) -> Option<Cached> {
        self.state()
            .cached
            .clone()
            .filter(|c| now < c.expires_at && rejected.is_none_or(|r| !Arc::ptr_eq(r, &c.token)))
    }

    /// An access token: the kept one while it is valid (and not `rejected`). From its renewal time
    /// one caller renews it while the others keep using it. Without a usable token, callers wait
    /// for the one renewal in progress. After a failed request none is made for a few seconds.
    async fn access_token(
        &self,
        rejected: Option<&Arc<Zeroizing<String>>>,
    ) -> Option<Arc<Zeroizing<String>>> {
        let now = now_ms();
        if let Some(c) = self.usable(now, rejected) {
            if now < c.renew_at {
                return Some(c.token);
            }
            // Time to renew: whoever gets the lock does it; everyone else uses the valid token.
            if let Ok(guard) = self.renewing.try_lock() {
                return self.renew(guard, rejected).await.or(Some(c.token));
            }
            return Some(c.token);
        }
        let guard = self.renewing.lock().await;
        // Someone else may have renewed it meanwhile.
        if let Some(c) = self.usable(now_ms(), rejected) {
            return Some(c.token);
        }
        self.renew(guard, rejected).await
    }

    /// Asks Google for a token (holding the renewal lock). A failure keeps a token that is still
    /// valid, unless Google refused that one.
    async fn renew(
        &self,
        _guard: tokio::sync::MutexGuard<'_, ()>,
        rejected: Option<&Arc<Zeroizing<String>>>,
    ) -> Option<Arc<Zeroizing<String>>> {
        let now = now_ms();
        if now < self.state().retry_at {
            return None;
        }
        let fetched = tokio::time::timeout(TOKEN_TIMEOUT, self.request_token(now)).await;
        let Ok(Some((token, expires_in))) = fetched else {
            let mut st = self.state();
            st.retry_at = now_ms() + TOKEN_BACKOFF_MS;
            if let (Some(r), Some(c)) = (rejected, st.cached.as_ref()) {
                if Arc::ptr_eq(r, &c.token) {
                    st.cached = None;
                }
            }
            return None;
        };
        let life = expires_in.clamp(60, 24 * 3600) * 1000;
        let token = Arc::new(token);
        self.state().cached = Some(Cached {
            token: token.clone(),
            renew_at: now + life - RENEW_BEFORE_MS.min(life / 2),
            expires_at: now + life,
        });
        Some(token)
    }

    /// One token request: the token and its life in seconds.
    async fn request_token(&self, now: i64) -> Option<(Zeroizing<String>, i64)> {
        let assertion = self.account.assertion(now / 1000)?;
        let form = fetch::form(&[
            ("grant_type", "urn:ietf:params:oauth:grant-type:jwt-bearer"),
            ("assertion", assertion.as_str()),
        ]);
        let answer = fetch::post_form(&self.account.token_uri, &form, None, MAX_ANSWER)
            .await
            .ok()?;
        if answer.status != 200 {
            return None;
        }
        #[derive(Deserialize)]
        struct TokenAnswer {
            access_token: Zeroizing<String>,
            expires_in: i64,
        }
        let t: TokenAnswer = serde_json::from_slice(&answer.body).ok()?;
        if t.access_token.is_empty() || !t.access_token.bytes().all(|b| b.is_ascii_graphic()) {
            return None;
        }
        Some((t.access_token, t.expires_in))
    }

    /// Sends one message (at most [`MAX_CALLS`] at once).
    pub async fn send(&self, m: &Message<'_>) -> Delivery {
        let Ok(Ok(_slot)) = tokio::time::timeout(SLOT_WAIT, self.calls.acquire()).await else {
            return Delivery::Busy;
        };
        let json = serde_json::json!({
            "message": {
                "token": m.token,
                "data": { "wp": URL_SAFE_NO_PAD.encode(m.body) },
                "android": {
                    "priority": if m.high { "HIGH" } else { "NORMAL" },
                    "ttl": format!("{}s", m.ttl_s),
                },
            }
        });
        let Ok(json) = serde_json::to_vec(&json).map(Zeroizing::new) else {
            return Delivery::Failed;
        };
        let url = format!(
            "{}/v1/projects/{}/messages:send",
            self.base, self.account.project_id
        );
        // A token Google no longer takes (`401`) is renewed once, and only if no one else did.
        let mut rejected = None;
        for _ in 0..2 {
            let Some(token) = self.access_token(rejected.as_ref()).await else {
                return Delivery::Failed;
            };
            let mut authorization = Zeroizing::new(String::with_capacity(7 + token.len()));
            authorization.push_str("Bearer ");
            authorization.push_str(&token);
            let answer = fetch::post_json(&url, &authorization, &json, MAX_ANSWER).await;
            match answer {
                Ok(a) if (200..300).contains(&a.status) => return Delivery::Sent,
                Ok(a) if a.status == 401 && rejected.is_none() => rejected = Some(token),
                Ok(a) => return classify(a.status, &a.body),
                Err(_) => return Delivery::Failed,
            }
        }
        Delivery::Failed
    }
}

/// What FCM's error answer says about the token.
///
/// Only FCM's own words count: a bare `404` (a wrong address, a proxy's page) or a project
/// mismatch (a wrong service account, which would hit every token) is the gateway's problem, never
/// a reason for Companions to drop subscriptions.
fn classify(status: u16, body: &[u8]) -> Delivery {
    let Ok(v) = serde_json::from_slice::<serde_json::Value>(body) else {
        return Delivery::Failed;
    };
    let details = v["error"]["details"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let code_is = |want: &str| details.iter().any(|d| d["errorCode"] == want);
    let about_token = details.iter().any(|d| {
        d["fieldViolations"]
            .as_array()
            .is_some_and(|f| f.iter().any(|f| f["field"] == "message.token"))
    });
    if (status == 404 && code_is("UNREGISTERED")) || (status == 400 && about_token) {
        Delivery::Gone
    } else {
        Delivery::Failed
    }
}

/// Milliseconds since 1970.
pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn fcm_errors_about_the_token_end_the_endpoint_and_others_do_not() {
        // A 404 that is not FCM's "unregistered" (a wrong path, a proxy's page) is not "gone".
        assert_eq!(classify(404, b""), Delivery::Failed);
        assert_eq!(classify(404, b"<html>Not Found</html>"), Delivery::Failed);
        let unregistered = br#"{"error":{"code":404,"status":"NOT_FOUND","details":[{"@type":"type.googleapis.com/google.firebase.fcm.v1.FcmError","errorCode":"UNREGISTERED"}]}}"#;
        assert_eq!(classify(404, unregistered), Delivery::Gone);
        // A wrong service account gives this for every token: never "gone".
        let mismatch = br#"{"error":{"details":[{"errorCode":"SENDER_ID_MISMATCH"}]}}"#;
        assert_eq!(classify(403, mismatch), Delivery::Failed);
        let bad_token =
            br#"{"error":{"details":[{"fieldViolations":[{"field":"message.token"}]}]}}"#;
        assert_eq!(classify(400, bad_token), Delivery::Gone);
        let bad_other =
            br#"{"error":{"details":[{"fieldViolations":[{"field":"message.data"}]}]}}"#;
        assert_eq!(classify(400, bad_other), Delivery::Failed);
        let quota = br#"{"error":{"details":[{"errorCode":"QUOTA_EXCEEDED"}]}}"#;
        assert_eq!(classify(429, quota), Delivery::Failed);
        assert_eq!(classify(503, b"oops"), Delivery::Failed);
    }

    #[test]
    fn a_service_account_file_is_checked() {
        assert_eq!(
            ServiceAccount::from_json(b"{}").unwrap_err(),
            AccountError::Malformed
        );
        let not_a_key = br#"{"client_email":"a@b","private_key":"-----BEGIN PRIVATE KEY-----\nAAAA\n-----END PRIVATE KEY-----\n","token_uri":"https://oauth2.googleapis.com/token","project_id":"p"}"#;
        assert_eq!(
            ServiceAccount::from_json(not_a_key).unwrap_err(),
            AccountError::Key
        );
        assert!(pem_body("no pem").is_none());
        let m = Message {
            token: "made-up-secret-token",
            body: b"secret body",
            high: true,
            ttl_s: 60,
        };
        let shown = format!("{m:?}");
        assert!(!shown.contains("made-up-secret") && !shown.contains("115, 101"));
    }
}
