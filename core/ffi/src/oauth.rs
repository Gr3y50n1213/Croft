// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Bring-your-own OAuth through the surface (T21).
//!
//! **The sign-in.** `oauth_begin` makes a PKCE pair and a `state`, starts listening on
//! `127.0.0.1` (the loopback redirect, see `croft_protocols::oauth::loopback`) and returns the
//! provider's URL for the app to open in a browser tab. `oauth_finish` waits for the browser to
//! come back (at most ten minutes), checks the `state`, exchanges the code with the verifier and
//! returns the account's **sign-in secret**: an opaque blob the app keeps in the keystore exactly
//! like a password and hands back as `FfiCredential::oauth_stored`. The code never passes through
//! an Android intent or another app.
//!
//! **The secret.** The blob holds the refresh token, the person's own client id and (for a
//! Google "Desktop app" client) its client secret, the account name, and the last access token
//! with its expiry. It is JSON; the app never reads it. The core never writes it anywhere: when
//! the provider rotates the refresh token, the core hands the new blob to the app's
//! `CredentialProvider::save_tokens`.
//!
//! **Using it.** An account's [`OAuthSession`] refreshes the access token in memory shortly
//! before it expires, and once more when the server refuses it, before the account is declared
//! "needs sign-in" (`invalid_grant`).

use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use croft_protocols::oauth::loopback::Loopback;
use croft_protocols::oauth::token::{OAuthClient, TokenEndpoint};
use croft_protocols::oauth::{
    authorization_url, parse_redirect, random_token, AuthRequest, OAuthError, OAuthProvider, Pkce,
};
use croft_protocols::{BackendError, Credential};
use serde::{Deserialize, Serialize};
use zeroize::{Zeroize, Zeroizing};

use crate::core::Core;
use crate::credentials::CredentialProvider;

/// A pending sign-in is forgotten after this.
const FLOW_LIFETIME: Duration = Duration::from_secs(10 * 60);
/// An access token is refreshed when it has less than this left.
const REFRESH_MARGIN_MS: i64 = 60_000;

/// Now, Unix milliseconds (for the backend wrapper's expiry check).
pub(crate) fn now_ms_pub() -> i64 {
    now_ms()
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}

/// For tests on the host only: the token endpoint to use instead of the provider's (must be a
/// loopback address; see `TokenEndpoint::loopback`).
static TOKEN_ENDPOINT_FOR_TESTS: LazyLock<Mutex<Option<String>>> =
    LazyLock::new(|| Mutex::new(None));

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The token endpoint of `provider` (or the host tests' loopback one).
fn endpoint(provider: OAuthProvider) -> Result<TokenEndpoint, OAuthError> {
    if let Some(url) = lock(&TOKEN_ENDPOINT_FOR_TESTS).clone() {
        return TokenEndpoint::loopback(provider, &url)
            .ok_or(OAuthError::Refused("endpoint".into()));
    }
    TokenEndpoint::for_provider(provider).map_err(|_| OAuthError::Refused("endpoint".into()))
}

/// Sets the token endpoint the host tests use. Only a loopback `http://` address is accepted.
#[cfg(any(test, feature = "test-server"))]
pub(crate) fn set_token_endpoint_for_tests(url: Option<String>) {
    *lock(&TOKEN_ENDPOINT_FOR_TESTS) = url;
}

/// The stored shape of the sign-in secret (version 1).
#[derive(Serialize, Deserialize)]
struct StoredJson {
    v: u32,
    provider: String,
    client_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    client_secret: Option<String>,
    user: String,
    refresh_token: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    access_token: Option<String>,
    #[serde(default)]
    expires_at_ms: i64,
}

impl Drop for StoredJson {
    fn drop(&mut self) {
        self.refresh_token.zeroize();
        if let Some(s) = self.client_secret.as_mut() {
            s.zeroize();
        }
        if let Some(a) = self.access_token.as_mut() {
            a.zeroize();
        }
    }
}

/// An OAuth account's sign-in, as the keystore keeps it.
pub(crate) struct StoredOAuth {
    pub(crate) provider: OAuthProvider,
    pub(crate) client: OAuthClient,
    pub(crate) user: String,
    refresh_token: Zeroizing<String>,
    access: Option<(Zeroizing<String>, i64)>,
}

impl std::fmt::Debug for StoredOAuth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StoredOAuth")
            .field("provider", &self.provider)
            .finish_non_exhaustive()
    }
}

