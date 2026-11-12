// This Source Code Form is subject to the terms of the GNU Affero General Public License,
// version 3 or later. See LICENSE-AGPL-3.0 at the repository root.

//! Sending to FCM (HTTP v1 API) as the app's publisher: a Google service account signs a JWT
//! (RS256), which its token endpoint exchanges for an access token, kept until 5 minutes before it
//! expires. Requests go through the core's HTTPS client, to the hosts the admin configured.

use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use base64::Engine as _;
use croft_protocols::oauth::fetch;
use ring::rand::SystemRandom;
use ring::signature::{RsaKeyPair, RSA_PKCS1_SHA256};
use serde::Deserialize;
use zeroize::Zeroizing;

/// What the access token may do.
pub const SCOPE: &str = "https://www.googleapis.com/auth/firebase.messaging";
/// FCM's address.
pub const FCM_BASE: &str = "https://fcm.googleapis.com";
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
        let uri_ok =
            file.token_uri.starts_with("https://") || fetch::is_loopback_http(&file.token_uri);
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
        let claims = serde_json::json!({
            "iss": self.client_email,
            "scope": SCOPE,
            "aud": self.token_uri,
            "iat": now_s,
            "exp": now_s + JWT_LIFETIME_S,
        });
        let claims = URL_SAFE_NO_PAD.encode(serde_json::to_vec(&claims).ok()?);
        let signed = format!("{header}.{claims}");
        let mut signature = vec![0u8; self.key.public().modulus_len()];
        self.key
            .sign(
                &RSA_PKCS1_SHA256,
                &SystemRandom::new(),
                signed.as_bytes(),
                &mut signature,
            )
            .ok()?;
        Some(Zeroizing::new(format!(
            "{signed}.{}",
            URL_SAFE_NO_PAD.encode(signature)
        )))
    }
}

/// The DER inside a PEM block (`-----BEGIN PRIVATE KEY-----` ... `-----END PRIVATE KEY-----`).
fn pem_body(pem: &str) -> Option<Zeroizing<Vec<u8>>> {
    let start = pem.find("-----BEGIN PRIVATE KEY-----")? + "-----BEGIN PRIVATE KEY-----".len();
    let end = pem[start..].find("-----END PRIVATE KEY-----")? + start;
    let body: Zeroizing<String> = Zeroizing::new(
        pem[start..end]
            .chars()
            .filter(|c| !c.is_ascii_whitespace())
            .collect(),
    );
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
}

/// One message for a device.
#[derive(Debug, Clone, Copy)]
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

struct Cached {
    token: Zeroizing<String>,
    until_ms: i64,
}

/// FCM, for one service account.
pub struct Fcm {
    account: ServiceAccount,
    base: String,
    cached: tokio::sync::Mutex<Option<Cached>>,
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
            cached: tokio::sync::Mutex::new(None),
        }
    }

    /// An access token: the kept one, or a new one when it is near its end or `renew` is set. One
    /// request for a token at a time (the others wait for it).
    async fn access_token(&self, renew: bool) -> Option<Zeroizing<String>> {
        let mut cached = self.cached.lock().await;
        let now = now_ms();
        if !renew {
            if let Some(c) = cached.as_ref().filter(|c| c.until_ms > now) {
                return Some(c.token.clone());
            }
        }
        let assertion = self.account.assertion(now / 1000)?;
        let form = fetch::form(&[
            ("grant_type", "urn:ietf:params:oauth:grant-type:jwt-bearer"),
            ("assertion", &assertion),
        ]);
        let answer = fetch::post_form(&self.account.token_uri, &form, None, MAX_ANSWER)
            .await
            .ok()?;
        if answer.status != 200 {
            *cached = None;
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
        let until_ms = now + t.expires_in.clamp(0, 24 * 3600) * 1000 - RENEW_BEFORE_MS;
        *cached = Some(Cached {
            token: t.access_token.clone(),
            until_ms,
        });
        Some(t.access_token)
    }

    /// Sends one message.
    pub async fn send(&self, m: &Message<'_>) -> Delivery {
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
        // A token Google no longer takes (`401`) is renewed once.
        for renew in [false, true] {
            let Some(token) = self.access_token(renew).await else {
                return Delivery::Failed;
            };
            let authorization = Zeroizing::new(format!("Bearer {}", token.as_str()));
            let answer = fetch::post_json(&url, &authorization, &json, MAX_ANSWER).await;
            match answer {
                Ok(a) if (200..300).contains(&a.status) => return Delivery::Sent,
                Ok(a) if a.status == 401 && !renew => continue,
                Ok(a) => return classify(a.status, &a.body),
                Err(_) => return Delivery::Failed,
            }
        }
        Delivery::Failed
    }
}

/// What FCM's error answer says about the token.
fn classify(status: u16, body: &[u8]) -> Delivery {
    if status == 404 {
        return Delivery::Gone;
    }
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
    if code_is("UNREGISTERED") || code_is("SENDER_ID_MISMATCH") || (status == 400 && about_token) {
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
        assert_eq!(classify(404, b""), Delivery::Gone);
        let unregistered = br#"{"error":{"code":404,"status":"NOT_FOUND","details":[{"@type":"type.googleapis.com/google.firebase.fcm.v1.FcmError","errorCode":"UNREGISTERED"}]}}"#;
        assert_eq!(classify(400, unregistered), Delivery::Gone);
        let mismatch = br#"{"error":{"details":[{"errorCode":"SENDER_ID_MISMATCH"}]}}"#;
        assert_eq!(classify(403, mismatch), Delivery::Gone);
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
        let not_a_key = br#"{"client_email":"a@b","private_key":"-----BEGIN PRIVATE KEY-----\nAAAA\n-----END PRIVATE KEY-----\n","token_uri":"https://oauth2.example/token","project_id":"p"}"#;
        assert_eq!(
            ServiceAccount::from_json(not_a_key).unwrap_err(),
            AccountError::Key
        );
        assert!(pem_body("no pem").is_none());
    }
}
