// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Adapter tests against real servers in containers: GreenMail and Dovecot.
//!
//! These only compile with `--features integration`, and they need the servers running:
//!
//! ```text
//! docker compose -f test-servers/docker-compose.yml up -d greenmail dovecot
//! cd core && cargo test -p croft-protocols --features integration
//! docker compose -f test-servers/docker-compose.yml down
//! ```
//!
//! Ports on localhost: GreenMail IMAP 3143 (plaintext, allowed because it is localhost) and IMAPS
//! 3993; Dovecot IMAP 31143 (STARTTLS) and IMAPS 31993. Both TLS ports use self-signed
//! certificates, so these tests also prove the pinned-fingerprint path: GreenMail's certificate
//! changes at every start (read with `probe_certificate`, as the account-setup screen would),
//! Dovecot's is the fixed test certificate in `test-servers/dovecot/ssl/tls.crt`, which the test
//! reads from disk, so a pin on a known certificate is checked as well.
#![cfg(feature = "integration")]
#![allow(clippy::unwrap_used, clippy::expect_used)]

use croft_protocols::*;
use std::collections::BTreeSet;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const GREENMAIL_HOST: &str = "127.0.0.1";

fn unique(prefix: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    format!("{prefix}{nanos}")
}

fn message(id: &str, subject: &str, extra_headers: &str) -> String {
    format!(
        "From: Ann Example <ann@example.org>\r\nTo: Test <test@localhost>\r\nSubject: {subject}\r\n\
         Date: Mon, 02 Mar 2026 09:30:00 +0000\r\nMessage-ID: <{id}>\r\n{extra_headers}\r\n\
         Body of {id}.\r\n"
    )
}

/// SHA-256 fingerprint of the PEM certificate at `path` (the Dovecot test certificate).
fn fingerprint_of_pem(path: &str) -> CertFingerprint {
    use base64::Engine;
    let pem = std::fs::read_to_string(path).unwrap();
    let b64: String = pem.lines().filter(|l| !l.starts_with("-----")).collect();
    let der = base64::engine::general_purpose::STANDARD
        .decode(b64)
        .unwrap();
    CertFingerprint::of_der(&der)
}

fn dovecot_cert() -> CertFingerprint {
    fingerprint_of_pem(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../test-servers/dovecot/ssl/tls.crt"
    ))
}

struct Target {
    label: &'static str,
    config: ImapConfig,
    password: String,
    /// The server is expected to tell which folders are Trash etc. with attributes.
    has_special_use: bool,
    has_condstore: bool,
    /// Folders to create first (GreenMail starts with INBOX only).
    create: Vec<&'static str>,
}

impl Target {
    fn backend(&self) -> ImapBackend {
        ImapBackend::new(
            self.config.clone().with_timeout(Duration::from_secs(20)),
            Credential::Password(self.password.clone()),
        )
        .unwrap()
    }
}

async fn empty(b: &ImapBackend, folder: &str) {
    let snap = b.snapshot(folder).await.unwrap();
    let uids: Vec<Uid> = snap.messages.keys().copied().collect();
    if !uids.is_empty() {
        b.delete_forever(folder, &uids).await.unwrap();
    }
    assert!(b.snapshot(folder).await.unwrap().messages.is_empty());
}

