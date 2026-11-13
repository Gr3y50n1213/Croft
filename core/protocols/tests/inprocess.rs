// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Adapter tests against the in-process server from `croft-test-support`. No network, no
//! containers: these run on every `cargo test`.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use croft_protocols::*;
use croft_test_support::{
    sample_message, sample_reply, Caps, Misbehavior, ServerOptions, TestServer, TlsMode,
};
use std::collections::BTreeMap;
use std::time::Duration;
use std::time::Instant;

fn plain_config(server: &TestServer) -> ImapConfig {
    ImapConfig::new(
        server.host(),
        server.port(),
        Security::PlaintextForTestsOnly,
        server.username(),
    )
    .with_timeout(Duration::from_secs(5))
}

fn backend(server: &TestServer) -> ImapBackend {
    ImapBackend::new(
        plain_config(server),
        Credential::password(server.password()),
    )
    .unwrap()
}

async fn server_with(caps: Caps) -> TestServer {
    TestServer::start(ServerOptions {
        caps,
        ..ServerOptions::default()
    })
    .await
}

fn msg(id: &str) -> String {
    sample_message(
        id,
        &format!("Subject of {id}"),
        "Ann Example <ann@example.org>",
    )
}

// ------------------------------------------------------------------------------------------
// Folders and capabilities
// ------------------------------------------------------------------------------------------

#[tokio::test]
async fn lists_folders_with_special_use_attributes_first() {
    let server = TestServer::start_default().await;
    // A folder whose name says nothing, labelled by the server.
    server.add_folder("Gesendet", Some("\\Sent"));
    // A folder whose name looks like Trash but the server says it is the archive.
    server.add_folder("Deleted Items", Some("\\Archive"));
    let folders = backend(&server).list_folders().await.unwrap();
    let by_path: BTreeMap<_, _> = folders.iter().map(|f| (f.path.as_str(), f)).collect();
    assert_eq!(by_path["INBOX"].special_use, SpecialUse::Inbox);
    assert_eq!(by_path["Sent"].special_use, SpecialUse::Sent);
    assert_eq!(by_path["Trash"].special_use, SpecialUse::Trash);
    assert_eq!(by_path["Junk"].special_use, SpecialUse::Junk);
    assert_eq!(by_path["Archive"].special_use, SpecialUse::Archive);
    assert_eq!(by_path["Gesendet"].special_use, SpecialUse::Sent);
    assert_eq!(by_path["Deleted Items"].special_use, SpecialUse::Archive);
    assert_eq!(by_path["INBOX"].delimiter, Some('/'));
    assert!(by_path["INBOX"].selectable);
}

#[tokio::test]
async fn falls_back_to_folder_names_without_special_use() {
    let server = server_with(Caps::bare()).await;
    server.add_folder("Deleted Items", None);
    server.add_folder("Projects", None);
    let b = backend(&server);
    let folders = b.list_folders().await.unwrap();
    let kind = |p: &str| folders.iter().find(|f| f.path == p).unwrap().special_use;
    assert_eq!(kind("Sent"), SpecialUse::Sent);
    assert_eq!(kind("Drafts"), SpecialUse::Drafts);
    assert_eq!(kind("Deleted Items"), SpecialUse::Trash);
    assert_eq!(kind("Projects"), SpecialUse::Other);
    assert!(!b.capabilities().await.unwrap().special_use);
}

#[tokio::test]
async fn uses_list_return_special_use_when_the_plain_list_has_no_attributes() {
    let server = TestServer::start_default().await;
    server.set_plain_list_special_use(false);
    server.add_folder("Gesendet", Some("\\Sent"));
    let folders = backend(&server).list_folders().await.unwrap();
    let kind = folders
        .iter()
        .find(|f| f.path == "Gesendet")
        .unwrap()
        .special_use;
    assert_eq!(kind, SpecialUse::Sent);
    assert!(server
        .commands()
        .iter()
        .any(|c| c.contains("RETURN (SPECIAL-USE)")));
}

#[tokio::test]
async fn reports_capabilities() {
    let server = TestServer::start_default().await;
    let caps = backend(&server).capabilities().await.unwrap();
    assert_eq!(
        caps,
        Capabilities {
            move_cmd: true,
            uidplus: true,
            special_use: true,
            idle: true,
            condstore: true,
            qresync: true,
        }
    );
    let bare = server_with(Caps::bare()).await;
    assert_eq!(
        backend(&bare).capabilities().await.unwrap(),
        Capabilities::default()
    );
}

#[tokio::test]
async fn folder_names_with_non_ascii_text_round_trip() {
    let server = TestServer::start_default().await;
    server.add_folder("Caf&AOk-", None); // server-side (encoded) name of "Café"
    let b = backend(&server);
    let folders = b.list_folders().await.unwrap();
    assert!(folders.iter().any(|f| f.path == "Caf\u{e9}"));
    let status = b.folder_status("Caf\u{e9}").await.unwrap();
    assert_eq!(status.exists, 0);
}

// ------------------------------------------------------------------------------------------
// Status, UIDVALIDITY, snapshots
// ------------------------------------------------------------------------------------------

