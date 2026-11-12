// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! `fetch_body` against real servers in containers: a message with an attachment is put on the
//! server, fetched whole, parsed with `croft-reader`, and its attachment comes back byte for byte;
//! the message is still unread on the server afterwards. GreenMail (plain IMAP on localhost),
//! Dovecot (IMAPS with the pinned test certificate) and Stalwart (JMAP).
//!
//! Needs `--features integration` and the servers (see `tests/integration.rs` and
//! `tests/jmap_integration.rs`; the same `CROFT_TEST_*_PORT` variables move the ports).
#![cfg(feature = "integration")]
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use common::rawjmap::RawJmap;
use croft_protocols::*;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// The attachment's bytes (made up), and their base64.
const PDF: &[u8] = b"%PDF-1.4 Croft integration test attachment, made up.\n";
const PDF_BASE64: &str = "JVBERi0xLjQgQ3JvZnQgaW50ZWdyYXRpb24gdGVzdCBhdHRhY2htZW50LCBtYWRlIHVwLgo=";

fn port(variable: &str, default: u16) -> u16 {
    std::env::var(variable)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

fn unique(prefix: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    format!("{prefix}{nanos}")
}

fn with_attachment(id: &str, to: &str) -> String {
    format!(
        "From: Ann Example <ann@example.org>\r\nTo: {to}\r\nSubject: Quarterly report\r\n\
         Date: Mon, 02 Mar 2026 09:30:00 +0000\r\nMessage-ID: <{id}>\r\nMIME-Version: 1.0\r\n\
         Content-Type: multipart/mixed; boundary=\"b1\"\r\n\r\n--b1\r\n\
         Content-Type: multipart/alternative; boundary=\"b2\"\r\n\r\n--b2\r\n\
         Content-Type: text/plain; charset=utf-8\r\n\r\nThe report is attached.\r\n--b2\r\n\
         Content-Type: text/html; charset=utf-8\r\n\r\n<p>The report is <b>attached</b>.\
         <img src=\"https://img.example.org/chart.png\"></p>\r\n--b2--\r\n--b1\r\n\
         Content-Type: application/pdf; name=\"report.pdf\"\r\nContent-Transfer-Encoding: base64\r\n\
         Content-Disposition: attachment; filename=\"report.pdf\"\r\n\r\n{PDF_BASE64}\r\n--b1--\r\n"
    )
}

/// Parses what the server sent and checks the body and the attachment.
fn check(label: &str, raw: &[u8]) {
    let r = croft_reader::render(raw, false).unwrap();
    assert!(
        r.text
            .as_deref()
            .unwrap()
            .contains("The report is attached."),
        "{label}"
    );
    let html = r.html.unwrap();
    assert!(html.contains("<b>attached</b>"), "{label}: {html}");
    assert_eq!(r.blocked_remote, 1, "{label}");
    assert_eq!(r.attachments.len(), 1, "{label}");
    let a = &r.attachments[0];
    assert_eq!(a.file_name, "report.pdf", "{label}");
    assert_eq!(a.mime_type, "application/pdf", "{label}");
    let bytes = croft_reader::part_content(raw, &a.part_id)
        .unwrap()
        .unwrap();
    assert_eq!(bytes, PDF, "{label}: the attachment round-trips");
}

async fn imap_case(label: &str, config: ImapConfig, password: &str) {
    let b = ImapBackend::new(
        config.with_timeout(Duration::from_secs(20)),
        Credential::password(password),
    )
    .unwrap();
    let id = unique("body-");
    let appended = b
        .append_message("INBOX", &[], &with_attachment(&id, "test@localhost"))
        .await
        .unwrap();
    let uid = match appended {
        Some(uid) => uid,
        None => b
            .find_by_message_id("INBOX", &format!("<{id}>"))
            .await
            .unwrap()
            .unwrap(),
    };
    let RawMessage::Found(raw) = b.fetch_body("INBOX", uid).await.unwrap() else {
        panic!("{label}: the message is there")
    };
    check(label, &raw);
    let snap = b.snapshot("INBOX").await.unwrap();
    assert!(!snap.messages[&uid].seen, "{label}: still unread");
    b.delete_forever("INBOX", &[uid]).await.unwrap();
    assert_eq!(
        b.fetch_body("INBOX", uid).await.unwrap(),
        RawMessage::Gone,
        "{label}"
    );
}

#[tokio::test]
async fn greenmail_fetches_a_body_and_its_attachment_without_marking_it_read() {
    let config = ImapConfig::new(
        "127.0.0.1",
        port("CROFT_TEST_GREENMAIL_IMAP_PORT", 3143),
        Security::PlaintextForTestsOnly,
        "test",
    );
    imap_case("greenmail", config, "test").await;
}

#[tokio::test]
async fn dovecot_fetches_a_body_and_its_attachment_without_marking_it_read() {
    use base64::Engine;
    let pem = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../test-servers/dovecot/ssl/tls.crt"
    ))
    .unwrap();
    let b64: String = pem.lines().filter(|l| !l.starts_with("-----")).collect();
    let der = base64::engine::general_purpose::STANDARD
        .decode(b64)
        .unwrap();
    let config = ImapConfig::new(
        "localhost",
        port("CROFT_TEST_DOVECOT_IMAPS_PORT", 31993),
        Security::Tls,
        unique("body"),
    )
    .with_pinned_cert(CertFingerprint::of_der(&der));
    imap_case("dovecot", config, "testpass").await;
}

#[tokio::test]
async fn stalwart_downloads_a_body_and_its_attachment_without_marking_it_read() {
    const BOB: &str = "bob@example.org";
    const BOB_PW: &str = "croft-bob-yellow-lantern-5530";
    let http = port("CROFT_TEST_STALWART_HTTP_PORT", 8080);
    let raw_client = RawJmap::connect(http, BOB, BOB_PW).await;
    let id = unique("body-");
    raw_client
        .import("inbox", &with_attachment(&id, BOB), &[])
        .await;
    let b = JmapBackend::new(
        JmapConfig::new(format!("http://127.0.0.1:{http}/jmap/session"), BOB)
            .with_timeout(Duration::from_secs(20)),
        Credential::password(BOB_PW),
        Arc::new(MemoryIdStore::new()),
    )
    .unwrap();
    let inbox = b
        .list_folders()
        .await
        .unwrap()
        .into_iter()
        .find(|f| f.special_use == SpecialUse::Inbox)
        .unwrap()
        .path;
    let uid = b
        .find_by_message_id(&inbox, &format!("<{id}>"))
        .await
        .unwrap()
        .expect("the imported message");
    let RawMessage::Found(raw) = b.fetch_body(&inbox, uid).await.unwrap() else {
        panic!("stalwart: the message is there")
    };
    check("stalwart", &raw);
    let snap = b.snapshot(&inbox).await.unwrap();
    assert!(!snap.messages[&uid].seen, "stalwart: still unread");
}
