// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The draft copy on the server (`croft_protocols::drafts`) and the copy in Sent
//! (`file_copy_in_sent`) against the in-process server. No network.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use croft_protocols::drafts::{
    delete_draft_copies, drafts_folder, save_draft_copy, DEFAULT_DRAFTS_FOLDER,
};
use croft_protocols::submit::file_copy_in_sent;
use croft_protocols::*;
use croft_test_support::{Caps, ServerOptions, TestServer};
use std::time::Duration;

fn backend(server: &TestServer) -> ImapBackend {
    ImapBackend::new(
        ImapConfig::new(
            server.host(),
            server.port(),
            Security::PlaintextForTestsOnly,
            server.username(),
        )
        .with_timeout(Duration::from_secs(5)),
        Credential::password(server.password()),
    )
    .unwrap()
}

fn draft(id: &str, body: &str) -> Vec<u8> {
    format!(
        "From: me@example.org\r\nTo: you@example.org\r\nSubject: A draft\r\nMessage-ID: <{id}>\r\n\r\n{body}\r\n"
    )
    .into_bytes()
}

#[tokio::test]
async fn the_first_save_appends_and_the_next_replaces_without_ever_having_none() {
    let server = TestServer::start_default().await;
    let b = backend(&server);
    let folder = drafts_folder(&b, None).await.unwrap();
    assert_eq!(folder, "Drafts");

    let first = save_draft_copy(&b, &folder, "<d1@x>", &draft("d1@x", "one"))
        .await
        .unwrap();
    assert!(first.uid.is_some());
    let stored = server.messages("Drafts");
    assert_eq!(stored.len(), 1);
    assert!(stored[0].flags.contains("\\Draft") && stored[0].flags.contains("\\Seen"));
    assert!(stored[0].raw.contains("one"));

    let second = save_draft_copy(&b, &folder, "<d1@x>", &draft("d1@x", "two"))
        .await
        .unwrap();
    let stored = server.messages("Drafts");
    assert_eq!(stored.len(), 1, "the old copy is gone");
    assert!(stored[0].raw.contains("two"));
    assert_eq!(Some(stored[0].uid), second.uid);
    assert_ne!(first.uid, second.uid);
}

#[tokio::test]
async fn another_draft_with_another_id_is_left_alone() {
    let server = TestServer::start_default().await;
    let b = backend(&server);
    save_draft_copy(&b, "Drafts", "<a@x>", &draft("a@x", "A"))
        .await
        .unwrap();
    save_draft_copy(&b, "Drafts", "<b@x>", &draft("b@x", "B1"))
        .await
        .unwrap();
    save_draft_copy(&b, "Drafts", "<b@x>", &draft("b@x", "B2"))
        .await
        .unwrap();
    let raws: Vec<String> = server
        .messages("Drafts")
        .into_iter()
        .map(|m| m.raw)
        .collect();
    assert_eq!(raws.len(), 2);
    assert!(raws.iter().any(|r| r.contains("\r\nA\r\n")));
    assert!(raws.iter().any(|r| r.contains("B2")) && !raws.iter().any(|r| r.contains("B1")));
}

#[tokio::test]
async fn without_uidplus_the_new_copy_is_still_found_and_the_old_one_deleted() {
    let server = TestServer::start(ServerOptions {
        caps: Caps {
            uidplus: false,
            ..Caps::all()
        },
        ..ServerOptions::default()
    })
    .await;
    let b = backend(&server);
    let first = save_draft_copy(&b, "Drafts", "<d@x>", &draft("d@x", "one"))
        .await
        .unwrap();
    assert!(first.uid.is_some(), "found by Message-ID");
    save_draft_copy(&b, "Drafts", "<d@x>", &draft("d@x", "two"))
        .await
        .unwrap();
    let stored = server.messages("Drafts");
    assert_eq!(stored.len(), 1);
    assert!(stored[0].raw.contains("two"));
}

