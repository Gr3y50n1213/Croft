// This Source Code Form is subject to the terms of the GNU Affero General Public License,
// version 3 or later. See LICENSE-AGPL-3.0 at the repository root.

//! Sign-in through an OpenID Connect provider (T25): the Companion as a relying party.
//!
//! - **Discovery.** The provider's `/.well-known/openid-configuration` is fetched and cached; its
//!   `issuer` must equal the configured one exactly and every endpoint must be `https`.
//! - **Flow.** Authorization Code with PKCE (S256), `state` and `nonce`, each random and single
//!   use, kept in memory for 10 minutes. The redirect URI is fixed: `{public_url}/oidc/callback`.
//! - **ID token.** Checked by [`crate::jwt`] with the provider's key set, refetched at most once a
//!   minute when a token names a key it does not know.
//! - **Hand-over.** The provider's answer never acts on its own: it becomes a one-time code (60
//!   seconds) handed to the app that started the flow, which redeems it with the PKCE verifier only
//!   it knows. A flow started by someone else and finished in another person's browser (login CSRF,
//!   or linking a stranger's identity to an account) therefore goes nowhere.
//!
//! Nothing here logs; no error carries a code, a token or a secret.

use std::sync::{Mutex, PoisonError};

use base64::Engine;
use ring::digest;
use serde_json::Value;
use zeroize::Zeroizing;

use crate::auth::random_token;
use crate::ceremony::{Ceremonies, Owner};
use crate::config::OidcSettings;
use crate::jwt::{self, Identity, Jwk, JwtError};
use crate::store::StoreError;
use croft_protocols::oauth::fetch::{self, FetchError};

/// How long a flow waits for the provider.
pub const FLOW_TTL_MS: i64 = 10 * 60 * 1000;
/// How long a hand-over code lasts.
pub const HANDOVER_TTL_MS: i64 = 60 * 1000;
/// How long discovery is cached.
const DISCOVERY_TTL_MS: i64 = 24 * 60 * 60 * 1000;
/// The key set is fetched again (unknown key id, a signature that fails, a failed fetch) at most
/// this often.
const JWKS_REFETCH_MS: i64 = 60 * 1000;
/// The key set is fetched again after this long in any case (a key rotated under the same id, or
/// withdrawn, is noticed within a day).
const JWKS_MAX_AGE_MS: i64 = 24 * 60 * 60 * 1000;
/// After a failed discovery, the next attempt waits this long (a provider that is down does not
/// hold every sign-in start for the full request time).
const DISCOVERY_RETRY_MS: i64 = 30 * 1000;
/// Largest discovery document, key set or token answer read.
const MAX_DOCUMENT: usize = 256 * 1024;

/// Why an OIDC step failed. No variant carries a code, a token or a secret.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OidcError {
    /// The provider could not be reached (try later).
    Unreachable,
    /// The provider's discovery document is wrong (issuer, endpoints).
    Discovery(&'static str),
    /// The token endpoint refused the code.
    TokenRefused,
    /// The ID token did not check out.
    Token(JwtError),
    /// No such flow or hand-over (unknown, used or expired), or a wrong verifier.
    Invalid,
    /// The store failed.
    Server,
}

impl From<StoreError> for OidcError {
    fn from(_: StoreError) -> Self {
        Self::Server
    }
}

impl From<FetchError> for OidcError {
    fn from(e: FetchError) -> Self {
        match e {
            FetchError::Unreachable => Self::Unreachable,
            _ => Self::Discovery("the provider's answer could not be read"),
        }
    }
}

/// What the provider publishes.
#[derive(Debug, Clone)]
struct Discovery {
    authorization_endpoint: String,
    token_endpoint: String,
    jwks_uri: String,
    /// `client_secret_basic` (the default) rather than `client_secret_post`.
    basic_auth: bool,
    fetched_ms: i64,
}

/// What a flow is for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Purpose {
    /// Signing in.
    SignIn,
    /// Linking the identity to this signed-in user and session.
    Link {
        /// The user.
        user_id: i64,
        /// Their session.
        session_id: i64,
    },
}

