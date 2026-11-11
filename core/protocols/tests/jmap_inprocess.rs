// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The JMAP adapter against the in-process JMAP server from `croft-test-support`, and the shared
//! behaviour test against both adapters. No network, no containers: these run on every
//! `cargo test`.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use common::behaviour::{behaviour, seed_message, Seeded, DATE_HEADER_MS};
use croft_protocols::discovery::{AuthHint, DiscoveredSettings, Source};
use croft_protocols::*;
use croft_test_support::{JmapOptions, JmapServer, TestServer, JMAP_PASSWORD, JMAP_USER};
use std::sync::Arc;
use std::time::Duration;

fn config(server: &JmapServer) -> JmapConfig {
    JmapConfig::new(server.session_url(), JMAP_USER).with_timeout(Duration::from_secs(5))
}

fn backend_with(server: &JmapServer, store: Arc<MemoryIdStore>) -> JmapBackend {
    JmapBackend::new(config(server), Credential::password(JMAP_PASSWORD), store).unwrap()
}

fn backend(server: &JmapServer) -> JmapBackend {
    backend_with(server, Arc::new(MemoryIdStore::new()))
}

fn ids(prefix: &str) -> [String; 3] {
    [
        format!("{prefix}-one@example.org"),
        format!("{prefix}-two@example.org"),
        format!("{prefix}-three@example.org"),
    ]
}

fn raw(id: &str, subject: &str) -> String {
    format!(
        "From: Ann Example <ann@example.org>\r\nTo: Test <{JMAP_USER}>\r\nSubject: {subject}\r\n\
         Date: Mon, 02 Mar 2026 09:30:00 +0000\r\nMessage-ID: <{id}>\r\n\r\nBody of {id}.\r\n"
    )
}

/// The three messages the shared behaviour test expects, in the Inbox.
fn seed_jmap(server: &JmapServer) -> Seeded {
    let ids = ids("shared");
    for (n, keywords) in [(0, &[][..]), (1, &[][..]), (2, &["$seen"][..])] {
        server.add_message("inbox", &seed_message(&ids, n, JMAP_USER), keywords);
    }
    Seeded { ids }
}

// ------------------------------------------------------------------------------------------
// The shared behaviour, over both adapters
// ------------------------------------------------------------------------------------------

#[tokio::test]
async fn the_jmap_adapter_passes_the_shared_behaviour() {
    let server = JmapServer::start().await;
    let seeded = seed_jmap(&server);
    behaviour(
        &backend(&server),
        "jmap/in-process",
        &seeded,
        Some(DATE_HEADER_MS),
    )
    .await;
}

#[tokio::test]
async fn the_imap_adapter_passes_the_shared_behaviour() {
    let server = TestServer::start_default().await;
    let seeded = Seeded { ids: ids("shared") };
    for (n, flags) in [(0, &[][..]), (1, &[][..]), (2, &["\\Seen"][..])] {
        server.add_message_with_flags(
            "INBOX",
            &seed_message(&seeded.ids, n, "test@localhost"),
            flags,
        );
    }
    let b = ImapBackend::new(
        ImapConfig::new(
            server.host(),
            server.port(),
            Security::PlaintextForTestsOnly,
            server.username(),
        )
        .with_timeout(Duration::from_secs(5)),
        Credential::password(server.password()),
    )
    .unwrap();
    behaviour(&b, "imap/in-process", &seeded, None).await;
}

// ------------------------------------------------------------------------------------------
// Folders, status and snapshots
// ------------------------------------------------------------------------------------------

