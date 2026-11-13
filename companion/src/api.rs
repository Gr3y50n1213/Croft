// This Source Code Form is subject to the terms of the GNU Affero General Public License,
// version 3 or later. See LICENSE-AGPL-3.0 at the repository root.

//! The sign-in API (T24): enrolment, passkey and recovery sign-in, tokens, the user's own devices,
//! passkeys and audit log, and the admin's user management.
//!
//! Every route is in [`routes`], with who may call it ([`Access`]); the router is built from that
//! table and the tests walk the same table, so a route cannot be added without its access being
//! tested. Handlers take the user from the access token, never from the request, and pass that id
//! to the store, whose every query is scoped by it. Answers never say which part of a sign-in was
//! wrong, and nothing here logs.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::{Arc, Mutex, PoisonError};

use axum::extract::rejection::JsonRejection;
use axum::extract::{FromRequestParts, Path, State};
use axum::http::request::Parts;
use axum::http::{header, HeaderMap, HeaderValue, Method, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post, MethodRouter};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};
use webauthn_rs::prelude::{
    PublicKeyCredential, RegisterPublicKeyCredential, Url, Webauthn, WebauthnBuilder,
};
use zeroize::Zeroizing;

use crate::auth::{self, Caller, RefreshError, SignInError, Tokens};
use crate::ceremony::{Ceremonies, Owner, Pending};
use crate::crypto::{self, Key};
use crate::http::App;
use crate::limits::{self, Limits};
use crate::store::{Role, Store, StoreError};
use crate::totp;

/// The longest device name kept.
const MAX_DEVICE: usize = 64;
/// Audit entries returned at most.
const AUDIT_LIMIT: u32 = 200;

/// The clock (Unix milliseconds); tests move it.
pub type Clock = Arc<dyn Fn() -> i64 + Send + Sync>;

/// The address of the connection, put on each request by the server loop.
#[derive(Debug, Clone, Copy)]
pub struct Peer(pub SocketAddr);

/// What sign-in needs besides the store.
pub struct Auth {
    webauthn: Webauthn,
    public_url: String,
    master: Key,
    ceremonies: Mutex<Ceremonies>,
    /// Phones' sign-in hand-over codes (T34a).
    app_handovers: crate::app_signin::AppHandovers,
    limits: Mutex<Limits>,
    /// Per-user limits for the mail routes that connect out (T26).
    mail_limits: Mutex<Limits>,
    /// Per-user limits for the JMAP API (T27), and its requests running per user.
    jmap_limits: Mutex<Limits>,
    /// Per-user sending budget (T31b): messages and recipients sent, by time.
    sends: Mutex<std::collections::HashMap<i64, std::collections::VecDeque<(i64, u32)>>>,
    /// Each user's server searches in the last minute (T33b).
    server_searches: Mutex<std::collections::HashMap<i64, std::collections::VecDeque<i64>>>,
    jmap_running: Arc<Mutex<std::collections::HashMap<i64, usize>>>,
    trusted_proxies: Vec<IpAddr>,
    clock: Clock,
    oidc: Option<Arc<crate::oidc::Provider>>,
}

impl std::fmt::Debug for Auth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Auth")
    }
}

/// Why the WebAuthn settings could not be made from `public_url`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BadPublicUrl;

impl std::fmt::Display for BadPublicUrl {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("public_url must be a domain name (passkeys do not work with an IP address)")
    }
}

impl std::error::Error for BadPublicUrl {}

impl Auth {
    /// Sign-in for the Companion at `public_url` (the WebAuthn origin; its host is the relying
    /// party id).
    pub fn new(
        public_url: &str,
        master: Key,
        trusted_proxies: Vec<IpAddr>,
        clock: Clock,
    ) -> Result<Self, BadPublicUrl> {
        let origin = Url::parse(public_url).map_err(|_| BadPublicUrl)?;
        let rp_id = origin.domain().ok_or(BadPublicUrl)?.to_owned();
        let webauthn = WebauthnBuilder::new(&rp_id, &origin)
            .and_then(|b| b.rp_name("Croft Companion").build())
            .map_err(|_| BadPublicUrl)?;
        Ok(Self {
            webauthn,
            public_url: public_url.trim_end_matches('/').to_owned(),
            master,
            ceremonies: Mutex::new(Ceremonies::default()),
            app_handovers: crate::app_signin::AppHandovers::default(),
            limits: Mutex::new(Limits::default()),
            mail_limits: Mutex::new(Limits::default()),
            sends: Mutex::new(std::collections::HashMap::new()),
            server_searches: Mutex::new(std::collections::HashMap::new()),
            jmap_limits: Mutex::new(Limits::default()),
            jmap_running: Arc::new(Mutex::new(std::collections::HashMap::new())),
            trusted_proxies: trusted_proxies.iter().map(IpAddr::to_canonical).collect(),
            clock,
            oidc: None,
        })
    }

    /// The same, with sign-in through an OpenID Connect provider (T25).
    pub fn with_oidc(mut self, provider: crate::oidc::Provider) -> Self {
        self.oidc = Some(Arc::new(provider));
        self
    }

    /// The Companion's public address (no trailing slash).
    pub(crate) fn public_url(&self) -> &str {
        &self.public_url
    }

    /// The master key (to open users' data keys).
    pub(crate) fn master(&self) -> &Key {
        &self.master
    }

    /// The system clock.
    pub fn system_clock() -> Clock {
        Arc::new(crate::store::now_ms)
    }

    /// The time now, by the Companion's clock (a test's own clock in tests).
    pub(crate) fn now(&self) -> i64 {
        (self.clock)()
    }

    /// The reverse proxies whose `X-Forwarded-For` is believed (canonical addresses).
    pub fn trusted_proxies(&self) -> &[IpAddr] {
        &self.trusted_proxies
    }

    /// The link a person opens to enrol.
    pub fn enrolment_link(&self, token: &str) -> Zeroizing<String> {
        enrolment_link(&self.public_url, token)
    }

    fn ceremonies(&self) -> std::sync::MutexGuard<'_, Ceremonies> {
        self.ceremonies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// Counts a mail request (discover, verify, add) against the user (30 a minute).
    pub(crate) fn mail_limit(&self, user_id: i64) -> Result<(), ApiError> {
        let key = (1u128 << 126) | u128::from(user_id.unsigned_abs());
        self.mail_limits
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .request_key(key, self.now())
            .map_err(|r| ApiError::RateLimited(r.retry_after_s))
    }

