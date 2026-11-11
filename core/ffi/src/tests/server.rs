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

/// A certificate the person trusted before stays the one that counts when they sign in again: with
/// a stored pin for this address and server, `verify` without a pin reports another certificate as
/// changed (not as an ordinary first-use "untrusted"), and the account only takes the new pin
/// after the change was acknowledged. The pin check accepts exactly one fingerprint and skips the
/// chain checks, so a certificate that chains to a trusted authority is a change just the same.
#[test]
fn a_stored_pin_makes_another_certificate_a_change_and_not_a_first_use() {
    let server = start_server();
    let db = TempDb::new();
    let core = db.open(&MemProvider::new());
    let found = discovered_for(&server);
    let good = || password_cred(PASSWORD);
    let seen = fingerprint_of(&server);
    let earlier = CertFingerprint::of_der(b"the certificate trusted earlier").to_string();

    // No account yet: the first use.
    assert_eq!(
        block(core.verify(found.clone(), good(), None)).unwrap_err(),
        VerifyFailure::UntrustedCertificate {
            fingerprint: seen.clone()
        }
    );

    // The address is stored with the earlier certificate pinned.
    let added = block(core.add_account(
        found.clone(),
        "Alice".into(),
        Some(earlier.clone()),
        None,
        good(),
    ))
    .unwrap();
    assert!(!added.updated);

    // Now the server shows another certificate: a change, with the fingerprint seen.
    assert_eq!(
        block(core.verify(found.clone(), good(), None)).unwrap_err(),
        VerifyFailure::CertificateChanged {
            fingerprint: seen.clone()
        }
    );
    // The same address on another server is not that account: first use again.
    let mut elsewhere = found.clone();
    elsewhere.imap.as_mut().unwrap().username = "someone-else".into();
    assert!(matches!(
        block(core.verify(elsewhere, good(), None)).unwrap_err(),
        VerifyFailure::UntrustedCertificate { .. }
    ));
    // The person confirmed the warning: the app verifies with the new fingerprint, which works and
    // is what `verify` says to pass on.
    assert_eq!(
        block(core.verify(found.clone(), good(), Some(seen.clone()))).unwrap(),
        Some(seen.clone())
    );
    // Adding it needs the confirmation of that fingerprint; without it nothing changes.
    assert_eq!(
        block(core.add_account(
            found.clone(),
            "Alice".into(),
            Some(seen.clone()),
            None,
            good()
        )),
        Err(CoreError::CertificateChanged)
    );
    let confirmed = block(core.add_account(
        found.clone(),
        "Alice".into(),
        Some(seen.clone()),
        Some(seen.clone()),
        good(),
    ))
    .unwrap();
    assert!(confirmed.updated && confirmed.id == added.id);

    // The stored pin is the new one now: `verify` passes with no pin, and says which one it used.
    assert_eq!(
        block(core.verify(found.clone(), good(), None)).unwrap(),
        Some(seen.clone())
    );
    // A sign-in again with it needs no confirmation.
    let again = block(core.add_account(found, "Alice".into(), Some(seen), None, good())).unwrap();
    assert!(again.updated && again.id == added.id);
}

/// An account stored without a pin reached its server with a certificate that chains to an
/// authority. A self-signed certificate now is a change (the stronger warning), not a first use:
/// pinning it is a downgrade the person must knowingly accept. A new address still gets the
/// ordinary first-use question.
#[test]
fn an_account_stored_without_a_pin_reports_a_self_signed_certificate_as_a_change() {
    let server = start_server();
    let db = TempDb::new();
    let core = db.open(&MemProvider::new());
    let found = discovered_for(&server);
    let good = || password_cred(PASSWORD);
    let seen = fingerprint_of(&server);

    // Stored without a pin (it never reached the server over a self-signed certificate).
    let added = block(core.add_account(found.clone(), "Alice".into(), None, None, good())).unwrap();
    assert!(!added.updated);

    assert_eq!(
        block(core.verify(found.clone(), good(), None)).unwrap_err(),
        VerifyFailure::CertificateChanged {
            fingerprint: seen.clone()
        }
    );
    // Another address on the same server is a new account: the first-use question.
    let mut other = found.clone();
    other.email = "bob@example.org".into();
    assert!(matches!(
        block(core.verify(other, good(), None)).unwrap_err(),
        VerifyFailure::UntrustedCertificate { .. }
    ));
    // The person confirmed the warning: verify with that fingerprint works, and adding the pin needs it.
    assert_eq!(
        block(core.verify(found.clone(), good(), Some(seen.clone()))).unwrap(),
        Some(seen.clone())
    );
    assert_eq!(
        block(core.add_account(
            found.clone(),
            "Alice".into(),
            Some(seen.clone()),
            None,
            good()
        )),
        Err(CoreError::CertificateChanged)
    );
    let pinned = block(core.add_account(
        found.clone(),
        "Alice".into(),
        Some(seen.clone()),
        Some(seen.clone()),
        good(),
    ))
    .unwrap();
    assert!(pinned.updated && pinned.id == added.id);
    // Now it is an account with a pin: no more warning for that certificate.
    assert_eq!(block(core.verify(found, good(), None)).unwrap(), Some(seen));
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
    // A completed sync stamps the account's last-synced time (the clock is the wall clock).
    wait_server("the last-synced time", || {
        rig.core.list_accounts().unwrap()[0]
            .last_synced_ms
            .is_some_and(|t| t > 1_700_000_000_000)
    });

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
        None,
        password_cred("the wrong password"),
    ))
    .unwrap()
    .id;
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