#[tokio::test]
async fn deleting_removes_every_copy_including_one_left_by_a_crash() {
    let server = TestServer::start_default().await;
    let b = backend(&server);
    // Two copies of one draft (a crash between the append and the delete), and another draft.
    server.add_message_with_flags(
        "Drafts",
        &String::from_utf8(draft("c@x", "old")).unwrap(),
        &["\\Draft"],
    );
    server.add_message_with_flags(
        "Drafts",
        &String::from_utf8(draft("c@x", "new")).unwrap(),
        &["\\Draft"],
    );
    server.add_message_with_flags(
        "Drafts",
        &String::from_utf8(draft("other@x", "keep")).unwrap(),
        &["\\Draft"],
    );
    assert_eq!(delete_draft_copies(&b, "Drafts", "<c@x>").await.unwrap(), 2);
    let stored = server.messages("Drafts");
    assert_eq!(stored.len(), 1);
    assert!(stored[0].raw.contains("keep"));
    // Repeating is harmless.
    assert_eq!(delete_draft_copies(&b, "Drafts", "<c@x>").await.unwrap(), 0);
}

#[tokio::test]
async fn a_save_that_replaces_a_leftover_pair_removes_only_the_newest_old_one_and_cleanup_the_rest()
{
    let server = TestServer::start_default().await;
    let b = backend(&server);
    server.add_message_with_flags(
        "Drafts",
        &String::from_utf8(draft("c@x", "old1")).unwrap(),
        &["\\Draft"],
    );
    server.add_message_with_flags(
        "Drafts",
        &String::from_utf8(draft("c@x", "old2")).unwrap(),
        &["\\Draft"],
    );
    save_draft_copy(&b, "Drafts", "<c@x>", &draft("c@x", "fresh"))
        .await
        .unwrap();
    let raws: Vec<String> = server
        .messages("Drafts")
        .into_iter()
        .map(|m| m.raw)
        .collect();
    assert_eq!(
        raws.len(),
        2,
        "one leftover remains until the final clean-up"
    );
    assert!(raws.iter().any(|r| r.contains("fresh")));
    assert_eq!(delete_draft_copies(&b, "Drafts", "<c@x>").await.unwrap(), 2);
    assert!(server.messages("Drafts").is_empty());
}

#[tokio::test]
async fn the_folder_mapping_wins_then_the_marked_folder_then_a_created_one() {
    let server = TestServer::start_default().await;
    let b = backend(&server);
    assert_eq!(drafts_folder(&b, Some("Mine")).await.unwrap(), "Mine");
    assert_eq!(drafts_folder(&b, Some("")).await.unwrap(), "Drafts");
    server.add_folder("Entwürfe", Some("\\Drafts"));
    // Two marked folders: the first listed one is used; just check it is one of them.
    let found = drafts_folder(&b, None).await.unwrap();
    assert!(found == "Drafts" || found == "Entwürfe");

    // A server with no Drafts folder at all gets one.
    let bare = TestServer::start(ServerOptions {
        caps: Caps::bare(),
        ..ServerOptions::default()
    })
    .await;
    let b2 = backend(&bare);
    let path = drafts_folder(&b2, None).await.unwrap();
    // Without SPECIAL-USE the folder is recognised by its name.
    assert_eq!(path, DEFAULT_DRAFTS_FOLDER);
}

#[tokio::test]
async fn a_draft_that_is_not_text_is_refused_before_anything_is_sent() {
    let server = TestServer::start_default().await;
    let b = backend(&server);
    let err = save_draft_copy(&b, "Drafts", "<x@x>", &[0xff, 0xfe])
        .await
        .unwrap_err();
    assert!(matches!(err, BackendError::Permanent(_)));
    assert!(server.messages("Drafts").is_empty());
}

#[tokio::test]
async fn the_copy_in_sent_is_made_once_even_when_asked_twice() {
    let server = TestServer::start_default().await;
    let b = backend(&server);
    let mime = draft("s@x", "sent body");
    file_copy_in_sent(&b, Some("<s@x>"), &mime).await.unwrap();
    file_copy_in_sent(&b, Some("<s@x>"), &mime).await.unwrap();
    let stored = server.messages("Sent");
    assert_eq!(stored.len(), 1);
    assert!(stored[0].flags.contains("\\Seen"));
    // The draft copy with the same id in Drafts does not count as "already in Sent".
    save_draft_copy(&b, "Drafts", "<t@x>", &draft("t@x", "d"))
        .await
        .unwrap();
    file_copy_in_sent(&b, Some("<t@x>"), &draft("t@x", "d"))
        .await
        .unwrap();
    assert_eq!(server.messages("Sent").len(), 2);
}
