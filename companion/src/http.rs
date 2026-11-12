// This Source Code Form is subject to the terms of the GNU Affero General Public License,
// version 3 or later. See LICENSE-AGPL-3.0 at the repository root.

//! The Companion's HTTP side (T23, T24): `/healthz`, `/version` and the sign-in API, with strict
//! security headers on every answer, a request body limit and a time limit. The server loop limits
//! open connections and how long a client may take to send its headers (slow clients). Plain HTTP:
//! TLS is the reverse proxy's job (see the README).

use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use axum::body::Body;
use axum::extract::{DefaultBodyLimit, State};
use axum::http::{header, HeaderValue, Request, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Router;

use crate::api::{self, Auth, Peer};
use crate::store::Store;

/// Largest request body accepted.
pub const MAX_BODY: usize = 64 * 1024;
/// Longest a request may take.
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
/// Longest a client may take to send a request's body (at most [`MAX_BODY`]).
pub const BODY_READ_TIMEOUT: Duration = Duration::from_secs(10);
/// Longest a connection is kept: a kept-alive connection is then closed after its current request,
/// so no client holds a slot for good.
pub const MAX_CONNECTION_AGE: Duration = Duration::from_secs(120);
/// Connections served at once from one address (IPv6: one /64); a trusted proxy is not limited.
pub const MAX_PER_ADDRESS: usize = 32;
/// Longest a client may take to send a request's headers (also how long an idle kept-alive
/// connection stays open).
pub const HEADER_READ_TIMEOUT: Duration = Duration::from_secs(10);
/// Connections served at once; more wait in the kernel's backlog.
pub const MAX_CONNECTIONS: usize = 512;

/// The server loop's limits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ServerLimits {
    /// See [`HEADER_READ_TIMEOUT`].
    pub header_read_timeout: Duration,
    /// See [`MAX_CONNECTIONS`].
    pub max_connections: usize,
    /// See [`MAX_PER_ADDRESS`].
    pub max_per_address: usize,
    /// See [`MAX_CONNECTION_AGE`].
    pub max_connection_age: Duration,
}

impl Default for ServerLimits {
    fn default() -> Self {
        Self {
            header_read_timeout: HEADER_READ_TIMEOUT,
            max_connections: MAX_CONNECTIONS,
            max_per_address: MAX_PER_ADDRESS,
            max_connection_age: MAX_CONNECTION_AGE,
        }
    }
}

/// What the handlers share.
#[derive(Clone)]
pub struct App {
    pub(crate) store: Arc<Mutex<Store>>,
    pub(crate) auth: Arc<Auth>,
    pub(crate) body_read_timeout: Duration,
}

impl App {
    /// The state over an open store and the sign-in settings.
    pub fn new(store: Store, auth: Auth) -> Self {
        Self {
            store: Arc::new(Mutex::new(store)),
            auth: Arc::new(auth),
            body_read_timeout: BODY_READ_TIMEOUT,
        }
    }

    /// The same, with another body read timeout (tests).
    pub fn with_body_read_timeout(mut self, t: Duration) -> Self {
        self.body_read_timeout = t;
        self
    }
}

/// The routes, with the headers, the body limit and the time limit around every one.
pub fn router(app: App) -> Router {
    let mut r = Router::new()
        .route("/healthz", get(healthz))
        .route("/version", get(version));
    for route in api::routes() {
        r = r.route(route.path, route.handler);
    }
    r.fallback(not_found)
        .layer(DefaultBodyLimit::max(MAX_BODY))
        .layer(middleware::from_fn_with_state(app.clone(), guard))
        .with_state(app)
}

async fn healthz(State(app): State<App>) -> Response {
    let store = app.store.clone();
    let ok = tokio::task::spawn_blocking(move || {
        store.lock().unwrap_or_else(PoisonError::into_inner).ping()
    })
    .await
    .unwrap_or(false);
    if ok {
        (StatusCode::OK, "ok").into_response()
    } else {
        (StatusCode::SERVICE_UNAVAILABLE, "unavailable").into_response()
    }
}

