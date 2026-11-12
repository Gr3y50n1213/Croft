// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Bring-your-own OAuth (T21) through the surface: the sign-in against the fake token endpoint
//! with a simulated browser, the account over XOAUTH2 on the in-process IMAP server, refresh
//! with rotation handed back to the app, `invalid_grant` as needs-sign-in, and cancel. Every
//! token is made up.

use super::*;
use crate::oauth::set_token_endpoint_for_tests;
use croft_test_support::oauth::FakeTokenEndpoint;
use std::io::{Read, Write};
use std::sync::Mutex;

/// The token endpoint override is process-wide: these tests run one at a time.
static SERIAL: Mutex<()> = Mutex::new(());

fn param(url: &str, name: &str) -> String {
    let query = url.split_once('?').unwrap().1;
    let raw = query
        .split('&')
        .find_map(|p| p.strip_prefix(&format!("{name}=")))
        .unwrap();
    raw.replace("%3A", ":").replace("%2F", "/")
}

/// What the browser does after the person agreed: it opens the redirect with the code.
fn browser_comes_back(url: &str, code: &str) -> std::thread::JoinHandle<String> {
    let redirect = param(url, "redirect_uri");
    let state = param(url, "state");
    let port: u16 = redirect.rsplit(':').next().unwrap().parse().unwrap();
    let target = format!("/?state={state}&code={code}");
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(100));
        let mut s = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
        write!(s, "GET {target} HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n").unwrap();
        let mut page = String::new();
        s.read_to_string(&mut page).unwrap();
        page
    })
}

struct Fixture {
    _serial: std::sync::MutexGuard<'static, ()>,
    fake: FakeTokenEndpoint,
    server: TestServer,
    db: TempDb,
    provider: Arc<MemProvider>,
    core: Arc<Core>,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        set_token_endpoint_for_tests(None);
    }
}

fn fixture() -> Fixture {
    let serial = SERIAL
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let fake = block(FakeTokenEndpoint::start());
    set_token_endpoint_for_tests(Some(fake.url()));
    let server = start_server();
    let db = TempDb::new();
    let provider = MemProvider::new();
    let core = db.open(&provider);
    Fixture {
        _serial: serial,
        fake,
        server,
        db,
        provider,
        core,
    }
}

/// The whole sign-in: begin, the browser's redirect, finish. Returns the sign-in secret.
fn sign_in(f: &Fixture) -> Vec<u8> {
    let start = block(f.core.oauth_begin(
        "google".into(),
        "made-up-client.apps.example".into(),
        None,
        f.server.username(),
        "You can go back to Croft & carry on.".into(),
    ))
    .unwrap();
    assert!(start.url.starts_with("https://accounts.google.com/"));
    assert!(start.url.contains("redirect_uri=http%3A%2F%2F127.0.0.1%3A"));
    f.fake
        .add_code("code-1", &param(&start.url, "code_challenge"), "refresh-0");
    let browser = browser_comes_back(&start.url, "code-1");
    let done = block(f.core.oauth_finish(start.flow_id)).unwrap();
    let page = browser.join().unwrap();
    assert!(
        page.contains("You can go back to Croft &amp; carry on."),
        "{page}"
    );
    done.secret
}

fn add(f: &Fixture, secret: &[u8]) -> i64 {
    let fp = fingerprint_of(&f.server);
    let cred = FfiCredential::oauth_stored(secret.to_vec());
    let pin = block(
        f.core
            .verify(discovered_for(&f.server), cred.clone(), Some(fp)),
    )
    .unwrap();
    let id = block(
        f.core
            .add_account(discovered_for(&f.server), "Gmail".into(), pin, None, cred),
    )
    .unwrap()
    .id;
    f.provider
        .store(id, FfiCredential::oauth_stored(secret.to_vec()));
    id
}