#[tokio::test]
async fn folder_status_reports_counters_and_detects_uidvalidity_change() {
    let server = TestServer::start_default().await;
    server.add_message("INBOX", &msg("a@x"));
    server.add_message("INBOX", &msg("b@x"));
    let b = backend(&server);
    let before = b.folder_status("INBOX").await.unwrap();
    assert_eq!(before.exists, 2);
    assert_eq!(before.uid_next, 3);
    assert_eq!(before.uid_validity, server.uid_validity("INBOX"));
    assert!(before.highest_modseq.is_some());

    server.change_uid_validity("INBOX", 4242);
    let after = b.folder_status("INBOX").await.unwrap();
    assert_ne!(before.uid_validity, after.uid_validity);
    assert_eq!(after.uid_validity, 4242);
}

#[tokio::test]
async fn snapshot_after_uidvalidity_change_returns_the_new_picture_not_a_stale_one() {
    let server = TestServer::start_default().await;
    for i in 0..3 {
        server.add_message("INBOX", &msg(&format!("m{i}@x")));
    }
    let b = backend(&server);
    let first = b.snapshot("INBOX").await.unwrap();
    server.change_uid_validity("INBOX", 77);
    server.modify_flags("INBOX", 2, &["\\Seen"], &[]);
    let second = b.snapshot("INBOX").await.unwrap();
    assert_eq!(second.status.uid_validity, 77);
    assert_ne!(first.status.uid_validity, second.status.uid_validity);
    assert!(second.messages[&2].seen);
    assert!(!second.messages[&1].seen);
}

#[tokio::test]
async fn snapshot_reads_flags_for_every_uid() {
    let server = server_with(Caps::bare()).await;
    server.add_message_with_flags("INBOX", &msg("a@x"), &["\\Seen"]);
    server.add_message_with_flags("INBOX", &msg("b@x"), &["\\Flagged", "\\Answered", "$Junk"]);
    server.add_message("INBOX", &msg("c@x"));
    let snap = backend(&server).snapshot("INBOX").await.unwrap();
    assert_eq!(snap.messages.len(), 3);
    assert!(snap.messages[&1].seen);
    assert!(snap.messages[&2].flagged && snap.messages[&2].answered);
    assert!(snap.messages[&2].keywords.contains("$Junk"));
    assert_eq!(snap.messages[&3], Flags::default());
    assert_eq!(snap.status.exists, 3);
    assert_eq!(snap.status.uid_next, 4);
    assert_eq!(snap.status.highest_modseq, None);
}

#[tokio::test]
async fn snapshot_of_an_empty_folder_and_a_missing_folder() {
    let server = TestServer::start_default().await;
    let b = backend(&server);
    assert!(b.snapshot("INBOX").await.unwrap().messages.is_empty());
    let err = b.snapshot("No such folder").await.unwrap_err();
    assert!(matches!(err, BackendError::Permanent(_)), "{err:?}");
}

async fn delta_scenario(caps: Caps, expect_in_log: &str) {
    let server = server_with(caps).await;
    for i in 1..=5 {
        server.add_message("INBOX", &msg(&format!("m{i}@x")));
    }
    let b = backend(&server);
    let first = b.snapshot("INBOX").await.unwrap();
    assert_eq!(first.messages.len(), 5);

    // Nothing changed: still the full picture.
    server.clear_commands();
    let same = b.snapshot("INBOX").await.unwrap();
    assert_eq!(same.messages, first.messages);

    // Another client flags #2, expunges #4 and delivers #6.
    server.modify_flags("INBOX", 2, &["\\Seen"], &[]);
    server.remove_messages("INBOX", &[4]);
    server.add_message("INBOX", &msg("m6@x"));
    server.clear_commands();
    let next = b.snapshot("INBOX").await.unwrap();
    let truth: BTreeMap<u32, bool> = server
        .messages("INBOX")
        .iter()
        .map(|m| (m.uid, m.flags.contains("\\Seen")))
        .collect();
    let got: BTreeMap<u32, bool> = next.messages.iter().map(|(u, f)| (*u, f.seen)).collect();
    assert_eq!(got, truth);
    assert!(!next.messages.contains_key(&4));
    assert!(next.messages.contains_key(&6));
    assert!(next.messages[&2].seen);
    assert_eq!(next.status.exists, 5);
    let log = server.commands().join("\n");
    assert!(
        log.contains(expect_in_log),
        "expected {expect_in_log} in:\n{log}"
    );
    assert!(
        !log.contains("UID FETCH 1:* (FLAGS)\n") && !log.ends_with("UID FETCH 1:* (FLAGS)"),
        "full fetch used:\n{log}"
    );
}

#[tokio::test]
async fn snapshot_uses_condstore_deltas_when_advertised() {
    let caps = Caps {
        qresync: false,
        ..Caps::all()
    };
    delta_scenario(caps, "CHANGEDSINCE").await;
}

#[tokio::test]
async fn snapshot_uses_qresync_when_advertised() {
    delta_scenario(Caps::all(), "QRESYNC (").await;
}

#[tokio::test]
async fn snapshot_without_condstore_does_a_full_fetch_every_time() {
    let server = server_with(Caps::bare()).await;
    server.add_message("INBOX", &msg("a@x"));
    let b = backend(&server);
    b.snapshot("INBOX").await.unwrap();
    server.clear_commands();
    b.snapshot("INBOX").await.unwrap();
    assert!(server
        .commands()
        .iter()
        .any(|c| c == "UID FETCH 1:* (FLAGS)"));
}