/// The whole "Done when" list for one server.
async fn suite(t: Target) {
    let b = t.backend();
    let label = t.label;

    // Capabilities.
    let caps = b.capabilities().await.unwrap();
    println!("{label}: {caps:?}");
    assert!(caps.move_cmd && caps.uidplus, "{label}: {caps:?}");
    assert_eq!(caps.special_use, t.has_special_use, "{label}");
    assert_eq!(caps.condstore, t.has_condstore, "{label}");

    // List folders.
    for f in &t.create {
        b.create_folder(f).await.unwrap();
        b.create_folder(f).await.unwrap(); // safe to repeat
    }
    let folders = b.list_folders().await.unwrap();
    let kind = |special: SpecialUse| folders.iter().find(|f| f.special_use == special);
    assert!(kind(SpecialUse::Inbox).is_some(), "{label}: {folders:?}");
    let trash =
        kind(SpecialUse::Trash).unwrap_or_else(|| panic!("{label}: no Trash in {folders:?}"));
    let trash_path = trash.path.clone();
    if t.has_special_use {
        for s in [SpecialUse::Sent, SpecialUse::Drafts, SpecialUse::Junk] {
            assert!(kind(s).is_some(), "{label}: missing {s:?} in {folders:?}");
        }
    }
    assert!(folders.iter().all(|f| !f.path.is_empty()));

    // Start clean.
    empty(&b, "INBOX").await;
    empty(&b, &trash_path).await;

    // Seed three messages (the second is a reply, the third arrives already read).
    let ids = [unique("one-"), unique("two-"), unique("three-")];
    let m1 = message(&ids[0], "First \u{2014} caf\u{e9}", "");
    let m1 = m1.replace("\u{2014} caf\u{e9}", "=?UTF-8?Q?=E2=80=94_caf=C3=A9?=");
    let m2 = message(
        &ids[1],
        "Re: First",
        &format!("References: <{}>\r\nIn-Reply-To: <{}>\r\n", ids[0], ids[0]),
    );
    let m3 = message(&ids[2], "Third", "");
    let u1 = b.append_message("INBOX", &[], &m1).await.unwrap();
    let u2 = b.append_message("INBOX", &[], &m2).await.unwrap();
    let u3 = b.append_message("INBOX", &["\\Seen"], &m3).await.unwrap();
    println!("{label}: APPENDUID {u1:?} {u2:?} {u3:?}");

    // Status.
    let status = b.folder_status("INBOX").await.unwrap();
    assert_eq!(status.exists, 3, "{label}");
    assert!(status.uid_validity > 0 && status.uid_next >= 4);
    assert_eq!(status.highest_modseq.is_some(), t.has_condstore, "{label}");

    // Snapshot.
    let snap = b.snapshot("INBOX").await.unwrap();
    assert_eq!(snap.messages.len(), 3);
    assert_eq!(snap.status.uid_validity, status.uid_validity);
    let uids: Vec<Uid> = snap.messages.keys().copied().collect();
    let (uid1, uid2, uid3) = (uids[0], uids[1], uids[2]);
    assert!(!snap.messages[&uid1].seen && !snap.messages[&uid2].seen);
    assert!(snap.messages[&uid3].seen);
    if let (Some(a), Some(c)) = (u1, u3) {
        assert_eq!((a, c), (uid1, uid3));
    }

    // Headers.
    let headers = b.fetch_headers("INBOX", &uids).await.unwrap();
    assert_eq!(headers.len(), 3);
    let h1 = headers.iter().find(|h| h.uid == uid1).unwrap();
    let h2 = headers.iter().find(|h| h.uid == uid2).unwrap();
    assert_eq!(h1.subject, "First \u{2014} caf\u{e9}");
    assert_eq!(
        h1.message_id.as_deref(),
        Some(format!("<{}>", ids[0]).as_str())
    );
    assert_eq!(h1.from_name.as_deref(), Some("Ann Example"));
    assert_eq!(h1.from_addr.as_deref(), Some("ann@example.org"));
    assert_eq!(h1.size as usize, m1.len());
    assert!(
        h1.date_ms > 1_700_000_000_000,
        "{label}: internal date {}",
        h1.date_ms
    );
    assert_eq!(h2.references, vec![format!("<{}>", ids[0])]);
    assert_eq!(h2.subject, "Re: First");
    assert!(
        headers.windows(2).all(|w| w[0].date_ms >= w[1].date_ms),
        "newest first"
    );

    // Find by Message-ID.
    let found = b
        .find_by_message_id("INBOX", &format!("<{}>", ids[1]))
        .await
        .unwrap();
    assert_eq!(found, Some(uid2));
    assert_eq!(
        b.find_by_message_id("INBOX", "<nobody@nowhere>")
            .await
            .unwrap(),
        None
    );

    // Set \Seen (twice: safe to repeat), then clear it.
    b.set_flag("INBOX", &[uid1], FlagChange::Seen(true))
        .await
        .unwrap();
    b.set_flag("INBOX", &[uid1], FlagChange::Seen(true))
        .await
        .unwrap();
    b.set_flag("INBOX", &[uid2], FlagChange::Flagged(true))
        .await
        .unwrap();
    let snap = b.snapshot("INBOX").await.unwrap();
    assert!(snap.messages[&uid1].seen && snap.messages[&uid2].flagged);
    b.set_flag("INBOX", &[uid1], FlagChange::Seen(false))
        .await
        .unwrap();
    assert!(!b.snapshot("INBOX").await.unwrap().messages[&uid1].seen);

    // Another connection changes a flag and removes a message; our next snapshot (a delta read
    // on CONDSTORE/QRESYNC servers) must show the full, correct picture.
    {
        let other = t.backend();
        other
            .set_flag("INBOX", &[uid3], FlagChange::Seen(false))
            .await
            .unwrap();
        other
            .set_flag("INBOX", &[uid1], FlagChange::Seen(true))
            .await
            .unwrap();
        other.close().await;
    }
    let snap = b.snapshot("INBOX").await.unwrap();
    assert_eq!(
        snap.messages.keys().copied().collect::<BTreeSet<_>>(),
        BTreeSet::from([uid1, uid2, uid3])
    );
    assert!(snap.messages[&uid1].seen && !snap.messages[&uid3].seen);

    // Move to Trash.
    let moved = b
        .move_to("INBOX", &[uid1, uid2], &trash_path)
        .await
        .unwrap();
    println!("{label}: move result {moved:?}");
    assert_eq!(moved.dest_uids.len(), 2, "{label}: COPYUID expected");
    assert!(moved.dest_uids.contains_key(&uid1) && moved.dest_uids.contains_key(&uid2));
    let in_inbox = b.snapshot("INBOX").await.unwrap();
    assert_eq!(
        in_inbox.messages.keys().copied().collect::<Vec<_>>(),
        vec![uid3]
    );
    let in_trash = b.snapshot(&trash_path).await.unwrap();
    let new_uids: BTreeSet<Uid> = moved.dest_uids.values().copied().collect();
    assert_eq!(
        in_trash.messages.keys().copied().collect::<BTreeSet<_>>(),
        new_uids
    );
    // The new UID really is that message.
    let moved_headers = b
        .fetch_headers(&trash_path, &[moved.dest_uids[&uid2]])
        .await
        .unwrap();
    assert_eq!(moved_headers[0].subject, "Re: First");
    // Repeating the move: the source UIDs are gone, so nothing is reported and nothing breaks.
    let again = b
        .move_to("INBOX", &[uid1, uid2], &trash_path)
        .await
        .unwrap();
    assert!(again.dest_uids.is_empty());
    assert_eq!(b.snapshot(&trash_path).await.unwrap().messages.len(), 2);
    // Without COPYUID the engine falls back to Message-ID.
    let by_id = b
        .find_by_message_id(&trash_path, &format!("<{}>", ids[0]))
        .await
        .unwrap();
    assert_eq!(by_id, Some(moved.dest_uids[&uid1]));

    // Moving to a folder that does not exist is a permanent refusal.
    let err = b
        .move_to("INBOX", &[uid3], "No such folder 12345")
        .await
        .unwrap_err();
    assert!(
        matches!(err, BackendError::Permanent(_)),
        "{label}: {err:?}"
    );
    assert_eq!(b.snapshot("INBOX").await.unwrap().messages.len(), 1);

    // Delete forever from Trash, one message; the other stays.
    let gone = moved.dest_uids[&uid1];
    let stay = moved.dest_uids[&uid2];
    b.delete_forever(&trash_path, &[gone]).await.unwrap();
    b.delete_forever(&trash_path, &[gone]).await.unwrap(); // safe to repeat
    let left = b.snapshot(&trash_path).await.unwrap();
    assert_eq!(
        left.messages.keys().copied().collect::<Vec<_>>(),
        vec![stay]
    );

    // Clean up and sign out.
    empty(&b, "INBOX").await;
    empty(&b, &trash_path).await;
    b.close().await;
}