async fn version() -> Response {
    (StatusCode::OK, env!("CARGO_PKG_VERSION")).into_response()
}

async fn not_found() -> Response {
    StatusCode::NOT_FOUND.into_response()
}

/// Reads the whole body (at most [`MAX_BODY`]) within the body read timeout, so a client that
/// trickles its body is cut off long before the request timeout.
async fn read_body(req: Request<Body>, within: Duration) -> Result<Request<Body>, StatusCode> {
    use http_body_util::BodyExt;
    let (parts, body) = req.into_parts();
    let limited = http_body_util::Limited::new(body, MAX_BODY);
    match tokio::time::timeout(within, limited.collect()).await {
        Err(_) => Err(StatusCode::REQUEST_TIMEOUT),
        Ok(Err(e)) if e.is::<http_body_util::LengthLimitError>() => {
            Err(StatusCode::PAYLOAD_TOO_LARGE)
        }
        Ok(Err(_)) => Err(StatusCode::BAD_REQUEST),
        Ok(Ok(collected)) => Ok(Request::from_parts(parts, Body::from(collected.to_bytes()))),
    }
}

/// Security headers on every answer, a declared body over the limit refused before it is read,
/// the body read within its own time limit, and the request's time limit.
async fn guard(State(app): State<App>, req: Request<Body>, next: Next) -> Response {
    let too_big = req
        .headers()
        .get(header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<usize>().ok())
        .is_some_and(|n| n > MAX_BODY);
    let mut res = if too_big {
        StatusCode::PAYLOAD_TOO_LARGE.into_response()
    } else {
        match read_body(req, app.body_read_timeout).await {
            Err(status) => status.into_response(),
            Ok(req) => match tokio::time::timeout(REQUEST_TIMEOUT, next.run(req)).await {
                Ok(r) => r,
                Err(_) => StatusCode::REQUEST_TIMEOUT.into_response(),
            },
        }
    };
    let h = res.headers_mut();
    let set = |h: &mut axum::http::HeaderMap, name: header::HeaderName, value: &'static str| {
        h.insert(name, HeaderValue::from_static(value));
    };
    // A page that sets its own (the enrolment page) keeps it; everything else allows nothing.
    if !h.contains_key(header::CONTENT_SECURITY_POLICY) {
        set(
            h,
            header::CONTENT_SECURITY_POLICY,
            "default-src 'none'; frame-ancestors 'none'; base-uri 'none'; form-action 'none'",
        );
    }
    set(h, header::CACHE_CONTROL, "no-store");
    set(h, header::X_CONTENT_TYPE_OPTIONS, "nosniff");
    set(h, header::REFERRER_POLICY, "no-referrer");
    set(h, header::X_FRAME_OPTIONS, "DENY");
    set(
        h,
        header::HeaderName::from_static("cross-origin-opener-policy"),
        "same-origin",
    );
    set(
        h,
        header::HeaderName::from_static("cross-origin-resource-policy"),
        "same-origin",
    );
    set(
        h,
        header::HeaderName::from_static("permissions-policy"),
        "interest-cohort=()",
    );
    res
}