impl StoredOAuth {
    /// Reads the blob the app stored. `None` for anything this version cannot read.
    pub(crate) fn decode(bytes: &[u8]) -> Option<Self> {
        let j: StoredJson = serde_json::from_slice(bytes).ok()?;
        if j.v != 1 || j.refresh_token.is_empty() || j.client_id.is_empty() {
            return None;
        }
        Some(Self {
            provider: OAuthProvider::from_key(&j.provider)?,
            client: OAuthClient {
                client_id: j.client_id.clone(),
                client_secret: j.client_secret.clone().map(Zeroizing::new),
            },
            user: j.user.clone(),
            refresh_token: Zeroizing::new(j.refresh_token.clone()),
            access: j
                .access_token
                .clone()
                .map(|a| (Zeroizing::new(a), j.expires_at_ms)),
        })
    }

    /// The blob to keep in the keystore.
    pub(crate) fn encode(&self) -> Zeroizing<Vec<u8>> {
        let j = StoredJson {
            v: 1,
            provider: self.provider.key().to_owned(),
            client_id: self.client.client_id.clone(),
            client_secret: self.client.client_secret.as_ref().map(|s| s.to_string()),
            user: self.user.clone(),
            refresh_token: self.refresh_token.to_string(),
            access_token: self.access.as_ref().map(|(a, _)| a.to_string()),
            expires_at_ms: self.access.as_ref().map_or(0, |(_, e)| *e),
        };
        Zeroizing::new(serde_json::to_vec(&j).unwrap_or_default())
    }

    /// A copy (the secrets are copied into new wiping strings).
    fn duplicate(&self) -> Self {
        Self {
            provider: self.provider,
            client: self.client.clone(),
            user: self.user.clone(),
            refresh_token: self.refresh_token.clone(),
            access: self.access.clone(),
        }
    }
}

/// One OAuth account's tokens while it runs: the access token in memory, refreshed when needed,
/// a rotated refresh token handed to the app. One per account, shared by its IMAP and SMTP.
pub(crate) struct OAuthSession {
    account: i64,
    provider: Arc<dyn CredentialProvider>,
    stored: Mutex<StoredOAuth>,
    /// Held while refreshing, so two callers never refresh at once.
    refreshing: tokio::sync::Mutex<()>,
    /// The account has another sign-in now (or none): a rotation this session receives is not
    /// handed to the app, so it can never write an old secret over the new one.
    retired: std::sync::atomic::AtomicBool,
}

impl OAuthSession {
    pub(crate) fn new(
        account: i64,
        stored: StoredOAuth,
        provider: Arc<dyn CredentialProvider>,
    ) -> Self {
        Self {
            account,
            provider,
            stored: Mutex::new(stored),
            refreshing: tokio::sync::Mutex::new(()),
            retired: std::sync::atomic::AtomicBool::new(false),
        }
    }

    /// The account no longer uses this session (a new sign-in replaced it, or it was revoked).
    pub(crate) fn retire(&self) {
        self.retired
            .store(true, std::sync::atomic::Ordering::SeqCst);
    }

    /// The account name the tokens belong to.
    pub(crate) fn user(&self) -> String {
        lock(&self.stored).user.clone()
    }

    fn fresh(&self, now: i64) -> Option<(Zeroizing<String>, i64)> {
        lock(&self.stored)
            .access
            .as_ref()
            .filter(|(_, exp)| exp - now > REFRESH_MARGIN_MS)
            .map(|(a, e)| (a.clone(), *e))
    }

    /// A credential for IMAP or SMTP and when it expires: the access token in memory, or a new one
    /// from the provider when it is about to expire (or when `force`: the server refused it).
    /// `invalid_grant` is [`BackendError::Auth`]: the person signs in again.
    pub(crate) async fn credential(&self, force: bool) -> Result<(Credential, i64), BackendError> {
        let (token, expires) = self.access_token(force).await?;
        Ok((Credential::oauth2(self.user(), token.to_string()), expires))
    }

