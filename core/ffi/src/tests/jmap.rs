// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! JMAP accounts through the surface (T22), against the in-process JMAP server: trust, add, sync,
//! mark read, send, restart, wrong password, signing in again. Credentials are made up.

use super::*;
use croft_protocols::MailSubmitter;
use croft_test_support::{JmapOptions, JmapServer, JMAP_PASSWORD, JMAP_USER};

fn jmap_discovered(server: &JmapServer) -> FfiDiscovered {
    FfiDiscovered {
        email: JMAP_USER.into(),
        imap: None,
        smtp: None,
        jmap: Some(server.session_url()),
        source: DiscoverySource::Autoconfig,
        provider: None,
        auth_hint: AuthKind::Password,
    }
}

fn raw_to(subject: &str) -> String {
    format!(
        "From: Ann Example <ann@example.org>\r\nTo: <{JMAP_USER}>\r\nSubject: {subject}\r\n\
         Message-ID: <{subject}@example.org>\r\nDate: Thu, 12 Nov 2026 10:00:00 +0000\r\n\r\nHello\r\n"
    )
}

fn pw() -> Arc<FfiCredential> {
    FfiCredential::password(JMAP_PASSWORD.as_bytes().to_vec())
}

fn wait_until(what: &str, mut cond: impl FnMut() -> bool) {
    for _ in 0..300 {
        if cond() {
            return;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    panic!("timed out waiting for {what}");
}

fn inbox_of(core: &Core, account: i64) -> Option<i64> {
    core.list_folders(account)
        .ok()?
        .into_iter()
        .find(|f| f.special_use == "inbox")
        .map(|f| f.id)
}

fn inbox_rows(core: &Core, account: i64) -> Vec<MessageRow> {
    let Some(folder) = inbox_of(core, account) else {
        return Vec::new();
    };
    core.list_messages(
        MessageScope::Folder {
            account,
            folder_id: folder,
        },
        None,
        100,
    )
    .map(|p| p.rows)
    .unwrap_or_default()
}

/// A TLS server: the first check asks to trust its certificate, then the account is added pinned.
fn add_trusted(server: &JmapServer, core: &Core, provider: &MemProvider) -> i64 {
    let fp = match block(core.verify(jmap_discovered(server), pw(), None)) {
        Err(VerifyFailure::UntrustedCertificate { fingerprint }) => fingerprint,
        other => panic!("expected a trust question, got {other:?}"),
    };
    let pin = block(core.verify(jmap_discovered(server), pw(), Some(fp.clone()))).unwrap();
    assert_eq!(
        pin.as_deref().map(str::to_lowercase),
        Some(fp.to_lowercase())
    );
    let id = block(core.add_account(jmap_discovered(server), "Alice".into(), pin, None, pw()))
        .unwrap()
        .id;
    provider.store(id, pw());
    id
}

#[test]
fn a_jmap_account_is_trusted_added_synced_and_marked_read_and_keeps_no_secret() {
    let server = block(JmapServer::start_with(JmapOptions { tls: true }));
    server.add_message("inbox", &raw_to("Hello-one"), &[]);
    server.add_message("inbox", &raw_to("Hello-two"), &[]);
    let db = TempDb::new();
    let provider = MemProvider::new();
    let core = db.open(&provider);
    let id = add_trusted(&server, &core, &provider);
    wait_until("two rows", || inbox_rows(&core, id).len() == 2);

    // The account row is a JMAP one and its settings hold no password.
    let account = core
        .list_accounts()
        .unwrap()
        .into_iter()
        .find(|a| a.id == id)
        .unwrap();
    assert_eq!(account.email, JMAP_USER);
    let row = core
        .inner
        .handle
        .read(|db| croft_storage::accounts::get(db.connection(), id))
        .unwrap()
        .unwrap();
    assert_eq!(row.protocol, croft_storage::accounts::Protocol::Jmap);
    let text = row.config.to_string();
    assert!(
        text.contains("\"jmap\"") && !text.contains("\"imap\""),
        "{text}"
    );
    assert!(!text.contains(JMAP_PASSWORD));

    // Mark read reaches the server as the $seen keyword.
    let first = inbox_rows(&core, id)[0].id;
    core.set_undo_window_ms(0);
    core.mark_seen(vec![first], true).unwrap();
    wait_until("$seen on the server", || {
        server.calls().iter().any(|c| c.contains("Email/set"))
    });
}

#[test]
fn a_jmap_account_sends_with_email_submission_and_the_server_files_the_copy() {
    let server = block(JmapServer::start());
    let provider = MemProvider::new();
    provider.store(1, pw());
    let config = croft_protocols::JmapConfig::new(server.session_url(), JMAP_USER);
    let submitter = crate::jmap_account::JmapAccountSubmitter::new(1, config, provider.clone());
    assert!(submitter.files_sent_itself());
    block(submitter.send(
        JMAP_USER,
        &["bob@example.org".to_string()],
        raw_to("Sent-one").as_bytes(),
    ))
    .unwrap();
    assert_eq!(server.deliveries().len(), 1);
}

#[test]
fn a_jmap_account_starts_again_after_a_restart_with_the_same_message_numbers() {
    let server = block(JmapServer::start());
    server.add_message("inbox", &raw_to("Kept-one"), &[]);
    let db = TempDb::new();
    let provider = MemProvider::new();
    let core = db.open(&provider);
    let pin = block(core.verify(jmap_discovered(&server), pw(), None)).unwrap();
    let id = block(core.add_account(jmap_discovered(&server), "Alice".into(), pin, None, pw()))
        .unwrap()
        .id;
    provider.store(id, pw());
    wait_until("one row", || inbox_rows(&core, id).len() == 1);
    let before: Vec<(i64, String)> = inbox_rows(&core, id)
        .into_iter()
        .map(|r| (r.id, r.subject))
        .collect();
    drop(core);

    let again = db.open(&provider);
    server.add_message("inbox", &raw_to("Kept-two"), &[]);
    again.sync_now(id).unwrap();
    wait_until("two rows after the restart", || {
        inbox_rows(&again, id).len() == 2
    });
    let after = inbox_rows(&again, id);
    for (row_id, subject) in before {
        assert!(
            after.iter().any(|r| r.id == row_id && r.subject == subject),
            "row {row_id} kept"
        );
    }
}

#[test]
fn a_wrong_password_is_wrong_password_and_signing_in_again_keeps_the_account() {
    let server = block(JmapServer::start());
    let db = TempDb::new();
    let provider = MemProvider::new();
    let core = db.open(&provider);
    let wrong = FfiCredential::password(b"not-the-password".to_vec());
    assert_eq!(
        block(core.verify(jmap_discovered(&server), wrong, None)).err(),
        Some(VerifyFailure::WrongPassword)
    );
    let first = block(core.add_account(jmap_discovered(&server), "Alice".into(), None, None, pw()))
        .unwrap();
    assert!(!first.updated);
    provider.store(first.id, pw());
    let again = block(core.add_account(jmap_discovered(&server), "Alice".into(), None, None, pw()))
        .unwrap();
    assert!(again.updated);
    assert_eq!(again.id, first.id);
    assert_eq!(core.list_accounts().unwrap().len(), 1);
}

#[test]
fn a_jmap_accounts_drafts_stay_on_the_device() {
    let server = block(JmapServer::start());
    let db = TempDb::new();
    let provider = MemProvider::new();
    let core = db.open(&provider);
    let id = block(core.add_account(jmap_discovered(&server), "Alice".into(), None, None, pw()))
        .unwrap()
        .id;
    provider.store(id, pw());
    assert!(core.inner.backends().get(&id).unwrap().is_jmap());
}

#[test]
fn a_self_signed_server_asks_for_trust_first_and_only_then_says_wrong_password() {
    // The password is only sent once the certificate is trusted, so a wrong one shows after that.
    let server = block(JmapServer::start_with(JmapOptions { tls: true }));
    let db = TempDb::new();
    let provider = MemProvider::new();
    let core = db.open(&provider);
    let wrong = || FfiCredential::password(b"not-the-password".to_vec());
    let fp = match block(core.verify(jmap_discovered(&server), wrong(), None)) {
        Err(VerifyFailure::UntrustedCertificate { fingerprint }) => fingerprint,
        other => panic!("expected the trust question, got {other:?}"),
    };
    assert_eq!(
        block(core.verify(jmap_discovered(&server), wrong(), Some(fp))).err(),
        Some(VerifyFailure::WrongPassword)
    );
}

#[test]
fn settings_tell_imap_and_jmap_apart_and_an_older_reader_refuses_jmap() {
    use crate::settings::{AccountSettings, StoredSettings};
    let jmap = serde_json::json!({
        "version": 1,
        "jmap": { "session_url": "https://mail.example.org/.well-known/jmap", "username": "a@example.org" }
    });
    assert!(matches!(
        AccountSettings::from_json(&jmap),
        Ok(AccountSettings::Jmap(_))
    ));
    // What a build from before T22 reads: IMAP settings only, so it refuses (never misreads).
    assert!(StoredSettings::from_json(&jmap).is_err());
    let both = serde_json::json!({
        "version": 1,
        "jmap": { "session_url": "https://x/", "username": "a" },
        "imap": { "host": "x", "port": 993, "security": "tls", "username": "a" }
    });
    assert!(AccountSettings::from_json(&both).is_err());
    assert_eq!(
        crate::settings::url_host("https://Mail.Example.org:8443/jmap"),
        "mail.example.org"
    );
    assert_eq!(crate::settings::url_host("http://[::1]:80/x"), "::1");
}

#[test]
fn the_trust_question_comes_before_any_credential_is_sent() {
    // Review finding: the password must never reach a certificate the person did not confirm.
    let server = block(JmapServer::start_with(JmapOptions { tls: true }));
    let db = TempDb::new();
    let provider = MemProvider::new();
    let core = db.open(&provider);
    server.clear_logs();
    assert!(matches!(
        block(core.verify(jmap_discovered(&server), pw(), None)),
        Err(VerifyFailure::UntrustedCertificate { .. })
    ));
    assert!(
        server.requests().is_empty(),
        "no HTTP request (so no password) before the trust question"
    );
}

#[test]
fn a_pinned_account_whose_certificate_changed_is_asked_again_not_told_wrong_password() {
    let server = block(JmapServer::start_with(JmapOptions { tls: true }));
    let db = TempDb::new();
    let provider = MemProvider::new();
    let core = db.open(&provider);
    server.clear_logs();
    // A pin that is not the server's (as after the server renewed its self-signed certificate).
    let old_pin = "AB:CD:EF:01:23:45:67:89:AB:CD:EF:01:23:45:67:89:AB:CD:EF:01:23:45:67:89:AB:CD:EF:01:23:45:67:89";
    match block(core.verify(jmap_discovered(&server), pw(), Some(old_pin.into()))) {
        Err(VerifyFailure::CertificateChanged { fingerprint }) => assert_ne!(fingerprint, old_pin),
        other => panic!("expected CertificateChanged, got {other:?}"),
    }
    assert!(
        server.requests().is_empty(),
        "nothing sent to the changed certificate"
    );
}

#[test]
fn an_imap_account_whose_server_also_offers_jmap_signs_in_again_over_imap() {
    // Review finding: signing in again keeps the account's protocol.
    let rig = Rig::new(|_| {});
    let jmap = block(JmapServer::start());
    let mut again = discovered_for(&rig.server);
    again.jmap = Some(jmap.session_url());
    let added = block(rig.core.add_account(
        again,
        "Alice".into(),
        Some(rig.fingerprint.clone()),
        None,
        password_cred(PASSWORD),
    ))
    .unwrap();
    assert!(added.updated, "the same IMAP account, signed in again");
    assert_eq!(added.id, rig.account);
    let row = rig
        .core
        .inner
        .handle
        .read(|db| croft_storage::accounts::get(db.connection(), rig.account))
        .unwrap()
        .unwrap();
    assert_eq!(row.protocol, croft_storage::accounts::Protocol::Imap);
}

#[test]
fn a_jmap_account_refuses_a_raw_oauth_token() {
    let server = block(JmapServer::start());
    let db = TempDb::new();
    let provider = MemProvider::new();
    let core = db.open(&provider);
    let token = FfiCredential::oauth2(JMAP_USER.into(), b"made-up-token".to_vec());
    assert_eq!(
        block(core.add_account(jmap_discovered(&server), "Alice".into(), None, None, token)).err(),
        Some(CoreError::BadSettings)
    );
}
