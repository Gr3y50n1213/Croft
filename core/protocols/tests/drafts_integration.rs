// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The draft copy (`croft_protocols::drafts`) and the copy in Sent (`file_copy_in_sent`) against
//! GreenMail in a container. Only compiles with `--features integration`:
//!
//! ```text
//! docker compose -f test-servers/docker-compose.yml up -d --wait greenmail
//! cd core && cargo test -p croft-protocols --features integration --test drafts_integration
//! docker compose -f test-servers/docker-compose.yml down -v
//! ```
//!
//! IMAP is on `CROFT_GREENMAIL_IMAP_PORT` (default 3143), user `alice` / `alice`. GreenMail starts
//! with an INBOX only, so these also cover creating the Drafts and Sent folders.
#![cfg(feature = "integration")]
#![allow(clippy::unwrap_used, clippy::expect_used)]

use croft_protocols::drafts::{delete_draft_copies, drafts_folder, save_draft_copy};
use croft_protocols::submit::file_copy_in_sent;
use croft_protocols::*;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

fn imap() -> ImapBackend {
    let port = std::env::var("CROFT_GREENMAIL_IMAP_PORT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(3143);
    ImapBackend::new(
        ImapConfig::new("127.0.0.1", port, Security::PlaintextForTestsOnly, "alice")
            .with_timeout(Duration::from_secs(20)),
        Credential::password("alice"),
    )
    .unwrap()
}

fn unique(prefix: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    format!("{prefix}{nanos}@example.org")
}

fn message(id: &str, body: &str) -> Vec<u8> {
    format!(
        "From: alice@localhost\r\nTo: test@localhost\r\nSubject: A draft\r\n\
         Date: Mon, 02 Mar 2026 09:30:00 +0000\r\nMessage-ID: <{id}>\r\n\r\n{body}\r\n"
    )
    .into_bytes()
}

async fn copies(b: &ImapBackend, folder: &str, id: &str) -> usize {
    let snapshot = b.snapshot(folder).await.unwrap();
    let uids: Vec<Uid> = snapshot.messages.keys().copied().collect();
    if uids.is_empty() {
        return 0;
    }
    b.fetch_headers(folder, &uids)
        .await
        .unwrap()
        .iter()
        .filter(|h| h.message_id.as_deref() == Some(&format!("<{id}>")))
        .count()
}

#[tokio::test]
async fn a_draft_is_appended_then_replaced_then_deleted() {
    let b = imap();
    let folder = drafts_folder(&b, None).await.unwrap();
    let id = unique("draft-");
    let mid = format!("<{id}>");

    let first = save_draft_copy(&b, &folder, &mid, &message(&id, "one"))
        .await
        .unwrap();
    assert!(first.uid.is_some());
    assert_eq!(copies(&b, &folder, &id).await, 1);
    let flags = &b.snapshot(&folder).await.unwrap().messages[&first.uid.unwrap()];
    assert!(flags.draft && flags.seen);

    let second = save_draft_copy(&b, &folder, &mid, &message(&id, "two"))
        .await
        .unwrap();
    assert_eq!(copies(&b, &folder, &id).await, 1, "replaced, never two");
    assert_ne!(first.uid, second.uid);
    assert_eq!(
        b.find_by_message_id(&folder, &mid).await.unwrap(),
        second.uid
    );

    assert_eq!(delete_draft_copies(&b, &folder, &mid).await.unwrap(), 1);
    assert_eq!(copies(&b, &folder, &id).await, 0);
    assert_eq!(delete_draft_copies(&b, &folder, &mid).await.unwrap(), 0);
}

#[tokio::test]
async fn the_copy_in_sent_is_made_once() {
    let b = imap();
    let id = unique("sent-");
    let mid = format!("<{id}>");
    let mime = message(&id, "sent body");
    file_copy_in_sent(&b, Some(&mid), &mime).await.unwrap();
    file_copy_in_sent(&b, Some(&mid), &mime).await.unwrap();
    let folders = b.list_folders().await.unwrap();
    let sent = folders
        .iter()
        .find(|f| f.special_use == SpecialUse::Sent || f.path == "Sent")
        .unwrap();
    assert_eq!(copies(&b, &sent.path, &id).await, 1);
}