#[tokio::test]
async fn lists_mailboxes_with_roles_paths_and_rights() {
    let server = JmapServer::start().await;
    server.add_mailbox("Projects", None);
    server.add_mailbox("2026", Some("Projects"));
    let folders = backend(&server).list_folders().await.unwrap();
    let kind = |p: &str| folders.iter().find(|f| f.path == p).unwrap().special_use;
    assert_eq!(kind("Inbox"), SpecialUse::Inbox);
    assert_eq!(kind("Sent Items"), SpecialUse::Sent);
    assert_eq!(kind("Drafts"), SpecialUse::Drafts);
    assert_eq!(kind("Deleted Items"), SpecialUse::Trash);
    assert_eq!(kind("Junk Mail"), SpecialUse::Junk);
    assert_eq!(kind("Archive"), SpecialUse::Archive);
    assert_eq!(kind("Projects"), SpecialUse::Other);
    assert_eq!(kind("Projects/2026"), SpecialUse::Other);
    assert!(folders
        .iter()
        .all(|f| f.delimiter == Some('/') && f.selectable));
    let caps = backend(&server).capabilities().await.unwrap();
    assert!(caps.move_cmd && caps.uidplus && caps.special_use && !caps.idle && !caps.qresync);
}

#[tokio::test]
async fn status_number_follows_the_server_state_and_is_stable_across_processes() {
    let server = JmapServer::start().await;
    server.add_message("inbox", &raw("a@x", "A"), &[]);
    let (one, two) = (backend(&server), backend(&server));
    let s1 = one.folder_status("Inbox").await.unwrap();
    let s2 = two.folder_status("Inbox").await.unwrap();
    // Same server state, same number, whichever adapter instance asks.
    assert_eq!(s1.highest_modseq, s2.highest_modseq);
    assert!(s1.highest_modseq.is_some());
    assert_eq!(s1.exists, 1);
    server.add_message("inbox", &raw("b@x", "B"), &[]);
    let s3 = one.folder_status("Inbox").await.unwrap();
    assert_ne!(s1.highest_modseq, s3.highest_modseq);
    assert_eq!(s3.exists, 2);
    assert_eq!(s1.uid_validity, s3.uid_validity);
}

#[tokio::test]
async fn uids_follow_arrival_order_and_are_never_reused() {
    let server = JmapServer::start().await;
    let a = server.add_message("inbox", &raw("a@x", "A"), &[]);
    server.add_message("inbox", &raw("b@x", "B"), &[]);
    let b = backend(&server);
    let first = b.snapshot("Inbox").await.unwrap();
    assert_eq!(
        first.messages.keys().copied().collect::<Vec<_>>(),
        vec![1, 2]
    );
    assert_eq!(first.status.uid_next, 3);
    // The newest message goes away and a new one arrives: it does not get the freed UID.
    let newest = server.ids_in("inbox").pop().unwrap();
    server.destroy_message(&newest);
    server.add_message("inbox", &raw("c@x", "C"), &[]);
    let second = b.snapshot("Inbox").await.unwrap();
    assert_eq!(
        second.messages.keys().copied().collect::<Vec<_>>(),
        vec![1, 3]
    );
    // The first message kept its UID throughout.
    let h = b.fetch_headers("Inbox", &[1]).await.unwrap();
    assert_eq!(h[0].message_id.as_deref(), Some("<a@x>"));
    let _ = a;
}

#[tokio::test]
async fn a_lost_id_map_gives_the_folder_a_new_validity() {
    let server = JmapServer::start().await;
    server.add_message("inbox", &raw("a@x", "A"), &[]);
    let store = Arc::new(MemoryIdStore::new());
    let b = backend_with(&server, store.clone());
    let before = b.snapshot("Inbox").await.unwrap();
    store.forget_all();
    let after = b.snapshot("Inbox").await.unwrap();
    assert_ne!(before.status.uid_validity, after.status.uid_validity);
    assert_eq!(after.messages.len(), 1);
}

#[tokio::test]
async fn an_inconsistent_id_map_is_reset_and_the_call_fails_as_retryable() {
    let server = JmapServer::start().await;
    server.add_message("inbox", &raw("a@x", "A"), &[]);
    let store = Arc::new(MemoryIdStore::new());
    let b = backend_with(&server, store.clone());
    let before = b.snapshot("Inbox").await.unwrap();
    store.fail_next(Some(IdStoreError::Corrupt));
    let err = b.snapshot("Inbox").await.unwrap_err();
    assert!(err.is_retryable(), "{err:?}");
    store.fail_with(None);
    let after = b.snapshot("Inbox").await.unwrap();
    assert_ne!(before.status.uid_validity, after.status.uid_validity);
    // A map that is merely unavailable is retryable and does not reset anything.
    let validity = after.status.uid_validity;
    store.fail_with(Some(IdStoreError::Unavailable));
    assert!(b.snapshot("Inbox").await.unwrap_err().is_retryable());
    store.fail_with(None);
    assert_eq!(
        b.snapshot("Inbox").await.unwrap().status.uid_validity,
        validity
    );
}