    /// Counts a send of a message to `recipients` addresses against the user's sending budget
    /// (T31b): at most [`SENDS_PER_MINUTE`] messages a minute, [`RECIPIENTS_PER_HOUR`] recipients
    /// an hour and [`RECIPIENTS_PER_DAY`] a day. False when over it (nothing is counted then). A
    /// stolen session cannot turn the account into a spam relay at full speed.
    pub(crate) fn send_budget(&self, user_id: i64, recipients: usize) -> bool {
        let now = self.now();
        let n = u32::try_from(recipients).unwrap_or(u32::MAX);
        let mut all = self.sends.lock().unwrap_or_else(PoisonError::into_inner);
        let sent = all.entry(user_id).or_default();
        while sent.front().is_some_and(|(t, _)| *t <= now - DAY_MS) {
            sent.pop_front();
        }
        let since = |ms: i64| sent.iter().filter(move |(t, _)| *t > now - ms);
        let messages_minute = since(60_000).count();
        let hour: u64 = since(3_600_000).map(|(_, r)| u64::from(*r)).sum();
        let day: u64 = sent.iter().map(|(_, r)| u64::from(*r)).sum();
        if messages_minute >= SENDS_PER_MINUTE
            || hour + u64::from(n) > RECIPIENTS_PER_HOUR
            || day + u64::from(n) > RECIPIENTS_PER_DAY
        {
            return false;
        }
        sent.push_back((now, n));
        true
    }

    /// Counts a search on the user's mail servers against the user (T33b): at most
    /// [`SERVER_SEARCHES_PER_MINUTE`]. Each one reaches out to every folder searched on the mail
    /// server, so a client cannot turn the Companion into a hammer on it. False when over.
    pub(crate) fn server_search_budget(&self, user_id: i64) -> bool {
        let now = self.now();
        let mut all = self
            .server_searches
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let done = all.entry(user_id).or_default();
        while done.front().is_some_and(|t| *t <= now - 60_000) {
            done.pop_front();
        }
        if done.len() >= SERVER_SEARCHES_PER_MINUTE {
            return false;
        }
        done.push_back(now);
        true
    }

    /// Counts a JMAP request against the user.
    pub(crate) fn jmap_limit(&self, user_id: i64) -> Result<(), ApiError> {
        let key = (1u128 << 125) | u128::from(user_id.unsigned_abs());
        self.jmap_limits
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .request_key_with(key, self.now(), jmap_api::JMAP_PER_MINUTE)
            .map_err(|r| ApiError::RateLimited(r.retry_after_s))
    }

    /// A place among the user's JMAP requests running at once; `None` when they are all taken.
    pub(crate) fn jmap_slot(&self, user_id: i64) -> Option<JmapSlot> {
        let mut running = self
            .jmap_running
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let n = running.entry(user_id).or_insert(0);
        if *n >= jmap_api::JMAP_CONCURRENT {
            return None;
        }
        *n += 1;
        Some(JmapSlot {
            running: self.jmap_running.clone(),
            user_id,
        })
    }

    fn limits(&self) -> std::sync::MutexGuard<'_, Limits> {
        self.limits.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The client's address: the connection's, or, from a trusted proxy, the last address in
    /// `X-Forwarded-For` that is not itself a trusted proxy. Mapped IPv4 addresses are IPv4.
    fn client(&self, peer: Option<Peer>, headers: &HeaderMap) -> IpAddr {
        let Some(Peer(addr)) = peer else {
            return IpAddr::V4(Ipv4Addr::UNSPECIFIED);
        };
        let mut ip = addr.ip().to_canonical();
        if !self.trusted_proxies.contains(&ip) {
            return ip;
        }
        let forwarded: Vec<&str> = headers
            .get_all("x-forwarded-for")
            .iter()
            .filter_map(|v| v.to_str().ok())
            .flat_map(|v| v.split(','))
            .collect();
        // From the right: each hop was added by the one after it. Stop at the first that is not a
        // trusted proxy, or at anything that does not parse (never skip it: what is left of it
        // was written by the client).
        for hop in forwarded.into_iter().rev() {
            let Ok(hop) = hop.trim().parse::<IpAddr>() else {
                break;
            };
            ip = hop.to_canonical();
            if !self.trusted_proxies.contains(&ip) {
                break;
            }
        }
        ip
    }
}

/// A running JMAP request of a user (freed when dropped).
pub struct JmapSlot {
    running: Arc<Mutex<std::collections::HashMap<i64, usize>>>,
    user_id: i64,
}

impl Drop for JmapSlot {
    fn drop(&mut self) {
        let mut running = self.running.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(n) = running.get_mut(&self.user_id) {
            *n -= 1;
            if *n == 0 {
                running.remove(&self.user_id);
            }
        }
    }
}

/// The link a person opens to enrol. The token is in the fragment, which browsers never send to a
/// server, so it stays out of proxy and access logs.
pub fn enrolment_link(public_url: &str, token: &str) -> Zeroizing<String> {
    Zeroizing::new(format!(
        "{}/enrol#{token}",
        public_url.trim_end_matches('/')
    ))
}

/// A refusal. The body is `{"error": "<code>"}`; no code says which part of a sign-in was wrong.
#[derive(Debug)]
pub enum ApiError {
    /// The request is malformed.
    BadRequest,
    /// No valid access token.
    Unauthorized,
    /// Signed in, but not allowed (a recovery session, or not an admin).
    Forbidden,
    /// Not found (or not yours).
    NotFound,
    /// The sign-in, link or ceremony did not check out.
    Invalid,
    /// Too many failed sign-ins for this user.
    Locked(i64),
    /// Too many requests from this address.
    RateLimited(i64),
    /// It conflicts with what exists (a passkey registered twice, the last passkey).
    Conflict,
    /// Something failed on the server (no detail is given).
    Server,
    /// An outside service (the OIDC provider) could not be reached; try later.
    Unavailable,
}

