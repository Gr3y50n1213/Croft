// This Source Code Form is subject to the terms of the GNU Affero General Public License,
// version 3 or later. See LICENSE-AGPL-3.0 at the repository root.

//! The gateway end to end through its router, against a fake Google token endpoint (which checks
//! the signed JWT with the made-up service account's public key) and a fake FCM, both on this
//! machine. The service account and every token here are made up.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::net::SocketAddr;
use std::sync::atomic::{AtomicU16, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use axum::body::Body;
use axum::extract::{Path as AxPath, State as AxState};
use axum::http::{HeaderMap, Method, Request, StatusCode};
use axum::response::IntoResponse;
use axum::routing::post;
use axum::{Form, Json, Router};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use http_body_util::BodyExt;
use serde_json::{json, Value};
use tower::ServiceExt;

use crate::config::Config;
use crate::fcm::{Fcm, ServiceAccount, SCOPE};
use crate::seal::GatewayKey;
use crate::server::{router, App, Peer};

const TOKEN: &str = "made-up-fcm-token:APA91b_test-1";

/// A made-up RSA key for the service account (made once: it is slow) and its public key (PKCS#1).
fn rsa_key() -> &'static (String, Vec<u8>) {
    static KEY: OnceLock<(String, Vec<u8>)> = OnceLock::new();
    KEY.get_or_init(|| {
        use rsa::pkcs1::EncodeRsaPublicKey;
        use rsa::pkcs8::EncodePrivateKey;
        let key = rsa::RsaPrivateKey::new(&mut rand::thread_rng(), 2048).unwrap();
        let pem = key.to_pkcs8_pem(rsa::pkcs8::LineEnding::LF).unwrap();
        let public = key.to_public_key().to_pkcs1_der().unwrap();
        (pem.to_string(), public.as_bytes().to_vec())
    })
}

#[derive(Default)]
struct Google {
    token_calls: AtomicUsize,
    token_uri: Mutex<String>,
    /// FCM's answer: 200, 404 (unregistered), 403 (another sender), 500, or 401 once then 200.
    fcm_mode: AtomicU16,
    fcm_401_given: AtomicUsize,
    sent: Mutex<Vec<(String, Value)>>,
}

async fn token_endpoint(
    AxState(g): AxState<Arc<Google>>,
    Form(form): Form<std::collections::HashMap<String, String>>,
) -> axum::response::Response {
    assert_eq!(
        form["grant_type"],
        "urn:ietf:params:oauth:grant-type:jwt-bearer"
    );
    let parts: Vec<&str> = form["assertion"].split('.').collect();
    assert_eq!(parts.len(), 3);
    let header: Value = serde_json::from_slice(&URL_SAFE_NO_PAD.decode(parts[0]).unwrap()).unwrap();
    assert_eq!(header["alg"], "RS256");
    let signed = format!("{}.{}", parts[0], parts[1]);
    let signature = URL_SAFE_NO_PAD.decode(parts[2]).unwrap();
    let ok = ring::signature::UnparsedPublicKey::new(
        &ring::signature::RSA_PKCS1_2048_8192_SHA256,
        &rsa_key().1,
    )
    .verify(signed.as_bytes(), &signature)
    .is_ok();
    if !ok {
        return StatusCode::BAD_REQUEST.into_response();
    }
    let claims: Value = serde_json::from_slice(&URL_SAFE_NO_PAD.decode(parts[1]).unwrap()).unwrap();
    assert_eq!(
        claims["iss"],
        "gateway@made-up-project.iam.gserviceaccount.com"
    );
    assert_eq!(claims["scope"], SCOPE);
    assert_eq!(claims["aud"], g.token_uri.lock().unwrap().as_str());
    assert_eq!(
        claims["exp"].as_i64().unwrap() - claims["iat"].as_i64().unwrap(),
        3600
    );
    let n = g.token_calls.fetch_add(1, Ordering::SeqCst) + 1;
    Json(json!({"access_token": format!("made-up-access-token-{n}"), "expires_in": 3599, "token_type": "Bearer"}))
        .into_response()
}

