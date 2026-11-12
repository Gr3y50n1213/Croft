// This Source Code Form is subject to the terms of the GNU Affero General Public License,
// version 3 or later. See LICENSE-AGPL-3.0 at the repository root.

//! Tests of the push relay (T28) through the real router and a user's core, against a fake push
//! service on this machine (it records each message and answers 201, or 410 when told).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::atomic::{AtomicI64, AtomicU16, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::body::{Body, Bytes};
use axum::extract::State as AxState;
use axum::http::{HeaderMap, Method, Request, StatusCode};
use axum::routing::post;
use axum::Router;
use croft_ffi::TestImapServer;
use http_body_util::BodyExt;
use serde_json::{json, Value};
use tower::ServiceExt;

use super::*;
use crate::http::{router, App};
use crate::mail::MailCores;
use crate::push::{Relay, PAYLOAD};
use crate::store::Role;

const ORIGIN: &str = "https://companion.test";
const PW: &str = "made-up-imap-password";
const WINDOW_MS: i64 = 800;

#[derive(Default)]
struct PushService {
    received: Mutex<Vec<(HeaderMap, Vec<u8>)>>,
    arrived_ms: Mutex<Vec<i64>>,
    status: AtomicU16,
}

async fn receive(
    AxState(svc): AxState<Arc<PushService>>,
    headers: HeaderMap,
    body: Bytes,
) -> StatusCode {
    svc.received.lock().unwrap().push((headers, body.to_vec()));
    svc.arrived_ms.lock().unwrap().push(crate::store::now_ms());
    StatusCode::from_u16(svc.status.load(Ordering::SeqCst)).unwrap()
}

/// A device: its push key pair (held with p256, so it can decrypt more than one message).
struct Device {
    secret: p256::SecretKey,
    public: Vec<u8>,
    auth: [u8; 16],
}

impl Device {
    fn new() -> Self {
        let secret = loop {
            let mut bytes = [0u8; 32];
            ring::rand::SecureRandom::fill(&ring::rand::SystemRandom::new(), &mut bytes).unwrap();
            if let Ok(k) = p256::SecretKey::from_slice(&bytes) {
                break k;
            }
        };
        let public = secret.public_key().to_sec1_bytes().to_vec();
        Self {
            secret,
            public,
            auth: [9u8; 16],
        }
    }

    fn decrypt(&self, body: &[u8]) -> Vec<u8> {
        let as_public = p256::PublicKey::from_sec1_bytes(&body[21..86]).unwrap();
        let shared =
            p256::ecdh::diffie_hellman(self.secret.to_nonzero_scalar(), as_public.as_affine());
        crate::webpush::tests::decrypt_with(
            shared.raw_secret_bytes(),
            &self.public,
            &self.auth,
            body,
        )
    }
}

struct H {
    app: App,
    router: Router,
    master: Key,
    svc: Arc<PushService>,
    push_url: String,
    _dir: tempfile::TempDir,
}

async fn h() -> H {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let push_url = format!(
        "http://127.0.0.1:{}/push",
        listener.local_addr().unwrap().port()
    );
    let svc = Arc::new(PushService::default());
    svc.status.store(201, Ordering::SeqCst);
    let fake = Router::new()
        .route("/push/{n}", post(receive))
        .with_state(svc.clone());
    tokio::spawn(async move { axum::serve(listener, fake).await });
    let dir = tempfile::tempdir().unwrap();
    let master = Key::generate().unwrap();
    let store = Store::in_memory().unwrap();
    store.check_master_key(&master).unwrap();
    let auth = Auth::new(ORIGIN, master.clone(), Vec::new(), Auth::system_clock()).unwrap();
    let mut relay = Relay::with_window(WINDOW_MS);
    relay.allow_loopback_http = true;
    let app = App::new(store, auth)
        .with_mail(MailCores::new(dir.path().to_path_buf()))
        .with_relay(relay);
    H {
        router: router(app.clone()),
        app,
        master,
        svc,
        push_url,
        _dir: dir,
    }
}

impl H {
    fn user(&self, name: &str) -> (i64, String) {
        let mut st = self.app.store.lock().unwrap();
        let id = st.add_user(&self.master, name, Role::User).unwrap().id;
        let token = st
            .start_session(id, "test", false, crate::store::now_ms())
            .unwrap()
            .access
            .to_string();
        (id, token)
    }