impl From<StoreError> for ApiError {
    fn from(e: StoreError) -> Self {
        match e {
            StoreError::Conflict | StoreError::LastPasskey => Self::Conflict,
            StoreError::NoSuchUser => Self::NotFound,
            _ => Self::Server,
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, code) = match self {
            Self::BadRequest => (StatusCode::BAD_REQUEST, "bad_request"),
            Self::Unauthorized => (StatusCode::UNAUTHORIZED, "unauthorized"),
            Self::Forbidden => (StatusCode::FORBIDDEN, "forbidden"),
            Self::NotFound => (StatusCode::NOT_FOUND, "not_found"),
            Self::Invalid => (StatusCode::UNAUTHORIZED, "invalid"),
            Self::Locked(until) => {
                return (
                    StatusCode::LOCKED,
                    Json(json!({"error": "locked", "until_ms": until})),
                )
                    .into_response()
            }
            Self::RateLimited(secs) => {
                let mut res = (
                    StatusCode::TOO_MANY_REQUESTS,
                    Json(json!({"error": "rate_limited"})),
                )
                    .into_response();
                if let Ok(v) = HeaderValue::from_str(&secs.to_string()) {
                    res.headers_mut().insert(header::RETRY_AFTER, v);
                }
                return res;
            }
            Self::Conflict => (StatusCode::CONFLICT, "conflict"),
            Self::Server => (StatusCode::INTERNAL_SERVER_ERROR, "server"),
            Self::Unavailable => (StatusCode::BAD_GATEWAY, "unavailable"),
        };
        let mut res = (status, Json(json!({ "error": code }))).into_response();
        if status == StatusCode::UNAUTHORIZED {
            res.headers_mut()
                .insert(header::WWW_AUTHENTICATE, HeaderValue::from_static("Bearer"));
        }
        res
    }
}

type ApiResult = Result<Response, ApiError>;

fn body<T>(r: Result<Json<T>, JsonRejection>) -> Result<T, ApiError> {
    r.map(|Json(v)| v).map_err(|_| ApiError::BadRequest)
}

/// Runs `f` on the store off the async threads.
async fn with_store<R: Send + 'static>(
    app: &App,
    f: impl FnOnce(&mut Store) -> R + Send + 'static,
) -> Result<R, ApiError> {
    let store = app.store.clone();
    tokio::task::spawn_blocking(move || {
        let mut s = store.lock().unwrap_or_else(PoisonError::into_inner);
        f(&mut s)
    })
    .await
    .map_err(|_| ApiError::Server)
}

/// Counts an unauthenticated request against the client's address.
/// Messages a user may send a minute (T31b).
pub const SENDS_PER_MINUTE: usize = 30;
/// Searches on a user's mail servers a minute (T33b).
pub const SERVER_SEARCHES_PER_MINUTE: usize = 10;
/// Recipients a user may send to in an hour (T31b).
pub const RECIPIENTS_PER_HOUR: u64 = 1000;
/// Recipients a user may send to in a day (T31b).
pub const RECIPIENTS_PER_DAY: u64 = 3000;
const DAY_MS: i64 = 24 * 60 * 60 * 1000;

fn limit(app: &App, peer: Option<Peer>, headers: &HeaderMap) -> Result<IpAddr, ApiError> {
    let ip = app.auth.client(peer, headers);
    app.auth
        .limits()
        .request(ip, app.auth.now())
        .map_err(|r| ApiError::RateLimited(r.retry_after_s))?;
    Ok(ip)
}

fn failed(app: &App, ip: IpAddr) {
    app.auth.limits().failed(ip, app.auth.now());
}

fn device_name(raw: Option<String>) -> String {
    let cleaned: String = raw
        .unwrap_or_default()
        .chars()
        .filter(|c| !c.is_control() && !is_format_char(*c))
        .take(MAX_DEVICE)
        .collect();
    let cleaned = cleaned.trim();
    if cleaned.is_empty() {
        "Unnamed device".to_owned()
    } else {
        cleaned.to_owned()
    }
}

/// Invisible formatting characters (bidirectional overrides, zero-width ones, the soft hyphen, the
/// byte order mark): they could make one device's name look like another's.
fn is_format_char(c: char) -> bool {
    matches!(c,
        '\u{00AD}' | '\u{061C}' | '\u{180E}' | '\u{200B}'..='\u{200F}' | '\u{202A}'..='\u{202E}'
        | '\u{2060}'..='\u{206F}' | '\u{FEFF}' | '\u{FFF9}'..='\u{FFFB}')
}

fn tokens_json(tokens: &Tokens, recovery: bool) -> Response {
    // Apps: both tokens in the body. The web app's sign-ins go through `web_api::tokens_response`.
    Json(json!({
        "access_token": tokens.access.as_str(),
        "refresh_token": tokens.refresh.as_str(),
        "access_expires_ms": tokens.access_expires_ms,
        "recovery": recovery,
    }))
    .into_response()
}

fn ok() -> Response {
    Json(json!({"ok": true})).into_response()
}

fn role_str(r: Role) -> &'static str {
    r.as_str()
}

// ---- Who is calling ----

fn bearer(parts: &Parts) -> Option<Zeroizing<String>> {
    let v = parts.headers.get(header::AUTHORIZATION)?.to_str().ok()?;
    let (scheme, token) = v.split_once(' ')?;
    scheme
        .eq_ignore_ascii_case("bearer")
        .then(|| Zeroizing::new(token.trim().to_owned()))
}

async fn caller(parts: &Parts, app: &App) -> Result<Caller, ApiError> {
    let token = bearer(parts).ok_or(ApiError::Unauthorized)?;
    let now = app.auth.now();
    with_store(app, move |s| s.authenticate(&token, now))
        .await??
        .ok_or(ApiError::Unauthorized)
}

/// Any signed-in caller, a recovery session included (it may only register a passkey).
pub struct Signed(pub Caller);
/// A signed-in caller with a full session.
pub struct Full(pub Caller);
/// An admin with a full session.
pub struct Admin(pub Caller);

impl FromRequestParts<App> for Signed {
    type Rejection = ApiError;
    async fn from_request_parts(parts: &mut Parts, app: &App) -> Result<Self, ApiError> {
        Ok(Self(caller(parts, app).await?))
    }
}

impl FromRequestParts<App> for Full {
    type Rejection = ApiError;
    async fn from_request_parts(parts: &mut Parts, app: &App) -> Result<Self, ApiError> {
        let c = caller(parts, app).await?;
        if c.recovery {
            return Err(ApiError::Forbidden);
        }
        Ok(Self(c))
    }
}