/// Serves `app` on `listen` until SIGTERM or Ctrl-C, then lets requests in flight finish (for up
/// to [`REQUEST_TIMEOUT`]). At most [`MAX_CONNECTIONS`] are served at once, and a client that does
/// not send its headers within [`HEADER_READ_TIMEOUT`] is dropped.
pub async fn serve(listen: &str, app: App) -> std::io::Result<()> {
    let listener = tokio::net::TcpListener::bind(listen).await?;
    serve_on(listener, app, ServerLimits::default(), shutdown_signal()).await
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

/// [`serve`] on a bound listener, with `limits`, until `shutdown` completes.
pub async fn serve_on(
    listener: tokio::net::TcpListener,
    app: App,
    limits: ServerLimits,
    shutdown: impl std::future::Future<Output = ()>,
) -> std::io::Result<()> {
    use hyper_util::rt::{TokioIo, TokioTimer};
    use tower::ServiceExt;

    let trusted: Vec<std::net::IpAddr> = app.auth.trusted_proxies().to_vec();
    let router = router(app);
    let slots = Arc::new(tokio::sync::Semaphore::new(limits.max_connections));
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
                    // Out of file descriptors or a reset before accept: pause, never spin.
                    tokio::time::sleep(Duration::from_millis(50)).await;
                    continue;
                }
            },
            () = &mut shutdown => break,
        };
        let ip = peer.ip().to_canonical();
        let address_slot = if trusted.contains(&ip) {
            None
        } else {
            let key = crate::limits::who(ip);
            let mut c = per_address.lock().unwrap_or_else(PoisonError::into_inner);
            let n = c.entry(key).or_insert(0);
            if *n >= limits.max_per_address {
                // Over this address's share: close at once (the slot is freed by the drop).
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
        let max_age = limits.max_connection_age;
        let header_timeout = limits.header_read_timeout;
        tokio::spawn(async move {
            let conn = hyper::server::conn::http1::Builder::new()
                .timer(TokioTimer::new())
                .header_read_timeout(header_timeout)
                .serve_connection(TokioIo::new(stream), service);
            let mut conn = std::pin::pin!(conn);
            let ended = tokio::select! {
                _ = conn.as_mut() => true,
                _ = stop.wait_for(|s| *s) => false,
                () = tokio::time::sleep(max_age) => false,
            };
            if !ended {
                // Finish the request in progress (if any), then close.
                conn.as_mut().graceful_shutdown();
                let _ = tokio::time::timeout(REQUEST_TIMEOUT, conn).await;
            }
            drop(address_slot);
            drop(slot);
        });
    }
    drop(listener);
    let _ = stop_tx.send(true);
    // Every connection has returned its slot, or the grace time is over.
    let all = u32::try_from(limits.max_connections).unwrap_or(u32::MAX);
    let _ = tokio::time::timeout(REQUEST_TIMEOUT, slots.acquire_many(all)).await;
    Ok(())
}

