// This Source Code Form is subject to the terms of the GNU Affero General Public License,
// version 3 or later. See LICENSE-AGPL-3.0 at the repository root.

//! The Companion's HTTP side (T23): `/healthz` and `/version` so far, with strict security headers
//! on every answer, a request body limit and a time limit. Plain HTTP: TLS is the reverse proxy's
//! job (see the README).

use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use axum::body::Body;
use axum::extract::{DefaultBodyLimit, State};
use axum::http::{header, HeaderValue, Request, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Router;

use crate::store::Store;

/// Largest request body accepted.
pub const MAX_BODY: usize = 64 * 1024;
/// Longest a request may take.
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// What the handlers share.
#[derive(Clone)]
pub struct App {
    store: Arc<Mutex<Store>>,
}

impl App {
    /// The state over an open store.
    pub fn new(store: Store) -> Self {
        Self {
            store: Arc::new(Mutex::new(store)),
        }
    }
}

/// The routes, with the headers, the body limit and the time limit around every one.
pub fn router(app: App) -> Router {
    Router::new()
        .route("/healthz", get(healthz))
        .route("/version", get(version))
        .fallback(not_found)
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
    set(h, header::CACHE_CONTROL, "no-store");
    set(h, header::X_CONTENT_TYPE_OPTIONS, "nosniff");
    set(h, header::REFERRER_POLICY, "no-referrer");
    set(h, header::X_FRAME_OPTIONS, "DENY");
    set(
        h,
        header::CONTENT_SECURITY_POLICY,
        "default-src 'none'; frame-ancestors 'none'; base-uri 'none'; form-action 'none'",
    );
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

/// Serves `app` on `listen` until SIGTERM or Ctrl-C, then lets requests in flight finish.
pub async fn serve(listen: &str, app: App) -> std::io::Result<()> {
    let listener = tokio::net::TcpListener::bind(listen).await?;
    axum::serve(listener, router(app))
        .with_graceful_shutdown(shutdown_signal())
        .await
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
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    fn app() -> Router {
        router(App::new(Store::in_memory().unwrap()))
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
        tokio::spawn(async move { axum::serve(listener, app()).await });
        let config = crate::config::Config {
            listen: addr,
            public_url: "http://localhost".into(),
            data_dir: std::path::PathBuf::from("/nonexistent"),
            master_key_file: std::path::PathBuf::from("/nonexistent/key"),
        };
        let out = tokio::task::spawn_blocking(move || {
            crate::cli::run(&config, &crate::cli::Command::Healthcheck)
        })
        .await
        .unwrap()
        .unwrap();
        assert_eq!(out, "ok");
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
