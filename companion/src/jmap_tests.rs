// This Source Code Form is subject to the terms of the GNU Affero General Public License,
// version 3 or later. See LICENSE-AGPL-3.0 at the repository root.

//! Tests of the JMAP API (T27) through the real router: a user's IMAP account on the core's
//! in-process test server (made-up password), read and organised through JMAP, with the changes
//! checked on the test server itself.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;
use std::time::Duration;

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
const PW: &str = "made-up-imap-password";
const USING: [&str; 2] = ["urn:ietf:params:jmap:core", "urn:ietf:params:jmap:mail"];

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
    fn user(&self, name: &str) -> (i64, String) {
        let id = self
            .app
            .store
            .lock()
            .unwrap()
            .add_user(&self.master, name, Role::User)
            .unwrap()
            .id;
        let token = self
            .app
            .store
            .lock()
            .unwrap()
            .start_session(id, "test", false, crate::store::now_ms())
            .unwrap()
            .access
            .to_string();
        (id, token)
    }

    async fn raw(
        &self,
        method: Method,
        path: &str,
        token: &str,
        body: Option<Vec<u8>>,
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
                .body(Body::from(b))
                .unwrap(),
            None => req.body(Body::empty()).unwrap(),
        };
        let ip = format!("10.6.{}.{}:4000", (n >> 8) & 255, n & 255);
        req.extensions_mut().insert(Peer(ip.parse().unwrap()));
        let res = self.router.clone().oneshot(req).await.unwrap();
        let status = res.status();
        let bytes = res.into_body().collect().await.unwrap().to_bytes();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }

    /// One JMAP request; the method responses.
    async fn jmap(&self, token: &str, calls: Value) -> Vec<Value> {
        let body = json!({ "using": USING, "methodCalls": calls });
        let (s, v) = self
            .raw(
                Method::POST,
                "/jmap/api",
                token,
                Some(body.to_string().into_bytes()),
            )
            .await;
        assert_eq!(s, StatusCode::OK, "{v}");
        v["methodResponses"].as_array().unwrap().clone()
    }

    /// Adds an account on the test server (confirming its certificate) and returns its JMAP id.
    async fn account(&self, token: &str, server: &TestImapServer) -> String {
        let d = json!({
            "email": "ann@example.test",
            "imap": {"host": server.host(), "port": server.port(), "security": "tls", "username": server.username()},
        });
        let fp = server.certificate_fingerprint();
        let body = json!({"discovered": d, "password": PW, "pin": fp});
        let (s, v) = self
            .raw(
                Method::POST,
                "/api/accounts",
                token,
                Some(body.to_string().into_bytes()),
            )
            .await;
        assert_eq!(s, StatusCode::OK, "{v}");
        format!("a{}", v["id"])
    }

    /// Waits until the account's Inbox shows `n` emails (the first sync).
    async fn inbox(&self, token: &str, account: &str, n: u64) -> String {
        for _ in 0..150 {
            let r = self
                .jmap(token, json!([["Mailbox/get", {"accountId": account}, "0"]]))
                .await;
            if let Some(inbox) = r[0][1]["list"]
                .as_array()
                .and_then(|l| l.iter().find(|m| m["role"] == "inbox"))
            {
                if inbox["totalEmails"].as_u64() == Some(n) {
                    return inbox["id"].as_str().unwrap().to_owned();
                }
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        panic!("the Inbox never showed {n} emails");
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

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_client_reads_the_inbox_through_jmap() {
    let h = h();
    let (_, t) = h.user("ann");
    let server = start_server(3).await;
    let account = h.account(&t, &server).await;
    // The session lists the account.
    let (s, session) = h.raw(Method::GET, "/jmap/session", &t, None).await;
    assert_eq!(s, StatusCode::OK);
    assert!(session["accounts"][&account].is_object(), "{session}");
    assert_eq!(
        session["primaryAccounts"]["urn:ietf:params:jmap:mail"],
        account
    );
    assert_eq!(session["apiUrl"], "https://companion.test/jmap/api");
    let inbox = h.inbox(&t, &account, 3).await;
    // Query then get by result reference, with the text body.
    let r = h
        .jmap(
            &t,
            json!([
                ["Email/query", {"accountId": account, "filter": {"inMailbox": inbox},
                                 "sort": [{"property": "receivedAt", "isAscending": false}],
                                 "calculateTotal": true}, "q"],
                ["Email/get", {"accountId": account,
                               "#ids": {"resultOf": "q", "name": "Email/query", "path": "/ids"},
                               "fetchTextBodyValues": true}, "g"],
            ]),
        )
        .await;
    assert_eq!(r[0][0], "Email/query", "{r:?}");
    let ids = r[0][1]["ids"].as_array().unwrap();
    assert_eq!(ids.len(), 3);
    assert_eq!(r[0][1]["total"], 3);
    assert_eq!(r[1][0], "Email/get", "{r:?}");
    let list = r[1][1]["list"].as_array().unwrap();
    assert_eq!(list.len(), 3);
    let dates: Vec<&str> = list
        .iter()
        .map(|e| e["receivedAt"].as_str().unwrap())
        .collect();
    let mut sorted = dates.clone();
    sorted.sort_by(|a, b| b.cmp(a));
    assert_eq!(dates, sorted, "newest first");
    for e in list {
        assert_eq!(e["mailboxIds"][&inbox], true);
        assert!(e["threadId"].as_str().unwrap().starts_with('t'));
        assert!(e["subject"].is_string());
        assert!(e["bodyValues"]["text"]["value"].is_string(), "{e}");
    }
    // Thread/get and Identity/get answer.
    let thread = list[0]["threadId"].clone();
    let r = h
        .jmap(
            &t,
            json!([
                ["Thread/get", {"accountId": account, "ids": [thread]}, "t"],
                ["Identity/get", {"accountId": account}, "i"],
                ["Core/echo", {"hello": true}, "e"],
            ]),
        )
        .await;
    assert!(!r[0][1]["list"][0]["emailIds"]
        .as_array()
        .unwrap()
        .is_empty());
    assert_eq!(r[1][0], "Identity/get");
    assert_eq!(r[2], json!(["Core/echo", {"hello": true}, "e"]));
    stop(server).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn changes_made_through_jmap_reach_the_server() {
    let h = h();
    let (ann, t) = h.user("ann");
    let server = start_server(3).await;
    let account = h.account(&t, &server).await;
    // Moves and deletes wait the undo window; short for the test.
    h.app
        .mail
        .clone()
        .unwrap()
        .open_core(ann)
        .unwrap()
        .set_undo_window_ms(100);
    let inbox = h.inbox(&t, &account, 3).await;
    let r = h
        .jmap(
            &t,
            json!([
                ["Mailbox/get", {"accountId": account}, "m"],
                ["Email/query", {"accountId": account, "filter": {"inMailbox": inbox}}, "q"],
                ["Email/get", {"accountId": account,
                               "#ids": {"resultOf": "q", "name": "Email/query", "path": "/ids"},
                               "properties": ["subject"]}, "g"],
            ]),
        )
        .await;
    let mailbox = |role: &str| -> String {
        r[0][1]["list"]
            .as_array()
            .unwrap()
            .iter()
            .find(|m| m["role"] == role)
            .unwrap()["id"]
            .as_str()
            .unwrap()
            .to_owned()
    };
    let archive = mailbox("archive");
    let emails = r[2][1]["list"].as_array().unwrap().clone();
    let (a, b, c) = (&emails[0], &emails[1], &emails[2]);
    let subject = |e: &Value| e["subject"].as_str().unwrap().to_owned();
    let r = h
        .jmap(
            &t,
            json!([["Email/set", {"accountId": account,
                "update": {
                    a["id"].as_str().unwrap(): {"keywords/$seen": true, "keywords/$flagged": true},
                    b["id"].as_str().unwrap(): {"mailboxIds": {archive.clone(): true}},
                    "m999999": {"keywords/$seen": true},
                },
                "destroy": [c["id"]],
            }, "s"]]),
        )
        .await;
    let set = &r[0][1];
    assert_eq!(r[0][0], "Email/set", "{r:?}");
    assert!(
        set["updated"].get(a["id"].as_str().unwrap()).is_some(),
        "{set}"
    );
    assert!(
        set["updated"].get(b["id"].as_str().unwrap()).is_some(),
        "{set}"
    );
    assert_eq!(set["notUpdated"]["m999999"]["type"], "notFound");
    assert_eq!(set["destroyed"], json!([c["id"]]));
    // On the server: read and flagged, moved to Archive, moved to Trash.
    let (sa, sb, sc) = (subject(a), subject(b), subject(c));
    let mut ok = false;
    for _ in 0..100 {
        let s2 = server.clone();
        let (flags, archived, trashed) = tokio::task::spawn_blocking({
            let (sa, sb, sc) = (sa.clone(), sb.clone(), sc.clone());
            move || {
                (
                    s2.flags_of_subject("INBOX".into(), sa),
                    s2.subjects("Archive".into()).contains(&sb),
                    s2.subjects("Trash".into()).contains(&sc),
                )
            }
        })
        .await
        .unwrap();
        if flags.iter().any(|f| f == "\\Seen")
            && flags.iter().any(|f| f == "\\Flagged")
            && archived
            && trashed
        {
            ok = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(ok, "the changes did not reach the server");
    stop(server).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn states_and_changes_follow_the_core() {
    let h = h();
    let (_, t) = h.user("ann");
    let server = start_server(1).await;
    let account = h.account(&t, &server).await;
    h.inbox(&t, &account, 1).await;
    let r = h
        .jmap(&t, json!([["Mailbox/get", {"accountId": account}, "m"]]))
        .await;
    let state = r[0][1]["state"].as_str().unwrap().to_owned();
    let r = h
        .jmap(
            &t,
            json!([
                ["Mailbox/changes", {"accountId": account, "sinceState": state}, "a"],
                ["Email/changes", {"accountId": account, "sinceState": "0-0"}, "b"],
            ]),
        )
        .await;
    assert_eq!(r[0][0], "Mailbox/changes", "{r:?}");
    assert_eq!(r[0][1]["created"], json!([]));
    assert_eq!(
        r[1],
        json!(["error", {"type": "cannotCalculateChanges"}, "b"])
    );
    stop(server).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn one_user_never_reaches_another_users_account_or_mail() {
    let h = h();
    let (_, ann_t) = h.user("ann");
    let (_, bob_t) = h.user("bob");
    let ann_server = start_server(2).await;
    let bob_server = start_server(1).await;
    let ann_account = h.account(&ann_t, &ann_server).await;
    let bob_account = h.account(&bob_t, &bob_server).await;
    let ann_inbox = h.inbox(&ann_t, &ann_account, 2).await;
    h.inbox(&bob_t, &bob_account, 1).await;
    let r = h
        .jmap(
            &ann_t,
            json!([["Email/query", {"accountId": ann_account, "filter": {"inMailbox": ann_inbox}}, "q"]]),
        )
        .await;
    let ann_email = r[0][1]["ids"][0].clone();
    // Both users' first account has the same id in their own core: Bob naming "Ann's" account
    // names his own, whose objects are his. Ann's email and mailbox ids are not his.
    assert_eq!(ann_account, bob_account);
    let r = h
        .jmap(
            &bob_t,
            json!([
                ["Email/get", {"accountId": bob_account, "ids": [ann_email.clone()]}, "g"],
                ["Email/query", {"accountId": bob_account, "filter": {"inMailbox": ann_inbox}}, "q"],
                ["Mailbox/get", {"accountId": "a999"}, "m"],
                ["Email/set", {"accountId": bob_account, "destroy": [ann_email.clone()]}, "s"],
            ]),
        )
        .await;
    // Bob's own message 1 or 2 may share Ann's id; what he gets is his own mail, never Ann's.
    if let Some(got) = r[0][1]["list"].as_array().and_then(|l| l.first()) {
        let ann_subjects = tokio::task::spawn_blocking({
            let s = ann_server.clone();
            move || s.subjects("INBOX".into())
        })
        .await
        .unwrap();
        let bob_subjects = tokio::task::spawn_blocking({
            let s = bob_server.clone();
            move || s.subjects("INBOX".into())
        })
        .await
        .unwrap();
        let subj = got["subject"].as_str().unwrap().to_owned();
        assert!(bob_subjects.contains(&subj) || !ann_subjects.contains(&subj));
    }
    assert_eq!(r[2], json!(["error", {"type": "accountNotFound"}, "m"]));
    // Ann's mail is untouched.
    let r = h
        .jmap(
            &ann_t,
            json!([["Email/query", {"accountId": ann_account, "filter": {"inMailbox": ann_inbox}}, "q"]]),
        )
        .await;
    assert_eq!(r[0][1]["ids"].as_array().unwrap().len(), 2);
    stop(ann_server).await;
    stop(bob_server).await;
}

#[tokio::test]
async fn malformed_requests_get_the_rfc_errors() {
    let h = h();
    let (_, t) = h.user("ann");
    let cases: Vec<(Vec<u8>, &str)> = vec![
        (b"not json".to_vec(), "urn:ietf:params:jmap:error:notJSON"),
        (
            json!({"methodCalls": []}).to_string().into_bytes(),
            "urn:ietf:params:jmap:error:notRequest",
        ),
        (
            json!({"using": ["urn:example:unknown"], "methodCalls": []})
                .to_string()
                .into_bytes(),
            "urn:ietf:params:jmap:error:unknownCapability",
        ),
        (
            json!({"using": USING, "methodCalls": vec![json!(["Core/echo", {}, "x"]); 17]})
                .to_string()
                .into_bytes(),
            "urn:ietf:params:jmap:error:limit",
        ),
    ];
    for (body, kind) in cases {
        let (s, v) = h.raw(Method::POST, "/jmap/api", &t, Some(body)).await;
        assert_eq!(
            (s, v["type"].as_str()),
            (StatusCode::BAD_REQUEST, Some(kind))
        );
    }
    let r = h
        .jmap(
            &t,
            json!([
                ["Email/import", {}, "a"],
                ["Mailbox/get", {"accountId": "a1"}, "b"],
                ["Email/get", {"accountId": "a1", "#ids": {"resultOf": "zz", "name": "Email/query", "path": "/ids"}}, "c"],
            ]),
        )
        .await;
    assert_eq!(r[0], json!(["error", {"type": "unknownMethod"}, "a"]));
    assert_eq!(r[1], json!(["error", {"type": "accountNotFound"}, "b"]));
    assert_eq!(
        r[2],
        json!(["error", {"type": "invalidResultReference"}, "c"])
    );
    // No route works without a full session.
    let (s, _) = h
        .raw(Method::GET, "/jmap/session", "not-a-token", None)
        .await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
}

// ---- Regression tests for the independent review's findings ----

impl H {
    async fn post_raw(&self, token: &str, body: Value) -> (StatusCode, usize, Value) {
        let bytes = body.to_string().into_bytes();
        let mut req = Request::builder()
            .method(Method::POST)
            .uri("/jmap/api")
            .header("authorization", format!("Bearer {token}"))
            .header("content-type", "application/json")
            .body(Body::from(bytes))
            .unwrap();
        req.extensions_mut()
            .insert(Peer("10.9.9.9:4000".parse().unwrap()));
        let res = self.router.clone().oneshot(req).await.unwrap();
        let status = res.status();
        let out = res.into_body().collect().await.unwrap().to_bytes();
        (
            status,
            out.len(),
            serde_json::from_slice(&out).unwrap_or(Value::Null),
        )
    }

    async fn second_account(&self, token: &str, server: &TestImapServer, email: &str) -> String {
        let d = json!({
            "email": email,
            "imap": {"host": server.host(), "port": server.port(), "security": "tls", "username": server.username()},
        });
        let body =
            json!({"discovered": d, "password": PW, "pin": server.certificate_fingerprint()});
        let (s, v) = self
            .raw(
                Method::POST,
                "/api/accounts",
                token,
                Some(body.to_string().into_bytes()),
            )
            .await;
        assert_eq!(s, StatusCode::OK, "{v}");
        format!("a{}", v["id"])
    }
}

/// H1: result references multiplied a small request into a huge answer.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn result_references_cannot_amplify_a_request() {
    let h = h();
    let (_, t) = h.user("ann");
    let mut refs = serde_json::Map::new();
    for i in 0..8 {
        refs.insert(
            format!("#k{i}"),
            json!({"resultOf": "0", "name": "Core/echo", "path": ""}),
        );
    }
    let mut calls = vec![json!(["Core/echo", {"b": "x".repeat(30_000)}, "0"])];
    for i in 1..16 {
        let mut r = refs.clone();
        for v in r.values_mut() {
            v["resultOf"] = json!((i - 1).to_string());
        }
        calls.push(json!(["Core/echo", r, i.to_string()]));
    }
    let (s, len, v) = h
        .post_raw(&t, json!({"using": USING, "methodCalls": calls}))
        .await;
    assert_eq!(s, StatusCode::OK);
    assert!(
        len < 2 * crate::api::jmap_api::MAX_RESPONSE_BYTES,
        "{len} bytes"
    );
    assert!(v["methodResponses"]
        .as_array()
        .unwrap()
        .iter()
        .any(|r| r[1]["type"] == "requestTooLarge"));
    // More than eight references in one call are refused.
    let mut many = serde_json::Map::new();
    for i in 0..9 {
        many.insert(
            format!("#k{i}"),
            json!({"resultOf": "0", "name": "Core/echo", "path": ""}),
        );
    }
    let r = h
        .jmap(
            &t,
            json!([["Core/echo", {}, "0"], ["Core/echo", many, "1"]]),
        )
        .await;
    assert_eq!(r[1], json!(["error", {"type": "requestTooLarge"}, "1"]));
}

/// M5: JMAP requests shared the 30-a-minute account limit.
#[tokio::test]
async fn ordinary_use_is_not_rate_limited() {
    let h = h();
    let (_, t) = h.user("ann");
    for i in 0..60 {
        let (s, _) = h
            .raw(
                Method::POST,
                "/jmap/api",
                &t,
                Some(
                    json!({"using": USING, "methodCalls": [["Core/echo", {}, "e"]]})
                        .to_string()
                        .into_bytes(),
                ),
            )
            .await;
        assert_eq!(s, StatusCode::OK, "request {i}");
    }
}

/// M2: a restore from Trash followed by a destroy deleted the email for good.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_destroy_while_a_restore_is_pending_never_deletes_for_good() {
    let h = h();
    let (ann, t) = h.user("ann");
    let server = start_server(1).await;
    let s2 = server.clone();
    tokio::task::spawn_blocking(move || s2.inject_message("Trash".into(), "Victim".into()))
        .await
        .unwrap();
    let account = h.account(&t, &server).await;
    let core = h.app.mail.clone().unwrap().open_core(ann).unwrap();
    let inbox = h.inbox(&t, &account, 1).await;
    let mut trash = String::new();
    for _ in 0..150 {
        let r = h
            .jmap(&t, json!([["Mailbox/get", {"accountId": account}, "0"]]))
            .await;
        if let Some(tr) = r[0][1]["list"]
            .as_array()
            .unwrap()
            .iter()
            .find(|m| m["role"] == "trash")
        {
            if tr["totalEmails"] == 1 {
                trash = tr["id"].as_str().unwrap().to_owned();
                break;
            }
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let r = h
        .jmap(
            &t,
            json!([["Email/query", {"accountId": account, "filter": {"inMailbox": trash}}, "q"]]),
        )
        .await;
    let victim = r[0][1]["ids"][0].as_str().unwrap().to_owned();
    core.set_undo_window_ms(1500);
    h.jmap(
        &t,
        json!([["Email/set", {"accountId": account, "update": {victim.clone(): {"mailboxIds": {inbox.clone(): true}}}}, "s"]]),
    )
    .await;
    let r = h
        .jmap(
            &t,
            json!([["Email/set", {"accountId": account, "destroy": [victim.clone()]}, "d"]]),
        )
        .await;
    assert_eq!(
        r[0][1]["notDestroyed"][&victim]["type"], "forbidden",
        "{r:?}"
    );
    tokio::time::sleep(Duration::from_millis(3000)).await;
    let s2 = server.clone();
    let (inb, tr) = tokio::task::spawn_blocking(move || {
        (s2.subjects("INBOX".into()), s2.subjects("Trash".into()))
    })
    .await
    .unwrap();
    assert!(
        inb.iter().chain(tr.iter()).any(|s| s.contains("Victim")),
        "deleted for good"
    );
    stop(server).await;
}

/// M4 and M6: the total counted only the rows read; a huge position overflowed.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn totals_are_whole_and_positions_are_bounded() {
    let h = h();
    let (_, t) = h.user("ann");
    let server = start_server(250).await;
    let account = h.account(&t, &server).await;
    let inbox = h.inbox(&t, &account, 250).await;
    let r = h
        .jmap(
            &t,
            json!([
                ["Email/query", {"accountId": account, "filter": {"inMailbox": inbox}, "limit": 10, "calculateTotal": true}, "a"],
                ["Email/query", {"accountId": account, "filter": {"inMailbox": inbox}, "position": u64::MAX, "limit": 10}, "b"],
                ["Email/query", {"accountId": account, "filter": {"inMailbox": inbox}, "position": -1}, "c"],
                ["Email/query", {"accountId": account, "filter": {"inMailbox": inbox}, "limit": 100000}, "d"],
            ]),
        )
        .await;
    assert_eq!(r[0][1]["total"], 250);
    assert_eq!(r[0][1]["ids"].as_array().unwrap().len(), 10);
    assert_eq!(r[1][1]["ids"], json!([]));
    assert_eq!(r[2][0], "error");
    assert_eq!(r[3][1]["limit"], 256);
    stop(server).await;
}

/// M6: an Email/get of many emails with bodies is refused, not run.
#[tokio::test]
async fn email_get_is_bounded() {
    let h = h();
    let (_, t) = h.user("ann");
    let server = start_server(1).await;
    let account = h.account(&t, &server).await;
    let ids: Vec<String> = (1..=17).map(|i| format!("m{i}")).collect();
    let many: Vec<String> = (1..=65).map(|i| format!("m{i}")).collect();
    let r = h
        .jmap(
            &t,
            json!([
                ["Email/get", {"accountId": account, "ids": ids, "fetchTextBodyValues": true}, "a"],
                ["Email/get", {"accountId": account, "ids": many}, "b"],
            ]),
        )
        .await;
    assert_eq!(r[0], json!(["error", {"type": "requestTooLarge"}, "a"]));
    assert_eq!(r[1], json!(["error", {"type": "requestTooLarge"}, "b"]));
    stop(server).await;
}

/// M7 and the patch rules: `mailboxIds/<id>` moves; keyword values other than true or null are
/// refused before anything is applied.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn patches_follow_rfc_8620() {
    let h = h();
    let (ann, t) = h.user("ann");
    let server = start_server(2).await;
    let account = h.account(&t, &server).await;
    h.app
        .mail
        .clone()
        .unwrap()
        .open_core(ann)
        .unwrap()
        .set_undo_window_ms(100);
    let inbox = h.inbox(&t, &account, 2).await;
    let r = h
        .jmap(
            &t,
            json!([
                ["Mailbox/get", {"accountId": account}, "m"],
                ["Email/query", {"accountId": account, "filter": {"inMailbox": inbox}}, "q"],
            ]),
        )
        .await;
    let archive = r[0][1]["list"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["role"] == "archive")
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let a = r[1][1]["ids"][0].as_str().unwrap().to_owned();
    let b = r[1][1]["ids"][1].as_str().unwrap().to_owned();
    let r = h
        .jmap(
            &t,
            json!([["Email/set", {"accountId": account, "update": {
                a.clone(): {format!("mailboxIds/{inbox}"): null, format!("mailboxIds/{archive}"): true},
                b.clone(): {"keywords/$seen": "yes"},
            }}, "s"]]),
        )
        .await;
    assert!(r[0][1]["updated"].get(&a).is_some(), "{r:?}");
    assert_eq!(r[0][1]["notUpdated"][&b]["type"], "invalidPatch");
    assert_ne!(
        r[0][1]["oldState"], r[0][1]["newState"],
        "the new state covers the change"
    );
    let mut moved = false;
    for _ in 0..100 {
        let s2 = server.clone();
        if tokio::task::spawn_blocking(move || s2.subjects("Archive".into()).len())
            .await
            .unwrap()
            == 1
        {
            moved = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(moved);
    stop(server).await;
}

/// Isolation within one user: one account's id never names another account's mail.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn one_accounts_id_never_reaches_another_accounts_mail() {
    let h = h();
    let (_, t) = h.user("ann");
    let s1 = start_server(1).await;
    let s2 = start_server(1).await;
    let a1 = h.account(&t, &s1).await;
    let a2 = h.second_account(&t, &s2, "ann2@example.test").await;
    let inbox2 = h.inbox(&t, &a2, 1).await;
    h.inbox(&t, &a1, 1).await;
    let r = h
        .jmap(
            &t,
            json!([["Email/query", {"accountId": a2, "filter": {"inMailbox": inbox2}}, "q"]]),
        )
        .await;
    let email2 = r[0][1]["ids"][0].clone();
    let r = h
        .jmap(
            &t,
            json!([["Email/get", {"accountId": a2, "ids": [email2.clone()], "properties": ["threadId"]}, "t"]]),
        )
        .await;
    let thread2 = r[0][1]["list"][0]["threadId"].clone();
    let r = h
        .jmap(
            &t,
            json!([
                ["Email/get", {"accountId": a1, "ids": [email2.clone()]}, "g"],
                ["Email/query", {"accountId": a1, "filter": {"inMailbox": inbox2}}, "q"],
                ["Thread/get", {"accountId": a1, "ids": [thread2.clone()]}, "t"],
                ["Email/set", {"accountId": a1, "update": {email2.as_str().unwrap(): {"keywords/$flagged": true}},
                               "destroy": [email2.clone()]}, "s"],
            ]),
        )
        .await;
    assert_eq!(r[0][1]["notFound"], json!([email2]));
    assert_eq!(r[1][0], "error");
    assert_eq!(r[2][1]["notFound"], json!([thread2]));
    assert_eq!(
        r[3][1]["notUpdated"][email2.as_str().unwrap()]["type"],
        "notFound"
    );
    assert_eq!(
        r[3][1]["notDestroyed"][email2.as_str().unwrap()]["type"],
        "notFound"
    );
    stop(s1).await;
    stop(s2).await;
}

#[tokio::test]
async fn a_recovery_sign_in_cannot_use_jmap() {
    let h = h();
    let (ann, _) = h.user("ann");
    let recovery = h
        .app
        .store
        .lock()
        .unwrap()
        .start_session(ann, "test", true, crate::store::now_ms())
        .unwrap()
        .access
        .to_string();
    let (s, _) = h.raw(Method::GET, "/jmap/session", &recovery, None).await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    let (s, _) = h
        .raw(
            Method::POST,
            "/jmap/api",
            &recovery,
            Some(
                json!({"using": USING, "methodCalls": []})
                    .to_string()
                    .into_bytes(),
            ),
        )
        .await;
    assert_eq!(s, StatusCode::FORBIDDEN);
}