    async fn call(
        &self,
        method: Method,
        path: &str,
        token: &str,
        body: Option<Value>,
    ) -> (StatusCode, Value) {
        static N: AtomicI64 = AtomicI64::new(1);
        let n = N.fetch_add(1, Ordering::SeqCst);
        let req = Request::builder()
            .method(method)
            .uri(path)
            .header("authorization", format!("Bearer {token}"));
        let mut req = match body {
            Some(b) => req
                .header("content-type", "application/json")
                .body(Body::from(b.to_string()))
                .unwrap(),
            None => req.body(Body::empty()).unwrap(),
        };
        req.extensions_mut().insert(Peer(
            format!("10.5.{}.{}:4000", (n >> 8) & 255, n & 255)
                .parse()
                .unwrap(),
        ));
        let res = self.router.clone().oneshot(req).await.unwrap();
        let status = res.status();
        let bytes = res.into_body().collect().await.unwrap().to_bytes();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }

    async fn subscribe(&self, token: &str, device: &Device, n: u32) -> i64 {
        let b64 = |b: &[u8]| crate::webpush::b64_public(b);
        let (s, v) = self
            .call(
                Method::POST,
                "/api/push/subscriptions",
                token,
                Some(json!({"endpoint": format!("{}/{n}", self.push_url),
                            "keys": {"p256dh": b64(&device.public), "auth": b64(&device.auth)}})),
            )
            .await;
        assert_eq!(s, StatusCode::OK, "{v}");
        v["id"].as_i64().unwrap()
    }

    async fn account(&self, token: &str, server: &TestImapServer) {
        let d = json!({"email": "ann@example.test",
                       "imap": {"host": server.host(), "port": server.port(), "security": "tls", "username": server.username()}});
        let (s, v) = self
            .call(Method::POST, "/api/accounts", token,
                  Some(json!({"discovered": d, "password": PW, "pin": server.certificate_fingerprint()})))
            .await;
        assert_eq!(s, StatusCode::OK, "{v}");
    }

    fn received(&self) -> Vec<(HeaderMap, Vec<u8>)> {
        self.svc.received.lock().unwrap().clone()
    }

    /// Waits until no ping has arrived for two windows; the number received.
    async fn settle(&self) -> usize {
        let mut last = self.received().len();
        let mut quiet = 0;
        while quiet < 2 * WINDOW_MS / 100 + 5 {
            tokio::time::sleep(Duration::from_millis(100)).await;
            let now = self.received().len();
            if now == last {
                quiet += 1;
            } else {
                last = now;
                quiet = 0;
            }
        }
        last
    }

    fn core(&self, user: i64) -> Arc<croft_ffi::Core> {
        self.app.mail.clone().unwrap().open_core(user).unwrap()
    }
}

async fn start_server(messages: u32) -> Arc<TestImapServer> {
    tokio::task::spawn_blocking(move || TestImapServer::start(messages, PW.into()))
        .await
        .unwrap()
}

async fn stop(server: Arc<TestImapServer>) {
    tokio::task::spawn_blocking(move || server.stop())
        .await
        .unwrap();
}

/// Marks a message read and unread a few times (each one a change in the core).
async fn changes(h: &H, user: i64, n: usize) {
    let core = h.core(user);
    let id = tokio::task::spawn_blocking({
        let core = core.clone();
        move || {
            let accounts = core.list_accounts().unwrap();
            let folders = core.list_folders(accounts[0].id).unwrap();
            let inbox = folders
                .iter()
                .find(|f| f.path.eq_ignore_ascii_case("INBOX"))
                .unwrap();
            let page = core
                .list_messages(
                    croft_ffi::MessageScope::Folder {
                        account: accounts[0].id,
                        folder_id: inbox.id,
                    },
                    None,
                    10,
                )
                .unwrap();
            page.rows[0].id
        }
    })
    .await
    .unwrap();
    for i in 0..n {
        let core = core.clone();
        tokio::task::spawn_blocking(move || core.mark_seen(vec![id], i % 2 == 0))
            .await
            .unwrap()
            .unwrap();
        tokio::time::sleep(Duration::from_millis(30)).await;
    }
}