/// Signing in again for an address that is already added, with the same server: the account keeps
/// its id, leaves the needs-sign-in state and syncs with the new password the call brought. The
/// provider is not asked: the credential came with `add_account`, and the app stores it under the
/// same id afterwards.
#[test]
fn signing_in_again_with_the_right_password_resumes_the_account_under_its_id() {
    let server = start_server();
    three_messages(&server);
    let db = TempDb::new();
    let provider = MemProvider::new();
    let core = db.open(&provider);
    let account = block(core.add_account(
        discovered_for(&server),
        "Alice".into(),
        Some(fingerprint_of(&server)),
        None,
        password_cred("the wrong password"),
    ))
    .unwrap()
    .id;
    provider.store(account, password_cred("the wrong password"));
    block(wait_for(&core, "needs sign-in", || {
        core.list_accounts().unwrap()[0].status == AccountStatus::NeedsSignIn
    }));
    assert!(core.list_folders(account).unwrap().is_empty());

    let again = block(core.add_account(
        discovered_for(&server),
        "Alice".into(),
        Some(fingerprint_of(&server)),
        None,
        password_cred(PASSWORD),
    ))
    .unwrap()
    .id;
    assert_eq!(again, account, "the same account");
    // What the app does next: the new password replaces the old one under that id.
    provider.store(account, password_cred(PASSWORD));

    block(wait_for(
        &core,
        "the first sync with the new password",
        || !core.list_folders(account).unwrap().is_empty(),
    ));
    let accounts = core.list_accounts().unwrap();
    assert_eq!(accounts.len(), 1);
    assert_eq!(accounts[0].status, AccountStatus::Ok);
    assert_eq!(
        provider.sign_ins(),
        vec![account],
        "told once, at the first failure"
    );
    assert!(provider.asked().is_empty(), "the provider was not asked");
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
        None,
        password_cred("not the password"),
    ))
    .unwrap()
    .id;
    assert_eq!(account, 1);
    block(wait_for(&core, "needs sign-in", || {
        core.list_accounts().unwrap()[0].status == AccountStatus::NeedsSignIn
    }));
    assert!(provider.asked().is_empty(), "the provider was not asked");
    assert!(core.list_folders(account).unwrap().is_empty());
}

/// A keystore that is busy when the core asks (`ProviderError::NotReady`) is not a lost
/// credential: the account is not marked as needing a sign-in, `sign_in_needed` is not called,
/// and the core asks again later and then syncs.
#[test]
fn a_provider_that_is_not_ready_does_not_pause_the_account_and_is_asked_again() {
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

    // A new run: the keystore answers "not ready" twice (at open, then at the first retry).
    let provider = MemProvider::new();
    provider.store(account, password_cred(PASSWORD));
    provider.not_ready_for(2);
    let core = db.open(&provider);
    assert_eq!(provider.asked(), vec![account], "asked once at open");
    assert_eq!(
        core.list_accounts().unwrap()[0].status,
        AccountStatus::Ok,
        "not a sign-in problem"
    );

    // Each poke asks again once the back-off is over; the third answer is the credential.
    let end = Instant::now() + Duration::from_secs(20);
    while provider.asked().len() < 3 {
        assert!(Instant::now() < end, "the provider was not asked again");
        core.sync_now(account).unwrap();
        std::thread::sleep(Duration::from_millis(50));
    }
    block(wait_for(&core, "a sync after the retry", || {
        core.sync_now(account).unwrap();
        core.list_folders(account)
            .unwrap()
            .iter()
            .any(|f| f.path == "INBOX")
    }));
    assert!(
        provider.sign_ins().is_empty(),
        "sign_in_needed was never called"
    );
    assert_eq!(core.list_accounts().unwrap()[0].status, AccountStatus::Ok);
    drop(server);
}