    async fn access_token(&self, force: bool) -> Result<(Zeroizing<String>, i64), BackendError> {
        if !force {
            if let Some(t) = self.fresh(now_ms()) {
                return Ok(t);
            }
        }
        // The token the caller saw refused (when `force`): a different one after the wait means
        // another caller already refreshed.
        let seen = lock(&self.stored).access.as_ref().map(|(a, _)| a.clone());
        let _one = self.refreshing.lock().await;
        if let Some(t) = self.fresh(now_ms()) {
            if !force || seen.as_ref() != Some(&t.0) {
                return Ok(t);
            }
        }
        let (provider, client, refresh) = {
            let s = lock(&self.stored);
            (s.provider, s.client.clone(), s.refresh_token.clone())
        };
        let tokens = endpoint(provider)?
            .refresh(&client, &refresh, now_ms())
            .await?;
        let rotated = {
            let mut s = lock(&self.stored);
            s.access = Some((tokens.access_token.clone(), tokens.expires_at_ms));
            match tokens.refresh_token {
                Some(new) if *new != *s.refresh_token => {
                    s.refresh_token = new;
                    Some(s.encode())
                }
                _ => None,
            }
        };
        if let Some(blob) =
            rotated.filter(|_| !self.retired.load(std::sync::atomic::Ordering::SeqCst))
        {
            // The app keeps the new refresh token; the old one may stop working at once.
            let provider = self.provider.clone();
            let account = self.account;
            let _ =
                tokio::task::spawn_blocking(move || provider.save_tokens(account, blob.to_vec()))
                    .await;
        }
        Ok((tokens.access_token, tokens.expires_at_ms))
    }
}

/// Whether every server the account would use belongs to the OAuth `provider` (Gmail's servers for
/// Google, Outlook's for Microsoft), so the access token never goes to another host. A JMAP URL
/// is never one of theirs.
pub(crate) fn servers_belong_to(
    provider: OAuthProvider,
    imap: Option<&crate::types::ServerSettings>,
    smtp: Option<&crate::types::ServerSettings>,
    has_jmap: bool,
) -> bool {
    use croft_protocols::discovery::{provider_for_host, Provider};
    let want = match provider {
        OAuthProvider::Google => Provider::Gmail,
        OAuthProvider::Microsoft => Provider::Outlook,
    };
    let Some(imap) = imap else {
        return false;
    };
    // Host tests only: with the fake token endpoint on (never in an Android build), the test
    // servers on this machine stand in for the provider's.
    let ok = |host: &str| {
        provider_for_host(host) == Some(want)
            || (lock(&TOKEN_ENDPOINT_FOR_TESTS).is_some()
                && (host == "127.0.0.1" || host == "localhost"))
    };
    !has_jmap && ok(&imap.host) && smtp.is_none_or(|s| ok(&s.host))
}

/// A credential to check a sign-in with right away (`verify`): the access token the sign-in just
/// got, or a fresh one (not saved: the account does not exist yet).
pub(crate) async fn one_off_credential(stored: &StoredOAuth) -> Result<Credential, BackendError> {
    let now = now_ms();
    if let Some((a, exp)) = stored.access.as_ref() {
        if exp - now > REFRESH_MARGIN_MS {
            return Ok(Credential::oauth2(stored.user.clone(), a.to_string()));
        }
    }
    let t = endpoint(stored.provider)?
        .refresh(&stored.client, &stored.refresh_token, now)
        .await?;
    Ok(Credential::oauth2(
        stored.user.clone(),
        t.access_token.to_string(),
    ))
}

/// Why a sign-in through the provider's page did not work. No field holds a token, a code or a
/// URL.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Error)]
pub enum OAuthFailure {
    /// The person closed the tab or went back (`oauth_cancel`): no message is needed.
    Cancelled,
    /// The person said no on the provider's page.
    AccessDenied,
    /// The browser came back from another (older) sign-in: start again.
    StateMismatch,
    /// The provider refused the client id, the client type or the redirect. `code` is the
    /// provider's short error code (for example `invalid_client`), for the help text.
    Refused {
        /// The provider's error code.
        code: String,
    },
    /// The sign-in took more than ten minutes, or the app was restarted meanwhile: start again.
    Expired,
    /// The provider could not be reached.
    Network,
    /// The client id is empty or not a client id, or the provider is unknown.
    BadClient,
    /// Something else went wrong inside the core.
    Internal,
}

impl std::fmt::Display for OAuthFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}

impl std::error::Error for OAuthFailure {}