// ------------------------------------------------------------------------------------------
// Headers
// ------------------------------------------------------------------------------------------

#[tokio::test]
async fn fetch_headers_returns_envelope_references_size_and_date() {
    let server = TestServer::start_default().await;
    let root = sample_message(
        "root@x",
        "=?UTF-8?Q?Caf=C3=A9_menu?=",
        "=?UTF-8?B?w4VzYQ==?= <asa@example.org>",
    );
    let reply = sample_reply(
        "reply@x",
        "Re: menu",
        "Bob <bob@example.org>",
        "root@x",
        "origin@x",
    );
    let u1 = server.add_message_with_flags("INBOX", &root, &["\\Seen"]);
    let u2 = server.add_message("INBOX", &reply);
    let headers = backend(&server)
        .fetch_headers("INBOX", &[u1, u2, 99])
        .await
        .unwrap();
    assert_eq!(headers.len(), 2, "unknown UID 99 is left out");
    // Newest first (same internal date minute, higher UID later).
    assert_eq!(headers[0].uid, u2);
    let r = &headers[1];
    assert_eq!(r.uid, u1);
    assert_eq!(r.subject, "Caf\u{e9} menu");
    assert_eq!(r.from_name.as_deref(), Some("\u{c5}sa"));
    assert_eq!(r.from_addr.as_deref(), Some("asa@example.org"));
    assert_eq!(r.message_id.as_deref(), Some("<root@x>"));
    assert!(r.flags.seen);
    assert_eq!(r.size as usize, root.len());
    // INTERNALDATE of the test server: 2026-01-01 10:MM:SS UTC from the UID.
    assert_eq!(
        r.date_ms,
        1_767_261_600_000 + 1000 * i64::from(u1 % 60) + 60_000 * i64::from((u1 / 60) % 60)
    );
    let h = &headers[0];
    assert_eq!(h.references, vec!["<origin@x>", "<root@x>"]);
    assert_eq!(h.message_id.as_deref(), Some("<reply@x>"));
    assert_eq!(h.from_name.as_deref(), Some("Bob"));
    assert!(!h.flags.seen);
}

#[tokio::test]
async fn find_by_message_id_finds_the_newest_match() {
    let server = TestServer::start_default().await;
    server.add_message("INBOX", &msg("one@x"));
    let two = server.add_message("INBOX", &msg("two@x"));
    let b = backend(&server);
    assert_eq!(
        b.find_by_message_id("INBOX", "<two@x>").await.unwrap(),
        Some(two)
    );
    assert_eq!(
        b.find_by_message_id("INBOX", "two@x").await.unwrap(),
        Some(two)
    );
    assert_eq!(
        b.find_by_message_id("INBOX", "<none@x>").await.unwrap(),
        None
    );
    assert_eq!(
        b.find_by_message_id("INBOX", "<caf\u{e9}@x>")
            .await
            .unwrap(),
        None
    );
}

// ------------------------------------------------------------------------------------------
// Flags
// ------------------------------------------------------------------------------------------

#[tokio::test]
async fn set_flag_sets_and_clears_and_is_safe_to_repeat() {
    let server = TestServer::start_default().await;
    let u1 = server.add_message("INBOX", &msg("a@x"));
    let u2 = server.add_message("INBOX", &msg("b@x"));
    let b = backend(&server);
    for _ in 0..2 {
        b.set_flag("INBOX", &[u1, u2, 500], FlagChange::Seen(true))
            .await
            .unwrap();
    }
    b.set_flag("INBOX", &[u2], FlagChange::Flagged(true))
        .await
        .unwrap();
    let m = server.messages("INBOX");
    assert!(m[0].flags.contains("\\Seen") && !m[0].flags.contains("\\Flagged"));
    assert!(m[1].flags.contains("\\Seen") && m[1].flags.contains("\\Flagged"));
    b.set_flag("INBOX", &[u1], FlagChange::Seen(false))
        .await
        .unwrap();
    assert!(!server.messages("INBOX")[0].flags.contains("\\Seen"));
}

/// T36b: pin and follow-up are keywords the server keeps; clearing one leaves the other.
#[tokio::test]
async fn croft_keywords_are_set_and_cleared() {
    let server = TestServer::start_default().await;
    let uid = server.add_message("INBOX", &msg("a@x"));
    let b = backend(&server);
    b.set_flag("INBOX", &[uid], FlagChange::Keyword(Keyword::Pinned, true))
        .await
        .unwrap();
    b.set_flag(
        "INBOX",
        &[uid],
        FlagChange::Keyword(Keyword::FollowUp, true),
    )
    .await
    .unwrap();
    let flags = &server.messages("INBOX")[0].flags;
    assert!(flags.contains("$Pinned") && flags.contains("$FollowUp"));
    b.set_flag("INBOX", &[uid], FlagChange::Keyword(Keyword::Pinned, false))
        .await
        .unwrap();
    let flags = &server.messages("INBOX")[0].flags;
    assert!(!flags.contains("$Pinned") && flags.contains("$FollowUp"));
    let snap = b.snapshot("INBOX").await.unwrap();
    assert!(
        snap.messages[&uid].has(Keyword::FollowUp) && !snap.messages[&uid].has(Keyword::Pinned)
    );
}