async fn wrong_password_is_auth(t: Target) {
    let b = ImapBackend::new(
        t.config.clone().with_timeout(Duration::from_secs(20)),
        Credential::Password("definitely-not-the-password".into()),
    )
    .unwrap();
    for _ in 0..2 {
        match b.list_folders().await {
            Err(BackendError::Auth(m)) => assert!(!m.contains("definitely")),
            other => panic!("{}: expected Auth, got {other:?}", t.label),
        }
    }
}

// ------------------------------------------------------------------------------------------
// GreenMail
// ------------------------------------------------------------------------------------------

fn greenmail_plain(user: &str) -> Target {
    Target {
        label: "greenmail-plain",
        config: ImapConfig::new(GREENMAIL_HOST, 3143, Security::PlaintextForTestsOnly, user),
        password: user.to_string(),
        has_special_use: false,
        has_condstore: false,
        create: vec!["Trash"],
    }
}

async fn greenmail_tls(user: &str) -> Target {
    let probe = ImapConfig::new(GREENMAIL_HOST, 3993, Security::Tls, user);
    // Trust on first use, as the account-setup screen would after the person confirms it.
    let fp = probe_certificate(&probe).await.unwrap();
    Target {
        label: "greenmail-imaps-pinned",
        config: probe.with_pinned_cert(fp),
        password: user.to_string(),
        has_special_use: false,
        has_condstore: false,
        create: vec!["Trash"],
    }
}