async fn wait_for_inbox(h: &H, user: i64) {
    for _ in 0..150 {
        let core = h.core(user);
        let ready = tokio::task::spawn_blocking(move || {
            let accounts = core.list_accounts().unwrap();
            accounts.first().is_some_and(|a| {
                core.list_folders(a.id)
                    .unwrap()
                    .iter()
                    .any(|f| f.path.eq_ignore_ascii_case("INBOX") && f.total_count > 0)
            })
        })
        .await
        .unwrap();
        if ready {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("no Inbox");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_change_sends_one_encrypted_content_free_ping() {
    let h = h().await;
    let (ann, t) = h.user("ann");
    let server = start_server(2).await;
    h.account(&t, &server).await;
    let device = Device::new();
    h.subscribe(&t, &device, 1).await;
    wait_for_inbox(&h, ann).await;
    let before = h.settle().await;
    changes(&h, ann, 5).await;
    let after = h.settle().await;
    assert!(after > before, "no ping for the changes");
    // Never more than one ping per window (the changes and the server's confirmations of them
    // spread over more than one window, so there may be several, each a window apart).
    let times = h.svc.arrived_ms.lock().unwrap().clone();
    for pair in times.windows(2) {
        assert!(
            pair[1] - pair[0] >= WINDOW_MS - 50,
            "pings {} ms apart",
            pair[1] - pair[0]
        );
    }
    let (headers, body) = h.received().last().cloned().unwrap();
    assert_eq!(device.decrypt(&body), PAYLOAD);
    assert_eq!(headers["content-encoding"], "aes128gcm");
    assert_eq!(headers["ttl"], "3600");
    assert_eq!(headers["urgency"], "normal");
    let auth = headers["authorization"].to_str().unwrap();
    assert!(
        auth.starts_with("vapid t=") && auth.contains(", k="),
        "{auth}"
    );
    // The published key is the one that signed.
    let (_, vapid) = h.call(Method::GET, "/api/push/vapid", &t, None).await;
    assert!(auth.ends_with(vapid["key"].as_str().unwrap()));
    stop(server).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_gone_subscription_is_removed_and_a_signed_out_device_gets_nothing() {
    let h = h().await;
    let (ann, t) = h.user("ann");
    let server = start_server(2).await;
    h.account(&t, &server).await;
    let device = Device::new();
    h.subscribe(&t, &device, 1).await;
    wait_for_inbox(&h, ann).await;
    h.settle().await;
    // 410: the subscription goes.
    h.svc.status.store(410, Ordering::SeqCst);
    changes(&h, ann, 1).await;
    h.settle().await;
    assert!(h
        .app
        .store
        .lock()
        .unwrap()
        .push_list(ann)
        .unwrap()
        .is_empty());
    // A new subscription, then the device signs out: no more pings.
    h.svc.status.store(201, Ordering::SeqCst);
    h.subscribe(&t, &device, 2).await;
    let (s, _) = h.call(Method::POST, "/api/sign-out", &t, None).await;
    assert_eq!(s, StatusCode::OK);
    let before = h.settle().await;
    changes(&h, ann, 3).await;
    assert_eq!(h.settle().await, before, "a signed-out device was pinged");
    stop(server).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn one_users_changes_never_ping_another_users_devices() {
    let h = h().await;
    let (ann, ann_t) = h.user("ann");
    let (_, bob_t) = h.user("bob");
    let server = start_server(2).await;
    h.account(&ann_t, &server).await;
    let bob_device = Device::new();
    h.subscribe(&bob_t, &bob_device, 7).await;
    wait_for_inbox(&h, ann).await;
    h.settle().await;
    changes(&h, ann, 3).await;
    h.settle().await;
    assert!(
        h.received().is_empty(),
        "Bob's device was pinged for Ann's mail"
    );
    // Bob cannot see or remove Ann's subscriptions (and the list holds no endpoint).
    let ann_device = Device::new();
    let ann_sub = h.subscribe(&ann_t, &ann_device, 8).await;
    let (_, list) = h
        .call(Method::GET, "/api/push/subscriptions", &bob_t, None)
        .await;
    assert!(list["subscriptions"]
        .as_array()
        .unwrap()
        .iter()
        .all(|s| s["id"] != ann_sub));
    assert!(!list.to_string().contains("push/"));
    let (s, _) = h
        .call(
            Method::DELETE,
            &format!("/api/push/subscriptions/{ann_sub}"),
            &bob_t,
            None,
        )
        .await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    stop(server).await;
}

#[tokio::test]
async fn bad_subscriptions_are_refused() {
    let h = h().await;
    let (_, t) = h.user("ann");
    let device = Device::new();
    let b64 = |b: &[u8]| crate::webpush::b64_public(b);
    for (endpoint, p256dh, auth) in [
        (
            "http://push.example/x".to_owned(),
            b64(&device.public),
            b64(&device.auth),
        ),
        (
            "https://user:pw@push.example/x".to_owned(),
            b64(&device.public),
            b64(&device.auth),
        ),
        (
            "not a url".to_owned(),
            b64(&device.public),
            b64(&device.auth),
        ),
        (
            format!("https://push.example/{}", "x".repeat(3000)),
            b64(&device.public),
            b64(&device.auth),
        ),
        (
            "https://push.example/x".to_owned(),
            b64(&[4u8; 65]),
            b64(&device.auth),
        ),
        (
            "https://push.example/x".to_owned(),
            b64(&device.public),
            b64(&[1u8; 8]),
        ),
    ] {
        let (s, _) = h
            .call(
                Method::POST,
                "/api/push/subscriptions",
                &t,
                Some(json!({"endpoint": endpoint, "keys": {"p256dh": p256dh, "auth": auth}})),
            )
            .await;
        assert_eq!(s, StatusCode::BAD_REQUEST, "{endpoint}");
    }
}
