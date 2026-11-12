// This Source Code Form is subject to the terms of the GNU Affero General Public License,
// version 3 or later. See LICENSE-AGPL-3.0 at the repository root.

//! Tests of mail accounts in the Companion (T26) through the real router, against the core's
//! in-process IMAP test server (a self-signed certificate, made-up passwords).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use axum::Router;
use croft_ffi::TestImapServer;
use http_body_util::BodyExt;
use serde_json::{json, Value};
use tower::ServiceExt;

use super::*;
use crate::http::{router, App};
use crate::mail::MailCores;
use crate::store::Role;

const ORIGIN: &str = "https://companion.test";
const ANN_PW: &str = "made-up-imap-password-ann";
const BOB_PW: &str = "made-up-imap-password-bob";

struct H {
    app: App,
    router: Router,
    master: Key,
    _dir: tempfile::TempDir,
}

fn h() -> H {
    let dir = tempfile::tempdir().unwrap();
    let master = Key::generate().unwrap();
    let store = Store::in_memory().unwrap();
    store.check_master_key(&master).unwrap();
    let auth = Auth::new(ORIGIN, master.clone(), Vec::new(), Auth::system_clock()).unwrap();
    let app = App::new(store, auth).with_mail(MailCores::new(dir.path().to_path_buf()));
    H {
        router: router(app.clone()),
        app,
        master,
        _dir: dir,
    }
}

impl H {
    fn store(&self) -> std::sync::MutexGuard<'_, Store> {
        self.app.store.lock().unwrap()
    }

    fn user(&self, name: &str) -> (i64, String) {
        let id = self
            .store()
            .add_user(&self.master, name, Role::User)
            .unwrap()
            .id;
        let token = self
            .store()
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
        let ip = format!("10.7.{}.{}:4000", (n >> 8) & 255, n & 255);
        req.extensions_mut().insert(Peer(ip.parse().unwrap()));
        let res = self.router.clone().oneshot(req).await.unwrap();
        let status = res.status();
        let bytes = res.into_body().collect().await.unwrap().to_bytes();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }
}

/// The test server runs a runtime of its own: start and stop it off the test's async threads.
async fn start_server(messages: u32, password: &'static str) -> Arc<TestImapServer> {
    tokio::task::spawn_blocking(move || TestImapServer::start(messages, password.into()))
        .await
        .unwrap()
}

async fn stop(server: Arc<TestImapServer>) {
    tokio::task::spawn_blocking(move || server.stop())
        .await
        .unwrap();
}

fn discovered(email: &str, server: &TestImapServer) -> Value {
    json!({
        "email": email,
        "imap": {"host": server.host(), "port": server.port(), "security": "tls", "username": server.username()},
    })
}