impl FromRequestParts<App> for Admin {
    type Rejection = ApiError;
    async fn from_request_parts(parts: &mut Parts, app: &App) -> Result<Self, ApiError> {
        let c = caller(parts, app).await?;
        if c.recovery || c.role != Role::Admin {
            return Err(ApiError::Forbidden);
        }
        Ok(Self(c))
    }
}

/// The connection's address, if the server loop put it there.
pub struct MaybePeer(pub Option<Peer>);

impl<S: Send + Sync> FromRequestParts<S> for MaybePeer {
    type Rejection = std::convert::Infallible;
    async fn from_request_parts(parts: &mut Parts, _: &S) -> Result<Self, Self::Rejection> {
        Ok(Self(parts.extensions.get::<Peer>().copied()))
    }
}

// ---- The route table ----

/// Who may call a route.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Access {
    /// Anyone (rate limited per address where it signs in).
    Public,
    /// A signed-in user, recovery sessions included.
    Signed,
    /// A signed-in user with a full session.
    Full,
    /// An admin.
    Admin,
}

/// One route.
pub struct Route {
    /// HTTP method.
    pub method: Method,
    /// Path (axum syntax).
    pub path: &'static str,
    /// Who may call it.
    pub access: Access,
    /// The handler.
    pub handler: MethodRouter<App>,
}

fn route(method: Method, path: &'static str, access: Access, handler: MethodRouter<App>) -> Route {
    Route {
        method,
        path,
        access,
        handler,
    }
}

/// Every sign-in route. Each path appears once.
pub fn routes() -> Vec<Route> {
    use Access::{Admin as A, Full as F, Public as P, Signed as S};
    vec![
        route(Method::GET, "/enrol", P, get(crate::enrol_page::page)),
        route(Method::GET, "/enrol.js", P, get(crate::enrol_page::script)),
        route(Method::GET, "/enrol.css", P, get(crate::enrol_page::style)),
        route(Method::POST, "/api/enrol/start", P, post(enrol_start)),
        route(Method::POST, "/api/enrol/finish", P, post(enrol_finish)),
        route(
            Method::POST,
            "/api/sign-in/passkey/start",
            P,
            post(sign_in_start),
        ),
        route(
            Method::POST,
            "/api/sign-in/passkey/finish",
            P,
            post(sign_in_finish),
        ),
        route(
            Method::POST,
            "/api/sign-in/recovery",
            P,
            post(sign_in_recovery),
        ),
        route(Method::POST, "/api/token/refresh", P, post(refresh)),
        route(
            Method::POST,
            "/api/sign-in/app/exchange",
            P,
            post(app_exchange),
        ),
        route(Method::POST, "/api/web/refresh", P, post(web_api::refresh)),
        route(
            Method::POST,
            "/api/web/sign-out",
            P,
            post(web_api::sign_out),
        ),
        route(Method::GET, "/api/me", S, get(me)),
        route(Method::POST, "/api/sign-out", S, post(sign_out)),
        route(Method::POST, "/api/passkeys/start", S, post(passkey_start)),
        route(
            Method::POST,
            "/api/passkeys/finish",
            S,
            post(passkey_finish),
        ),
        route(Method::GET, "/api/passkeys", F, get(passkey_list)),
        route(
            Method::DELETE,
            "/api/passkeys/{id}",
            F,
            delete(passkey_delete),
        ),
        route(Method::GET, "/api/sessions", F, get(session_list)),
        route(
            Method::DELETE,
            "/api/sessions/{id}",
            F,
            delete(session_delete),
        ),
        route(Method::GET, "/api/audit", F, get(own_audit)),
        route(Method::GET, "/api/admin/users", A, get(admin_users)),
        route(
            Method::POST,
            "/api/admin/users/{id}/disable",
            A,
            post(admin_disable),
        ),
        route(
            Method::POST,
            "/api/admin/users/{id}/enable",
            A,
            post(admin_enable),
        ),
        route(Method::GET, "/api/admin/audit", A, get(admin_audit)),
        route(Method::GET, "/api/oidc", P, get(oidc_api::info)),
        route(
            Method::POST,
            "/api/accounts/discover",
            F,
            post(mail_api::discover),
        ),
        route(
            Method::POST,
            "/api/accounts/verify",
            F,
            post(mail_api::verify),
        ),
        route(Method::POST, "/api/accounts", F, post(mail_api::add)),
        route(Method::GET, "/jmap/session", F, get(jmap_api::session)),
        route(Method::GET, "/api/push/vapid", F, get(push_api::vapid)),
        route(
            Method::POST,
            "/api/push/subscriptions",
            F,
            post(push_api::subscribe),
        ),
        route(
            Method::GET,
            "/api/push/subscriptions",
            F,
            get(push_api::list),
        ),
        route(
            Method::DELETE,
            "/api/push/subscriptions/{id}",
            F,
            delete(push_api::remove),
        ),
        route(Method::POST, "/jmap/api", F, post(jmap_api::api)),
        route(Method::GET, "/api/accounts", F, get(mail_api::list)),
        route(
            Method::DELETE,
            "/api/accounts/{id}",
            F,
            delete(mail_api::remove),
        ),
        route(
            Method::GET,
            "/api/sign-in/oidc/start",
            P,
            get(oidc_api::sign_in_start),
        ),
        route(Method::GET, "/oidc/callback", P, get(oidc_api::callback)),
        route(
            Method::POST,
            "/api/sign-in/oidc/exchange",
            P,
            post(oidc_api::exchange),
        ),
        route(
            Method::POST,
            "/api/oidc/link/start",
            F,
            post(oidc_api::link_start),
        ),
        route(
            Method::POST,
            "/api/oidc/link/finish",
            F,
            post(oidc_api::link_finish),
        ),
        route(
            Method::DELETE,
            "/api/oidc/link",
            F,
            delete(oidc_api::unlink),
        ),
        route(
            Method::POST,
            "/api/admin/users/{id}/oidc/unlink",
            A,
            post(oidc_api::admin_unlink),
        ),
    ]
}

// ---- Enrolment ----

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EnrolStart {
    token: String,
}