#[tokio::test]
async fn a_restart_with_the_same_map_keeps_every_uid() {
    let server = JmapServer::start().await;
    for n in 0..5 {
        server.add_message("inbox", &raw(&format!("m{n}@x"), "M"), &[]);
    }
    let store = Arc::new(MemoryIdStore::new());
    let first = backend_with(&server, store.clone())
        .snapshot("Inbox")
        .await
        .unwrap();
    server.add_message("inbox", &raw("late@x", "L"), &[]);
    let second = backend_with(&server, store)
        .snapshot("Inbox")
        .await
        .unwrap();
    assert_eq!(second.status.uid_validity, first.status.uid_validity);
    assert_eq!(
        first.messages.keys().copied().collect::<Vec<_>>(),
        (1..=5).collect::<Vec<u32>>()
    );
    assert_eq!(
        second.messages.keys().copied().collect::<Vec<_>>(),
        (1..=6).collect::<Vec<u32>>()
    );
}

// ------------------------------------------------------------------------------------------
// Email/changes
// ------------------------------------------------------------------------------------------

fn count(server: &JmapServer, method: &str) -> usize {
    server.calls().iter().filter(|c| *c == method).count()
}

#[tokio::test]
async fn the_second_snapshot_asks_for_changes_and_sees_what_another_client_did() {
    let server = JmapServer::start().await;
    let a = server.add_message("inbox", &raw("a@x", "A"), &[]);
    let bb = server.add_message("inbox", &raw("b@x", "B"), &["$seen"]);
    let c = server.add_message("inbox", &raw("c@x", "C"), &[]);
    let b = backend(&server);
    let first = b.snapshot("Inbox").await.unwrap();
    assert_eq!(count(&server, "Email/changes"), 0);
    server.clear_logs();

    server.set_keyword(&a, "$seen", true);
    server.set_keyword(&bb, "$flagged", true);
    server.destroy_message(&c);
    let d = server.add_message("inbox", &raw("d@x", "D"), &["$answered"]);
    let moved_out = server.add_message("inbox", &raw("e@x", "E"), &[]);
    server.move_message(&moved_out, "inbox", "archive");

    let second = b.snapshot("Inbox").await.unwrap();
    assert_eq!(count(&server, "Email/changes"), 1, "{:?}", server.calls());
    assert_eq!(
        count(&server, "Email/query"),
        0,
        "no full read: {:?}",
        server.calls()
    );
    assert_eq!(second.messages.len(), 3, "{second:?}");
    assert!(second.messages[&1].seen);
    assert!(second.messages[&2].seen && second.messages[&2].flagged);
    assert!(!second.messages.contains_key(&3));
    let new_uid = *second.messages.keys().max().unwrap();
    assert!(new_uid >= first.status.uid_next);
    assert!(second.messages[&new_uid].answered);
    assert_ne!(first.status.highest_modseq, second.status.highest_modseq);
    let _ = d;

    // Nothing changed: the same answer, still without a full read.
    let third = b.snapshot("Inbox").await.unwrap();
    assert_eq!(third.messages, second.messages);
    assert_eq!(third.status, second.status);
    assert_eq!(count(&server, "Email/query"), 0);
}

#[tokio::test]
async fn cannot_calculate_changes_falls_back_to_a_full_read_with_the_same_uids() {
    let server = JmapServer::start().await;
    let a = server.add_message("inbox", &raw("a@x", "A"), &[]);
    server.add_message("inbox", &raw("b@x", "B"), &[]);
    let b = backend(&server);
    let first = b.snapshot("Inbox").await.unwrap();
    server.set_keyword(&a, "$seen", true);
    server.add_message("inbox", &raw("c@x", "C"), &[]);
    server.set_cannot_calculate_changes(true);
    server.clear_logs();
    let second = b.snapshot("Inbox").await.unwrap();
    assert_eq!(count(&server, "Email/changes"), 1);
    assert_eq!(
        count(&server, "Email/query"),
        1,
        "the full read: {:?}",
        server.calls()
    );
    assert_eq!(second.messages.len(), 3);
    assert!(second.messages[&1].seen);
    assert_eq!(second.status.uid_validity, first.status.uid_validity);
    // The old messages kept their UIDs.
    assert_eq!(
        second.messages.keys().copied().collect::<Vec<_>>(),
        vec![1, 2, 3]
    );
}

