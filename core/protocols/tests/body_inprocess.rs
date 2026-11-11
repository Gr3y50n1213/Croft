// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! `MailBackend::fetch_body` against the in-process IMAP and JMAP servers: the commands the IMAP
//! adapter sends (read-only folder, `BODY.PEEK[]`), the size limit on both adapters (nothing is
//! downloaded past it), messages that are gone, and the sample mailbox. The shared behaviour test
//! (`common/behaviour.rs`) covers the normal fetch on every adapter, Stalwart included.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use croft_protocols::*;
use croft_test_support::{JmapServer, TestServer, JMAP_PASSWORD, JMAP_USER};
use std::sync::Arc;
use std::time::Duration;

fn message(id: &str, body: &str) -> String {
    format!(
        "From: Ann Example <ann@example.org>\r\nTo: Test <test@localhost>\r\nSubject: Hello\r\n\
         Date: Mon, 02 Mar 2026 09:30:00 +0000\r\nMessage-ID: <{id}>\r\n\r\n{body}\r\n"
    )
}

fn imap(server: &TestServer) -> ImapBackend {
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

fn jmap(server: &JmapServer) -> JmapBackend {
    JmapBackend::new(
        JmapConfig::new(server.session_url(), JMAP_USER).with_timeout(Duration::from_secs(5)),
        Credential::password(JMAP_PASSWORD),
        Arc::new(MemoryIdStore::new()),
    )
    .unwrap()
}

/// A body just over the limit (the headers make it bigger still).
fn too_big_body() -> String {
    "x".repeat(usize::try_from(MAX_MESSAGE_BYTES).unwrap() + 1)
}

#[tokio::test]
async fn imap_reads_the_size_first_then_peeks_on_a_read_only_folder() {
    let server = TestServer::start_default().await;
    let uid = server.add_message("INBOX", &message("peek@example.org", "Hello there."));
    let b = imap(&server);
    server.clear_commands();
    let RawMessage::Found(raw) = b.fetch_body("INBOX", uid).await.unwrap() else {
        panic!("the message is there")
    };
    assert_eq!(
        raw,
        message("peek@example.org", "Hello there.").into_bytes()
    );
    let commands = server.commands().join("\n");
    assert!(commands.contains("EXAMINE"), "{commands}");
    assert!(commands.contains("RFC822.SIZE"), "{commands}");
    assert!(commands.contains("BODY.PEEK[]"), "{commands}");
    assert!(!commands.contains(" BODY[]"), "{commands}");
    assert!(!commands.contains("STORE"), "{commands}");
    // Still unread on the server.
    let stored = server.messages("INBOX");
    assert!(!stored[0].flags.contains("\\Seen"));
}

#[tokio::test]
async fn imap_refuses_a_message_over_the_limit_without_downloading_it() {
    let server = TestServer::start_default().await;
    let uid = server.add_message("INBOX", &message("big@example.org", &too_big_body()));
    let b = imap(&server);
    server.clear_commands();
    match b.fetch_body("INBOX", uid).await.unwrap() {
        RawMessage::TooLarge { size } => assert!(size > MAX_MESSAGE_BYTES),
        other => panic!("{other:?}"),
    }
    let commands = server.commands().join("\n");
    assert!(!commands.contains("BODY"), "nothing downloaded: {commands}");
}

#[tokio::test]
async fn imap_reports_a_message_removed_by_another_client_as_gone() {
    let server = TestServer::start_default().await;
    let uid = server.add_message("INBOX", &message("gone@example.org", "Bye."));
    let b = imap(&server);
    server.remove_messages("INBOX", &[uid]);
    assert_eq!(b.fetch_body("INBOX", uid).await.unwrap(), RawMessage::Gone);
}

#[tokio::test]
async fn jmap_downloads_the_blob_and_never_changes_seen() {
    let server = JmapServer::start().await;
    server.add_message("inbox", &message("jmap@example.org", "Hi."), &[]);
    let b = jmap(&server);
    let snap = b.snapshot("Inbox").await.unwrap();
    let (&uid, flags) = snap.messages.iter().next().unwrap();
    assert!(!flags.seen);
    let RawMessage::Found(raw) = b.fetch_body("Inbox", uid).await.unwrap() else {
        panic!("the message is there")
    };
    assert_eq!(raw, message("jmap@example.org", "Hi.").into_bytes());
    assert!(!b.snapshot("Inbox").await.unwrap().messages[&uid].seen);
    // The download went to the session's own origin, signed in.
    assert!(server
        .requests()
        .iter()
        .any(|r| r.path.starts_with("/jmap/download/") && r.had_authorization));
}

#[tokio::test]
async fn jmap_refuses_a_message_over_the_limit_without_downloading_it() {
    let server = JmapServer::start().await;
    server.add_message("inbox", &message("big@example.org", &too_big_body()), &[]);
    let b = jmap(&server);
    let snap = b.snapshot("Inbox").await.unwrap();
    let uid = *snap.messages.keys().next().unwrap();
    match b.fetch_body("Inbox", uid).await.unwrap() {
        RawMessage::TooLarge { size } => assert!(size > MAX_MESSAGE_BYTES),
        other => panic!("{other:?}"),
    }
    assert!(!server
        .requests()
        .iter()
        .any(|r| r.path.starts_with("/jmap/download/")));
}

#[tokio::test]
async fn the_sample_mailbox_serves_its_messages_with_attachments_and_images() {
    let demo = DemoBackend::new(1_772_443_800_000);
    let mut found_attachment = false;
    let mut found_remote_image = false;
    for folder in demo.list_folders().await.unwrap() {
        let snap = demo.snapshot(&folder.path).await.unwrap();
        for uid in snap.messages.keys() {
            let RawMessage::Found(raw) = demo.fetch_body(&folder.path, *uid).await.unwrap() else {
                panic!("every sample message has a body")
            };
            let text = String::from_utf8_lossy(&raw).to_lowercase();
            found_attachment |= text.contains("content-disposition: attachment");
            found_remote_image |= text.contains("<img") && text.contains("https://");
        }
    }
    assert!(found_attachment && found_remote_image);
    assert_eq!(
        demo.fetch_body(demo::INBOX, 999_999).await.unwrap(),
        RawMessage::Gone
    );
}
