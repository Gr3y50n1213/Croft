// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The JMAP adapter against a real Stalwart server in a container.
//!
//! These only compile with `--features integration`, and they need Stalwart running with its two
//! test users (see `test-servers/docker-compose.yml` and `test-servers/stalwart/setup.sh`):
//!
//! ```text
//! docker compose -f test-servers/docker-compose.yml up -d --wait stalwart
//! test-servers/stalwart/setup.sh
//! cd core && cargo test -p croft-protocols --features integration --test jmap_integration
//! docker compose -f test-servers/docker-compose.yml down -v
//! ```
//!
//! Host ports come from `CROFT_TEST_STALWART_HTTP_PORT` (default 8080, plain HTTP, allowed because
//! it is localhost) and `CROFT_TEST_STALWART_HTTPS_PORT` (default 8443, self-signed certificate:
//! the test reads its fingerprint with `probe_jmap_certificate` and pins it). Both tests and
//! compose file read the same defaults; with a second copy of the servers, give both the same
//! remapped ports. The tests share two mailboxes, so they run one at a time and empty the mailbox
//! they use first.
#![cfg(feature = "integration")]
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use common::behaviour::{behaviour, seed_message, Seeded, DATE_HEADER_MS};
use common::rawjmap::RawJmap;
use croft_protocols::*;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::sync::Mutex;

const ALICE: &str = "alice@example.org";
const ALICE_PW: &str = "croft-alice-purple-orbit-7421";
const BOB: &str = "bob@example.org";
const BOB_PW: &str = "croft-bob-yellow-lantern-5530";

/// The tests share the server's two mailboxes, so they take turns.
static TURN: Mutex<()> = Mutex::const_new(());