/// The same answer from `signed_in` (the person just typed a password and the keystore is busy):
/// the account leaves the needs-sign-in state and is retried, not reported as having no credential.
#[test]
fn signed_in_with_a_provider_that_is_not_ready_is_not_a_missing_credential() {
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

    let provider = MemProvider::new();
    let core = db.open(&provider);
    block(wait_for(&core, "sign_in_needed", || {
        !provider.sign_ins().is_empty()
    }));
    provider.store(account, password_cred(PASSWORD));
    provider.not_ready_for(1);
    block(core.signed_in(account)).unwrap();
    assert_eq!(
        core.list_accounts().unwrap()[0].status,
        AccountStatus::Ok,
        "no longer needs a sign-in"
    );
    block(wait_for(&core, "folders after the retry", || {
        core.sync_now(account).unwrap();
        core.list_folders(account)
            .unwrap()
            .iter()
            .any(|f| f.path == "INBOX")
    }));
    assert_eq!(
        provider.sign_ins(),
        vec![account],
        "no second sign_in_needed"
    );
    drop(server);
}

/// A `SignInNeeded` that was emitted before `signed_in` but is applied after it (it waited in the
/// broadcast) must not flip the account back: the engine has resumed it.
#[test]
fn a_stale_sign_in_needed_after_signed_in_is_ignored() {
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

    let provider = MemProvider::new();
    let core = db.open(&provider);
    block(wait_for(&core, "sign_in_needed", || {
        !provider.sign_ins().is_empty()
    }));
    provider.store(account, password_cred(PASSWORD));
    block(core.signed_in(account)).unwrap();
    assert_eq!(core.list_accounts().unwrap()[0].status, AccountStatus::Ok);

    core.hub()
        .apply(croft_engine::Change::SignInNeeded { account });
    // The callback runs on a blocking task: give a wrongly sent one time to show up.
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(
        core.list_accounts().unwrap()[0].status,
        AccountStatus::Ok,
        "a stale notice does not pause the account"
    );
    assert_eq!(
        provider.sign_ins(),
        vec![account],
        "no second sign_in_needed"
    );
    drop(server);
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

#[test]
fn a_sign_in_refused_with_a_change_waiting_pauses_the_account_without_counting_an_attempt() {
    let rig = Rig::new(three_messages);
    rig.wait_for_inbox(3);
    let id = rig.core.list_messages(rig.inbox(), None, 1).unwrap().rows[0].id;
    std::thread::sleep(Duration::from_millis(300));
    rig.core.set_undo_window_ms(1_500);
    rig.server.clear_commands();

    // The person archives a message; before the window is over the server starts refusing the
    // login (the password was changed elsewhere) and the open connections are cut.
    let ops = rig.core.archive(vec![id]).unwrap();
    assert_eq!(ops.len(), 1);
    rig.server
        .set_misbehavior(croft_test_support::Misbehavior::AuthFailedMentioningUnavailable);
    rig.server.drop_connections();

    block(wait_for(&rig.core, "needs sign-in", || {
        rig.core.list_accounts().unwrap()[0].status == AccountStatus::NeedsSignIn
    }));
    block(wait_for(&rig.core, "sign_in_needed call", || {
        !rig.provider.sign_ins().is_empty()
    }));
    // The move is still queued, nothing was counted as an attempt, and the server saw no move.
    let (state, attempts): (String, i64) = {
        let d = Db::open(&rig.db.path, None).unwrap();
        d.connection()
            .query_row(
                "SELECT state, attempts FROM op_queue WHERE kind = 'move'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap()
    };
    assert!(state == "queued" || state == "applying", "{state}");
    assert_eq!(attempts, 0, "an Auth failure is not a Transient attempt");
    std::thread::sleep(Duration::from_millis(2_000));
    assert!(rig.server.commands().iter().all(|c| !c.contains("MOVE")));
    assert_eq!(rig.server.messages("Archive").len(), 0);
    // The row stays out of the list (the change is applied locally) and the paused account
    // keeps waiting: no further sign-in callbacks pile up.
    assert_eq!(rig.count(rig.inbox()), 2);

    // The server accepts the password again; signing in resumes the queue and the move is sent
    // exactly once.
    rig.server
        .set_misbehavior(croft_test_support::Misbehavior::None);
    block(rig.core.signed_in(rig.account)).unwrap();
    wait_server("the move on the server", || {
        rig.server.messages("Archive").len() == 1
    });
    std::thread::sleep(Duration::from_millis(500));
    let moves = rig
        .server
        .commands()
        .iter()
        .filter(|c| c.contains("MOVE") || c.contains("COPY"))
        .count();
    assert_eq!(moves, 1, "{:?}", rig.server.commands());
}
