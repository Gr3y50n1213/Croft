// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The SMTP sender against GreenMail in a container, with the message read back over IMAP.
//!
//! Only compiles with `--features integration`, and needs GreenMail running:
//!
//! ```text
//! docker compose -f test-servers/docker-compose.yml up -d --wait greenmail
//! cd core && cargo test -p croft-protocols --features integration --test smtp_integration
//! docker compose -f test-servers/docker-compose.yml down -v
//! ```
//!
//! GreenMail's ports on localhost are SMTP 3025 (plaintext, allowed because it is localhost),
//! SMTPS 3465 (self-signed certificate that changes at every start, so the test pins what
//! [`probe_smtp_certificate`] reads, as the account-setup screen would) and IMAP 3143. A second
//! copy started under another compose project name can use other host ports: set
//! `CROFT_GREENMAIL_SMTP_PORT`, `CROFT_GREENMAIL_SMTPS_PORT` and `CROFT_GREENMAIL_IMAP_PORT`.
//! The users are `test` (password `test`, address `test@localhost`) and `alice` (password
//! `alice`, address `alice@localhost`).
#![cfg(feature = "integration")]
#![allow(clippy::unwrap_used, clippy::expect_used)]

use croft_protocols::*;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const HOST: &str = "127.0.0.1";

