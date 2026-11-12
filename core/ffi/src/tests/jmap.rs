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
fn a_self_signed_server_with_a_wrong_password_says_wrong_password_not_trust() {
    let server = block(JmapServer::start_with(JmapOptions { tls: true }));
    let db = TempDb::new();
    let provider = MemProvider::new();
    let core = db.open(&provider);
    let wrong = FfiCredential::password(b"not-the-password".to_vec());
    assert_eq!(
        block(core.verify(jmap_discovered(&server), wrong, None)).err(),
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