impl From<OAuthError> for OAuthFailure {
    fn from(e: OAuthError) -> Self {
        match e {
            OAuthError::AccessDenied => Self::AccessDenied,
            OAuthError::StateMismatch | OAuthError::WrongRedirect | OAuthError::MissingCode => {
                Self::StateMismatch
            }
            OAuthError::Transient => Self::Network,
            OAuthError::InvalidGrant => Self::Refused {
                code: "invalid_grant".into(),
            },
            OAuthError::Refused(code) => Self::Refused { code },
            OAuthError::Malformed => Self::Refused {
                code: "malformed".into(),
            },
        }
    }
}

/// What `oauth_begin` gives the app.
#[derive(Debug, Clone, uniffi::Record)]
pub struct OAuthStart {
    /// The flow, for `oauth_finish` and `oauth_cancel`.
    pub flow_id: String,
    /// The provider's page, to open in a browser tab (never in a WebView).
    pub url: String,
}

/// The finished sign-in: the secret to keep in the keystore and to pass as
/// `FfiCredential::oauth_stored`. No `Debug` on the Kotlin side prints it: it is a `ByteArray`
/// that the app wipes once it has stored it.
#[derive(uniffi::Record)]
pub struct OAuthSignedIn {
    /// The opaque sign-in secret.
    pub secret: Vec<u8>,
}

impl std::fmt::Debug for OAuthSignedIn {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("OAuthSignedIn(***)")
    }
}

struct Flow {
    provider: OAuthProvider,
    client: OAuthClient,
    user: String,
    pkce: Pkce,
    state: String,
    loopback: Arc<Loopback>,
    page: String,
    started: Instant,
    cancel: Arc<tokio::sync::Notify>,
}

static FLOWS: LazyLock<Mutex<HashMap<String, Flow>>> = LazyLock::new(|| Mutex::new(HashMap::new()));

fn client_id_is_plausible(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 256
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_'))
}

fn html_escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// The setting that remembers the person's client id for a provider (not a secret).
fn client_id_key(provider: OAuthProvider) -> String {
    format!("oauth.client_id.{}", provider.key())
}

#[uniffi::export]
impl Core {
    /// Starts a sign-in with the person's own OAuth client of `provider` (`"google"` or
    /// `"microsoft"`). `client_secret` is only for a Google "Desktop app" client (Google gives
    /// one to that client type; it is the person's, kept in the keystore with the account).
    /// `email` preselects the account on the provider's page. `done_page` is the sentence the
    /// browser shows once it has handed the sign-in back ("You can go back to Croft"). Returns the
    /// URL to open in a browser tab. The client id is remembered for next time
    /// (`oauth_client_id`).
    pub async fn oauth_begin(
        &self,
        provider: String,
        client_id: String,
        client_secret: Option<Vec<u8>>,
        email: String,
        done_page: String,
    ) -> Result<OAuthStart, OAuthFailure> {
        let provider = OAuthProvider::from_key(&provider).ok_or(OAuthFailure::BadClient)?;
        let client_id = client_id.trim().to_owned();
        if !client_id_is_plausible(&client_id) {
            return Err(OAuthFailure::BadClient);
        }
        let client_secret = match client_secret {
            Some(bytes) => {
                let s = String::from_utf8(bytes).map_err(|_| OAuthFailure::BadClient)?;
                let s = Zeroizing::new(s);
                (!s.trim().is_empty()).then(|| Zeroizing::new(s.trim().to_owned()))
            }
            None => None,
        };
        let task = self.spawn(async move { Loopback::bind().await }, false);
        let loopback = task
            .await
            .map_err(|_| OAuthFailure::Internal)?
            .map_err(OAuthFailure::from)?;
        let pkce = Pkce::generate().map_err(|_| OAuthFailure::Internal)?;
        let state = random_token().map_err(|_| OAuthFailure::Internal)?;
        let flow_id = random_token().map_err(|_| OAuthFailure::Internal)?;
        let url = authorization_url(&AuthRequest {
            provider,
            client_id: &client_id,
            redirect_uri: &loopback.redirect_uri(),
            state: &state,
            challenge: &pkce.challenge,
            login_hint: Some(email.trim()),
        });
        let page = format!(
            "<!doctype html><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width\"><title>Croft</title><p style=\"font:1.2em sans-serif;margin:2em\">{}</p>",
            html_escape(&done_page)
        );
        {
            let mut flows = lock(&FLOWS);
            flows.retain(|_, f| f.started.elapsed() < FLOW_LIFETIME);
            flows.insert(
                flow_id.clone(),
                Flow {
                    provider,
                    client: OAuthClient {
                        client_id: client_id.clone(),
                        client_secret,
                    },
                    user: email.trim().to_owned(),
                    pkce,
                    state,
                    loopback: Arc::new(loopback),
                    page,
                    started: Instant::now(),
                    cancel: Arc::new(tokio::sync::Notify::new()),
                },
            );
        }
        let key = client_id_key(provider);
        let _ = self
            .inner
            .handle
            .read(|db| croft_storage::settings::set(db.connection(), &key, &client_id, now_ms()));
        Ok(OAuthStart { flow_id, url })
    }