#[tokio::test]
async fn a_state_the_server_forgot_also_falls_back() {
    let server = JmapServer::start().await;
    server.add_message("inbox", &raw("a@x", "A"), &[]);
    let b = backend(&server);
    b.snapshot("Inbox").await.unwrap();
    server.add_message("inbox", &raw("b@x", "B"), &[]);
    server.forget_changes();
    let second = b.snapshot("Inbox").await.unwrap();
    assert_eq!(second.messages.len(), 2);
}

#[tokio::test]
async fn long_change_lists_are_followed_page_by_page() {
    let server = JmapServer::start().await;
    server.add_message("inbox", &raw("seed@x", "S"), &[]);
    let b = backend(&server);
    b.snapshot("Inbox").await.unwrap();
    // The server hands out 50 objects at a time.
    for n in 0..130 {
        server.add_message("inbox", &raw(&format!("n{n}@x"), "N"), &[]);
    }
    server.clear_logs();
    let snap = b.snapshot("Inbox").await.unwrap();
    assert_eq!(snap.messages.len(), 131);
    assert!(count(&server, "Email/changes") >= 3, "{:?}", server.calls());
    assert_eq!(count(&server, "Email/query"), 0);
    assert_eq!(
        snap.messages.keys().copied().collect::<Vec<_>>(),
        (1..=131).collect::<Vec<u32>>()
    );
    // A full read of a folder that big pages through Email/query too.
    let fresh = backend(&server).snapshot("Inbox").await.unwrap();
    assert_eq!(fresh.messages.len(), 131);
}

// ------------------------------------------------------------------------------------------
// Actions
// ------------------------------------------------------------------------------------------

#[tokio::test]
async fn move_reports_new_uids_and_ignores_messages_that_are_no_longer_there() {
    let server = JmapServer::start().await;
    let a = server.add_message("inbox", &raw("a@x", "A"), &[]);
    server.add_message("inbox", &raw("b@x", "B"), &[]);
    let b = backend(&server);
    let snap = b.snapshot("Inbox").await.unwrap();
    assert_eq!(snap.messages.len(), 2);
    // Someone else moves the first message to Archive before our move runs.
    server.move_message(&a, "inbox", "archive");
    let out = b.move_to("Inbox", &[1, 2], "Deleted Items").await.unwrap();
    assert_eq!(out.dest_uids.len(), 1, "{out:?}");
    assert!(out.dest_uids.contains_key(&2));
    assert_eq!(server.ids_in("archive"), vec![a.clone()]);
    assert_eq!(server.ids_in("trash").len(), 1);
    // The destination numbers its own UIDs from 1.
    assert_eq!(out.dest_uids[&2], 1);
    // Moving to a folder that does not exist is a permanent refusal and changes nothing.
    let err = b.move_to("Inbox", &[1], "Nowhere").await.unwrap_err();
    assert!(matches!(err, BackendError::Permanent(_)), "{err:?}");
}

#[tokio::test]
async fn delete_forever_destroys_and_a_label_only_leaves_the_folder() {
    let server = JmapServer::start().await;
    let only = server.add_message("inbox", &raw("only@x", "Only"), &[]);
    let both = server.add_message("inbox", &raw("both@x", "Both"), &[]);
    // `both` is filed in Archive too.
    server.add_to_mailbox(&both, "archive");
    let b = backend(&server);
    let snap = b.snapshot("Inbox").await.unwrap();
    let uids: Vec<Uid> = snap.messages.keys().copied().collect();
    // In two mailboxes it has a UID in each, numbered independently.
    assert_eq!(b.snapshot("Archive").await.unwrap().messages.len(), 1);
    b.delete_forever("Inbox", &uids).await.unwrap();
    assert!(server.ids_in("inbox").is_empty());
    assert_eq!(
        server.ids_in("archive"),
        vec![both.clone()],
        "still in Archive"
    );
    assert!(server.keywords_of(&only).is_empty());
    assert_eq!(b.snapshot("Archive").await.unwrap().messages.len(), 1);
    assert!(b.snapshot("Inbox").await.unwrap().messages.is_empty());
    // Again: nothing left, no error.
    b.delete_forever("Inbox", &uids).await.unwrap();
}