#[tokio::test]
async fn greenmail_plaintext_localhost_full_suite() {
    suite(greenmail_plain("test")).await;
}

#[tokio::test]
async fn greenmail_imaps_with_pinned_certificate_full_suite() {
    suite(greenmail_tls("alice").await).await;
}

#[tokio::test]
async fn greenmail_wrong_password_is_auth() {
    wrong_password_is_auth(greenmail_plain("test")).await;
    wrong_password_is_auth(greenmail_tls("test").await).await;
}

#[tokio::test]
async fn greenmail_imaps_refuses_without_a_pin_and_with_the_wrong_pin() {
    let cfg = ImapConfig::new(GREENMAIL_HOST, 3993, Security::Tls, "test")
        .with_timeout(Duration::from_secs(20));
    let none = ImapBackend::new(cfg.clone(), Credential::Password("test".into())).unwrap();
    assert!(matches!(
        none.list_folders().await,
        Err(BackendError::Permanent(_))
    ));
    let wrong = ImapBackend::new(
        cfg.with_pinned_cert(CertFingerprint::of_der(b"not the certificate")),
        Credential::Password("test".into()),
    )
    .unwrap();
    assert!(matches!(
        wrong.list_folders().await,
        Err(BackendError::Permanent(_))
    ));
}

// ------------------------------------------------------------------------------------------
// Dovecot
// ------------------------------------------------------------------------------------------

fn dovecot_imaps(user: &str) -> Target {
    Target {
        label: "dovecot-imaps-pinned",
        config: ImapConfig::new("localhost", 31993, Security::Tls, user)
            .with_pinned_cert(dovecot_cert()),
        password: "testpass".into(),
        has_special_use: true,
        has_condstore: true,
        create: vec![],
    }
}

fn dovecot_starttls(user: &str) -> Target {
    Target {
        label: "dovecot-starttls-pinned",
        config: ImapConfig::new("127.0.0.1", 31143, Security::StartTls, user)
            .with_pinned_cert(dovecot_cert()),
        password: "testpass".into(),
        has_special_use: true,
        has_condstore: true,
        create: vec![],
    }
}

#[tokio::test]
async fn dovecot_imaps_with_pinned_certificate_full_suite() {
    suite(dovecot_imaps(&unique("imaps"))).await;
}

#[tokio::test]
async fn dovecot_starttls_with_pinned_certificate_full_suite() {
    suite(dovecot_starttls(&unique("starttls"))).await;
}

#[tokio::test]
async fn dovecot_wrong_password_is_auth() {
    let user = unique("badpw");
    wrong_password_is_auth(dovecot_imaps(&user)).await;
    wrong_password_is_auth(dovecot_starttls(&user)).await;
}

#[tokio::test]
async fn dovecot_probe_reads_the_known_certificate_and_a_wrong_pin_is_refused() {
    let seen = probe_certificate(&ImapConfig::new("127.0.0.1", 31993, Security::Tls, "u"))
        .await
        .unwrap();
    assert_eq!(seen, dovecot_cert());
    let seen = probe_certificate(&ImapConfig::new(
        "127.0.0.1",
        31143,
        Security::StartTls,
        "u",
    ))
    .await
    .unwrap();
    assert_eq!(seen, dovecot_cert());

    let user = unique("pin");
    let mut cfg = dovecot_imaps(&user).config;
    cfg.pinned_cert = Some(CertFingerprint::of_der(b"someone else"));
    let b = ImapBackend::new(cfg, Credential::Password("testpass".into())).unwrap();
    assert!(matches!(
        b.list_folders().await,
        Err(BackendError::Permanent(_))
    ));

    // Unpinned: the self-signed certificate does not validate against public roots.
    let unpinned = ImapConfig::new("localhost", 31993, Security::Tls, &user);
    let b = ImapBackend::new(unpinned, Credential::Password("testpass".into())).unwrap();
    assert!(matches!(
        b.list_folders().await,
        Err(BackendError::Permanent(_))
    ));
}

#[tokio::test]
async fn dovecot_oauth_token_is_refused_as_auth() {
    // Dovecot's example config does not offer XOAUTH2: the failure must still be Auth.
    let cfg = dovecot_imaps(&unique("oauth")).config;
    let b = ImapBackend::new(
        cfg,
        Credential::OAuth2 {
            user: "someone".into(),
            access_token: "bogus-token".into(),
        },
    )
    .unwrap();
    assert!(matches!(b.list_folders().await, Err(BackendError::Auth(_))));
}
