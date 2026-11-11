// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The surface against the in-process IMAP server (TLS, self-signed, trusted by fingerprint).

use std::time::{Duration, Instant};

use super::*;

/// Waits (up to 20 s) for something on the server side to hold.
fn wait_server(what: &str, mut cond: impl FnMut() -> bool) {
    let end = Instant::now() + Duration::from_secs(20);
    while !cond() {
        assert!(Instant::now() < end, "timed out waiting for: {what}");
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn three_messages(server: &TestServer) {
    for i in 1..=3 {
        server.add_message(
            "INBOX",
            &raw(&format!("m{i}@example.org"), &format!("Hello {i}")),
        );
    }
}

#[test]
fn verify_walks_the_certificate_story_and_reports_typed_failures() {
    let server = start_server();
    let db = TempDb::new();
    let core = db.open(&MemProvider::new());
    let found = discovered_for(&server);
    let good = || password_cred(PASSWORD);

    // 1. No pin: the self-signed certificate is untrusted; the failure carries its fingerprint.
    let err = block(core.verify(found.clone(), good(), None)).unwrap_err();
    assert_eq!(
        err,
        VerifyFailure::UntrustedCertificate {
            fingerprint: fingerprint_of(&server)
        }
    );
    assert_eq!(
        verify_failure_message_key(err.clone()),
        "verify.untrusted_certificate"
    );
    assert!(verify_failure_default_message(err).contains("fingerprint"));

    // 2. Pinned, wrong password.
    let pin = Some(fingerprint_of(&server));
    let err = block(core.verify(
        found.clone(),
        password_cred("not the password"),
        pin.clone(),
    ))
    .unwrap_err();
    assert_eq!(err, VerifyFailure::WrongPassword);
    assert_eq!(verify_failure_message_key(err), "verify.wrong_password");

    // 3. Pinned, right password.
    block(core.verify(found.clone(), good(), pin)).unwrap();

    // 4. A pin of some other certificate.
    let other = CertFingerprint::of_der(b"someone else").to_string();
    let err = block(core.verify(found.clone(), good(), Some(other))).unwrap_err();
    assert!(
        matches!(err, VerifyFailure::CertificateChanged { .. }),
        "{err:?}"
    );

    // 5. Nobody listening.
    let mut dead = found.clone();
    dead.imap.as_mut().unwrap().port = 1;
    assert_eq!(
        block(core.verify(dead, good(), None)).unwrap_err(),
        VerifyFailure::CannotConnect
    );

    // 6. Bad input.
    assert_eq!(
        block(core.verify(found.clone(), good(), Some("zz".into()))).unwrap_err(),
        VerifyFailure::BadSettings
    );
    let jmap = FfiDiscovered {
        imap: None,
        ..found
    };
    assert_eq!(
        block(core.verify(jmap, good(), None)).unwrap_err(),
        VerifyFailure::JmapOnly
    );
}

#[test]
fn add_list_and_remove_an_account_then_list_its_folders_after_the_first_sync() {
    let rig = Rig::new(three_messages);
    let accounts = rig.core.list_accounts().unwrap();
    assert_eq!(accounts.len(), 1);
    assert_eq!(accounts[0].id, rig.account);
    assert_eq!(accounts[0].label, "Alice");
    assert_eq!(accounts[0].email, "alice@example.org");
    // The credential came with `add_account`: the provider was not asked about the new account.
    assert!(rig.provider.asked().is_empty());

    // The first sync brings the folders and the messages.
    rig.wait_for_inbox(3);
    let folders = rig.core.list_folders(rig.account).unwrap();
    let paths: Vec<&str> = folders.iter().map(|f| f.path.as_str()).collect();
    for p in ["INBOX", "Sent", "Drafts", "Trash", "Junk", "Archive"] {
        assert!(paths.contains(&p), "{paths:?}");
    }
    let inbox = folders.iter().find(|f| f.path == "INBOX").unwrap();
    assert_eq!(
        (
            inbox.special_use.as_str(),
            inbox.total_count,
            inbox.unread_count
        ),
        ("inbox", 3, 3)
    );
    assert_eq!(
        rig.core.list_accounts().unwrap()[0].status,
        AccountStatus::Ok
    );

    // The unified Inbox shows the same three rows, newest (highest UID) first.
    let unified = rig
        .core
        .list_messages(MessageScope::UnifiedInbox, None, 50)
        .unwrap();
    let subjects: Vec<&str> = unified.rows.iter().map(|r| r.subject.as_str()).collect();
    assert_eq!(subjects, vec!["Hello 3", "Hello 2", "Hello 1"]);

    // The stored settings hold the pin and no secret.
    let config: String = {
        let d = Db::open(&rig.db.path, None).unwrap();
        d.connection()
            .query_row("SELECT config_json FROM account", [], |r| r.get(0))
            .unwrap()
    };
    assert!(config.contains(&rig.fingerprint));
    assert!(!config.contains(PASSWORD));

    block(rig.core.remove_account(rig.account)).unwrap();
    assert!(rig.core.list_accounts().unwrap().is_empty());
    assert!(rig.core.list_folders(rig.account).unwrap().is_empty());
}

#[test]
fn a_reopened_core_restarts_the_account_with_the_stored_pin_and_credential() {
    let rig = Rig::new(three_messages);
    rig.wait_for_inbox(3);
    let Rig {
        server,
        db,
        provider,
        core,
        account,
        ..
    } = rig;
    drop(core);

    server.add_message("INBOX", &raw("m4@example.org", "Hello 4"));
    let core = db.open(&provider);
    assert_eq!(provider.asked(), vec![account]);
    // The cache is there at once, before any network.
    assert_eq!(core.list_accounts().unwrap().len(), 1);
    let inbox = core
        .list_folders(account)
        .unwrap()
        .into_iter()
        .find(|f| f.path == "INBOX")
        .unwrap();
    let scope = MessageScope::Folder {
        account,
        folder_id: inbox.id,
    };
    assert_eq!(core.list_messages(scope, None, 50).unwrap().rows.len(), 3);
    // And the new mail arrives through the stored settings (the pin is what lets TLS work).
    block(wait_for(&core, "the fourth message", || {
        core.list_messages(scope, None, 50).unwrap().rows.len() == 4
    }));
}

#[test]
fn next_change_returns_after_a_server_side_change_and_a_sync() {
    let rig = Rig::new(three_messages);
    rig.wait_for_inbox(3);
    let inbox_id = rig.folder_id("INBOX").unwrap();
    // Let the first sync's changes settle, then take the position.
    std::thread::sleep(Duration::from_millis(500));
    let mut seq = block(rig.core.next_change(0)).unwrap().seq;
    while let Some(b) = block_for(Duration::from_millis(500), rig.core.next_change(seq)) {
        seq = b.unwrap().seq;
    }

    // Another client delivers mail; the app pulls to refresh.
    rig.server
        .add_message("INBOX", &raw("m4@example.org", "Hello 4"));
    rig.core.sync_now(rig.account).unwrap();
    let batch = block_for(Duration::from_secs(20), rig.core.next_change(seq))
        .expect("next_change should return after the sync")
        .unwrap();
    assert!(batch.seq > seq);
    assert!(!batch.everything);
    assert!(
        batch.folders.contains(&FolderRef {
            account: rig.account,
            folder_id: inbox_id
        }),
        "{batch:?}"
    );
    assert_eq!(rig.count(rig.inbox()), 4);
}

#[test]
fn mark_seen_then_undo_sends_nothing_to_the_server() {
    let rig = Rig::new(three_messages);
    rig.wait_for_inbox(3);
    let id = rig.core.list_messages(rig.inbox(), None, 1).unwrap().rows[0].id;
    std::thread::sleep(Duration::from_millis(300));
    rig.server.clear_commands();

    rig.core.mark_seen(vec![id], true).unwrap();
    assert!(rig.core.list_messages(rig.inbox(), None, 1).unwrap().rows[0].seen);
    // The undo of a read change is the opposite change inside the 1.5 s window.
    rig.core.mark_seen(vec![id], false).unwrap();
    assert!(!rig.core.list_messages(rig.inbox(), None, 1).unwrap().rows[0].seen);

    // Past the window, and a sync on top: the server heard nothing about flags.
    std::thread::sleep(Duration::from_millis(2_000));
    rig.core.sync_now(rig.account).unwrap();
    std::thread::sleep(Duration::from_millis(800));
    let commands = rig.server.commands();
    assert!(
        commands.iter().all(|c| !c.contains("STORE")),
        "unexpected commands: {commands:?}"
    );
    assert!(rig
        .server
        .messages("INBOX")
        .iter()
        .all(|m| !m.flags.contains("\\Seen")));
}

#[test]
fn archive_then_undo_sends_nothing_and_a_kept_archive_reaches_the_server() {
    let rig = Rig::new(three_messages);
    rig.wait_for_inbox(3);
    let ids: Vec<i64> = rig
        .core
        .list_messages(rig.inbox(), None, 10)
        .unwrap()
        .rows
        .iter()
        .map(|r| r.id)
        .collect();
    std::thread::sleep(Duration::from_millis(300));
    rig.server.clear_commands();

    // Archive one, undo inside the window: back in the list, nothing sent.
    let ops = rig.core.archive(vec![ids[0]]).unwrap();
    assert_eq!(rig.count(rig.inbox()), 2);
    assert!(rig.core.undo(ops[0]).unwrap());
    assert_eq!(rig.count(rig.inbox()), 3);
    std::thread::sleep(Duration::from_millis(500));
    rig.core.set_undo_window_ms(0);
    rig.core.sync_now(rig.account).unwrap();
    std::thread::sleep(Duration::from_millis(800));
    assert!(
        rig.server
            .commands()
            .iter()
            .all(|c| !c.contains("MOVE") && !c.contains("COPY")),
        "{:?}",
        rig.server.commands()
    );
    assert_eq!(rig.server.messages("INBOX").len(), 3);

    // Archive another with no window: it reaches the server. Trash too.
    rig.core.archive(vec![ids[1]]).unwrap();
    rig.core.trash(vec![ids[2]]).unwrap();
    wait_server("both moves on the server", || {
        rig.server.messages("Archive").len() == 1 && rig.server.messages("Trash").len() == 1
    });
    wait_server("the inbox shrank", || {
        rig.server.messages("INBOX").len() == 1
    });
    rig.wait_for_inbox(1);
}

#[test]
fn a_wrong_password_pauses_the_account_and_calls_sign_in_needed_until_it_is_fixed() {
    let server = start_server();
    three_messages(&server);
    let db = TempDb::new();
    let provider = MemProvider::new();
    let core = db.open(&provider);
    let account = block(core.add_account(
        discovered_for(&server),
        "Alice".into(),
        Some(fingerprint_of(&server)),
        password_cred("the wrong password"),
    ))
    .unwrap();
    // The app stores what the person typed under the returned id.
    provider.store(account, password_cred("the wrong password"));

    block(wait_for(&core, "needs sign-in", || {
        core.list_accounts().unwrap()[0].status == AccountStatus::NeedsSignIn
    }));
    block(wait_for(&core, "sign_in_needed call", || {
        !provider.sign_ins().is_empty()
    }));
    // Exactly once, however many times the engine reported it.
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(provider.sign_ins(), vec![account]);
    // Nothing was synced: the account is paused.
    assert!(core.list_folders(account).unwrap().is_empty());

    // The person fixes the password: the app stores it and calls `signed_in`.
    provider.store(account, password_cred(PASSWORD));
    block(core.signed_in(account)).unwrap();
    assert_eq!(core.list_accounts().unwrap()[0].status, AccountStatus::Ok);
    block(wait_for(&core, "folders after sign-in", || {
        !core.list_folders(account).unwrap().is_empty()
    }));
    assert_eq!(provider.sign_ins(), vec![account], "no further calls");
}

/// The provider is never consulted for a new account: a secret that happens to be stored under the
/// id the new account will get (a leftover of another account) is not used, only the credential
/// `add_account` was given.
#[test]
fn a_new_account_uses_the_credential_it_was_given_not_the_providers() {
    let server = start_server();
    three_messages(&server);
    let db = TempDb::new();
    let provider = MemProvider::new();
    // The id the first account will get is 1.
    provider.store(1, password_cred(PASSWORD));
    let core = db.open(&provider);
    let account = block(core.add_account(
        discovered_for(&server),
        "Alice".into(),
        Some(fingerprint_of(&server)),
        password_cred("not the password"),
    ))
    .unwrap();
    assert_eq!(account, 1);
    block(wait_for(&core, "needs sign-in", || {
        core.list_accounts().unwrap()[0].status == AccountStatus::NeedsSignIn
    }));
    assert!(provider.asked().is_empty(), "the provider was not asked");
    assert!(core.list_folders(account).unwrap().is_empty());
}

#[test]
fn an_account_with_no_credential_at_open_starts_paused_and_syncs_after_signed_in() {
    let rig = Rig::new(three_messages);
    rig.wait_for_inbox(3);
    let Rig {
        server,
        db,
        core,
        account,
        ..
    } = rig;
    drop(core);

    // A new run where the keystore has lost the secret.
    let provider = MemProvider::new();
    let core = db.open(&provider);
    assert_eq!(
        core.list_accounts().unwrap()[0].status,
        AccountStatus::NeedsSignIn
    );
    block(wait_for(&core, "sign_in_needed", || {
        !provider.sign_ins().is_empty()
    }));
    assert_eq!(provider.sign_ins(), vec![account]);

    server.add_message("INBOX", &raw("m4@example.org", "Hello 4"));
    provider.store(account, password_cred(PASSWORD));
    block(core.signed_in(account)).unwrap();
    let inbox = core
        .list_folders(account)
        .unwrap()
        .into_iter()
        .find(|f| f.path == "INBOX")
        .unwrap();
    let scope = MessageScope::Folder {
        account,
        folder_id: inbox.id,
    };
    block(wait_for(&core, "mail after sign-in", || {
        core.list_messages(scope, None, 50).unwrap().rows.len() == 4
    }));
}

#[test]
fn going_offline_and_online_is_reported_for_every_account() {
    let rig = Rig::new(three_messages);
    rig.wait_for_inbox(3);
    rig.core.set_network_available(false).unwrap();
    assert_eq!(
        rig.core.list_accounts().unwrap()[0].status,
        AccountStatus::Offline
    );
    // While offline an action still works locally.
    let id = rig.core.list_messages(rig.inbox(), None, 1).unwrap().rows[0].id;
    rig.core.mark_seen(vec![id], true).unwrap();
    rig.core.set_network_available(true).unwrap();
    assert_eq!(
        rig.core.list_accounts().unwrap()[0].status,
        AccountStatus::Ok
    );
    wait_server("the flag reached the server", || {
        rig.server
            .messages("INBOX")
            .iter()
            .any(|m| m.flags.contains("\\Seen"))
    });
}