#[tokio::test]
async fn flag_changes_are_keyword_patches_and_repeat_safely() {
    let server = JmapServer::start().await;
    let a = server.add_message("inbox", &raw("a@x", "A"), &["work"]);
    let b = backend(&server);
    b.snapshot("Inbox").await.unwrap();
    b.set_flag("Inbox", &[1], FlagChange::Seen(true))
        .await
        .unwrap();
    b.set_flag("Inbox", &[1], FlagChange::Seen(true))
        .await
        .unwrap();
    b.set_flag("Inbox", &[1], FlagChange::Flagged(true))
        .await
        .unwrap();
    let mut kw = server.keywords_of(&a);
    kw.sort();
    assert_eq!(kw, vec!["$flagged", "$seen", "work"]);
    b.set_flag("Inbox", &[1], FlagChange::Flagged(false))
        .await
        .unwrap();
    assert_eq!(server.keywords_of(&a), vec!["$seen", "work"]);
    // The snapshot after our own change shows it (and custom keywords survive).
    let snap = b.snapshot("Inbox").await.unwrap();
    assert!(snap.messages[&1].seen && !snap.messages[&1].flagged);
    assert!(snap.messages[&1].keywords.contains("work"));
}

#[tokio::test]
async fn find_by_message_id_uses_the_header_filter_or_scans_when_the_server_cannot() {
    let server = JmapServer::start().await;
    server.add_message("inbox", &raw("one@x", "One"), &[]);
    server.add_message("inbox", &raw("two@x", "Two"), &[]);
    server.add_message("archive", &raw("two@x", "Two elsewhere"), &[]);
    let b = backend(&server);
    server.clear_logs();
    assert_eq!(
        b.find_by_message_id("Inbox", "<two@x>").await.unwrap(),
        Some(1)
    );
    assert_eq!(
        count(&server, "Email/query"),
        1,
        "the header filter answered"
    );
    // A header search that finds nothing (Stalwart does not index Message-ID): scan the newest mail.
    server.set_header_search(false);
    server.clear_logs();
    assert_eq!(
        b.find_by_message_id("Inbox", "<one@x>").await.unwrap(),
        Some(2)
    );
    assert!(count(&server, "Email/query") >= 2);
    assert_eq!(
        b.find_by_message_id("Inbox", "<absent@x>").await.unwrap(),
        None
    );
    assert_eq!(b.find_by_message_id("Inbox", "").await.unwrap(), None);
    // The same Message-ID in another folder is another message with its own UID there.
    assert_eq!(
        b.find_by_message_id("Archive", "<two@x>").await.unwrap(),
        Some(1)
    );
}

// ------------------------------------------------------------------------------------------
// Errors
// ------------------------------------------------------------------------------------------

#[tokio::test]
async fn http_statuses_map_to_error_classes() {
    let server = JmapServer::start().await;
    server.add_message("inbox", &raw("a@x", "A"), &[]);
    let b = backend(&server);
    b.list_folders().await.unwrap(); // session fetched
    for (status, class) in [
        (401u16, "auth"),
        (403, "auth"),
        (429, "transient"),
        (500, "transient"),
        (503, "transient"),
        (400, "permanent"),
    ] {
        server.set_api_status(Some(status));
        let err = b.list_folders().await.unwrap_err();
        let got = match &err {
            BackendError::Auth(_) => "auth",
            BackendError::Transient(_) => "transient",
            BackendError::Permanent(_) => "permanent",
        };
        assert_eq!(got, class, "{status}: {err:?}");
    }
    server.set_api_status(None);
    assert!(b.list_folders().await.is_ok());
}

