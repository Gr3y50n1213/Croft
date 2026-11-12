// This Source Code Form is subject to the terms of the GNU Affero General Public License,
// version 3 or later. See LICENSE-AGPL-3.0 at the repository root.

//! The gateway's HTTP side: `GET /health`, `POST /register` (an app's FCM token → its Web Push
//! endpoint) and `POST /wp/{sealed}` (a Companion's Web Push message → FCM). Plain HTTP behind a
//! reverse proxy, with the Companion's protections against slow and greedy clients. Nothing about
//! a request (token, endpoint, body, address) is logged: only counts.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use axum::body::{Body, Bytes};
use axum::extract::{DefaultBodyLimit, Path, State};
use axum::http::{header, HeaderMap, HeaderValue, Request, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;

use crate::config::{Config, Network};
use crate::fcm::{now_ms, Delivery, Fcm, Message};
use crate::limits::{self, Limits};
use crate::seal::{self, GatewayKey};

/// The largest Web Push body forwarded (FCM's data limit is 4 KB once encoded).
pub const MAX_PUSH_BODY: usize = 2048;
/// The largest request read.
const MAX_REQUEST: usize = 8 * 1024;
/// The longest `TTL` passed on (one day).
pub const MAX_TTL_S: u32 = 24 * 3600;
/// Longest a request may take, reading its body included.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
/// Longest a client may take to send its headers (and an idle kept-alive connection stays).
const HEADER_READ_TIMEOUT: Duration = Duration::from_secs(10);
/// Longest a connection is kept.
const MAX_CONNECTION_AGE: Duration = Duration::from_secs(120);
/// Connections served at once.
const MAX_CONNECTIONS: usize = 1024;
/// Connections at once from one address (a trusted proxy is not limited).
const MAX_PER_ADDRESS: usize = 64;

/// The client's address, set by the server loop.
#[derive(Debug, Clone, Copy)]
pub struct Peer(pub SocketAddr);

/// What happened, counted (the only thing logged).
#[derive(Debug, Default)]
pub struct Counts {
    /// Endpoints made.
    pub registered: AtomicU64,
    /// Messages FCM took.
    pub sent: AtomicU64,
    /// Messages for a token that is no more.
    pub gone: AtomicU64,
    /// Messages FCM did not take.
    pub failed: AtomicU64,
    /// Requests refused by a limit.
    pub limited: AtomicU64,
}

/// What the handlers share.
#[derive(Clone)]
pub struct App {
    key: Arc<GatewayKey>,
    fcm: Arc<Fcm>,
    public_url: Arc<str>,
    /// The gateway's origin: the `aud` a bound endpoint's VAPID JWT must name.
    origin: Arc<str>,
    trusted_proxies: Arc<[Network]>,
    push_limits: Arc<Mutex<Limits>>,
    address_limits: Arc<Mutex<Limits>>,
    total_limits: Arc<Mutex<Limits>>,
    register_limits: Arc<Mutex<Limits>>,
    gone: Arc<Mutex<Gone>>,
    /// The counts.
    pub counts: Arc<Counts>,
}

impl App {
    /// The state from the settings (already checked by [`Config::load`]), the key and FCM.
    pub fn new(config: &Config, key: GatewayKey, fcm: Fcm) -> Self {
        let limits = |per_hour: u32, burst: u32| Arc::new(Mutex::new(Limits::new(per_hour, burst)));
        Self {
            key: Arc::new(key),
            fcm: Arc::new(fcm),
            public_url: Arc::from(config.public_url.as_str()),
            origin: Arc::from(crate::vapid::origin(&config.public_url).as_str()),
            trusted_proxies: Arc::from(config.proxies().unwrap_or_default().as_slice()),
            push_limits: limits(config.push_per_hour, config.push_burst),
            address_limits: limits(
                config.push_per_address_per_hour,
                (config.push_per_address_per_hour / 60).max(10),
            ),
            total_limits: limits(
                config.total_per_hour,
                (config.total_per_hour / 1800).max(10),
            ),
            register_limits: limits(config.register_per_hour, config.register_per_hour.min(10)),
            gone: Arc::new(Mutex::new(Gone::default())),
            counts: Arc::new(Counts::default()),
        }
    }

    fn trusted(&self, ip: IpAddr) -> bool {
        self.trusted_proxies.iter().any(|n| n.contains(ip))
    }

    /// The client's address: the peer, or for a trusted proxy the nearest hop of `X-Forwarded-For`
    /// that is not itself a trusted proxy. `None` when a trusted proxy did not say who the client
    /// is (no header, or a hop that is not an address): a broken proxy set-up must not put every
    /// client in one bucket.
    fn client(&self, peer: Option<Peer>, headers: &HeaderMap) -> Option<IpAddr> {
        let Some(Peer(addr)) = peer else {
            return Some(IpAddr::V4(Ipv4Addr::UNSPECIFIED));
        };
        let ip = addr.ip().to_canonical();
        if !self.trusted(ip) {
            return Some(ip);
        }
        let forwarded: Vec<&str> = headers
            .get_all("x-forwarded-for")
            .iter()
            .filter_map(|v| v.to_str().ok())
            .flat_map(|v| v.split(','))
            .collect();
        for hop in forwarded.into_iter().rev() {
            let hop = parse_hop(hop.trim())?;
            if !self.trusted(hop) {
                return Some(hop);
            }
        }
        None
    }

    fn take(limits: &Mutex<Limits>, key: u128) -> bool {
        limits
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take(key, now_ms())
    }
}

/// An `X-Forwarded-For` hop: an address, or an address with a port (`1.2.3.4:5`, `[::1]:5`).
fn parse_hop(hop: &str) -> Option<IpAddr> {
    hop.parse::<IpAddr>()
        .or_else(|_| hop.parse::<SocketAddr>().map(|s| s.ip()))
        .ok()
        .map(|ip| ip.to_canonical())
}

/// Tokens FCM said are no more, for a day: another message to one is answered `410` without asking
/// FCM again. Two generations, so memory stays bounded.
#[derive(Debug, Default)]
struct Gone {
    current: std::collections::HashSet<u128>,
    previous: std::collections::HashSet<u128>,
    since: i64,
}

impl Gone {
    const GENERATION_MS: i64 = 12 * 60 * 60 * 1000;
    const MAX: usize = 200_000;

    fn contains(&self, key: u128) -> bool {
        self.current.contains(&key) || self.previous.contains(&key)
    }

    fn insert(&mut self, key: u128, now: i64) {
        if now - self.since >= Self::GENERATION_MS || self.current.len() >= Self::MAX {
            self.previous = std::mem::take(&mut self.current);
            self.since = now;
        }
        self.current.insert(key);
    }
}

/// The routes.
pub fn router(app: App) -> Router {
    Router::new()
        .route("/health", get(|| async { "ok" }))
        .route("/register", post(register))
        .route("/wp/{sealed}", post(push))
        .layer(DefaultBodyLimit::max(MAX_REQUEST))
        .layer(middleware::from_fn(guard))
        .with_state(app)
}

/// A time limit on every request, and no answer kept by a cache.
async fn guard(req: Request<Body>, next: Next) -> Response {
    let mut res = match tokio::time::timeout(REQUEST_TIMEOUT, next.run(req)).await {
        Ok(r) => r,
        Err(_) => StatusCode::REQUEST_TIMEOUT.into_response(),
    };
    let h = res.headers_mut();
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    h.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    res
}

#[derive(Deserialize)]
struct Register {
    token: zeroize::Zeroizing<String>,
    /// The Companion's VAPID public key (base64url): the endpoint then forwards only messages
    /// that key signed for this gateway.
    vapid: Option<String>,
}

async fn register(
    State(app): State<App>,
    peer: Option<axum::Extension<Peer>>,
    headers: HeaderMap,
    req: Result<Json<Register>, axum::extract::rejection::JsonRejection>,
) -> Response {
    let Some(ip) = app.client(peer.map(|p| p.0), &headers) else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    if !App::take(&app.register_limits, limits::who(ip)) {
        app.counts.limited.fetch_add(1, Ordering::Relaxed);
        return StatusCode::TOO_MANY_REQUESTS.into_response();
    }
    let Ok(Json(req)) = req else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    if !seal::token_ok(&req.token) {
        return StatusCode::BAD_REQUEST.into_response();
    }
    let vapid = match req.vapid.as_deref().map(crate::vapid::parse_key) {
        None => None,
        Some(Some(k)) => Some(k),
        Some(None) => return StatusCode::BAD_REQUEST.into_response(),
    };
    let Ok(sealed) = app.key.seal(&req.token, vapid.as_ref()) else {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    };
    app.counts.registered.fetch_add(1, Ordering::Relaxed);
    Json(serde_json::json!({ "endpoint": format!("{}/wp/{sealed}", app.public_url) }))
        .into_response()
}

async fn push(
    State(app): State<App>,
    peer: Option<axum::Extension<Peer>>,
    Path(sealed): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> StatusCode {
    let text = |name: &str| headers.get(name).and_then(|v| v.to_str().ok());
    let Some(ip) = app.client(peer.map(|p| p.0), &headers) else {
        return StatusCode::BAD_REQUEST;
    };
    if !App::take(&app.address_limits, limits::who(ip)) {
        app.counts.limited.fetch_add(1, Ordering::Relaxed);
        return StatusCode::TOO_MANY_REQUESTS;
    }
    let Some(opened) = app.key.open(&sealed) else {
        return StatusCode::NOT_FOUND;
    };
    if body.len() > MAX_PUSH_BODY {
        return StatusCode::PAYLOAD_TOO_LARGE;
    }
    // A Web Push message (RFC 8030, RFC 8291): encrypted, with a TTL.
    let encrypted = text("content-encoding").is_some_and(|v| v.eq_ignore_ascii_case("aes128gcm"));
    let ttl = text("ttl").and_then(|v| v.trim().parse::<u64>().ok());
    let (true, Some(ttl), false) = (encrypted, ttl, body.is_empty()) else {
        return StatusCode::BAD_REQUEST;
    };
    // A bound endpoint forwards only what its Companion signed for this gateway (RFC 8292).
    if let Some(key) = &opened.vapid {
        let signed = text("authorization")
            .is_some_and(|a| crate::vapid::verify(a, key, &app.origin, now_ms() / 1000));
        if !signed {
            return StatusCode::UNAUTHORIZED;
        }
    }
    let device = limits::token_key(&opened.token);
    if app
        .gone
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .contains(device)
    {
        app.counts.gone.fetch_add(1, Ordering::Relaxed);
        return StatusCode::GONE;
    }
    if !App::take(&app.push_limits, device) {
        app.counts.limited.fetch_add(1, Ordering::Relaxed);
        return StatusCode::TOO_MANY_REQUESTS;
    }
    if !App::take(&app.total_limits, 0) {
        app.counts.limited.fetch_add(1, Ordering::Relaxed);
        return StatusCode::SERVICE_UNAVAILABLE;
    }
    let message = Message {
        token: &opened.token,
        body: &body,
        high: text("urgency").is_some_and(|v| v.trim().eq_ignore_ascii_case("high")),
        ttl_s: u32::try_from(ttl.min(u64::from(MAX_TTL_S))).unwrap_or(MAX_TTL_S),
    };
    match app.fcm.send(&message).await {
        Delivery::Sent => {
            app.counts.sent.fetch_add(1, Ordering::Relaxed);
            StatusCode::CREATED
        }
        Delivery::Gone => {
            app.gone
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .insert(device, now_ms());
            app.counts.gone.fetch_add(1, Ordering::Relaxed);
            StatusCode::GONE
        }
        Delivery::Failed => {
            app.counts.failed.fetch_add(1, Ordering::Relaxed);
            StatusCode::BAD_GATEWAY
        }
        Delivery::Busy => {
            app.counts.limited.fetch_add(1, Ordering::Relaxed);
            StatusCode::SERVICE_UNAVAILABLE
        }
    }
}

/// Counts a connection for its address until dropped.
struct AddressSlot {
    counts: Arc<Mutex<std::collections::HashMap<u128, usize>>>,
    key: u128,
}

impl Drop for AddressSlot {
    fn drop(&mut self) {
        let mut c = self.counts.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(n) = c.get_mut(&self.key) {
            *n -= 1;
            if *n == 0 {
                c.remove(&self.key);
            }
        }
    }
}

/// Serves `app` on a bound listener until `shutdown` completes: at most [`MAX_CONNECTIONS`] at
/// once, [`MAX_PER_ADDRESS`] from one address, headers within [`HEADER_READ_TIMEOUT`], each
/// connection at most [`MAX_CONNECTION_AGE`].
pub async fn serve_on(
    listener: tokio::net::TcpListener,
    app: App,
    shutdown: impl std::future::Future<Output = ()>,
) -> std::io::Result<()> {
    use hyper_util::rt::{TokioIo, TokioTimer};
    use tower::ServiceExt;

    let trusted = app.trusted_proxies.clone();
    let router = router(app);
    let slots = Arc::new(tokio::sync::Semaphore::new(MAX_CONNECTIONS));
    let per_address = Arc::new(Mutex::new(std::collections::HashMap::<u128, usize>::new()));
    let (stop_tx, stop_rx) = tokio::sync::watch::channel(false);
    let mut shutdown = std::pin::pin!(shutdown);
    loop {
        let slot = tokio::select! {
            s = slots.clone().acquire_owned() => s.map_err(std::io::Error::other)?,
            () = &mut shutdown => break,
        };
        let (stream, peer) = tokio::select! {
            r = listener.accept() => match r {
                Ok(c) => c,
                Err(_) => {
                    tokio::time::sleep(Duration::from_millis(50)).await;
                    continue;
                }
            },
            () = &mut shutdown => break,
        };
        let ip = peer.ip().to_canonical();
        let address_slot = if trusted.iter().any(|n| n.contains(ip)) {
            None
        } else {
            let key = limits::who(ip);
            let mut c = per_address.lock().unwrap_or_else(PoisonError::into_inner);
            let n = c.entry(key).or_insert(0);
            if *n >= MAX_PER_ADDRESS {
                continue;
            }
            *n += 1;
            Some(AddressSlot {
                counts: per_address.clone(),
                key,
            })
        };
        let svc = router.clone();
        let service = hyper::service::service_fn(move |mut req: Request<hyper::body::Incoming>| {
            req.extensions_mut().insert(Peer(peer));
            svc.clone().oneshot(req.map(Body::new))
        });
        let mut stop = stop_rx.clone();
        tokio::spawn(async move {
            let conn = hyper::server::conn::http1::Builder::new()
                .timer(TokioTimer::new())
                .header_read_timeout(HEADER_READ_TIMEOUT)
                .serve_connection(TokioIo::new(stream), service);
            let mut conn = std::pin::pin!(conn);
            let ended = tokio::select! {
                _ = conn.as_mut() => true,
                _ = stop.wait_for(|s| *s) => false,
                () = tokio::time::sleep(MAX_CONNECTION_AGE) => false,
            };
            if !ended {
                conn.as_mut().graceful_shutdown();
                let _ = tokio::time::timeout(REQUEST_TIMEOUT, conn).await;
            }
            drop(address_slot);
            drop(slot);
        });
    }
    drop(listener);
    let _ = stop_tx.send(true);
    let all = u32::try_from(MAX_CONNECTIONS).unwrap_or(u32::MAX);
    let _ = tokio::time::timeout(REQUEST_TIMEOUT, slots.acquire_many(all)).await;
    Ok(())
}

/// Writes the counts to standard error every ten minutes when something happened.
pub async fn report(counts: Arc<Counts>) {
    let mut every = tokio::time::interval(Duration::from_secs(600));
    every.tick().await;
    loop {
        every.tick().await;
        let take = |c: &AtomicU64| c.swap(0, Ordering::Relaxed);
        let (r, s, g, f, l) = (
            take(&counts.registered),
            take(&counts.sent),
            take(&counts.gone),
            take(&counts.failed),
            take(&counts.limited),
        );
        if r + s + g + f + l > 0 {
            eprintln!(
                "croft-push-gateway: last 10 minutes: {r} registered, {s} sent, {g} gone, {f} failed, {l} limited"
            );
        }
    }
}