/// A flow waiting for the provider.
struct Flow {
    nonce: Zeroizing<String>,
    verifier: Zeroizing<String>,
    /// The app's PKCE challenge (S256 of a verifier only the app knows).
    app_challenge: String,
    /// Where the app wants the browser back (already checked).
    app_redirect: String,
    purpose: Purpose,
}

/// What a hand-over code stands for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// Sign in this user.
    SignIn {
        /// The user.
        user_id: i64,
    },
    /// Link this identity to this user (if the redeemer is that user).
    Link {
        /// The user.
        user_id: i64,
        /// The session that started it.
        session_id: i64,
        /// The provider.
        issuer: String,
        /// The person there.
        subject: String,
    },
}

struct Handover {
    app_challenge: String,
    outcome: Outcome,
}

/// The configured provider and the flows in progress.
pub struct Provider {
    settings: OidcSettings,
    secret: Zeroizing<String>,
    redirect_uri: String,
    public_url: String,
    allow_loopback_http: bool,
    discovery: tokio::sync::Mutex<(Option<Discovery>, i64)>,
    jwks: tokio::sync::Mutex<(Vec<Jwk>, i64)>,
    flows: Mutex<Ceremonies<Flow>>,
    handovers: Mutex<Ceremonies<Handover>>,
}

impl std::fmt::Debug for Provider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Provider")
            .field("issuer", &self.settings.issuer)
            .finish_non_exhaustive()
    }
}

fn b64url(bytes: &[u8]) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

/// The PKCE S256 challenge of a verifier.
pub fn s256(verifier: &str) -> String {
    b64url(digest::digest(&digest::SHA256, verifier.as_bytes()).as_ref())
}

fn query_escape(s: &str) -> String {
    s.bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
                char::from(b).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect()
}

/// Appends query parameters to a URL that may already have some.
pub fn with_query(url: &str, pairs: &[(&str, &str)]) -> String {
    let mut out = url.to_owned();
    for (i, (k, v)) in pairs.iter().enumerate() {
        out.push(if i == 0 && !url.contains('?') {
            '?'
        } else {
            '&'
        });
        out.push_str(&query_escape(k));
        out.push('=');
        out.push_str(&query_escape(v));
    }
    out
}

impl Provider {
    /// The provider from the settings, with its client secret (read from its file by the caller).
    pub fn new(settings: OidcSettings, secret: Zeroizing<String>, public_url: &str) -> Self {
        let public_url = public_url.trim_end_matches('/').to_owned();
        Self {
            redirect_uri: format!("{public_url}/oidc/callback"),
            public_url,
            settings,
            secret,
            allow_loopback_http: false,
            discovery: tokio::sync::Mutex::new((None, i64::MIN / 2)),
            jwks: tokio::sync::Mutex::new((Vec::new(), i64::MIN / 2)),
            flows: Mutex::new(Ceremonies::with_ttl(FLOW_TTL_MS)),
            handovers: Mutex::new(Ceremonies::with_ttl(HANDOVER_TTL_MS)),
        }
    }

    /// The same, for a provider on this machine over plain `http` (tests only).
    #[cfg(test)]
    pub fn for_tests(settings: OidcSettings, secret: &str, public_url: &str) -> Self {
        let mut p = Self::new(settings, Zeroizing::new(secret.to_owned()), public_url);
        p.allow_loopback_http = true;
        p
    }

    /// The configured settings.
    pub fn settings(&self) -> &OidcSettings {
        &self.settings
    }

    fn endpoint_ok(&self, url: &str) -> bool {
        url.starts_with("https://") || (self.allow_loopback_http && fetch::is_loopback_http(url))
    }