#[tokio::test]
async fn a_wrong_password_is_auth_with_a_fixed_message() {
    let server = JmapServer::start().await;
    let b = JmapBackend::new(
        config(&server),
        Credential::password("hunter2-wrong"),
        Arc::new(MemoryIdStore::new()),
    )
    .unwrap();
    for e in [
        b.capabilities().await.unwrap_err(),
        b.list_folders().await.unwrap_err(),
        b.snapshot("Inbox").await.unwrap_err(),
    ] {
        assert!(matches!(e, BackendError::Auth(_)), "{e:?}");
        let shown = format!("{e:?} {e}");
        assert!(
            !shown.contains("hunter2")
                && !shown.contains(JMAP_USER)
                && !shown.contains("127.0.0.1")
        );
    }
}

#[tokio::test]
async fn an_oauth_token_is_sent_as_a_bearer_token() {
    let server = JmapServer::start().await;
    server.set_bearer_token("tok-123");
    let b = JmapBackend::new(
        config(&server),
        Credential::oauth2(JMAP_USER, "tok-123"),
        Arc::new(MemoryIdStore::new()),
    )
    .unwrap();
    assert!(b.list_folders().await.is_ok());
    let b = JmapBackend::new(
        config(&server),
        Credential::oauth2(JMAP_USER, "other"),
        Arc::new(MemoryIdStore::new()),
    )
    .unwrap();
    assert!(matches!(b.list_folders().await, Err(BackendError::Auth(_))));
}

#[tokio::test]
async fn an_unknown_folder_is_permanent_and_a_hanging_server_is_transient() {
    let server = JmapServer::start().await;
    let b = backend(&server);
    assert!(matches!(
        b.snapshot("Nope").await,
        Err(BackendError::Permanent(_))
    ));
    assert!(matches!(
        b.folder_status("Nope").await,
        Err(BackendError::Permanent(_))
    ));
    server.set_hang(true);
    let quick = JmapBackend::new(
        config(&server).with_timeout(Duration::from_millis(300)),
        Credential::password(JMAP_PASSWORD),
        Arc::new(MemoryIdStore::new()),
    )
    .unwrap();
    let started = std::time::Instant::now();
    let err = quick.list_folders().await.unwrap_err();
    assert!(err.is_retryable(), "{err:?}");
    assert!(started.elapsed() < Duration::from_secs(5));
}

#[tokio::test]
async fn an_oversized_answer_is_refused() {
    let server = JmapServer::start().await;
    for n in 0..40 {
        server.add_message("inbox", &raw(&format!("n{n}@x"), "N"), &[]);
    }
    let b = JmapBackend::new(
        config(&server).with_max_response_bytes(400),
        Credential::password(JMAP_PASSWORD),
        Arc::new(MemoryIdStore::new()),
    )
    .unwrap();
    let err = b.snapshot("Inbox").await.unwrap_err();
    assert!(matches!(err, BackendError::Permanent(_)), "{err:?}");
}

// ------------------------------------------------------------------------------------------
// Where the credential goes
// ------------------------------------------------------------------------------------------

#[tokio::test]
async fn the_credential_goes_only_to_the_session_origin() {
    let server = JmapServer::start().await;
    let spy = JmapServer::start().await;
    // The session names the spy as the API host.
    server.set_session_api_base(Some(&format!("http://127.0.0.1:{}", spy.port())));
    server.add_message("inbox", &raw("a@x", "A"), &[]);
    let b = backend(&server);
    assert_eq!(b.snapshot("Inbox").await.unwrap().messages.len(), 1);
    assert!(spy.requests().is_empty(), "{:?}", spy.requests());
    assert!(server
        .requests()
        .iter()
        .all(|r| r.had_authorization || r.path == "/.well-known/jmap"));

    // A redirect of the session to another origin is refused, and the other origin sees nothing.
    server.set_session_api_base(None);
    server.set_session_redirect(Some(&format!(
        "http://127.0.0.1:{}/jmap/session",
        spy.port()
    )));
    let b = backend(&server);
    let err = b.list_folders().await.unwrap_err();
    assert!(matches!(err, BackendError::Permanent(_)), "{err:?}");
    assert!(spy.requests().is_empty());

    // A same-origin redirect (as /.well-known/jmap gives) is followed.
    server.set_session_redirect(None);
    let wk = JmapBackend::new(
        JmapConfig::new(server.well_known_url(), JMAP_USER).with_timeout(Duration::from_secs(5)),
        Credential::password(JMAP_PASSWORD),
        Arc::new(MemoryIdStore::new()),
    )
    .unwrap();
    assert!(wk.list_folders().await.is_ok());
}