async fn fcm_endpoint(
    AxState(g): AxState<Arc<Google>>,
    AxPath(project): AxPath<String>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> axum::response::Response {
    assert_eq!(project, "made-up-project");
    let auth = headers["authorization"].to_str().unwrap().to_owned();
    let mode = g.fcm_mode.load(Ordering::SeqCst);
    if mode == 401 && g.fcm_401_given.fetch_add(1, Ordering::SeqCst) == 0 {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    g.sent.lock().unwrap().push((auth, body));
    match mode {
        404 => (
            StatusCode::NOT_FOUND,
            Json(json!({"error": {"code": 404, "status": "NOT_FOUND",
                "details": [{"@type": "type.googleapis.com/google.firebase.fcm.v1.FcmError", "errorCode": "UNREGISTERED"}]}})),
        )
            .into_response(),
        403 => (
            StatusCode::FORBIDDEN,
            Json(json!({"error": {"details": [{"errorCode": "SENDER_ID_MISMATCH"}]}})),
        )
            .into_response(),
        500 => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
        _ => Json(json!({"name": "projects/made-up-project/messages/1"})).into_response(),
    }
}

struct H {
    router: Router,
    google: Arc<Google>,
}

async fn harness(config_change: impl FnOnce(&mut Config)) -> H {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
    let google = Arc::new(Google::default());
    google.fcm_mode.store(200, Ordering::SeqCst);
    *google.token_uri.lock().unwrap() = format!("{base}/token");
    let fake = Router::new()
        .route("/token", post(token_endpoint))
        .route("/v1/projects/{project}/messages:send", post(fcm_endpoint))
        .with_state(google.clone());
    tokio::spawn(async move { axum::serve(listener, fake).await });
    let account = json!({
        "type": "service_account",
        "project_id": "made-up-project",
        "private_key_id": "made-up",
        "private_key": rsa_key().0,
        "client_email": "gateway@made-up-project.iam.gserviceaccount.com",
        "token_uri": format!("{base}/token"),
    });
    let account = ServiceAccount::from_json(account.to_string().as_bytes()).unwrap();
    let mut config = Config::load(
        Some("public_url = \"https://push.test\""),
        Vec::<(String, String)>::new(),
    )
    .unwrap();
    config.fcm_base = base;
    config_change(&mut config);
    let key = GatewayKey::from_hex(&GatewayKey::generate_hex().unwrap()).unwrap();
    let app = App::new(&config, key, Fcm::new(account, &config.fcm_base));
    H {
        router: router(app),
        google,
    }
}

impl H {
    async fn call(
        &self,
        method: Method,
        path: &str,
        headers: &[(&str, &str)],
        body: Vec<u8>,
        from: &str,
    ) -> (StatusCode, Vec<u8>) {
        let mut req = Request::builder().method(method).uri(path);
        for (k, v) in headers {
            req = req.header(*k, *v);
        }
        let mut req = req.body(Body::from(body)).unwrap();
        req.extensions_mut()
            .insert(Peer(from.parse::<SocketAddr>().unwrap()));
        let res = self.router.clone().oneshot(req).await.unwrap();
        let status = res.status();
        (
            status,
            res.into_body().collect().await.unwrap().to_bytes().to_vec(),
        )
    }

    async fn register(&self, token: &str, from: &str) -> (StatusCode, Value) {
        let (s, body) = self
            .call(
                Method::POST,
                "/register",
                &[("content-type", "application/json")],
                json!({ "token": token }).to_string().into_bytes(),
                from,
            )
            .await;
        (s, serde_json::from_slice(&body).unwrap_or(Value::Null))
    }

    /// The endpoint's path on the gateway.
    async fn endpoint(&self, token: &str) -> String {
        let (s, v) = self.register(token, "203.0.113.5:4000").await;
        assert_eq!(s, StatusCode::OK, "{v}");
        let endpoint = v["endpoint"].as_str().unwrap();
        assert!(endpoint.starts_with("https://push.test/wp/"), "{endpoint}");
        endpoint.trim_start_matches("https://push.test").to_owned()
    }

    async fn push(&self, path: &str, ttl: &str, urgency: &str, body: &[u8]) -> StatusCode {
        self.call(
            Method::POST,
            path,
            &[
                ("content-encoding", "aes128gcm"),
                ("ttl", ttl),
                ("urgency", urgency),
                ("authorization", "vapid t=made-up, k=made-up"),
            ],
            body.to_vec(),
            "198.51.100.7:5000",
        )
        .await
        .0
    }

    fn sent(&self) -> Vec<(String, Value)> {
        self.google.sent.lock().unwrap().clone()
    }
}

/// A body like a Companion's: opaque bytes (here not a real RFC 8291 message: the gateway never
/// opens it).
fn ping_body() -> Vec<u8> {
    (0u8..=150).collect()
}

#[tokio::test]
async fn a_message_reaches_fcm_unchanged_with_a_reused_access_token() {
    let h = harness(|_| {}).await;
    let path = h.endpoint(TOKEN).await;
    assert!(!path.contains("made-up-fcm-token"), "the token is sealed");
    let body = ping_body();
    assert_eq!(
        h.push(&path, "3600", "high", &body).await,
        StatusCode::CREATED
    );
    assert_eq!(
        h.push(&path, "999999", "normal", &body).await,
        StatusCode::CREATED
    );
    let sent = h.sent();
    assert_eq!(sent.len(), 2);
    let (auth, first) = &sent[0];
    assert_eq!(auth, "Bearer made-up-access-token-1");
    assert_eq!(first["message"]["token"], TOKEN);
    let wp = URL_SAFE_NO_PAD
        .decode(first["message"]["data"]["wp"].as_str().unwrap())
        .unwrap();
    assert_eq!(wp, body, "the bytes are forwarded unchanged");
    assert_eq!(first["message"]["android"]["priority"], "HIGH");
    assert_eq!(first["message"]["android"]["ttl"], "3600s");
    // Nothing else is in the message.
    assert_eq!(first["message"]["data"].as_object().unwrap().len(), 1);
    assert!(first["message"].get("notification").is_none());
    let (auth, second) = &sent[1];
    assert_eq!(auth, "Bearer made-up-access-token-1", "the token is reused");
    assert_eq!(second["message"]["android"]["priority"], "NORMAL");
    assert_eq!(
        second["message"]["android"]["ttl"], "86400s",
        "at most a day"
    );
    assert_eq!(h.google.token_calls.load(Ordering::SeqCst), 1);
    // Another registration of the same token is another endpoint to the same device.
    let again = h.endpoint(TOKEN).await;
    assert_ne!(again, path);
    assert_eq!(
        h.push(&again, "60", "low", &body).await,
        StatusCode::CREATED
    );
    assert_eq!(h.sent()[2].1["message"]["token"], TOKEN);
}

#[tokio::test]
async fn fcm_answers_become_web_push_answers() {
    let h = harness(|_| {}).await;
    let path = h.endpoint(TOKEN).await;
    let body = ping_body();
    // The token is no more: 410, and the Companion drops the subscription.
    h.google.fcm_mode.store(404, Ordering::SeqCst);
    assert_eq!(h.push(&path, "60", "normal", &body).await, StatusCode::GONE);
    h.google.fcm_mode.store(403, Ordering::SeqCst);
    assert_eq!(h.push(&path, "60", "normal", &body).await, StatusCode::GONE);
    // Anything else is the gateway's or Google's problem, not the subscription's.
    h.google.fcm_mode.store(500, Ordering::SeqCst);
    assert_eq!(
        h.push(&path, "60", "normal", &body).await,
        StatusCode::BAD_GATEWAY
    );
    // An access token Google no longer takes is renewed once.
    h.google.fcm_mode.store(401, Ordering::SeqCst);
    assert_eq!(
        h.push(&path, "60", "normal", &body).await,
        StatusCode::CREATED
    );
    assert_eq!(h.google.token_calls.load(Ordering::SeqCst), 2);
    assert_eq!(h.sent().last().unwrap().0, "Bearer made-up-access-token-2");
}

#[tokio::test]
async fn only_the_gateways_own_endpoints_and_web_push_messages_are_forwarded() {
    let h = harness(|_| {}).await;
    let path = h.endpoint(TOKEN).await;
    let body = ping_body();
    // Another gateway's endpoint (another key), a made-up one, an altered one.
    let other = harness(|_| {}).await.endpoint(TOKEN).await;
    let mut altered = path.clone().into_bytes();
    let i = altered.len() - 2;
    altered[i] = if altered[i] == b'A' { b'B' } else { b'A' };
    let altered = String::from_utf8(altered).unwrap();
    for p in [
        other.as_str(),
        "/wp/AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
        altered.as_str(),
    ] {
        assert_eq!(
            h.push(p, "60", "normal", &body).await,
            StatusCode::NOT_FOUND,
            "{p}"
        );
    }
    // Not a Web Push message: no encoding, no TTL, empty, too large.
    let (s, _) = h
        .call(
            Method::POST,
            &path,
            &[("ttl", "60")],
            body.clone(),
            "198.51.100.7:5000",
        )
        .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let (s, _) = h
        .call(
            Method::POST,
            &path,
            &[("content-encoding", "aes128gcm")],
            body.clone(),
            "198.51.100.7:5000",
        )
        .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    assert_eq!(
        h.push(&path, "60", "normal", b"").await,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        h.push(
            &path,
            "60",
            "normal",
            &vec![7u8; crate::server::MAX_PUSH_BODY + 1]
        )
        .await,
        StatusCode::PAYLOAD_TOO_LARGE
    );
    assert_eq!(
        h.push(&path, "60", "normal", &vec![7u8; 20_000]).await,
        StatusCode::PAYLOAD_TOO_LARGE
    );
    assert!(h.sent().is_empty(), "nothing reached FCM");
    // Tokens that are not FCM tokens are not registered.
    for bad in [
        "",
        "has space",
        "a/b",
        &"x".repeat(crate::seal::MAX_TOKEN + 1),
    ] {
        assert_eq!(
            h.register(bad, "203.0.113.9:1").await.0,
            StatusCode::BAD_REQUEST
        );
    }
    let (s, _) = h
        .call(
            Method::POST,
            "/register",
            &[("content-type", "application/json")],
            b"{not json".to_vec(),
            "203.0.113.9:1",
        )
        .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let (s, body) = h
        .call(Method::GET, "/health", &[], Vec::new(), "203.0.113.9:1")
        .await;
    assert_eq!((s, body.as_slice()), (StatusCode::OK, b"ok".as_slice()));
}

#[tokio::test]
async fn the_limits_hold_per_device_and_per_address() {
    let h = harness(|c| {
        c.push_burst = 3;
        c.register_per_hour = 4;
        c.trusted_proxies = vec!["10.0.0.1".parse().unwrap()];
    })
    .await;
    let path = h.endpoint(TOKEN).await;
    let body = ping_body();
    for _ in 0..3 {
        assert_eq!(
            h.push(&path, "60", "normal", &body).await,
            StatusCode::CREATED
        );
    }
    assert_eq!(
        h.push(&path, "60", "normal", &body).await,
        StatusCode::TOO_MANY_REQUESTS
    );
    // A new endpoint for the same token shares the device's limit.
    let again = h.endpoint(TOKEN).await;
    assert_eq!(
        h.push(&again, "60", "normal", &body).await,
        StatusCode::TOO_MANY_REQUESTS
    );
    // Another device is not limited by it.
    let other = h.endpoint("made-up-fcm-token:other").await;
    assert_eq!(
        h.push(&other, "60", "normal", &body).await,
        StatusCode::CREATED
    );
    // Registrations: 4 an hour from one address (3 used above from 203.0.113.5).
    assert_eq!(h.register("t1", "203.0.113.5:1").await.0, StatusCode::OK);
    assert_eq!(
        h.register("t2", "203.0.113.5:2").await.0,
        StatusCode::TOO_MANY_REQUESTS
    );
    assert_eq!(h.register("t3", "203.0.113.6:1").await.0, StatusCode::OK);
    // Behind the trusted proxy, the forwarded address counts; an untrusted peer's header is ignored.
    for _ in 0..4 {
        let (s, _) = h
            .call(
                Method::POST,
                "/register",
                &[
                    ("content-type", "application/json"),
                    ("x-forwarded-for", "192.0.2.50"),
                ],
                json!({"token": "t4"}).to_string().into_bytes(),
                "10.0.0.1:443",
            )
            .await;
        assert_eq!(s, StatusCode::OK);
    }
    let (s, _) = h
        .call(
            Method::POST,
            "/register",
            &[
                ("content-type", "application/json"),
                ("x-forwarded-for", "192.0.2.50"),
            ],
            json!({"token": "t4"}).to_string().into_bytes(),
            "10.0.0.1:443",
        )
        .await;
    assert_eq!(s, StatusCode::TOO_MANY_REQUESTS);
    let (s, _) = h
        .call(
            Method::POST,
            "/register",
            &[
                ("content-type", "application/json"),
                ("x-forwarded-for", "192.0.2.51"),
            ],
            json!({"token": "t4"}).to_string().into_bytes(),
            "10.0.0.1:443",
        )
        .await;
    assert_eq!(s, StatusCode::OK, "another client behind the proxy");
}

#[tokio::test]
async fn the_server_loop_serves_and_stops() {
    let h = harness(|_| {}).await;
    drop(h.router);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let account = ServiceAccount::from_json(
        json!({"project_id": "p", "private_key": rsa_key().0, "client_email": "a@b",
               "token_uri": "https://oauth2.example/token"})
        .to_string()
        .as_bytes(),
    )
    .unwrap();
    let config = Config::load(
        Some("public_url = \"https://push.test\""),
        Vec::<(String, String)>::new(),
    )
    .unwrap();
    let app = App::new(
        &config,
        GatewayKey::from_hex(&GatewayKey::generate_hex().unwrap()).unwrap(),
        Fcm::new(account, "https://fcm.example"),
    );
    let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
    let server = tokio::spawn(crate::server::serve_on(listener, app, async {
        let _ = stopped.await;
    }));
    let answer = tokio::task::spawn_blocking(move || {
        use std::io::{Read, Write};
        let mut s = std::net::TcpStream::connect(addr).unwrap();
        s.write_all(b"GET /health HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n")
            .unwrap();
        let mut out = String::new();
        s.read_to_string(&mut out).unwrap();
        out
    })
    .await
    .unwrap();
    assert!(answer.starts_with("HTTP/1.1 200"), "{answer}");
    assert!(answer.contains("cache-control: no-store"));
    stop.send(()).unwrap();
    server.await.unwrap().unwrap();
}
