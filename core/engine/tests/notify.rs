// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! `EngineHandle::sync_once` against the in-process IMAP server: the first check is a baseline,
//! later checks return exactly the new unread Inbox messages, the marker moves only when the app
//! says it posted, and a hanging server stops at the deadline. No network.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::sync::Arc;
use std::time::{Duration, Instant};

use common::create_account;
use croft_engine::{Engine, EngineConfig, EngineHandle, ManualClock};
use croft_protocols::{Credential, ImapBackend, ImapConfig, Security};
use croft_storage::Db;
use croft_test_support::{sample_message, TestServer};

fn imap(server: &TestServer) -> ImapBackend {
    let config = ImapConfig::new(
        server.host(),
        server.port(),
        Security::PlaintextForTestsOnly,
        server.username(),
    )
    .with_timeout(Duration::from_secs(5));
    ImapBackend::new(config, Credential::password(server.password())).unwrap()
}

struct Rig {
    server: TestServer,
    _dir: tempfile::TempDir,
    _engine: Engine,
    ui: EngineHandle,
    account: i64,
}

async fn rig() -> Rig {
    let server = TestServer::start_default().await;
    server.add_message(
        "INBOX",
        &sample_message("old1@x", "Old one", "Ann <ann@example.org>"),
    );
    server.add_message(
        "INBOX",
        &sample_message("old2@x", "Old two", "Ann <ann@example.org>"),
    );
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("croft.db");
    let account = create_account(&Db::open(&path, None).unwrap(), "me@example.org");
    let engine = Engine::open(
        Db::open(&path, None).unwrap(),
        Arc::new(ManualClock::new(1_000_000)),
        EngineConfig::default(),
    )
    .unwrap();
    engine
        .attach_account(account, Arc::new(imap(&server)))
        .unwrap();
    let ui = engine.handle();
    Rig {
        server,
        _dir: dir,
        _engine: engine,
        ui,
        account,
    }
}

const DEADLINE: Duration = Duration::from_secs(8);

#[tokio::test]
async fn the_first_check_is_a_baseline_then_only_new_unread_mail_is_returned_once_posted() {
    let r = rig().await;
    let first = r.ui.sync_once(r.account, DEADLINE, false).await.unwrap();
    assert!(
        first.new_mail.is_empty(),
        "the mail already there never notifies"
    );
    assert!(!first.timed_out);

    let new_uid = r.server.add_message(
        "INBOX",
        &sample_message("new@x", "Lunch tomorrow", "Ben <ben@example.org>"),
    );
    let read_uid = r.server.add_message_with_flags(
        "INBOX",
        &sample_message("seen@x", "Read on the phone", "Cy <cy@example.org>"),
        &["\\Seen"],
    );
    let second = r.ui.sync_once(r.account, DEADLINE, false).await.unwrap();
    let uids: Vec<u32> = second.new_mail.iter().map(|m| m.uid).collect();
    assert_eq!(uids, vec![new_uid], "unread and new only (not {read_uid})");
    assert_eq!(second.new_mail[0].subject, "Lunch tomorrow");
    assert_eq!(
        second.new_mail[0].from_addr.as_deref(),
        Some("ben@example.org")
    );

    // Not posted yet: offered again. Posted: never again.
    let again = r.ui.sync_once(r.account, DEADLINE, false).await.unwrap();
    assert_eq!(again.new_mail.len(), 1);
    r.ui.mark_notified(r.account, &[new_uid]).unwrap();
    let after = r.ui.sync_once(r.account, DEADLINE, false).await.unwrap();
    assert!(after.new_mail.is_empty());
}

#[tokio::test]
async fn the_sample_mailbox_flag_never_notifies() {
    let r = rig().await;
    r.ui.sync_once(r.account, DEADLINE, true).await.unwrap();
    r.server.add_message(
        "INBOX",
        &sample_message("n@x", "New", "Ann <ann@example.org>"),
    );
    let out = r.ui.sync_once(r.account, DEADLINE, true).await.unwrap();
    assert!(out.new_mail.is_empty());
}

#[tokio::test]
async fn a_hanging_server_stops_at_the_deadline() {
    let r = rig().await;
    r.ui.sync_once(r.account, DEADLINE, false).await.unwrap();
    r.server.set_hang(true);
    let started = Instant::now();
    let out =
        r.ui.sync_once(r.account, Duration::from_millis(500), false)
            .await
            .unwrap();
    assert!(out.timed_out);
    assert!(
        started.elapsed() < Duration::from_secs(3),
        "{:?}",
        started.elapsed()
    );
    r.server.set_hang(false);
}

#[tokio::test]
async fn a_sync_cut_short_decides_nothing_so_older_new_mail_still_notifies_later() {
    let r = rig().await;
    r.ui.sync_once(r.account, DEADLINE, false).await.unwrap();
    let uid = r.server.add_message(
        "INBOX",
        &sample_message("late@x", "Arrived while offline", "Ann <ann@example.org>"),
    );
    r.server.set_hang(true);
    let cut =
        r.ui.sync_once(r.account, Duration::from_millis(300), false)
            .await
            .unwrap();
    assert!(cut.timed_out);
    assert!(cut.new_mail.is_empty(), "nothing decided on a partial sync");
    r.server.set_hang(false);
    // The connection the hang left behind may fail once (a transient error): the next periodic
    // run is simply another call.
    let mut next = None;
    for _ in 0..3 {
        if let Ok(out) = r.ui.sync_once(r.account, DEADLINE, false).await {
            next = Some(out);
            break;
        }
    }
    let next = next.expect("a later check works");
    assert_eq!(
        next.new_mail.iter().map(|m| m.uid).collect::<Vec<_>>(),
        vec![uid]
    );
}

#[tokio::test]
async fn a_notification_acts_only_on_the_message_it_was_posted_for() {
    let r = rig().await;
    r.ui.sync_once(r.account, DEADLINE, false).await.unwrap();
    let uid = r.server.add_message(
        "INBOX",
        &sample_message("n@x", "New", "Ann <ann@example.org>"),
    );
    let out = r.ui.sync_once(r.account, DEADLINE, false).await.unwrap();
    let m = &out.new_mail[0];
    assert!(r
        .ui
        .notified_message_is(r.account, m.message_id, uid)
        .unwrap());
    assert!(
        !r.ui
            .notified_message_is(r.account, m.message_id, uid + 1)
            .unwrap(),
        "another UID"
    );
    assert!(
        !r.ui
            .notified_message_is(r.account + 1, m.message_id, uid)
            .unwrap(),
        "another account"
    );
    assert!(
        !r.ui
            .notified_message_is(r.account, m.message_id + 1000, uid)
            .unwrap(),
        "another row"
    );
}