#[tokio::test]
async fn plain_http_to_a_real_host_is_refused_when_the_adapter_is_made() {
    let err = JmapBackend::new(
        JmapConfig::new("http://mail.example.org/jmap/session", "u"),
        Credential::password("pw"),
        Arc::new(MemoryIdStore::new()),
    )
    .err();
    assert!(matches!(err, Some(BackendError::Permanent(_))));
    let b = backend(&JmapServer::start().await);
    let shown = format!("{b:?}");
    assert!(!shown.contains(JMAP_PASSWORD) && !shown.contains(JMAP_USER));
}

#[tokio::test]
async fn tls_needs_a_trusted_chain_or_the_exact_pin() {
    let server = JmapServer::start_with(JmapOptions { tls: true }).await;
    server.add_message("inbox", &raw("a@x", "A"), &[]);
    let cfg = JmapConfig::new(server.session_url(), JMAP_USER).with_timeout(Duration::from_secs(5));
    let make = |cfg: JmapConfig| {
        JmapBackend::new(
            cfg,
            Credential::password(JMAP_PASSWORD),
            Arc::new(MemoryIdStore::new()),
        )
        .unwrap()
    };

    // Self-signed and not pinned: refused, as an account-level problem rather than a queue failure.
    assert!(matches!(
        make(cfg.clone()).list_folders().await,
        Err(BackendError::Auth(_))
    ));

    // The probe reads the certificate without trusting it; pinning that works.
    let fp = probe_jmap_certificate(&cfg).await.unwrap();
    assert_eq!(
        fp,
        CertFingerprint::of_der(&server.certificate_der().unwrap())
    );
    let pinned = make(cfg.clone().with_pinned_cert(fp));
    assert_eq!(pinned.snapshot("Inbox").await.unwrap().messages.len(), 1);

    // Another pin is refused (no fallback to the roots).
    let wrong = make(
        cfg.clone()
            .with_pinned_cert(CertFingerprint::from_bytes([1; 32])),
    );
    assert!(matches!(
        wrong.list_folders().await,
        Err(BackendError::Auth(_))
    ));

    // No credential went out in the refused attempts: the server only saw handshakes, and the
    // requests it did see carried the credential (the pinned ones).
    assert!(server.requests().iter().all(|r| r.had_authorization));

    // Plain http has no certificate to read.
    let plain = JmapServer::start().await;
    assert!(probe_jmap_certificate(&config(&plain)).await.is_err());
}

// ------------------------------------------------------------------------------------------
// Discovery hook
// ------------------------------------------------------------------------------------------

#[tokio::test]
async fn a_discovered_jmap_address_builds_the_backend() {
    let server = JmapServer::start().await;
    server.add_message("inbox", &raw("a@x", "A"), &[]);
    let found = |jmap: Option<String>| DiscoveredSettings {
        imap: None,
        smtp: None,
        jmap,
        source: Source::JmapWellKnown,
        provider: None,
        auth_hint: AuthHint::Password,
    };
    let backend = JmapBackend::new_from(&found(Some(server.well_known_url())));
    let b = backend
        .expect("JMAP settings were found")
        .expect("usable settings");
    assert_eq!(b.snapshot("Inbox").await.unwrap().messages.len(), 1);
    assert!(JmapBackend::from_discovered(
        &found(None),
        JMAP_USER,
        Credential::password("x"),
        Arc::new(MemoryIdStore::new())
    )
    .is_none());
}

trait FromFound {
    fn new_from(found: &DiscoveredSettings) -> Option<Result<JmapBackend, BackendError>>;
}

