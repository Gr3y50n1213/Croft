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
}

impl Default for ServerLimits {
    fn default() -> Self {
        Self {
            header_read_timeout: HEADER_READ_TIMEOUT,
            max_connections: MAX_CONNECTIONS,
        }
    }
}

/// What the handlers share.
#[derive(Clone)]
pub struct App {
    pub(crate) store: Arc<Mutex<Store>>,
    pub(crate) auth: Arc<Auth>,
}

impl App {
    /// The state over an open store and the sign-in settings.
    pub fn new(store: Store, auth: Auth) -> Self {
        Self {
            store: Arc::new(Mutex::new(store)),
            auth: Arc::new(auth),
        }
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
        .layer(middleware::from_fn(guard))
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

/// Security headers on every answer, a declared body over the limit refused before it is read,
/// and the time limit.
async fn guard(req: Request<Body>, next: Next) -> Response {
    let too_big = req
        .headers()
        .get(header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<usize>().ok())
        .is_some_and(|n| n > MAX_BODY);
    let mut res = if too_big {
        StatusCode::PAYLOAD_TOO_LARGE.into_response()
    } else {
        match tokio::time::timeout(REQUEST_TIMEOUT, next.run(req)).await {
            Ok(r) => r,
            Err(_) => StatusCode::REQUEST_TIMEOUT.into_response(),
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

/// [`serve`] on a bound listener, with `limits`, until `shutdown` completes.
pub async fn serve_on(
    listener: tokio::net::TcpListener,
    app: App,
    limits: ServerLimits,
    shutdown: impl std::future::Future<Output = ()>,
) -> std::io::Result<()> {
    use hyper_util::rt::{TokioIo, TokioTimer};
    use tower::ServiceExt;

    let router = router(app);
    let slots = Arc::new(tokio::sync::Semaphore::new(limits.max_connections));
    let graceful = hyper_util::server::graceful::GracefulShutdown::new();
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
        let svc = router.clone();
        let service = hyper::service::service_fn(move |mut req: Request<hyper::body::Incoming>| {
            req.extensions_mut().insert(Peer(peer));
            svc.clone().oneshot(req.map(Body::new))
        });
        let conn = hyper::server::conn::http1::Builder::new()
            .timer(TokioTimer::new())
            .header_read_timeout(limits.header_read_timeout)
            .serve_connection(TokioIo::new(stream), service);
        let conn = graceful.watch(conn);
        tokio::spawn(async move {
            let _ = conn.await;
            drop(slot);
        });
    }
    drop(listener);
    let _ = tokio::time::timeout(REQUEST_TIMEOUT, graceful.shutdown()).await;
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
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(serve_on(
            listener,
            app_state(),
            limits,
            std::future::pending(),
        ));
        addr
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