fn wait_until(what: &str, mut cond: impl FnMut() -> bool) {
    for _ in 0..200 {
        if cond() {
            return;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    panic!("timed out waiting for {what}");
}

#[test]
fn signing_in_adds_an_account_that_syncs_over_xoauth2_and_no_token_reaches_the_database() {
    let f = fixture();
    f.server
        .add_message("INBOX", &raw("a@example.org", "Hello"));
    f.server.set_password("access-1"); // the access token the fake endpoint gives first
    let secret = sign_in(&f);
    let id = add(&f, &secret);
    let inbox = || {
        f.core
            .list_folders(id)
            .unwrap()
            .into_iter()
            .find(|x| x.path == "INBOX")
            .map(|x| x.id)
    };
    wait_until("the inbox", || inbox().is_some());
    let scope = MessageScope::Folder {
        account: id,
        folder_id: inbox().unwrap(),
    };
    wait_until("a message", || {
        !f.core
            .list_messages(scope, None, 10)
            .unwrap()
            .rows
            .is_empty()
    });
    // The verifier went to the token endpoint, no secret did.
    let sent = f.fake.requests().join("\n");
    assert!(sent.contains("code_verifier=") && !sent.contains("client_secret"));
    // The client id is remembered for next time.
    assert_eq!(
        f.core.oauth_client_id("google".into()).as_deref(),
        Some("made-up-client.apps.example")
    );
    // Nothing secret in the database file (this test database is not encrypted, so a token
    // would be readable in it).
    drop(f.core.clone());
    let bytes = std::fs::read(&f.db.path).unwrap();
    let text = String::from_utf8_lossy(&bytes);
    for secret in ["refresh-0", "access-1", "code-1"] {
        assert!(!text.contains(secret), "{secret} found in the database");
    }
}

/// Adds the account without `verify` (the server may not accept the sign-in's first token).
fn add_unverified(f: &Fixture, secret: &[u8]) -> i64 {
    let cred = FfiCredential::oauth_stored(secret.to_vec());
    let id = block(f.core.add_account(
        discovered_for(&f.server),
        "Gmail".into(),
        Some(fingerprint_of(&f.server)),
        None,
        cred,
    ))
    .unwrap()
    .id;
    f.provider
        .store(id, FfiCredential::oauth_stored(secret.to_vec()));
    id
}

#[test]
fn a_refused_token_is_refreshed_once_and_a_rotated_refresh_token_goes_back_to_the_app() {
    let f = fixture();
    let secret = sign_in(&f); // gives access-1
                              // The server no longer takes access-1 (it expired); the next refresh gives access-2 and,
                              // with rotation, a new refresh token.
    f.server.set_password("access-2");
    f.fake.set_rotation(true);
    f.server
        .add_message("INBOX", &raw("b@example.org", "After refresh"));
    let id = add_unverified(&f, &secret);
    wait_until("the sync with the new token", || {
        !f.core.list_folders(id).unwrap().is_empty()
    });
    wait_until("the rotated secret to be saved", || {
        f.provider.saved().contains(&id)
    });
    assert!(
        f.provider.sign_ins().is_empty(),
        "a refresh is not a sign-in problem"
    );
    assert_eq!(f.fake.last_access_token().as_deref(), Some("access-2"));
}

#[test]
fn a_token_about_to_expire_is_refreshed_before_it_is_used() {
    let f = fixture();
    let secret = sign_in(&f);
    // The stored access token expires in a moment: the core refreshes before connecting.
    let mut json: serde_json::Value = serde_json::from_slice(&secret).unwrap();
    json["expires_at_ms"] = serde_json::json!(1);
    let secret = serde_json::to_vec(&json).unwrap();
    f.server.set_password("access-2");
    let id = add_unverified(&f, &secret);
    wait_until("the sync", || !f.core.list_folders(id).unwrap().is_empty());
    assert!(f.provider.sign_ins().is_empty());
}

#[test]
fn a_revoked_sign_in_makes_the_account_need_a_sign_in() {
    let f = fixture();
    let secret = sign_in(&f);
    f.fake.revoke("refresh-0");
    f.server.set_password("something-else");
    let id = add_unverified(&f, &secret);
    wait_until("needs sign-in", || f.provider.sign_ins().contains(&id));
}

#[test]
fn cancelling_ends_the_wait_and_a_finished_flow_cannot_be_finished_again() {
    let f = fixture();
    let start = block(f.core.oauth_begin(
        "microsoft".into(),
        "00000000-made-up".into(),
        None,
        "ann@example.org".into(),
        "Done".into(),
    ))
    .unwrap();
    assert!(start
        .url
        .starts_with("https://login.microsoftonline.com/common/oauth2/v2.0/authorize?"));
    let core = f.core.clone();
    let id = start.flow_id.clone();
    let canceller = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(200));
        core.oauth_cancel(id);
    });
    assert_eq!(
        block(f.core.oauth_finish(start.flow_id.clone())).err(),
        Some(OAuthFailure::Cancelled)
    );
    canceller.join().unwrap();
    assert_eq!(
        block(f.core.oauth_finish(start.flow_id)).err(),
        Some(OAuthFailure::Expired)
    );
}

#[test]
fn a_bad_client_id_or_provider_is_refused_before_anything_opens() {
    let f = fixture();
    for (provider, id) in [("google", ""), ("google", "has spaces"), ("yahoo", "x")] {
        assert_eq!(
            block(
                f.core
                    .oauth_begin(provider.into(), id.into(), None, "a@b".into(), "x".into())
            )
            .err(),
            Some(OAuthFailure::BadClient),
            "{provider} {id}"
        );
    }
}

#[test]
fn the_person_saying_no_is_access_denied() {
    let f = fixture();
    let start = block(f.core.oauth_begin(
        "google".into(),
        "made-up-client".into(),
        None,
        "ann@example.org".into(),
        "x".into(),
    ))
    .unwrap();
    let redirect = param(&start.url, "redirect_uri");
    let state = param(&start.url, "state");
    let port: u16 = redirect.rsplit(':').next().unwrap().parse().unwrap();
    let browser = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(100));
        let mut s = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
        write!(
            s,
            "GET /?error=access_denied&state={state} HTTP/1.1\r\nHost: x\r\n\r\n"
        )
        .unwrap();
        let mut page = String::new();
        let _ = s.read_to_string(&mut page);
    });
    assert_eq!(
        block(f.core.oauth_finish(start.flow_id)).err(),
        Some(OAuthFailure::AccessDenied)
    );
    browser.join().unwrap();
}