impl FromFound for JmapBackend {
    fn new_from(found: &DiscoveredSettings) -> Option<Result<JmapBackend, BackendError>> {
        JmapBackend::from_discovered(
            found,
            JMAP_USER,
            Credential::password(JMAP_PASSWORD),
            Arc::new(MemoryIdStore::new()),
        )
    }
}

// ------------------------------------------------------------------------------------------
// Sending
// ------------------------------------------------------------------------------------------

fn submitter(server: &JmapServer) -> JmapSubmitter {
    JmapSubmitter::new(config(server), Credential::password(JMAP_PASSWORD)).unwrap()
}

const MIME: &str =
    "From: Alice <alice@example.org>\nTo: Bob <bob@example.org>\nSubject: Hello\n\nHi Bob.\n";

#[tokio::test]
async fn sending_submits_with_the_envelope_and_the_server_files_sent() {
    let server = JmapServer::start().await;
    let s = submitter(&server);
    let receipt = s
        .send(
            "alice@example.org",
            &["bob@example.org", "carol@example.org"],
            MIME.as_bytes(),
        )
        .await
        .unwrap();
    assert_eq!(receipt.recipients, 2);
    assert_eq!(receipt.bytes, MIME.len());
    let deliveries = server.deliveries();
    assert_eq!(deliveries.len(), 1);
    assert_eq!(deliveries[0].from, "alice@example.org");
    assert_eq!(
        deliveries[0].recipients,
        vec!["bob@example.org", "carol@example.org"]
    );
    // Line endings were made CRLF.
    assert!(deliveries[0]
        .raw
        .contains("Subject: Hello\r\n\r\nHi Bob.\r\n"));
    // One copy, in Sent, read and no longer a draft; Drafts is empty.
    let sent = server.ids_in("sent");
    assert_eq!(sent.len(), 1);
    let mut kw = server.keywords_of(&sent[0]);
    kw.sort();
    assert_eq!(kw, vec!["$seen"]);
    assert!(server.ids_in("drafts").is_empty());
}

#[tokio::test]
async fn a_refused_submission_leaves_nothing_behind_and_is_classified() {
    let server = JmapServer::start().await;
    let s = submitter(&server);
    for (kind, retryable) in [
        ("forbiddenFrom", false),
        ("tooManyRecipients", false),
        ("rateLimit", true),
    ] {
        server.set_submission_error(Some(kind));
        let err = s
            .send("alice@example.org", &["bob@example.org"], MIME.as_bytes())
            .await
            .unwrap_err();
        assert_eq!(err.is_retryable(), retryable, "{kind}: {err:?}");
        assert!(!matches!(err, BackendError::Auth(_)));
        let shown = format!("{err:?}");
        assert!(
            !shown.contains("alice") && !shown.contains("example.org"),
            "{shown}"
        );
        assert!(
            server.ids_in("drafts").is_empty(),
            "{kind}: the staged copy is removed"
        );
        assert!(server.ids_in("sent").is_empty());
    }
    assert!(server.deliveries().is_empty());
}

#[tokio::test]
async fn a_lost_answer_after_submission_is_never_retried_automatically() {
    let server = JmapServer::start().await;
    let s = submitter(&server);
    server.drop_answer_for(Some("EmailSubmission/set"));
    let err = s
        .send("alice@example.org", &["bob@example.org"], MIME.as_bytes())
        .await
        .unwrap_err();
    assert_eq!(err, BackendError::Permanent(MAYBE_SENT.into()));
    assert_eq!(server.deliveries().len(), 1, "the server did deliver it");
    // A failure before the last request (nobody home) is plainly retryable.
    let gone = {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap().port()
    };
    let nobody = JmapSubmitter::new(
        JmapConfig::new(format!("http://127.0.0.1:{gone}/jmap/session"), JMAP_USER)
            .with_timeout(Duration::from_secs(2)),
        Credential::password(JMAP_PASSWORD),
    )
    .unwrap();
    assert!(nobody
        .send("alice@example.org", &["bob@example.org"], MIME.as_bytes())
        .await
        .unwrap_err()
        .is_retryable());
}

const MAYBE_SENT: &str = "May not have been sent; check Sent before retrying";