fn port(variable: &str, default: u16) -> u16 {
    std::env::var(variable)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

fn http_port() -> u16 {
    port("CROFT_TEST_STALWART_HTTP_PORT", 8080)
}

fn https_port() -> u16 {
    port("CROFT_TEST_STALWART_HTTPS_PORT", 8443)
}

fn unique(prefix: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    format!("{prefix}{nanos}")
}

fn config(user: &str) -> JmapConfig {
    JmapConfig::new(
        format!("http://127.0.0.1:{}/jmap/session", http_port()),
        user,
    )
    .with_timeout(Duration::from_secs(20))
}

fn backend(user: &str, password: &str, store: Arc<MemoryIdStore>) -> JmapBackend {
    JmapBackend::new(config(user), Credential::password(password), store).unwrap()
}

/// Alice's account emptied and seeded with the three messages the behaviour test expects.
async fn seeded_alice() -> (RawJmap, Seeded) {
    let alice = RawJmap::connect(http_port(), ALICE, ALICE_PW).await;
    alice.empty().await;
    let ids = [unique("one-"), unique("two-"), unique("three-")];
    for (n, keywords) in [(0, &[][..]), (1, &[][..]), (2, &["$seen"][..])] {
        alice
            .import("inbox", &seed_message(&ids, n, ALICE), keywords)
            .await;
    }
    (alice, Seeded { ids })
}

#[tokio::test]
async fn the_shared_behaviour_holds_against_stalwart() {
    let _turn = TURN.lock().await;
    let (_alice, seeded) = seeded_alice().await;
    let b = backend(ALICE, ALICE_PW, Arc::new(MemoryIdStore::new()));
    behaviour(&b, "jmap/stalwart", &seeded, Some(DATE_HEADER_MS)).await;
}

#[tokio::test]
async fn stalwart_works_over_https_with_a_pinned_certificate_and_only_then() {
    let _turn = TURN.lock().await;
    let (_alice, _) = seeded_alice().await;
    let url = format!("https://127.0.0.1:{}/jmap/session", https_port());
    let cfg = JmapConfig::new(url, ALICE).with_timeout(Duration::from_secs(20));
    let fp = probe_jmap_certificate(&cfg).await.unwrap();
    // Reading the fingerprint twice gives the same certificate.
    assert_eq!(probe_jmap_certificate(&cfg).await.unwrap(), fp);

    // Pinned: works, although the certificate is self-signed.
    let b = JmapBackend::new(
        cfg.clone().with_pinned_cert(fp),
        Credential::password(ALICE_PW),
        Arc::new(MemoryIdStore::new()),
    )
    .unwrap();
    let folders = b.list_folders().await.unwrap();
    assert!(folders.iter().any(|f| f.special_use == SpecialUse::Inbox));
    assert_eq!(b.snapshot("Inbox").await.unwrap().messages.len(), 3);

    // Not pinned: a self-signed certificate is refused (validation is never switched off), and so is
    // a different pin. Both pause the account instead of failing its queue.
    let unpinned = JmapBackend::new(
        cfg.clone(),
        Credential::password(ALICE_PW),
        Arc::new(MemoryIdStore::new()),
    )
    .unwrap();
    assert!(matches!(
        unpinned.list_folders().await,
        Err(BackendError::Auth(_))
    ));
    let other = CertFingerprint::from_bytes([7; 32]);
    let wrong = JmapBackend::new(
        cfg.with_pinned_cert(other),
        Credential::password(ALICE_PW),
        Arc::new(MemoryIdStore::new()),
    )
    .unwrap();
    assert!(matches!(
        wrong.list_folders().await,
        Err(BackendError::Auth(_))
    ));
}

#[tokio::test]
async fn a_second_client_changes_flags_and_the_snapshot_sees_it() {
    let _turn = TURN.lock().await;
    let (alice, seeded) = seeded_alice().await;
    let store = Arc::new(MemoryIdStore::new());
    let b = backend(ALICE, ALICE_PW, store.clone());
    let first = b.snapshot("Inbox").await.unwrap();
    assert_eq!(first.messages.len(), 3);
    let headers = b
        .fetch_headers("Inbox", &first.messages.keys().copied().collect::<Vec<_>>())
        .await
        .unwrap();
    let uid_of = |n: usize| {
        headers
            .iter()
            .find(|h| h.message_id.as_deref() == Some(&format!("<{}>", seeded.ids[n])))
            .unwrap()
            .uid
    };
    let status_before = b.folder_status("Inbox").await.unwrap();

    // Another client (talking to the server directly) marks the first message read, adds a new
    // message and destroys the third.
    let ids = alice.all_email_ids().await;
    let mut first_email = None;
    for id in &ids {
        let r = alice.call(serde_json::json!([["Email/get", {"accountId": alice.account_id, "ids": [id], "properties": ["messageId"]}, "a"]])).await;
        if r[0][1]["list"][0]["messageId"][0] == seeded.ids[0].as_str() {
            first_email = Some(id.clone());
        }
        if r[0][1]["list"][0]["messageId"][0] == seeded.ids[2].as_str() {
            alice.call(serde_json::json!([["Email/set", {"accountId": alice.account_id, "destroy": [id]}, "a"]])).await;
        }
    }
    alice
        .set_keyword(&first_email.unwrap(), "$seen", true)
        .await;
    let fresh = unique("fresh-");
    let ids4 = [fresh.clone(), fresh.clone(), fresh.clone()];
    alice
        .import("inbox", &seed_message(&ids4, 2, ALICE), &[])
        .await;

    // The status number moved, so the engine knows to look.
    let status_after = b.folder_status("Inbox").await.unwrap();
    assert_ne!(status_before.highest_modseq, status_after.highest_modseq);
    assert_eq!(status_before.uid_validity, status_after.uid_validity);

    let second = b.snapshot("Inbox").await.unwrap();
    assert_eq!(second.messages.len(), 3, "{second:?}");
    assert!(
        second.messages[&uid_of(0)].seen,
        "the flag changed elsewhere is seen"
    );
    assert!(
        !second.messages.contains_key(&uid_of(2)),
        "the destroyed message is gone"
    );
    assert!(second.messages.contains_key(&uid_of(1)));
    let new_uid = second
        .messages
        .keys()
        .copied()
        .find(|u| *u > first.status.uid_next - 1)
        .unwrap();
    assert!(
        new_uid >= first.status.uid_next,
        "a new message gets a never-used UID"
    );
    assert_eq!(second.status.uid_validity, first.status.uid_validity);

    // A new adapter on the same map (a restart) keeps every UID.
    let restarted = backend(ALICE, ALICE_PW, store);
    let third = restarted.snapshot("Inbox").await.unwrap();
    assert_eq!(
        third.messages.keys().collect::<Vec<_>>(),
        second.messages.keys().collect::<Vec<_>>()
    );
    assert_eq!(third.status.uid_validity, first.status.uid_validity);
}

#[tokio::test]
async fn a_wrong_password_is_auth_and_never_transient() {
    let _turn = TURN.lock().await;
    for (user, password) in [
        (ALICE, "not-the-password"),
        ("nobody@example.org", "whatever-it-is-9"),
        (ALICE, ""),
    ] {
        let b = backend(user, password, Arc::new(MemoryIdStore::new()));
        for _ in 0..2 {
            let results = [
                b.capabilities().await.err(),
                b.list_folders().await.err(),
                b.snapshot("Inbox").await.err(),
                b.set_flag("Inbox", &[1], FlagChange::Seen(true))
                    .await
                    .err(),
            ];
            for e in results {
                let e = e.expect("must fail");
                assert!(matches!(e, BackendError::Auth(_)), "{user}: {e:?}");
                assert!(!e.is_retryable());
            }
        }
    }
    // The same for sending.
    let s = JmapSubmitter::new(config(ALICE), Credential::password("not-the-password")).unwrap();
    let e = s
        .send(ALICE, &[BOB], b"Subject: x\r\n\r\nx")
        .await
        .unwrap_err();
    assert!(matches!(e, BackendError::Auth(_)), "{e:?}");
}

#[tokio::test]
async fn a_server_that_is_not_there_is_transient() {
    // With `CROFT_TEST_STALWART_STOPPED=1` this is pointed at the real server's port, for running
    // the test while the container is stopped; otherwise at a port nothing listens on.
    let port = if std::env::var("CROFT_TEST_STALWART_STOPPED").is_ok() {
        http_port()
    } else {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap().port()
    };
    let cfg = JmapConfig::new(format!("http://127.0.0.1:{port}/jmap/session"), ALICE)
        .with_timeout(Duration::from_secs(5));
    let b = JmapBackend::new(
        cfg.clone(),
        Credential::password(ALICE_PW),
        Arc::new(MemoryIdStore::new()),
    )
    .unwrap();
    for e in [
        b.capabilities().await.err(),
        b.list_folders().await.err(),
        b.snapshot("Inbox").await.err(),
    ] {
        let e = e.expect("must fail");
        assert!(matches!(e, BackendError::Transient(_)), "{e:?}");
    }
    let s = JmapSubmitter::new(cfg, Credential::password(ALICE_PW)).unwrap();
    let e = s
        .send(ALICE, &[BOB], b"Subject: x\r\n\r\nx")
        .await
        .unwrap_err();
    assert!(e.is_retryable(), "{e:?}");
}

#[tokio::test]
async fn a_message_sent_with_email_submission_reaches_the_second_user() {
    let _turn = TURN.lock().await;
    let alice = RawJmap::connect(http_port(), ALICE, ALICE_PW).await;
    let bob = RawJmap::connect(http_port(), BOB, BOB_PW).await;
    alice.empty().await;
    bob.empty().await;

    let marker = unique("hello-bob-");
    let mime = format!(
        "From: Alice <{ALICE}>\nTo: Bob <{BOB}>\nSubject: {marker}\nDate: Mon, 02 Mar 2026 09:30:00 +0000\n\
         Message-ID: <{marker}@example.org>\n\nHello Bob, this came over JMAP.\n"
    );
    let sender = JmapSubmitter::new(config(ALICE), Credential::password(ALICE_PW)).unwrap();
    let receipt = sender.send(ALICE, &[BOB], mime.as_bytes()).await.unwrap();
    assert_eq!(receipt.recipients, 1);

    // Bob's inbox gets it (delivery is asynchronous on the server).
    let bobs = backend(BOB, BOB_PW, Arc::new(MemoryIdStore::new()));
    let deadline = Instant::now() + Duration::from_secs(30);
    let found = loop {
        let snap = bobs.snapshot("Inbox").await.unwrap();
        let uids: Vec<Uid> = snap.messages.keys().copied().collect();
        let headers = bobs.fetch_headers("Inbox", &uids).await.unwrap();
        if let Some(h) = headers.into_iter().find(|h| h.subject == marker) {
            break h;
        }
        assert!(Instant::now() < deadline, "the message never arrived");
        tokio::time::sleep(Duration::from_millis(500)).await;
    };
    assert_eq!(found.from_addr.as_deref(), Some(ALICE));
    assert_eq!(
        found.message_id.as_deref(),
        Some(format!("<{marker}@example.org>").as_str())
    );

    // The server filed Sent for Alice: one copy there, none left in Drafts.
    let alices = backend(ALICE, ALICE_PW, Arc::new(MemoryIdStore::new()));
    let sent_snap = alices.snapshot("Sent Items").await.unwrap();
    assert_eq!(sent_snap.messages.len(), 1);
    let sent_uids: Vec<Uid> = sent_snap.messages.keys().copied().collect();
    let sent = alices
        .fetch_headers("Sent Items", &sent_uids)
        .await
        .unwrap();
    assert_eq!(sent[0].subject, marker);
    assert!(sent[0].flags.seen && !sent[0].flags.draft);
    assert!(alices.snapshot("Drafts").await.unwrap().messages.is_empty());

    // A recipient the server refuses fails without leaving a draft behind.
    let e = sender
        .send(ALICE, &["x y@example.org"], mime.as_bytes())
        .await
        .unwrap_err();
    assert!(matches!(e, BackendError::Permanent(_)));
    assert!(alices.snapshot("Drafts").await.unwrap().messages.is_empty());
}

/// T19: server search on Stalwart (JMAP `Email/query`) finds the expected messages by sender,
/// subject and body text. Stalwart indexes in the background, so the search is retried for a while.
#[tokio::test]
async fn stalwart_server_search_by_from_subject_and_body() {
    let _turn = TURN.lock().await;
    let alice = RawJmap::connect(http_port(), ALICE, ALICE_PW).await;
    alice.empty().await;
    let word = unique("zq");
    let raw = |from: &str, subject: &str, body: &str| {
        format!(
            "From: {from}\r\nTo: {ALICE}\r\nSubject: {subject}\r\nMessage-ID: <{}@x>\r\n\
             Date: Mon, 02 Mar 2026 09:30:00 +0000\r\nContent-Type: text/plain\r\n\r\n{body}\r\n",
            unique("s")
        )
    };
    alice
        .import(
            "inbox",
            &raw("Ann <ann@example.org>", &format!("Plan {word}"), "x"),
            &[],
        )
        .await;
    alice
        .import(
            "inbox",
            &raw(
                "Ben <ben@example.org>",
                "Other",
                &format!("about {word} here"),
            ),
            &[],
        )
        .await;
    let b = backend(ALICE, ALICE_PW, Arc::new(MemoryIdStore::new()));
    let inbox = b
        .list_folders()
        .await
        .unwrap()
        .into_iter()
        .find(|f| f.special_use == SpecialUse::Inbox)
        .unwrap()
        .path;
    let count = |o: SearchOutcome| match o {
        SearchOutcome::Found(u) => u.len(),
        SearchOutcome::Unsupported => panic!("Stalwart can search"),
    };
    let text = ServerQuery {
        text: vec![word.clone()],
        ..ServerQuery::default()
    };
    let subject = ServerQuery {
        subject: vec![word.clone()],
        ..ServerQuery::default()
    };
    let from = ServerQuery {
        from: vec!["ben@example.org".into()],
        ..ServerQuery::default()
    };
    let end = Instant::now() + Duration::from_secs(15);
    loop {
        let n = count(b.search(&inbox, &text).await.unwrap());
        if n == 2 || Instant::now() > end {
            assert_eq!(n, 2, "subject and body");
            break;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    assert_eq!(count(b.search(&inbox, &subject).await.unwrap()), 1);
    assert_eq!(count(b.search(&inbox, &from).await.unwrap()), 1);
}