fn port(var: &str, default: u16) -> u16 {
    std::env::var(var)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

fn smtp_port() -> u16 {
    port("CROFT_GREENMAIL_SMTP_PORT", 3025)
}

fn smtps_port() -> u16 {
    port("CROFT_GREENMAIL_SMTPS_PORT", 3465)
}

fn imap_port() -> u16 {
    port("CROFT_GREENMAIL_IMAP_PORT", 3143)
}

fn unique(prefix: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    format!("{prefix}{nanos}")
}

fn message(id: &str, to: &str, subject: &str) -> Vec<u8> {
    format!(
        "From: Ann Example <ann@example.org>\r\nTo: {to}\r\nSubject: {subject}\r\n\
         Date: Mon, 02 Mar 2026 09:30:00 +0000\r\nMessage-ID: <{id}>\r\n\r\n\
         Body of {id}.\r\n.leading dot line\r\n.\r\nLast line.\r\n"
    )
    .into_bytes()
}

fn plain(user: &str, password: &str) -> SmtpSender {
    SmtpSender::new(
        SmtpConfig::new(HOST, smtp_port(), SmtpSecurity::PlaintextForTestsOnly, user)
            .with_timeout(Duration::from_secs(20)),
        Credential::password(password),
    )
    .unwrap()
}

fn imap(user: &str, password: &str) -> ImapBackend {
    ImapBackend::new(
        ImapConfig::new(HOST, imap_port(), Security::PlaintextForTestsOnly, user)
            .with_timeout(Duration::from_secs(20)),
        Credential::password(password),
    )
    .unwrap()
}

/// Waits up to ten seconds for the message to show up in the INBOX, then returns its header.
async fn find_in_inbox(backend: &ImapBackend, id: &str) -> RemoteHeader {
    let wanted = format!("<{id}>");
    for _ in 0..40 {
        if let Some(uid) = backend.find_by_message_id("INBOX", &wanted).await.unwrap() {
            let headers = backend.fetch_headers("INBOX", &[uid]).await.unwrap();
            return headers.into_iter().next().unwrap();
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    panic!("{wanted} never arrived in the INBOX");
}

#[tokio::test]
async fn a_message_sent_over_plain_smtp_is_readable_over_imap() {
    let id = unique("t06-plain-");
    let subject = format!("Subject {id}");
    let receipt = plain("test", "test")
        .send(
            "ann@example.org",
            &["test@localhost"],
            &message(&id, "test@localhost", &subject),
        )
        .await
        .unwrap();
    assert_eq!(receipt.recipients, 1);
    let header = find_in_inbox(&imap("test", "test"), &id).await;
    assert_eq!(header.subject, subject);
    assert_eq!(header.from_addr.as_deref(), Some("ann@example.org"));
}

#[tokio::test]
async fn the_envelope_decides_who_gets_it_not_the_headers() {
    // alice is in the envelope only: a Bcc recipient.
    let id = unique("t06-bcc-");
    plain("test", "test")
        .send(
            "ann@example.org",
            &["test@localhost", "alice@localhost"],
            &message(&id, "test@localhost", "Bcc test"),
        )
        .await
        .unwrap();
    find_in_inbox(&imap("test", "test"), &id).await;
    find_in_inbox(&imap("alice", "alice"), &id).await;
}

#[tokio::test]
async fn a_body_with_a_line_that_is_just_a_dot_is_not_cut_short() {
    let id = unique("t06-dot-");
    let sent = message(&id, "test@localhost", "Dots");
    plain("test", "test")
        .send("ann@example.org", &["test@localhost"], &sent)
        .await
        .unwrap();
    let backend = imap("test", "test");
    let header = find_in_inbox(&backend, &id).await;
    // The server adds its own `Received` header, so the stored message is a little longer than
    // what was sent, never shorter: a lone "." line that was not dot-stuffed would have ended the
    // message there and left out everything after it.
    let size = header.size as usize;
    assert!(
        size >= sent.len() && size < sent.len() + 600,
        "sent {} bytes, stored {size}",
        sent.len()
    );
}

#[tokio::test]
async fn a_wrong_password_is_an_auth_error() {
    let err = plain("test", "not-the-password")
        .send(
            "ann@example.org",
            &["test@localhost"],
            &message("wrong-password", "test@localhost", "x"),
        )
        .await
        .unwrap_err();
    assert!(matches!(err, BackendError::Auth(_)), "{err:?}");
}

#[tokio::test]
async fn smtps_with_the_pinned_certificate_sends_and_without_the_pin_is_refused() {
    let cfg = SmtpConfig::new(HOST, smtps_port(), SmtpSecurity::Tls, "test")
        .with_timeout(Duration::from_secs(20));
    // Not pinned, not signed by a trusted authority: refused before any credential is sent.
    let unpinned = SmtpSender::new(cfg.clone(), Credential::password("test")).unwrap();
    let id = unique("t06-tls-");
    let err = unpinned
        .send(
            "ann@example.org",
            &["test@localhost"],
            &message(&id, "test@localhost", "tls"),
        )
        .await
        .unwrap_err();
    assert!(matches!(err, BackendError::Auth(_)), "{err:?}");

    // The setup screen reads the fingerprint, the person confirms it, and it is pinned.
    let fingerprint = probe_smtp_certificate(&cfg).await.unwrap();
    let pinned = SmtpSender::new(
        cfg.clone().with_pinned_cert(fingerprint),
        Credential::password("test"),
    )
    .unwrap();
    pinned
        .send(
            "ann@example.org",
            &["test@localhost"],
            &message(&id, "test@localhost", "tls"),
        )
        .await
        .unwrap();
    find_in_inbox(&imap("test", "test"), &id).await;

    // A pin for another certificate is refused the same way.
    let wrong = SmtpSender::new(
        cfg.with_pinned_cert(CertFingerprint::of_der(b"not this server")),
        Credential::password("test"),
    )
    .unwrap();
    let err = wrong
        .send(
            "ann@example.org",
            &["test@localhost"],
            &message("wrong-pin", "test@localhost", "x"),
        )
        .await
        .unwrap_err();
    assert!(matches!(err, BackendError::Auth(_)), "{err:?}");
}

#[tokio::test]
async fn nothing_listening_is_transient() {
    let port = {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap().port()
    };
    let sender = SmtpSender::new(
        SmtpConfig::new(HOST, port, SmtpSecurity::PlaintextForTestsOnly, "test")
            .with_timeout(Duration::from_secs(3)),
        Credential::password("test"),
    )
    .unwrap();
    let err = sender
        .send(
            "ann@example.org",
            &["test@localhost"],
            &message("nobody-home", "test@localhost", "x"),
        )
        .await
        .unwrap_err();
    assert!(matches!(err, BackendError::Transient(_)), "{err:?}");
}