/// T36b: a folder whose PERMANENTFLAGS has no `\*` would drop the keyword at the next sync, so
/// setting one is refused before anything is sent; clearing one is still allowed.
#[tokio::test]
async fn a_keyword_the_folder_cannot_keep_is_refused_without_a_store() {
    let server = TestServer::start_default().await;
    let uid = server.add_message("INBOX", &msg("a@x"));
    server.set_keeps_new_keywords("INBOX", false);
    let b = backend(&server);
    server.clear_commands();
    let err = b
        .set_flag("INBOX", &[uid], FlagChange::Keyword(Keyword::Pinned, true))
        .await
        .unwrap_err();
    assert!(matches!(err, BackendError::Permanent(_)), "{err:?}");
    assert!(server.commands().iter().all(|c| !c.contains("STORE")));
    assert!(!server.messages("INBOX")[0].flags.contains("$Pinned"));
    b.set_flag("INBOX", &[uid], FlagChange::Keyword(Keyword::Pinned, false))
        .await
        .unwrap();
}

#[tokio::test]
async fn set_flag_on_a_read_only_folder_is_a_permanent_refusal() {
    let server = TestServer::start_default().await;
    let uid = server.add_message("INBOX", &msg("a@x"));
    server.set_read_only("INBOX", true);
    let err = backend(&server)
        .set_flag("INBOX", &[uid], FlagChange::Seen(true))
        .await
        .unwrap_err();
    assert!(matches!(err, BackendError::Permanent(_)), "{err:?}");
}

// ------------------------------------------------------------------------------------------
// Move and delete
// ------------------------------------------------------------------------------------------

#[tokio::test]
async fn move_uses_uid_move_and_returns_the_new_uids() {
    let server = TestServer::start_default().await;
    server.add_message("Trash", &msg("old@x"));
    let a = server.add_message("INBOX", &msg("a@x"));
    let b_uid = server.add_message("INBOX", &msg("b@x"));
    let c = server.add_message("INBOX", &msg("c@x"));
    let b = backend(&server);
    server.clear_commands();
    let out = b.move_to("INBOX", &[a, c], "Trash").await.unwrap();
    assert_eq!(out.dest_uids, BTreeMap::from([(a, 2), (c, 3)]));
    assert!(server.commands().iter().any(|c| c.starts_with("UID MOVE ")));
    assert_eq!(server.messages("INBOX").len(), 1);
    assert_eq!(server.messages("INBOX")[0].uid, b_uid);
    assert_eq!(server.messages("Trash").len(), 3);
}

#[tokio::test]
async fn repeating_a_move_is_harmless_and_reports_only_what_moved() {
    let server = TestServer::start_default().await;
    let a = server.add_message("INBOX", &msg("a@x"));
    let b = backend(&server);
    let first = b.move_to("INBOX", &[a], "Trash").await.unwrap();
    assert_eq!(first.dest_uids.len(), 1);
    let again = b.move_to("INBOX", &[a], "Trash").await.unwrap();
    assert!(again.dest_uids.is_empty());
    assert_eq!(server.messages("Trash").len(), 1);
    // A mix of gone and present UIDs moves what is there.
    let b2 = server.add_message("INBOX", &msg("b@x"));
    let mixed = b.move_to("INBOX", &[a, b2, 77], "Trash").await.unwrap();
    assert_eq!(
        mixed.dest_uids.keys().copied().collect::<Vec<_>>(),
        vec![b2]
    );
}

#[tokio::test]
async fn move_without_move_falls_back_to_copy_store_and_uid_expunge() {
    let caps = Caps {
        move_cmd: false,
        ..Caps::all()
    };
    let server = server_with(caps).await;
    let a = server.add_message("INBOX", &msg("a@x"));
    let other = server.add_message_with_flags("INBOX", &msg("keep@x"), &["\\Deleted"]);
    server.clear_commands();
    let out = backend(&server)
        .move_to("INBOX", &[a], "Archive")
        .await
        .unwrap();
    assert_eq!(out.dest_uids.get(&a), Some(&1));
    let log = server.commands().join("\n");
    assert!(log.contains("UID COPY") && log.contains("UID EXPUNGE") && !log.contains("UID MOVE"));
    // UID EXPUNGE touched only our message: the other \Deleted one is still there.
    assert_eq!(server.messages("INBOX").len(), 1);
    assert_eq!(server.messages("INBOX")[0].uid, other);
}

#[tokio::test]
async fn move_without_move_or_uidplus_gives_no_new_uid_and_never_expunges_others() {
    let server = server_with(Caps::bare()).await;
    let a = server.add_message("INBOX", &msg("a@x"));
    let b = backend(&server);
    let out = b.move_to("INBOX", &[a], "Archive").await.unwrap();
    assert!(out.dest_uids.is_empty(), "no UIDPLUS, no COPYUID");
    assert_eq!(server.messages("Archive").len(), 1);
    assert!(server.messages("INBOX").is_empty());
    // Found afterwards by Message-ID.
    assert_eq!(
        b.find_by_message_id("Archive", "<a@x>").await.unwrap(),
        Some(1)
    );

    // With someone else's \Deleted message in the folder, refuse before copying anything.
    let mine = server.add_message("INBOX", &msg("mine@x"));
    server.add_message_with_flags("INBOX", &msg("theirs@x"), &["\\Deleted"]);
    let err = b.move_to("INBOX", &[mine], "Archive").await.unwrap_err();
    assert!(matches!(err, BackendError::Permanent(_)), "{err:?}");
    assert_eq!(server.messages("INBOX").len(), 2);
    assert_eq!(server.messages("Archive").len(), 1);
}