async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let term = async {
        if let Ok(mut s) = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        {
            s.recv().await;
        }
    };
    #[cfg(not(unix))]
    let term = std::future::pending::<()>();
    tokio::select! {
        () = ctrl_c => {},
        () = term => {},
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    fn app_state() -> App {
        let store = Store::in_memory().unwrap();
        let auth = Auth::new(
            "https://companion.test",
            crate::crypto::Key::generate().unwrap(),
            Vec::new(),
            Auth::system_clock(),
        )
        .unwrap();
        App::new(store, auth)
    }

    fn app() -> Router {
        router(app_state())
    }

    async fn call(req: Request<Body>) -> (StatusCode, axum::http::HeaderMap, String) {
        let res = app().oneshot(req).await.unwrap();
        let status = res.status();
        let headers = res.headers().clone();
        let body = res.into_body().collect().await.unwrap().to_bytes();
        (status, headers, String::from_utf8_lossy(&body).into_owned())
    }

    fn assert_safe_headers(h: &axum::http::HeaderMap) {
        assert_eq!(h["cache-control"], "no-store");
        assert_eq!(h["x-content-type-options"], "nosniff");
        assert_eq!(h["x-frame-options"], "DENY");
        assert!(h["content-security-policy"]
            .to_str()
            .unwrap()
            .contains("frame-ancestors 'none'"));
        assert_eq!(h["referrer-policy"], "no-referrer");
    }

    #[tokio::test]
    async fn healthz_and_version_answer_with_the_security_headers() {
        let (s, h, b) = call(Request::get("/healthz").body(Body::empty()).unwrap()).await;
        assert_eq!((s, b.as_str()), (StatusCode::OK, "ok"));
        assert_safe_headers(&h);
        let (s, h, b) = call(Request::get("/version").body(Body::empty()).unwrap()).await;
        assert_eq!((s, b.as_str()), (StatusCode::OK, env!("CARGO_PKG_VERSION")));
        assert_safe_headers(&h);
        let (s, h, _) = call(Request::get("/nothing").body(Body::empty()).unwrap()).await;
        assert_eq!(s, StatusCode::NOT_FOUND);
        assert_safe_headers(&h);
    }

    #[tokio::test]
    async fn the_healthcheck_command_asks_the_running_server() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap().to_string();
        tokio::spawn(serve_on(
            listener,
            app_state(),
            ServerLimits::default(),
            std::future::pending(),
        ));
        let config = crate::config::Config {
            listen: addr,
            public_url: "http://localhost".into(),
            data_dir: std::path::PathBuf::from("/nonexistent"),
            master_key_file: std::path::PathBuf::from("/nonexistent/key"),
            trusted_proxies: Vec::new(),
            oidc: None,
            mail_allowed_networks: Vec::new(),
        };
        let out = tokio::task::spawn_blocking(move || {
            crate::cli::run(&config, &crate::cli::Command::Healthcheck)
        })
        .await
        .unwrap()
        .unwrap();
        assert_eq!(out, "ok");
    }

    async fn start(limits: ServerLimits) -> std::net::SocketAddr {
        start_with(limits, app_state()).await
    }

    async fn start_with(limits: ServerLimits, app: App) -> std::net::SocketAddr {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(serve_on(listener, app, limits, std::future::pending()));
        addr
    }

    /// Review finding: clients that keep sending small requests on kept-alive connections held
    /// every slot for good. Connections now have a maximum age.
    #[tokio::test]
    async fn kept_alive_connections_are_closed_at_their_maximum_age() {
        let addr = start(ServerLimits {
            header_read_timeout: Duration::from_millis(300),
            max_connections: 2,
            max_connection_age: Duration::from_millis(500),
            ..ServerLimits::default()
        })
        .await;
        let mut hogs = Vec::new();
        for _ in 0..2 {
            hogs.push(tokio::net::TcpStream::connect(addr).await.unwrap());
        }
        let hog = tokio::spawn(async move {
            loop {
                for s in &mut hogs {
                    let _ = tokio::time::timeout(Duration::from_millis(200), get_healthz(s)).await;
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        });
        tokio::time::sleep(Duration::from_millis(100)).await;
        let mut victim = tokio::net::TcpStream::connect(addr).await.unwrap();
        let r = tokio::time::timeout(Duration::from_secs(3), get_healthz(&mut victim)).await;
        hog.abort();
        assert!(r.unwrap().starts_with("HTTP/1.1 200"));
    }

    /// Review finding: a body trickled in held its slot until the 30 s request timeout.
    #[tokio::test]
    async fn a_slow_body_is_cut_off_by_the_body_read_timeout() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let addr = start_with(
            ServerLimits {
                header_read_timeout: Duration::from_millis(300),
                max_connections: 1,
                ..ServerLimits::default()
            },
            app_state().with_body_read_timeout(Duration::from_millis(400)),
        )
        .await;
        let mut slow = tokio::net::TcpStream::connect(addr).await.unwrap();
        slow.write_all(
            b"POST /api/sign-in/recovery HTTP/1.1\r\nhost: x\r\ncontent-type: application/json\r\ncontent-length: 1000\r\n\r\n{",
        )
        .await
        .unwrap();
        let (mut rd, mut wr) = slow.into_split();
        let trickle = tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_millis(100)).await;
                if wr.write_all(b" ").await.is_err() {
                    break;
                }
            }
        });
        let mut buf = vec![0u8; 1024];
        let n = tokio::time::timeout(Duration::from_secs(3), rd.read(&mut buf))
            .await
            .unwrap()
            .unwrap();
        assert!(String::from_utf8_lossy(&buf[..n]).starts_with("HTTP/1.1 408"));
        trickle.abort();
        drop(rd);
        let mut victim = tokio::net::TcpStream::connect(addr).await.unwrap();
        let r = tokio::time::timeout(Duration::from_secs(3), get_healthz(&mut victim)).await;
        assert!(r.unwrap().starts_with("HTTP/1.1 200"));
    }

    #[tokio::test]
    async fn one_address_gets_only_its_share_of_connections() {
        use tokio::io::AsyncReadExt;
        let addr = start(ServerLimits {
            max_per_address: 1,
            ..ServerLimits::default()
        })
        .await;
        let mut first = tokio::net::TcpStream::connect(addr).await.unwrap();
        tokio::time::sleep(Duration::from_millis(50)).await;
        let mut second = tokio::net::TcpStream::connect(addr).await.unwrap();
        let mut buf = [0u8; 16];
        let n = tokio::time::timeout(Duration::from_secs(3), second.read(&mut buf))
            .await
            .unwrap()
            .unwrap_or(0);
        assert_eq!(n, 0, "the second connection is closed at once");
        assert!(get_healthz(&mut first).await.starts_with("HTTP/1.1 200"));
        let key = |s: &str| crate::limits::who(s.parse().unwrap());
        assert_eq!(key("::ffff:10.0.0.1"), key("10.0.0.1"));
        assert_ne!(key("::ffff:10.0.0.1"), key("2001:db8::1"));
        assert_eq!(key("2001:db8:0:1::1"), key("2001:db8:0:1::2"));
    }

    async fn get_healthz(stream: &mut tokio::net::TcpStream) -> String {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        stream
            .write_all(b"GET /healthz HTTP/1.1\r\nhost: x\r\n\r\n")
            .await
            .unwrap();
        let mut buf = vec![0u8; 4096];
        let n = stream.read(&mut buf).await.unwrap();
        String::from_utf8_lossy(&buf[..n]).into_owned()
    }

    #[tokio::test]
    async fn a_client_that_does_not_send_its_headers_in_time_is_dropped() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let addr = start(ServerLimits {
            header_read_timeout: Duration::from_millis(300),
            max_connections: 4,
            ..ServerLimits::default()
        })
        .await;
        let mut slow = tokio::net::TcpStream::connect(addr).await.unwrap();
        slow.write_all(b"GET /healthz HTTP/1.1\r\nhost:")
            .await
            .unwrap();
        let mut buf = [0u8; 256];
        let n = tokio::time::timeout(Duration::from_secs(5), slow.read(&mut buf))
            .await
            .expect("the server closes the connection")
            .unwrap_or(0);
        // Closed, at most with an error status; never a 200.
        assert!(!String::from_utf8_lossy(&buf[..n]).contains("200 OK"));
        // A normal client is served.
        let mut ok = tokio::net::TcpStream::connect(addr).await.unwrap();
        assert!(get_healthz(&mut ok).await.starts_with("HTTP/1.1 200"));
    }

    #[tokio::test]
    async fn connections_over_the_limit_wait_for_a_free_slot() {
        let addr = start(ServerLimits {
            header_read_timeout: Duration::from_millis(500),
            max_connections: 2,
            ..ServerLimits::default()
        })
        .await;
        // Two idle connections take both slots.
        let _a = tokio::net::TcpStream::connect(addr).await.unwrap();
        let _b = tokio::net::TcpStream::connect(addr).await.unwrap();
        tokio::time::sleep(Duration::from_millis(100)).await;
        let mut third = tokio::net::TcpStream::connect(addr).await.unwrap();
        let waited = std::time::Instant::now();
        let reply = tokio::time::timeout(Duration::from_secs(5), get_healthz(&mut third))
            .await
            .unwrap();
        assert!(reply.starts_with("HTTP/1.1 200"), "{reply}");
        // Served only once an idle connection timed out and freed its slot.
        assert!(
            waited.elapsed() >= Duration::from_millis(250),
            "{:?}",
            waited.elapsed()
        );
    }

    #[tokio::test]
    async fn a_body_over_the_limit_is_refused() {
        let big = vec![b'x'; MAX_BODY + 1];
        let req = Request::post("/healthz")
            .header("content-length", big.len())
            .body(Body::from(big))
            .unwrap();
        let (s, h, _) = call(req).await;
        assert_eq!(s, StatusCode::PAYLOAD_TOO_LARGE);
        assert_safe_headers(&h);
    }
}