/// Adds an account the way an app does: verify, confirm the certificate, verify with the pin, add.
async fn add(h: &H, token: &str, email: &str, server: &TestImapServer, password: &str) -> i64 {
    let d = discovered(email, server);
    let (s, v) = h
        .call(
            Method::POST,
            "/api/accounts/verify",
            token,
            Some(json!({"discovered": d, "password": password})),
        )
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{v}");
    assert_eq!(v["reason"], "untrusted_certificate");
    let fingerprint = v["fingerprint"].as_str().unwrap().to_owned();
    assert_eq!(
        fingerprint.to_lowercase().replace(':', ""),
        server
            .certificate_fingerprint()
            .to_lowercase()
            .replace(':', "")
    );
    let (s, v) = h
        .call(
            Method::POST,
            "/api/accounts/verify",
            token,
            Some(json!({"discovered": d, "password": password, "pin": fingerprint})),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let (s, v) = h
        .call(
            Method::POST,
            "/api/accounts",
            token,
            Some(json!({"discovered": d, "password": password, "pin": fingerprint, "display_name": "Mail"})),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    v["id"].as_i64().unwrap()
}

async fn list(h: &H, token: &str) -> Vec<Value> {
    let (s, v) = h.call(Method::GET, "/api/accounts", token, None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    v["accounts"].as_array().unwrap().clone()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_user_adds_lists_and_removes_an_account_after_confirming_the_certificate() {
    let h = h();
    let (ann, t) = h.user("ann");
    let server = start_server(3, ANN_PW).await;
    // A wrong password is refused (once the certificate is confirmed).
    let d = discovered("ann@example.test", &server);
    let fp = server.certificate_fingerprint();
    let (s, v) = h
        .call(
            Method::POST,
            "/api/accounts/verify",
            &t,
            Some(json!({"discovered": d, "password": "made-up-wrong", "pin": fp})),
        )
        .await;
    assert_eq!(
        (s, v["reason"].as_str()),
        (StatusCode::UNPROCESSABLE_ENTITY, Some("wrong_password"))
    );
    let id = add(&h, &t, "ann@example.test", &server, ANN_PW).await;
    let accounts = list(&h, &t).await;
    assert_eq!(accounts.len(), 1);
    assert_eq!(accounts[0]["id"], id);
    assert_eq!(accounts[0]["email"], "ann@example.test");
    // The password is sealed, for this user and account only.
    let stored = h
        .store()
        .mail_credential(&h.master, ann, id)
        .unwrap()
        .unwrap();
    assert_eq!(stored.as_slice(), ANN_PW.as_bytes());
    // Removed: the account and its sealed password are gone.
    let (s, _) = h
        .call(Method::DELETE, &format!("/api/accounts/{id}"), &t, None)
        .await;
    assert_eq!(s, StatusCode::OK);
    assert!(list(&h, &t).await.is_empty());
    assert!(h
        .store()
        .mail_credential(&h.master, ann, id)
        .unwrap()
        .is_none());
    let (s, _) = h
        .call(Method::DELETE, &format!("/api/accounts/{id}"), &t, None)
        .await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    stop(server).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn one_user_never_reaches_another_users_accounts_or_mail_database() {
    let h = h();
    let (ann, ann_t) = h.user("ann");
    let (bob, bob_t) = h.user("bob");
    let ann_server = start_server(2, ANN_PW).await;
    let bob_server = start_server(2, BOB_PW).await;
    let ann_account = add(&h, &ann_t, "ann@example.test", &ann_server, ANN_PW).await;
    let bob_account = add(&h, &bob_t, "bob@example.test", &bob_server, BOB_PW).await;
    // Each sees only their own.
    let ann_list = list(&h, &ann_t).await;
    let bob_list = list(&h, &bob_t).await;
    assert_eq!(ann_list.len(), 1);
    assert_eq!(bob_list.len(), 1);
    assert_eq!(ann_list[0]["email"], "ann@example.test");
    assert_eq!(bob_list[0]["email"], "bob@example.test");
    // Ids are per user: Bob removing "Ann's id" removes Bob's own account of that id, if any, never
    // Ann's (here both are 1); Ann's account and password stay.
    assert_eq!(ann_account, bob_account);
    let (s, _) = h
        .call(
            Method::DELETE,
            &format!("/api/accounts/{ann_account}"),
            &bob_t,
            None,
        )
        .await;
    assert_eq!(s, StatusCode::OK);
    assert!(list(&h, &bob_t).await.is_empty());
    assert_eq!(list(&h, &ann_t).await.len(), 1);
    assert!(h
        .store()
        .mail_credential(&h.master, ann, ann_account)
        .unwrap()
        .is_some());
    // Each user's mail is a database of its own, under a key of its own: Bob's key does not open
    // Ann's file, and neither file is plain SQLite.
    let mail = h.app.mail.clone().unwrap();
    let ann_db = mail.database(ann);
    let bob_db = mail.database(bob);
    assert_ne!(ann_db, bob_db);
    let bob_key = h
        .store()
        .data_key(&h.master, bob)
        .unwrap()
        .derive(crate::mail::MAIL_DB_KEY);
    let path = ann_db.to_string_lossy().into_owned();
    let key = bob_key.to_vec();
    let opened = tokio::task::spawn_blocking(move || {
        croft_ffi::Core::open(path, Some(key), Arc::new(NoCredentials)).is_ok()
    })
    .await
    .unwrap();
    assert!(!opened, "Bob's key opened Ann's mail");
    for file in [&ann_db, &bob_db] {
        let bytes = std::fs::read(file).unwrap();
        assert!(
            !bytes.starts_with(b"SQLite format 3"),
            "{file:?} is not encrypted"
        );
        for pw in [ANN_PW, BOB_PW] {
            assert!(!bytes.windows(pw.len()).any(|w| w == pw.as_bytes()));
        }
    }
    stop(ann_server).await;
    stop(bob_server).await;
}

struct NoCredentials;

impl croft_ffi::CredentialProvider for NoCredentials {
    fn credential(
        &self,
        _: i64,
    ) -> Result<Option<Arc<croft_ffi::FfiCredential>>, croft_ffi::ProviderError> {
        Ok(None)
    }
    fn sign_in_needed(&self, _: i64) -> Result<(), croft_ffi::ProviderError> {
        Ok(())
    }
    fn save_tokens(&self, _: i64, _: Vec<u8>) -> Result<(), croft_ffi::ProviderError> {
        Ok(())
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn after_a_restart_the_core_signs_in_with_the_sealed_password() {
    let h = h();
    let (ann, t) = h.user("ann");
    let server = start_server(2, ANN_PW).await;
    add(&h, &t, "ann@example.test", &server, ANN_PW).await;
    // The server process restarts: the user's core is closed and opened again from disk.
    let mail = h.app.mail.clone().unwrap();
    tokio::task::spawn_blocking(move || mail.close(ann))
        .await
        .unwrap();
    server.clear_commands();
    let before = server.connection_count();
    let app = h.app.clone();
    tokio::task::spawn_blocking(move || app.open_all_mail())
        .await
        .unwrap();
    let mut logged_in = false;
    for _ in 0..100 {
        if server.connection_count() > before
            && server.commands().iter().any(|c| {
                c.to_ascii_uppercase().contains("LOGIN")
                    || c.to_ascii_uppercase().contains("AUTHENTICATE")
            })
        {
            logged_in = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    assert!(logged_in, "the reopened core did not sign in");
    let accounts = list(&h, &t).await;
    assert_ne!(accounts[0]["status"], "needs_sign_in");
    stop(server).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn no_password_is_stored_answered_or_audited_in_the_clear() {
    let h = h();
    let (_, t) = h.user("ann");
    let server = start_server(1, ANN_PW).await;
    add(&h, &t, "ann@example.test", &server, ANN_PW).await;
    let (_, v) = h.call(Method::GET, "/api/accounts", &t, None).await;
    assert!(!v.to_string().contains(ANN_PW));
    check_no_password_stored(&h);
    stop(server).await;
}

fn check_no_password_stored(h: &H) {
    let store = h.store();
    for (table, _) in store.tables_and_user_scope().unwrap() {
        let mut stmt = store
            .conn
            .prepare(&format!("SELECT * FROM {table}"))
            .unwrap();
        let n = stmt.column_count();
        let mut rows = stmt.query([]).unwrap();
        while let Some(row) = rows.next().unwrap() {
            for i in 0..n {
                if let rusqlite::types::ValueRef::Text(b) | rusqlite::types::ValueRef::Blob(b) =
                    row.get_ref(i).unwrap()
                {
                    assert!(
                        !b.windows(ANN_PW.len()).any(|w| w == ANN_PW.as_bytes()),
                        "{table}"
                    );
                }
            }
        }
    }
    assert!(store
        .audit_entries(None, 50)
        .unwrap()
        .iter()
        .any(|e| e.action == "mail_account.add"));
}

#[tokio::test]
async fn bad_settings_and_oauth_only_providers_are_refused() {
    let h = h();
    let (_, t) = h.user("ann");
    for bad in [
        json!({"email": "no-at-sign", "imap": {"host": "x", "port": 993, "security": "tls", "username": "u"}}),
        json!({"email": "a@b", "imap": {"host": "x", "port": 993, "security": "plain", "username": "u"}}),
        json!({"email": "a@b", "imap": {"host": "x\n", "port": 993, "security": "tls", "username": "u"}}),
        json!({"email": "a@b", "imap": {"host": "x", "port": 0, "security": "tls", "username": "u"}}),
        json!({"email": "a@b", "provider": "aol"}),
    ] {
        let (s, _) = h
            .call(
                Method::POST,
                "/api/accounts/verify",
                &t,
                Some(json!({"discovered": bad, "password": "x"})),
            )
            .await;
        assert_eq!(s, StatusCode::BAD_REQUEST, "{bad}");
    }
    let gmail = json!({"email": "a@gmail.com", "provider": "gmail", "auth_hint": "oauth2",
                       "imap": {"host": "imap.gmail.com", "port": 993, "security": "tls", "username": "a@gmail.com"}});
    let (s, v) = h
        .call(
            Method::POST,
            "/api/accounts",
            &t,
            Some(json!({"discovered": gmail, "password": "x"})),
        )
        .await;
    assert_eq!(
        (s, v["reason"].as_str()),
        (
            StatusCode::UNPROCESSABLE_ENTITY,
            Some("oauth_not_supported")
        )
    );
}