#[tokio::test]
async fn move_to_a_missing_folder_is_permanent() {
    let server = TestServer::start_default().await;
    let a = server.add_message("INBOX", &msg("a@x"));
    let err = backend(&server)
        .move_to("INBOX", &[a], "Nowhere")
        .await
        .unwrap_err();
    assert!(matches!(err, BackendError::Permanent(_)), "{err:?}");
    assert_eq!(server.messages("INBOX").len(), 1);
}

#[tokio::test]
async fn delete_forever_with_uidplus_expunges_only_the_given_uids() {
    let server = TestServer::start_default().await;
    let a = server.add_message("Trash", &msg("a@x"));
    let other = server.add_message_with_flags("Trash", &msg("marked@x"), &["\\Deleted"]);
    let keep = server.add_message("Trash", &msg("keep@x"));
    let b = backend(&server);
    server.clear_commands();
    b.delete_forever("Trash", &[a]).await.unwrap();
    assert!(server
        .commands()
        .iter()
        .any(|c| c.starts_with("UID EXPUNGE")));
    let left: Vec<u32> = server.messages("Trash").iter().map(|m| m.uid).collect();
    assert_eq!(left, vec![other, keep]);
    // Repeating is fine.
    b.delete_forever("Trash", &[a]).await.unwrap();
}

#[tokio::test]
async fn delete_forever_without_uidplus_refuses_when_others_are_marked_deleted() {
    let server = server_with(Caps::bare()).await;
    let a = server.add_message("Trash", &msg("a@x"));
    let marked = server.add_message_with_flags("Trash", &msg("marked@x"), &["\\Deleted"]);
    let b = backend(&server);
    let err = b.delete_forever("Trash", &[a]).await.unwrap_err();
    match err {
        BackendError::Permanent(m) => assert!(m.contains("marked deleted"), "{m}"),
        other => panic!("{other:?}"),
    }
    // Nothing was changed: not even the \Deleted flag on our message.
    let msgs = server.messages("Trash");
    assert_eq!(msgs.len(), 2);
    assert!(!msgs[0].flags.contains("\\Deleted"));

    // Once the folder has no other deleted messages a plain EXPUNGE is safe.
    server.remove_messages("Trash", &[marked]);
    b.delete_forever("Trash", &[a]).await.unwrap();
    assert!(server.messages("Trash").is_empty());
    assert!(server.commands().iter().any(|c| c == "EXPUNGE"));
}

#[tokio::test]
async fn append_returns_the_new_uid_with_uidplus() {
    let server = TestServer::start_default().await;
    let b = backend(&server);
    let uid = b
        .append_message("Drafts", &["\\Draft"], &msg("draft@x"))
        .await
        .unwrap();
    assert_eq!(uid, Some(1));
    let stored = server.messages("Drafts");
    assert_eq!(stored.len(), 1);
    assert!(stored[0].flags.contains("\\Draft"));
    assert_eq!(stored[0].raw, msg("draft@x"));
}

// ------------------------------------------------------------------------------------------
// Sign-in
// ------------------------------------------------------------------------------------------

#[tokio::test]
async fn wrong_password_is_auth_and_never_transient() {
    let server = TestServer::start_default().await;
    let b = ImapBackend::new(
        plain_config(&server),
        Credential::password("not the password"),
    )
    .unwrap();
    for _ in 0..2 {
        let err = b.list_folders().await.unwrap_err();
        match err {
            BackendError::Auth(m) => assert!(!m.contains("not the password")),
            other => panic!("expected Auth, got {other:?}"),
        }
    }
    assert!(!server
        .commands()
        .iter()
        .any(|c| c.contains("not the password")));
}

#[tokio::test]
async fn a_busy_login_is_transient_not_auth() {
    let server = TestServer::start_default().await;
    server.set_login_unavailable(true);
    let err = backend(&server).list_folders().await.unwrap_err();
    assert!(matches!(err, BackendError::Transient(_)), "{err:?}");
}

#[tokio::test]
async fn xoauth2_signs_in_and_a_bad_token_is_auth() {
    let server = TestServer::start_default().await;
    let good = ImapBackend::new(
        plain_config(&server),
        Credential::oauth2(server.username(), server.password()),
    )
    .unwrap();
    assert!(!good.list_folders().await.unwrap().is_empty());

    let bad = ImapBackend::new(
        plain_config(&server),
        Credential::oauth2(server.username(), "expired-token"),
    )
    .unwrap();
    let err = bad.list_folders().await.unwrap_err();
    match err {
        BackendError::Auth(m) => assert!(!m.contains("expired-token")),
        other => panic!("{other:?}"),
    }
}

// ------------------------------------------------------------------------------------------
// Connection handling
// ------------------------------------------------------------------------------------------