    /// Whether the app's redirect is acceptable: the Croft app's own redirect on a phone, or an
    /// address on the Companion's own origin, under
    /// its `public_url` path, written in normal form. The URL is parsed as a browser does (WHATWG)
    /// and must read back exactly as given, so dot segments (`/../`, `%2e%2e`), backslashes,
    /// user names, a fragment or another port, which a browser would resolve elsewhere, are
    /// refused (review finding: `/croft/%2e%2e/page` left the Companion's path with the code).
    pub fn app_redirect_ok(&self, url: &str) -> bool {
        // The Croft app on a phone (T34a): exactly its own redirect.
        if crate::app_signin::is_app_redirect(url) {
            return true;
        }
        if url.len() > 512 || !url.bytes().all(|b| (0x21..=0x7e).contains(&b)) {
            return false;
        }
        let (Ok(parsed), Ok(public)) = (url::Url::parse(url), url::Url::parse(&self.public_url))
        else {
            return false;
        };
        let lower = url.to_ascii_lowercase();
        let base = public.path().trim_end_matches('/');
        parsed.as_str() == url
            && !lower.contains("%2e")
            && !lower.contains("%2f")
            && !lower.contains("%5c")
            && parsed.origin() == public.origin()
            && parsed.username().is_empty()
            && parsed.password().is_none()
            && parsed.fragment().is_none()
            && parsed.path().starts_with(&format!("{base}/"))
            && !parsed.path().contains("//")
            && !parsed
                .path()
                .split('/')
                .any(|seg| seg == "." || seg == "..")
    }