/// The creation options with a discoverable credential preferred (so the passkey can sign in
/// without typing a name). The server's state does not depend on this field.
fn creation_options(
    ccr: &webauthn_rs::prelude::CreationChallengeResponse,
) -> Result<Value, ApiError> {
    let mut v = serde_json::to_value(ccr).map_err(|_| ApiError::Server)?;
    if let Some(sel) = v
        .pointer_mut("/publicKey/authenticatorSelection")
        .and_then(Value::as_object_mut)
    {
        sel.insert("residentKey".into(), json!("preferred"));
    }
    Ok(v)
}

async fn enrol_start(
    State(app): State<App>,
    MaybePeer(peer): MaybePeer,
    headers: HeaderMap,
    req: Result<Json<EnrolStart>, JsonRejection>,
) -> ApiResult {
    let ip = limit(&app, peer, &headers)?;
    let req = body(req)?;
    let token = Zeroizing::new(req.token);
    let now = app.auth.now();
    let t = token.clone();
    let Some(target) = with_store(&app, move |s| s.enrolment_target(&t, now)).await?? else {
        failed(&app, ip);
        return Err(ApiError::Invalid);
    };
    let (ccr, state) = app
        .auth
        .webauthn
        .start_passkey_registration(target.handle, &target.name, &target.name, None)
        .map_err(|_| ApiError::Server)?;
    let options = creation_options(&ccr)?;
    let id = app.auth.ceremonies().put(
        Owner::Address(limits::who(ip)),
        Pending::Enrol {
            user_id: target.user_id,
            link_hash: auth::token_hash(&token),
            state,
        },
        now,
    )?;
    Ok(
        Json(json!({"ceremony": id.as_str(), "user": target.name, "options": options}))
            .into_response(),
    )
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EnrolFinish {
    token: String,
    ceremony: String,
    credential: RegisterPublicKeyCredential,
}

/// The QR code of the `otpauth://` URI as rows of `0` and `1`, drawn by the page on a canvas.
fn qr_rows(uri: &str) -> Result<Vec<String>, ApiError> {
    let code = qrcode::QrCode::new(uri.as_bytes()).map_err(|_| ApiError::Server)?;
    let width = code.width();
    let colors = code.to_colors();
    Ok(colors
        .chunks(width)
        .map(|row| {
            row.iter()
                .map(|c| if *c == qrcode::Color::Dark { '1' } else { '0' })
                .collect()
        })
        .collect())
}

async fn enrol_finish(
    State(app): State<App>,
    MaybePeer(peer): MaybePeer,
    headers: HeaderMap,
    req: Result<Json<EnrolFinish>, JsonRejection>,
) -> ApiResult {
    let ip = limit(&app, peer, &headers)?;
    let req = body(req)?;
    let token = Zeroizing::new(req.token);
    let now = app.auth.now();
    let Some(Pending::Enrol {
        user_id,
        link_hash,
        state,
    }) = app.auth.ceremonies().take(&req.ceremony, now)
    else {
        failed(&app, ip);
        return Err(ApiError::Invalid);
    };
    if !crypto::ct_eq(&link_hash, &auth::token_hash(&token)) {
        failed(&app, ip);
        return Err(ApiError::Invalid);
    }
    let Ok(passkey) = app
        .auth
        .webauthn
        .finish_passkey_registration(&req.credential, &state)
    else {
        failed(&app, ip);
        return Err(ApiError::Invalid);
    };
    let auth_ = app.auth.clone();
    let user = with_store(&app, move |s| {
        let done = s.finish_enrolment(&auth_.master, &token, user_id, &passkey, now)?;
        let name = s.user_by_id(user_id)?.map(|u| u.name);
        Ok::<_, StoreError>(done.zip(name))
    })
    .await??;
    let Some((enrolled, name)) = user else {
        failed(&app, ip);
        return Err(ApiError::Invalid);
    };
    let uri = totp::otpauth_uri("Croft Companion", &name, &enrolled.totp_secret);
    let codes: Vec<&str> = enrolled.recovery_codes.iter().map(|c| c.as_str()).collect();
    Ok(Json(json!({
        "user": name,
        "totp": {
            "secret": totp::base32(&enrolled.totp_secret).as_str(),
            "uri": uri.as_str(),
            "qr": qr_rows(&uri)?,
        },
        "recovery_codes": codes,
    }))
    .into_response())
}

// ---- Sign-in ----

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SignInStart {
    user: Option<String>,
}

async fn sign_in_start(
    State(app): State<App>,
    MaybePeer(peer): MaybePeer,
    headers: HeaderMap,
    req: Result<Json<SignInStart>, JsonRejection>,
) -> ApiResult {
    let ip = limit(&app, peer, &headers)?;
    let req = body(req)?;
    let now = app.auth.now();
    // A named user with passkeys gets their credential ids listed; anything else (no name, an
    // unknown or disabled name, no passkeys) gets a discoverable challenge. This tells a known name
    // from an unknown one, which is accepted (DECISIONS): names are not secret on a household
    // server, and a passkey that is not discoverable needs its id listed.
    let named = match req.user {
        Some(name) => {
            with_store(&app, move |s| {
                let Some(u) = s.user_by_name(&name)? else {
                    return Ok(None);
                };
                if u.disabled {
                    return Ok(None);
                }
                let ids: Vec<_> = s
                    .passkeys(u.id)?
                    .into_iter()
                    .map(|(_, p)| p.cred_id().clone())
                    .collect();
                Ok::<_, StoreError>((!ids.is_empty()).then_some((u.id, ids)))
            })
            .await??
        }
        None => None,
    };
    let (rcr, state) = app
        .auth
        .webauthn
        .start_discoverable_authentication()
        .map_err(|_| ApiError::Server)?;
    let mut options = serde_json::to_value(&rcr).map_err(|_| ApiError::Server)?;
    let user_id = match named {
        Some((user_id, ids)) => {
            let allow: Vec<Value> = ids
                .iter()
                .map(|id| json!({"type": "public-key", "id": b64url(id.as_slice())}))
                .collect();
            if let Some(o) = options.as_object_mut() {
                o.remove("mediation");
            }
            if let Some(pk) = options.get_mut("publicKey").and_then(Value::as_object_mut) {
                pk.insert("allowCredentials".into(), Value::Array(allow));
            }
            Some(user_id)
        }
        None => None,
    };
    let id = app.auth.ceremonies().put(
        Owner::Address(limits::who(ip)),
        Pending::SignIn { user_id, state },
        now,
    )?;
    Ok(Json(json!({"ceremony": id.as_str(), "options": options})).into_response())
}

fn b64url(bytes: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SignInFinish {
    ceremony: String,
    credential: PublicKeyCredential,
    device: Option<String>,
    /// The web app's sign-in (T30): the refresh token goes into a cookie.
    #[serde(default)]
    web: bool,
    /// A phone's sign-in through `/app/sign-in` (T34a): no session here, a hand-over code for the
    /// app instead.
    app: Option<AppTarget>,
}

/// The app a passkey sign-in in the browser is for (T34a).
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AppTarget {
    /// The app's PKCE challenge (S256).
    challenge: String,
    /// The app's redirect: one of [`crate::app_signin::APP_SCHEMES`].
    redirect: String,
}

/// After a failed passkey check: count it for the address and audit it for the user. It never
/// counts toward the user's lock: a passkey answer cannot be guessed, and counting it would let
/// anyone who knows a name lock that person out of their own passkey.
async fn passkey_failed(app: &App, ip: IpAddr, user_id: Option<i64>) -> ApiError {
    failed(app, ip);
    let now = app.auth.now();
    match with_store(app, move |s| s.record_failure(user_id, now, false)).await {
        Ok(Ok(_)) => ApiError::Invalid,
        _ => ApiError::Server,
    }
}

async fn start_session(
    app: &App,
    user_id: i64,
    device: String,
    recovery: bool,
    web: bool,
) -> ApiResult {
    let now = app.auth.now();
    match with_store(app, move |s| {
        s.start_session(user_id, &device, recovery, now)
    })
    .await?
    {
        Ok(tokens) => Ok(web_api::tokens_response(app, &tokens, recovery, web)),
        Err(SignInError::Invalid) => Err(ApiError::Invalid),
        Err(SignInError::Locked { until_ms }) => Err(ApiError::Locked(until_ms)),
        Err(SignInError::Store(e)) => Err(e.into()),
    }
}

/// How a passkey check ended.
enum Checked {
    /// The passkey is the user's and the answer is good (the counter is stored).
    Good(i64),
    /// Not good; the user, when known, gets the failure audited.
    Bad(Option<i64>),
}

async fn sign_in_finish(
    State(app): State<App>,
    MaybePeer(peer): MaybePeer,
    headers: HeaderMap,
    req: Result<Json<SignInFinish>, JsonRejection>,
) -> ApiResult {
    let ip = limit(&app, peer, &headers)?;
    let req = body(req)?;
    web_api::check_web(&app, &headers, req.web)?;
    if let Some(a) = &req.app {
        // An app's sign-in never makes a browser session, and goes only to the Croft app.
        if req.web
            || !crate::app_signin::challenge_ok(&a.challenge)
            || !crate::app_signin::is_app_redirect(&a.redirect)
        {
            return Err(ApiError::Invalid);
        }
    }
    let now = app.auth.now();
    let Some(Pending::SignIn {
        user_id: named,
        state,
    }) = app.auth.ceremonies().take(&req.ceremony, now)
    else {
        failed(&app, ip);
        return Err(ApiError::Invalid);
    };
    let credential = req.credential;
    let auth_ = app.auth.clone();
    // One store lock around finding the passkey, checking the answer and storing the new counter:
    // two answers racing for one passkey are checked one after the other.
    let checked = with_store(&app, move |s| {
        // The user handle, when the passkey sends one, names the user; when a user was named at
        // the start, the handle must name the same one.
        let by_handle = match auth_
            .webauthn
            .identify_discoverable_authentication(&credential)
        {
            Ok((handle, _)) => match s.user_for_handle(&handle)? {
                Some(id) => Some(id),
                None => return Ok(Checked::Bad(named)),
            },
            Err(_) => None,
        };
        let user_id = match (named, by_handle) {
            (Some(n), Some(h)) if n != h => return Ok(Checked::Bad(Some(n))),
            (Some(n), _) => n,
            (None, Some(h)) => h,
            (None, None) => return Ok(Checked::Bad(None)),
        };
        // A lock (from failed recovery attempts) does not stop a valid passkey.
        let Some((row_id, mut passkey)) =
            s.passkey_by_credential(user_id, credential.get_credential_id())?
        else {
            return Ok(Checked::Bad(Some(user_id)));
        };
        let Ok(result) =
            auth_
                .webauthn
                .finish_discoverable_authentication_v2(&credential, state, &passkey)
        else {
            return Ok(Checked::Bad(Some(user_id)));
        };
        passkey.update_credential(&result);
        s.passkey_used(user_id, row_id, &passkey, now)?;
        Ok::<_, StoreError>(Checked::Good(user_id))
    })
    .await??;
    match checked {
        Checked::Good(user_id) => match req.app {
            Some(a) => {
                let code = app.auth.app_handovers.put(user_id, &a.challenge, now)?;
                let _ = with_store(&app, move |s| {
                    s.audit(Some(user_id), "sign_in.passkey.app_handover", "ok")
                })
                .await;
                Ok(
                    Json(json!({ "redirect": format!("{}?code={}", a.redirect, code.as_str()) }))
                        .into_response(),
                )
            }
            None => start_session(&app, user_id, device_name(req.device), false, req.web).await,
        },
        Checked::Bad(user_id) => Err(passkey_failed(&app, ip, user_id).await),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AppExchange {
    code: String,
    verifier: String,
    device: Option<String>,
}

/// `POST /api/sign-in/app/exchange` (T34a): a phone's hand-over code and its PKCE verifier for the
/// app's own tokens (a new device the person sees and can sign out).
async fn app_exchange(
    State(app): State<App>,
    MaybePeer(peer): MaybePeer,
    headers: HeaderMap,
    req: Result<Json<AppExchange>, JsonRejection>,
) -> ApiResult {
    let ip = limit(&app, peer, &headers)?;
    let req = body(req)?;
    let (code, verifier) = (
        zeroize::Zeroizing::new(req.code),
        zeroize::Zeroizing::new(req.verifier),
    );
    let Some(user_id) = app
        .auth
        .app_handovers
        .redeem(&code, &verifier, app.auth.now())
    else {
        failed(&app, ip);
        return Err(ApiError::Invalid);
    };
    start_session(&app, user_id, device_name(req.device), false, false).await
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Recovery {
    user: String,
    totp: String,
    recovery_code: String,
    device: Option<String>,
    /// The web app's sign-in (T30).
    #[serde(default)]
    web: bool,
}

/// How a recovery sign-in ended.
enum Recovered {
    Tokens(Tokens),
    Invalid,
    Locked(i64),
}

async fn sign_in_recovery(
    State(app): State<App>,
    MaybePeer(peer): MaybePeer,
    headers: HeaderMap,
    req: Result<Json<Recovery>, JsonRejection>,
) -> ApiResult {
    let ip = limit(&app, peer, &headers)?;
    let req = body(req)?;
    web_api::check_web(&app, &headers, req.web)?;
    let web = req.web;
    let now = app.auth.now();
    let name = req.user;
    let device = device_name(req.device);
    let (totp_code, code) = (Zeroizing::new(req.totp), Zeroizing::new(req.recovery_code));
    let auth_ = app.auth.clone();
    // One store lock around the lock check, the check itself, the failure count and the new
    // session: a burst of guesses is checked one at a time, and the eleventh never is.
    let outcome = with_store(&app, move |s| {
        let Some(user) = s.user_by_name(&name)?.filter(|u| !u.disabled) else {
            s.record_failure(None, now, false)?;
            return Ok(Recovered::Invalid);
        };
        if let Some(until) = s.locked_until(user.id, now)? {
            return Ok(Recovered::Locked(until));
        }
        if !s.check_recovery(&auth_.master, user.id, &totp_code, &code, now)? {
            return Ok(match s.record_failure(Some(user.id), now, true)? {
                Some(until) => Recovered::Locked(until),
                None => Recovered::Invalid,
            });
        }
        match s.start_session(user.id, &device, true, now) {
            Ok(tokens) => Ok(Recovered::Tokens(tokens)),
            Err(SignInError::Invalid) => Ok(Recovered::Invalid),
            Err(SignInError::Locked { until_ms }) => Ok(Recovered::Locked(until_ms)),
            Err(SignInError::Store(e)) => Err(e),
        }
    })
    .await??;
    match outcome {
        Recovered::Tokens(tokens) => Ok(web_api::tokens_response(&app, &tokens, true, web)),
        Recovered::Locked(until) => {
            failed(&app, ip);
            Err(ApiError::Locked(until))
        }
        Recovered::Invalid => {
            failed(&app, ip);
            Err(ApiError::Invalid)
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Refresh {
    refresh_token: String,
}

async fn refresh(
    State(app): State<App>,
    MaybePeer(peer): MaybePeer,
    headers: HeaderMap,
    req: Result<Json<Refresh>, JsonRejection>,
) -> ApiResult {
    let ip = limit(&app, peer, &headers)?;
    let token = Zeroizing::new(body(req)?.refresh_token);
    let now = app.auth.now();
    match with_store(&app, move |s| s.refresh(&token, now)).await? {
        Ok(tokens) => Ok(tokens_json(&tokens, false)),
        Err(RefreshError::Invalid | RefreshError::Reused) => {
            failed(&app, ip);
            Err(ApiError::Invalid)
        }
        Err(RefreshError::Store(e)) => Err(e.into()),
    }
}

// ---- The signed-in user ----

async fn me(State(app): State<App>, Signed(c): Signed) -> ApiResult {
    let issuer = app.auth.oidc.as_ref().map(|p| p.settings().issuer.clone());
    let (user, linked) = with_store(&app, move |s| {
        let user = s.user_by_id(c.user_id)?;
        let linked = match &issuer {
            Some(i) => s.has_identity(c.user_id, i)?,
            None => false,
        };
        Ok::<_, StoreError>((user, linked))
    })
    .await??;
    let user = user.ok_or(ApiError::Unauthorized)?;
    Ok(Json(json!({
        "id": user.id,
        "name": user.name,
        "role": role_str(user.role),
        "recovery": c.recovery,
        "oidc_linked": linked,
    }))
    .into_response())
}

async fn sign_out(State(app): State<App>, Signed(c): Signed) -> ApiResult {
    let now = app.auth.now();
    with_store(&app, move |s| {
        s.revoke_session(c.user_id, c.session_id, now)
    })
    .await??;
    Ok(ok())
}

async fn passkey_start(
    State(app): State<App>,
    MaybePeer(peer): MaybePeer,
    headers: HeaderMap,
    Signed(c): Signed,
) -> ApiResult {
    limit(&app, peer, &headers)?;
    let now = app.auth.now();
    let (handle, name, existing) = with_store(&app, move |s| {
        let name = s
            .user_by_id(c.user_id)?
            .map(|u| u.name)
            .ok_or(StoreError::NoSuchUser)?;
        let handle = s.user_handle(c.user_id)?;
        let existing: Vec<_> = s
            .passkeys(c.user_id)?
            .into_iter()
            .map(|(_, p)| p.cred_id().clone())
            .collect();
        Ok::<_, StoreError>((handle, name, existing))
    })
    .await??;
    let (ccr, state) = app
        .auth
        .webauthn
        .start_passkey_registration(handle, &name, &name, Some(existing))
        .map_err(|_| ApiError::Server)?;
    let options = creation_options(&ccr)?;
    let id = app.auth.ceremonies().put(
        Owner::User(c.user_id),
        Pending::AddPasskey {
            user_id: c.user_id,
            session_id: c.session_id,
            state,
        },
        now,
    )?;
    Ok(Json(json!({"ceremony": id.as_str(), "options": options})).into_response())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PasskeyFinish {
    ceremony: String,
    credential: RegisterPublicKeyCredential,
}

async fn passkey_finish(
    State(app): State<App>,
    Signed(c): Signed,
    req: Result<Json<PasskeyFinish>, JsonRejection>,
) -> ApiResult {
    let req = body(req)?;
    let now = app.auth.now();
    // Taken whoever asks, so a ceremony id that leaked is spent by the first try.
    let Some(Pending::AddPasskey {
        user_id,
        session_id,
        state,
    }) = app.auth.ceremonies().take(&req.ceremony, now)
    else {
        return Err(ApiError::Invalid);
    };
    if user_id != c.user_id || session_id != c.session_id {
        return Err(ApiError::Invalid);
    }
    let passkey = app
        .auth
        .webauthn
        .finish_passkey_registration(&req.credential, &state)
        .map_err(|_| ApiError::Invalid)?;
    let added = with_store(&app, move |s| s.add_passkey(&c, &passkey, now)).await??;
    if !added {
        return Err(ApiError::Unauthorized);
    }
    Ok(ok())
}

async fn passkey_list(State(app): State<App>, Full(c): Full) -> ApiResult {
    let list = with_store(&app, move |s| s.passkey_list(c.user_id)).await??;
    let items: Vec<Value> = list
        .iter()
        .map(|p| json!({"id": p.id, "created_ms": p.created_ms, "last_used_ms": p.last_used_ms}))
        .collect();
    Ok(Json(json!({ "passkeys": items })).into_response())
}

async fn passkey_delete(
    State(app): State<App>,
    Full(c): Full,
    id: Result<Path<i64>, axum::extract::rejection::PathRejection>,
) -> ApiResult {
    let Path(id) = id.map_err(|_| ApiError::NotFound)?;
    let now = app.auth.now();
    if with_store(&app, move |s| s.delete_passkey(c.user_id, id, now)).await?? {
        Ok(ok())
    } else {
        Err(ApiError::NotFound)
    }
}

async fn session_list(State(app): State<App>, Full(c): Full) -> ApiResult {
    let list = with_store(&app, move |s| s.sessions(c.user_id)).await??;
    let items: Vec<Value> = list
        .iter()
        .map(|s| {
            json!({
                "id": s.id,
                "device": s.device,
                "created_ms": s.created_ms,
                "last_seen_ms": s.last_seen_ms,
                "current": s.id == c.session_id,
            })
        })
        .collect();
    Ok(Json(json!({ "sessions": items })).into_response())
}

async fn session_delete(
    State(app): State<App>,
    Full(c): Full,
    id: Result<Path<i64>, axum::extract::rejection::PathRejection>,
) -> ApiResult {
    let Path(id) = id.map_err(|_| ApiError::NotFound)?;
    let now = app.auth.now();
    if with_store(&app, move |s| s.revoke_session(c.user_id, id, now)).await?? {
        Ok(ok())
    } else {
        Err(ApiError::NotFound)
    }
}

fn audit_json(entries: &[auth::AuditEntry]) -> Response {
    let items: Vec<Value> = entries
        .iter()
        .map(|e| {
            json!({
                "at_ms": e.at_ms,
                "user_id": e.user_id,
                "by_admin": e.actor_id,
                "action": e.action,
                "outcome": e.outcome,
            })
        })
        .collect();
    Json(json!({ "entries": items })).into_response()
}

async fn own_audit(State(app): State<App>, Full(c): Full) -> ApiResult {
    let entries =
        with_store(&app, move |s| s.audit_entries(Some(c.user_id), AUDIT_LIMIT)).await??;
    Ok(audit_json(&entries))
}

// ---- Admin ----

async fn admin_users(State(app): State<App>, Admin(_): Admin) -> ApiResult {
    let issuer = app.auth.oidc.as_ref().map(|p| p.settings().issuer.clone());
    let (users, linked) = with_store(&app, move |s| {
        let linked = match &issuer {
            Some(i) => s.users_with_identity(i)?,
            None => Default::default(),
        };
        Ok::<_, StoreError>((s.users()?, linked))
    })
    .await??;
    let items: Vec<Value> = users
        .iter()
        .map(|u| {
            json!({
                "id": u.id,
                "name": u.name,
                "role": role_str(u.role),
                "created_ms": u.created_ms,
                "disabled": u.disabled,
                "oidc_linked": linked.contains(&u.id),
            })
        })
        .collect();
    Ok(Json(json!({ "users": items })).into_response())
}

async fn admin_set_disabled(app: &App, actor: Caller, id: i64, disabled: bool) -> ApiResult {
    if id == actor.user_id {
        // An admin cannot lock themselves out (or re-enable themselves, which is meaningless).
        return Err(ApiError::Conflict);
    }
    let now = app.auth.now();
    if with_store(app, move |s| {
        s.set_disabled(actor.user_id, id, disabled, now)
    })
    .await??
    {
        // A disabled user's mail stops syncing at once (T26); it starts again on next use.
        if disabled {
            if let Some(mail) = app.mail.clone() {
                let _ = tokio::task::spawn_blocking(move || mail.close(id)).await;
            }
        }
        Ok(ok())
    } else {
        Err(ApiError::NotFound)
    }
}

async fn admin_disable(
    State(app): State<App>,
    Admin(c): Admin,
    id: Result<Path<i64>, axum::extract::rejection::PathRejection>,
) -> ApiResult {
    let Path(id) = id.map_err(|_| ApiError::NotFound)?;
    admin_set_disabled(&app, c, id, true).await
}

async fn admin_enable(
    State(app): State<App>,
    Admin(c): Admin,
    id: Result<Path<i64>, axum::extract::rejection::PathRejection>,
) -> ApiResult {
    let Path(id) = id.map_err(|_| ApiError::NotFound)?;
    admin_set_disabled(&app, c, id, false).await
}

async fn admin_audit(State(app): State<App>, Admin(_): Admin) -> ApiResult {
    let entries = with_store(&app, |s| s.audit_entries(None, AUDIT_LIMIT)).await??;
    Ok(audit_json(&entries))
}

#[path = "oidc_api.rs"]
pub(crate) mod oidc_api;

#[path = "mail_api.rs"]
pub(crate) mod mail_api;

#[path = "jmap_api.rs"]
pub(crate) mod jmap_api;

#[path = "push_api.rs"]
pub(crate) mod push_api;

#[path = "web_api.rs"]
pub(crate) mod web_api;

#[cfg(test)]
#[path = "api_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "oidc_tests.rs"]
mod oidc_tests;

#[cfg(test)]
#[path = "mail_tests.rs"]
mod mail_tests;

#[cfg(test)]
#[path = "jmap_tests.rs"]
mod jmap_tests;

#[cfg(test)]
#[path = "push_tests.rs"]
mod push_tests;