    /// Waits for the browser to come back from the provider's page (until the flow is ten minutes
    /// old, or `oauth_cancel`), then exchanges the code. Returns the sign-in secret: store it
    /// under the account and pass it as `FfiCredential::oauth_stored` to `verify` and
    /// `add_account`.
    pub async fn oauth_finish(&self, flow_id: String) -> Result<OAuthSignedIn, OAuthFailure> {
        let (loopback, state, page, cancel, left) = {
            let mut flows = lock(&FLOWS);
            // Sign-ins nobody finished (an app stopped mid-way) are dropped with their listeners.
            flows.retain(|_, f| f.started.elapsed() < FLOW_LIFETIME);
            let f = flows.get(&flow_id).ok_or(OAuthFailure::Expired)?;
            let left = FLOW_LIFETIME
                .checked_sub(f.started.elapsed())
                .ok_or(OAuthFailure::Expired)?;
            (
                f.loopback.clone(),
                f.state.clone(),
                f.page.clone(),
                f.cancel.clone(),
                left,
            )
        };
        let wait = self.spawn(
            async move {
                tokio::select! {
                    r = tokio::time::timeout(left, loopback.wait(&state, &page)) => match r {
                        Ok(r) => r.map_err(OAuthFailure::from),
                        Err(_) => Err(OAuthFailure::Expired),
                    },
                    () = cancel.notified() => Err(OAuthFailure::Cancelled),
                }
            },
            true,
        );
        let redirect = wait.await.map_err(|_| OAuthFailure::Internal)?;
        // Whatever happened, this flow is over (a cancel already removed it).
        let flow = lock(&FLOWS).remove(&flow_id);
        let redirect = redirect?;
        let flow = flow.ok_or(OAuthFailure::Expired)?;
        let code = parse_redirect(&redirect, &flow.loopback.redirect_uri(), &flow.state)?;
        let exchange = self.spawn(
            async move {
                let ep = endpoint(flow.provider)?;
                let tokens = ep
                    .exchange_code(
                        &flow.client,
                        &code,
                        &flow.loopback.redirect_uri(),
                        &flow.pkce.verifier,
                        now_ms(),
                    )
                    .await?;
                let refresh = tokens
                    .refresh_token
                    .ok_or(OAuthError::Refused("no_refresh_token".into()))?;
                Ok::<_, OAuthError>(StoredOAuth {
                    provider: flow.provider,
                    client: flow.client,
                    user: flow.user,
                    refresh_token: refresh,
                    access: Some((tokens.access_token, tokens.expires_at_ms)),
                })
            },
            false,
        );
        let stored = exchange.await.map_err(|_| OAuthFailure::Internal)??;
        Ok(OAuthSignedIn {
            secret: stored.encode().to_vec(),
        })
    }

    /// Ends a pending sign-in (the person closed the tab, or left the screen): its `oauth_finish`
    /// returns `Cancelled` and the listener closes.
    pub fn oauth_cancel(&self, flow_id: String) {
        let mut flows = lock(&FLOWS);
        if let Some(f) = flows.remove(&flow_id) {
            f.cancel.notify_one();
        }
        flows.retain(|_, f| f.started.elapsed() < FLOW_LIFETIME);
    }

    /// The client id the person used last for `provider`, to fill in the field.
    pub fn oauth_client_id(&self, provider: String) -> Option<String> {
        let provider = OAuthProvider::from_key(&provider)?;
        let key = client_id_key(provider);
        self.inner
            .handle
            .read(|db| croft_storage::settings::get::<String>(db.connection(), &key))
            .ok()
            .flatten()
    }
}

/// Makes a session from what the app stored (used when an account starts).
pub(crate) fn session(
    account: i64,
    stored: &StoredOAuth,
    provider: Arc<dyn CredentialProvider>,
) -> OAuthSession {
    OAuthSession::new(account, stored.duplicate(), provider)
}