    async fn discovery(&self, now: i64) -> Result<Discovery, OidcError> {
        let mut cached = self.discovery.lock().await;
        if let Some(d) = cached
            .0
            .as_ref()
            .filter(|d| now - d.fetched_ms < DISCOVERY_TTL_MS)
        {
            return Ok(d.clone());
        }
        if now - cached.1 < DISCOVERY_RETRY_MS {
            return Err(OidcError::Unreachable);
        }
        // Counted as a failure until it succeeds.
        cached.1 = now;
        let url = format!(
            "{}/.well-known/openid-configuration",
            self.settings.issuer.trim_end_matches('/')
        );
        if !self.endpoint_ok(&url) {
            return Err(OidcError::Discovery("the issuer is not https"));
        }
        let answer = fetch::get(&url, MAX_DOCUMENT).await?;
        if answer.status != 200 {
            return Err(OidcError::Discovery("no discovery document"));
        }
        let doc: Value = serde_json::from_slice(&answer.body)
            .map_err(|_| OidcError::Discovery("the discovery document is not JSON"))?;
        let text = |k: &str| doc.get(k).and_then(Value::as_str).map(str::to_owned);
        if text("issuer").as_deref() != Some(self.settings.issuer.as_str()) {
            return Err(OidcError::Discovery(
                "its issuer differs from the configured one",
            ));
        }
        let endpoint = |k: &'static str| -> Result<String, OidcError> {
            text(k)
                .filter(|u| self.endpoint_ok(u))
                .ok_or(OidcError::Discovery("an endpoint is missing or not https"))
        };
        let methods: Vec<&str> = doc
            .get("token_endpoint_auth_methods_supported")
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(Value::as_str).collect())
            .unwrap_or_default();
        let basic_auth = methods.is_empty() || methods.contains(&"client_secret_basic");
        // A provider that lists its PKCE methods must offer S256 (one that ignored PKCE would
        // leave only the nonce).
        if let Some(pkce) = doc
            .get("code_challenge_methods_supported")
            .and_then(Value::as_array)
        {
            if !pkce.iter().any(|m| m.as_str() == Some("S256")) {
                return Err(OidcError::Discovery(
                    "the provider does not support PKCE with S256",
                ));
            }
        }
        if !basic_auth && !methods.contains(&"client_secret_post") {
            return Err(OidcError::Discovery(
                "the provider supports neither client_secret_basic nor client_secret_post",
            ));
        }
        let d = Discovery {
            authorization_endpoint: endpoint("authorization_endpoint")?,
            token_endpoint: endpoint("token_endpoint")?,
            jwks_uri: endpoint("jwks_uri")?,
            basic_auth,
            fetched_ms: now,
        };
        *cached = (Some(d.clone()), i64::MIN / 2);
        Ok(d)
    }

    /// The provider's keys. Fetched again when they are older than a day, when the token names a
    /// key id they lack, or when `failed_with_these` says a signature did not verify with them;
    /// never more than once a minute (a failed fetch counts).
    async fn keys(
        &self,
        jwks_uri: &str,
        kid: Option<&str>,
        failed_with_these: bool,
        now: i64,
    ) -> Result<Vec<Jwk>, OidcError> {
        let mut cache = self.jwks.lock().await;
        let known = kid.is_none_or(|k| cache.0.iter().any(|j| j.kid() == Some(k)));
        let old = now - cache.1 >= JWKS_MAX_AGE_MS;
        let wanted = cache.0.is_empty() || !known || old || failed_with_these;
        if wanted && now - cache.1 >= JWKS_REFETCH_MS {
            cache.1 = now;
            if let Ok(answer) = fetch::get(jwks_uri, MAX_DOCUMENT).await {
                if answer.status == 200 {
                    cache.0 = jwt::parse_jwks(&answer.body);
                }
            }
        }
        Ok(cache.0.clone())
    }

    /// Starts a flow: returns the provider's authorization URL to send the browser to. `owner`
    /// caps how many flows one address or user keeps.
    pub async fn start(
        &self,
        owner: Owner,
        purpose: Purpose,
        app_challenge: &str,
        app_redirect: &str,
        now: i64,
    ) -> Result<String, OidcError> {
        if !self.app_redirect_ok(app_redirect) || !challenge_ok(app_challenge) {
            return Err(OidcError::Invalid);
        }
        let d = self.discovery(now).await?;
        let state = random_token()?;
        let nonce = random_token()?;
        let verifier = random_token()?;
        let challenge = s256(&verifier);
        let mut scope = "openid profile".to_owned();
        if self.settings.create_users {
            scope.push_str(" groups");
        }
        let mut params = vec![
            ("response_type", "code"),
            ("client_id", self.settings.client_id.as_str()),
            ("redirect_uri", self.redirect_uri.as_str()),
            ("scope", scope.as_str()),
            ("state", state.as_str()),
            ("nonce", nonce.as_str()),
            ("code_challenge", challenge.as_str()),
            ("code_challenge_method", "S256"),
        ];
        // For a phone (T34a review): the provider always asks the person, so a flow some other app
        // started never completes silently on a session the browser already has.
        if crate::app_signin::is_app_redirect(app_redirect) {
            params.push(("prompt", "login"));
        }
        let url = with_query(&d.authorization_endpoint, &params);
        let flow = Flow {
            nonce,
            verifier,
            app_challenge: app_challenge.to_owned(),
            app_redirect: app_redirect.to_owned(),
            purpose,
        };
        // The flow's id is the `state` value itself (stored as its hash).
        self.flows
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .put_with_id(owner, &state, flow, now);
        Ok(url)
    }

    /// The app redirect of a flow, for answering the browser even when the flow then fails.
    /// Takes the flow (single use).
    pub fn take_flow(&self, state: &str, now: i64) -> Option<TakenFlow> {
        let flow = self
            .flows
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take(state, now)?;
        Some(TakenFlow(flow))
    }

    /// Finishes a taken flow with the provider's `code`: the code is exchanged at the token
    /// endpoint and the ID token checked. Returns the identity and the flow's purpose.
    pub async fn finish(
        &self,
        flow: &TakenFlow,
        code: &str,
        now: i64,
    ) -> Result<Identity, OidcError> {
        let f = &flow.0;
        let d = self.discovery(now).await?;
        let mut pairs = vec![
            ("grant_type", "authorization_code"),
            ("code", code),
            ("redirect_uri", self.redirect_uri.as_str()),
            ("code_verifier", f.verifier.as_str()),
        ];
        let basic;
        let authorization = if d.basic_auth {
            // RFC 6749 2.3.1: the id and secret are form-encoded before Basic.
            let escaped = Zeroizing::new(query_escape(&self.secret));
            let user_pass = Zeroizing::new(format!(
                "{}:{}",
                query_escape(&self.settings.client_id),
                escaped.as_str()
            ));
            basic = Zeroizing::new(format!(
                "Basic {}",
                base64::engine::general_purpose::STANDARD.encode(user_pass.as_bytes())
            ));
            Some(basic.as_str())
        } else {
            pairs.push(("client_id", self.settings.client_id.as_str()));
            pairs.push(("client_secret", self.secret.as_str()));
            None
        };
        let form = fetch::form(&pairs);
        let answer =
            fetch::post_form(&d.token_endpoint, &form, authorization, MAX_DOCUMENT).await?;
        if answer.status != 200 {
            return Err(OidcError::TokenRefused);
        }
        // Only the ID token is read; the other fields (the access token among them) are skipped
        // without being copied.
        #[derive(serde::Deserialize)]
        struct TokenAnswer {
            id_token: String,
        }
        let answer: TokenAnswer =
            serde_json::from_slice(&answer.body).map_err(|_| OidcError::TokenRefused)?;
        let id_token = Zeroizing::new(answer.id_token);
        let kid = jwt::key_id(&id_token);
        let keys = self.keys(&d.jwks_uri, kid.as_deref(), false, now).await?;
        let verified = match jwt::verify_signature(&id_token, &keys) {
            // A key rotated under the same id: one fresh look at the key set.
            Err(JwtError::Signature | JwtError::UnknownKey) => {
                let keys = self.keys(&d.jwks_uri, kid.as_deref(), true, now).await?;
                jwt::verify_signature(&id_token, &keys)
            }
            other => other,
        }
        .map_err(OidcError::Token)?;
        jwt::check_claims(
            &verified,
            &jwt::Expected {
                issuer: &self.settings.issuer,
                client_id: &self.settings.client_id,
                nonce: &f.nonce,
                now_s: now / 1000,
                groups_claim: &self.settings.groups_claim,
            },
        )
        .map_err(OidcError::Token)
    }

    /// A one-time code standing for `outcome`, redeemable once by the app that holds the verifier
    /// of `flow`'s app challenge.
    pub fn handover(
        &self,
        flow: &TakenFlow,
        outcome: Outcome,
        now: i64,
    ) -> Result<Zeroizing<String>, OidcError> {
        let owner = match &outcome {
            Outcome::SignIn { user_id } | Outcome::Link { user_id, .. } => Owner::User(*user_id),
        };
        Ok(self
            .handovers
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .put(
                owner,
                Handover {
                    app_challenge: flow.0.app_challenge.clone(),
                    outcome,
                },
                now,
            )?)
    }

    /// Redeems a hand-over code (once, whatever the verifier): the outcome if the verifier
    /// matches the app challenge.
    pub fn redeem(&self, code: &str, verifier: &str, now: i64) -> Result<Outcome, OidcError> {
        let h = self
            .handovers
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take(code, now)
            .ok_or(OidcError::Invalid)?;
        if !crate::crypto::ct_eq(s256(verifier).as_bytes(), h.app_challenge.as_bytes()) {
            return Err(OidcError::Invalid);
        }
        Ok(h.outcome)
    }
}

/// A flow taken out of the table (single use), with where the browser goes back to.
pub struct TakenFlow(Flow);

impl TakenFlow {
    /// The app's redirect.
    pub fn app_redirect(&self) -> &str {
        &self.0.app_redirect
    }

    /// What the flow is for.
    pub fn purpose(&self) -> &Purpose {
        &self.0.purpose
    }

    /// The app's PKCE challenge.
    pub fn app_challenge(&self) -> &str {
        &self.0.app_challenge
    }
}

/// A PKCE S256 challenge: 43 base64url characters.
fn challenge_ok(c: &str) -> bool {
    c.len() == 43
        && c.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}