#[tokio::test]
async fn one_session_is_reused_and_a_dropped_one_is_replaced() {
    let server = TestServer::start_default().await;
    let b = backend(&server);
    b.list_folders().await.unwrap();
    b.folder_status("INBOX").await.unwrap();
    assert_eq!(server.connection_count(), 1);

    server.drop_connections();
    tokio::time::sleep(Duration::from_millis(100)).await;
    // The next call notices the dead session, reconnects and repeats.
    let folders = b.list_folders().await.unwrap();
    assert!(!folders.is_empty());
    assert_eq!(server.connection_count(), 2);
}

#[tokio::test]
async fn a_silent_server_times_out_as_transient_and_the_adapter_recovers() {
    let server = TestServer::start_default().await;
    let b = ImapBackend::new(
        plain_config(&server).with_timeout(Duration::from_millis(300)),
        Credential::password(server.password()),
    )
    .unwrap();
    b.list_folders().await.unwrap();
    server.set_hang(true);
    let err = b.folder_status("INBOX").await.unwrap_err();
    assert!(matches!(err, BackendError::Transient(_)), "{err:?}");
    server.set_hang(false);
    assert!(b.folder_status("INBOX").await.is_ok());
}

#[tokio::test]
async fn nothing_listening_is_transient() {
    let cfg = ImapConfig::new("127.0.0.1", 1, Security::PlaintextForTestsOnly, "u")
        .with_timeout(Duration::from_secs(2));
    let b = ImapBackend::new(cfg, Credential::password("pw")).unwrap();
    let err = b.list_folders().await.unwrap_err();
    assert!(matches!(err, BackendError::Transient(_)), "{err:?}");
}

#[tokio::test]
async fn the_server_never_sees_the_password_in_its_log_and_errors_carry_no_content() {
    let server = TestServer::start_default().await;
    let a = server.add_message("INBOX", &msg("secret-id@x"));
    let b = backend(&server);
    let err = b
        .move_to("INBOX", &[a], "Missing <secret-id@x>")
        .await
        .unwrap_err();
    let shown = format!("{err} {err:?}");
    for leak in [
        "secret-id",
        "Missing",
        "alice",
        "correct horse",
        "ann@example.org",
    ] {
        assert!(!shown.contains(leak), "{leak} leaked in {shown}");
    }
    assert!(server.commands().iter().any(|c| c == "LOGIN ***"));
}

// ------------------------------------------------------------------------------------------
// TLS
// ------------------------------------------------------------------------------------------

async fn tls_server(mode: TlsMode) -> (TestServer, CertFingerprint) {
    let server = TestServer::start(ServerOptions {
        tls: mode,
        ..ServerOptions::default()
    })
    .await;
    let fp = CertFingerprint::of_der(&server.certificate_der().unwrap());
    (server, fp)
}

fn tls_config(server: &TestServer, security: Security) -> ImapConfig {
    ImapConfig::new(server.host(), server.port(), security, server.username())
        .with_timeout(Duration::from_secs(5))
}

#[tokio::test]
async fn implicit_tls_with_the_pinned_fingerprint_works() {
    let (server, fp) = tls_server(TlsMode::Implicit).await;
    let b = ImapBackend::new(
        tls_config(&server, Security::Tls).with_pinned_cert(fp),
        Credential::password(server.password()),
    )
    .unwrap();
    assert!(!b.list_folders().await.unwrap().is_empty());
}

#[tokio::test]
async fn a_self_signed_certificate_is_refused_without_a_pin() {
    let (server, _fp) = tls_server(TlsMode::Implicit).await;
    let b = ImapBackend::new(
        tls_config(&server, Security::Tls),
        Credential::password(server.password()),
    )
    .unwrap();
    let err = b.list_folders().await.unwrap_err();
    match err {
        // A certificate that cannot be verified pauses the account (Auth); it is not Permanent.
        BackendError::Auth(m) => assert!(m.contains("certificate could not be verified"), "{m}"),
        other => panic!("{other:?}"),
    }
    // The password was never sent.
    assert!(server.commands().is_empty());
}

#[tokio::test]
async fn a_wrong_pin_is_refused_before_any_credentials_are_sent() {
    let (server, _fp) = tls_server(TlsMode::Implicit).await;
    let other = CertFingerprint::of_der(b"some other certificate");
    let b = ImapBackend::new(
        tls_config(&server, Security::Tls).with_pinned_cert(other),
        Credential::password(server.password()),
    )
    .unwrap();
    let err = b.list_folders().await.unwrap_err();
    match err {
        BackendError::Auth(m) => {
            assert!(
                m.contains("could not be verified") && m.contains("fingerprint"),
                "{m}"
            )
        }
        other => panic!("{other:?}"),
    }
    assert!(server.commands().is_empty());
}

#[tokio::test]
async fn starttls_upgrades_before_login_and_honours_the_pin() {
    let (server, fp) = tls_server(TlsMode::StartTls).await;
    let b = ImapBackend::new(
        tls_config(&server, Security::StartTls).with_pinned_cert(fp),
        Credential::password(server.password()),
    )
    .unwrap();
    assert!(!b.list_folders().await.unwrap().is_empty());
    let log = server.commands();
    let starttls = log.iter().position(|c| c == "STARTTLS").unwrap();
    let login = log.iter().position(|c| c == "LOGIN ***").unwrap();
    assert!(starttls < login, "{log:?}");

    // Wrong pin over STARTTLS: refused, and no LOGIN is ever sent.
    server.clear_commands();
    let bad = ImapBackend::new(
        tls_config(&server, Security::StartTls).with_pinned_cert(CertFingerprint::of_der(b"x")),
        Credential::password(server.password()),
    )
    .unwrap();
    assert!(matches!(
        bad.list_folders().await,
        Err(BackendError::Auth(_))
    ));
    assert!(!server.commands().iter().any(|c| c.starts_with("LOGIN")));
}

#[tokio::test]
async fn starttls_never_falls_back_to_plaintext() {
    // A server with no TLS at all, reached with StartTls on loopback.
    let server = TestServer::start_default().await;
    let b = ImapBackend::new(
        tls_config(&server, Security::StartTls),
        Credential::password(server.password()),
    )
    .unwrap();
    let err = b.list_folders().await.unwrap_err();
    assert!(matches!(err, BackendError::Permanent(_)), "{err:?}");
    assert!(!server.commands().iter().any(|c| c.starts_with("LOGIN")));
}

#[tokio::test]
async fn probe_certificate_reads_the_fingerprint_without_signing_in() {
    for mode in [TlsMode::Implicit, TlsMode::StartTls] {
        let (server, fp) = tls_server(mode).await;
        let security = if mode == TlsMode::Implicit {
            Security::Tls
        } else {
            Security::StartTls
        };
        let seen = probe_certificate(&tls_config(&server, security))
            .await
            .unwrap();
        assert_eq!(seen, fp);
        assert!(!server.commands().iter().any(|c| c.starts_with("LOGIN")));
    }
    let plain = TestServer::start_default().await;
    assert!(probe_certificate(&plain_config(&plain)).await.is_err());
}

#[tokio::test]
async fn create_folder_is_idempotent() {
    let server = TestServer::start_default().await;
    let b = backend(&server);
    b.create_folder("Receipts").await.unwrap();
    b.create_folder("Receipts").await.unwrap();
    let folders = b.list_folders().await.unwrap();
    assert_eq!(folders.iter().filter(|f| f.path == "Receipts").count(), 1);
}

// ------------------------------------------------------------------------------------------
// Sign-in refusals that must never be retried (review finding 2)
// ------------------------------------------------------------------------------------------

#[tokio::test]
async fn a_bye_during_login_is_auth_not_transient() {
    // Servers that drop the connection after too many failed sign-ins say `* BYE` and hang up.
    // Reading that as a network error made the queue retry the wrong password until lockout.
    let server = TestServer::start_default().await;
    server.set_misbehavior(Misbehavior::ByeOnLogin);
    let b = ImapBackend::new(
        plain_config(&server),
        Credential::password("not the password"),
    )
    .unwrap();
    for _ in 0..2 {
        let err = b.list_folders().await.unwrap_err();
        match err {
            BackendError::Auth(m) => assert!(!m.contains("not the password"), "{m}"),
            other => panic!("expected Auth, got {other:?}"),
        }
    }
    // The same for a token.
    let oauth = ImapBackend::new(
        plain_config(&server),
        Credential::oauth2(server.username(), "expired-token"),
    )
    .unwrap();
    let err = oauth.list_folders().await.unwrap_err();
    assert!(matches!(err, BackendError::Auth(_)), "{err:?}");
    // One try per call: nothing reconnected and tried again behind the caller's back.
    assert_eq!(server.connection_count(), 3);
}

#[tokio::test]
async fn an_auth_failure_whose_text_says_unavailable_is_auth_not_transient() {
    let server = TestServer::start_default().await;
    server.set_misbehavior(Misbehavior::AuthFailedMentioningUnavailable);
    let b = backend(&server);
    let err = b.list_folders().await.unwrap_err();
    assert!(matches!(err, BackendError::Auth(_)), "{err:?}");
    let oauth = ImapBackend::new(
        plain_config(&server),
        Credential::oauth2(server.username(), "token"),
    )
    .unwrap();
    let err = oauth.list_folders().await.unwrap_err();
    assert!(matches!(err, BackendError::Auth(_)), "{err:?}");
}

// ------------------------------------------------------------------------------------------
// A hostile server cannot make the client allocate wildly or run for ever (finding 3)
// ------------------------------------------------------------------------------------------

/// Peak resident memory of this process in KiB (Linux only; `None` elsewhere).
fn peak_rss_kib() -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    let line = status.lines().find(|l| l.starts_with("VmHWM:"))?;
    line.split_whitespace().nth(1)?.parse().ok()
}

#[tokio::test]
async fn a_500_megabyte_literal_is_refused_without_allocating_it() {
    let server = TestServer::start_default().await;
    server.set_misbehavior(Misbehavior::HugeLiteral(500_000_000));
    let b = ImapBackend::new(
        plain_config(&server),
        Credential::password(server.password()),
    )
    .unwrap();
    // Sign-in and the first replies are normal; the LIST answer announces 500 MB and sends two
    // bytes. The server then goes silent, so a client that waited for the data would run into
    // the 5 s timeout. Refusing the size up front returns at once.
    let before = peak_rss_kib();
    let started = Instant::now();
    let err = b.list_folders().await.unwrap_err();
    assert!(
        started.elapsed() < Duration::from_secs(3),
        "{:?}",
        started.elapsed()
    );
    assert!(matches!(err, BackendError::Transient(_)), "{err:?}");
    // The patched buffer limit is 64 MiB. The stock 512 MiB limit pre-sized a 500 MB buffer
    // (the reviewer measured +490 MB); allow a generous 150 MiB of growth for everything else
    // the test process does at the same time.
    if let (Some(before), Some(after)) = (before, peak_rss_kib()) {
        assert!(
            after.saturating_sub(before) < 150 * 1024,
            "peak memory grew by {} KiB",
            after.saturating_sub(before)
        );
    }
}

#[tokio::test]
async fn an_endless_stream_of_replies_hits_the_command_deadline() {
    let server = TestServer::start_default().await;
    server.set_misbehavior(Misbehavior::EndlessExists);
    // Replies arrive far faster than the 5 s silence timeout, so only the overall deadline can
    // stop this. The reply cap is set out of reach so the test is about the deadline.
    let b = ImapBackend::new(
        plain_config(&server)
            .with_command_deadline(Duration::from_millis(800))
            .with_max_untagged_replies(usize::MAX),
        Credential::password(server.password()),
    )
    .unwrap();
    let started = Instant::now();
    let err = b.list_folders().await.unwrap_err();
    let took = started.elapsed();
    assert!(
        took >= Duration::from_millis(700) && took < Duration::from_secs(4),
        "{took:?}"
    );
    match err {
        BackendError::Transient(m) => assert!(m.contains("took too long"), "{m}"),
        other => panic!("expected Transient, got {other:?}"),
    }
    // The connection was dropped: the next call opens a new one.
    server.set_misbehavior(Misbehavior::None);
    assert!(b.list_folders().await.is_ok());
    assert_eq!(server.connection_count(), 2);
}

#[tokio::test]
async fn too_many_untagged_replies_are_refused() {
    let server = TestServer::start_default().await;
    server.set_misbehavior(Misbehavior::EndlessExists);
    let b = ImapBackend::new(
        plain_config(&server).with_max_untagged_replies(1_000),
        Credential::password(server.password()),
    )
    .unwrap();
    let started = Instant::now();
    let err = b.list_folders().await.unwrap_err();
    assert!(
        started.elapsed() < Duration::from_secs(3),
        "{:?}",
        started.elapsed()
    );
    match err {
        BackendError::Transient(m) => assert!(m.contains("far more data"), "{m}"),
        other => panic!("expected Transient, got {other:?}"),
    }
}

// ------------------------------------------------------------------------------------------
// SEARCH answers are not trusted (finding 4)
// ------------------------------------------------------------------------------------------

fn uids_in(server: &TestServer, folder: &str) -> Vec<u32> {
    server.messages(folder).iter().map(|m| m.uid).collect()
}

#[tokio::test]
async fn a_search_that_lists_extra_uids_never_moves_or_deletes_other_messages() {
    for caps in [Caps::all(), Caps::bare()] {
        let server = server_with(caps).await;
        let a = server.add_message("INBOX", &msg("a@x"));
        let b_uid = server.add_message("INBOX", &msg("b@x"));
        let c = server.add_message("INBOX", &msg("c@x"));
        server.set_misbehavior(Misbehavior::SearchExtraUids);
        let backend = backend(&server);

        // Move only `a`. The server's SEARCH also names b, c and a UID that does not exist.
        backend.move_to("INBOX", &[a], "Trash").await.unwrap();
        assert_eq!(uids_in(&server, "INBOX"), vec![b_uid, c], "{caps:?}");
        assert_eq!(server.messages("Trash").len(), 1, "{caps:?}");
        for m in server.messages("INBOX") {
            assert!(!m.flags.contains("\\Deleted"), "{caps:?}");
        }

        // Delete only `b`.
        backend.delete_forever("INBOX", &[b_uid]).await.unwrap();
        assert_eq!(uids_in(&server, "INBOX"), vec![c], "{caps:?}");
        assert_eq!(server.messages("Trash").len(), 1, "{caps:?}");
    }
}

#[tokio::test]
async fn a_move_without_uidplus_stops_when_another_client_marks_a_later_chunk_deleted() {
    // 501 messages are two chunks of UIDs (500 + 1). Another client has already marked the last
    // one \Deleted. Without UIDPLUS the expunge after chunk 1 would remove it before it was
    // copied. The check before each chunk's expunge refuses instead, and nothing is lost.
    let server = server_with(Caps::bare()).await;
    let mut uids = Vec::new();
    for i in 0..501 {
        uids.push(server.add_message("INBOX", &msg(&format!("m{i}@x"))));
    }
    let last = *uids.last().unwrap();
    server.modify_flags("INBOX", last, &["\\Deleted"], &[]);
    let b = backend(&server);
    let err = b.move_to("INBOX", &uids, "Trash").await.unwrap_err();
    assert!(matches!(err, BackendError::Permanent(_)), "{err:?}");
    // Nothing was expunged from the source.
    assert_eq!(server.messages("INBOX").len(), 501);
    // The chunk's own \Deleted marks were taken off again.
    let marked = server
        .messages("INBOX")
        .iter()
        .filter(|m| m.flags.contains("\\Deleted"))
        .count();
    assert_eq!(marked, 1, "only the other client's mark is left");
}
