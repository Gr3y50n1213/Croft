// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! `add_account` for an address that is already added: the same server means the person signs in
//! again (the account keeps its id and gets the new settings and credential), another server is
//! `AccountExists`, and the sample mailbox is never the account that is meant. These tests use
//! accounts on a port nobody listens on, so they need no network.

use croft_protocols::demo::DEMO_ADDRESS;
use croft_storage::accounts::Account;
use croft_storage::Db;

use super::*;

/// What `discover` would return for `email` on the dead server of `insert_account`
/// (127.0.0.1 port 1, user "nobody"), with the parts a test changes.
fn found(email: &str, host: &str, port: u16, username: &str) -> FfiDiscovered {
    FfiDiscovered {
        email: email.into(),
        imap: Some(ServerSettings {
            host: host.into(),
            port,
            security: SecurityMode::Tls,
            username: username.into(),
        }),
        smtp: None,
        jmap: None,
        source: DiscoverySource::Autoconfig,
        provider: None,
        auth_hint: AuthKind::Password,
    }
}

fn same_server(email: &str) -> FfiDiscovered {
    found(email, "127.0.0.1", 1, "nobody")
}

fn stored(core: &Core, id: i64) -> Account {
    core.stored_account(id).unwrap()
}

fn pin_of(account: &Account) -> Option<String> {
    account.config["pinned_fingerprint"]
        .as_str()
        .map(str::to_owned)
}

/// A core whose database holds `insert_account(1)` (user1@example.org on the dead server), with
/// no stored credential: the account starts in the needs-sign-in state, as after a lost password.
fn core_with_account_needing_sign_in() -> (TempDb, Arc<MemProvider>, Arc<Core>, i64) {
    let db = TempDb::new();
    let id = {
        let d = Db::open(&db.path, None).unwrap();
        insert_account(&d, 1)
    };
    let provider = MemProvider::new();
    let core = db.open(&provider);
    assert_eq!(
        core.list_accounts().unwrap()[0].status,
        AccountStatus::NeedsSignIn
    );
    (db, provider, core, id)
}

#[test]
fn the_same_address_and_server_updates_the_account_and_keeps_its_id() {
    let (_db, provider, core, id) = core_with_account_needing_sign_in();
    block(wait_for(&core, "sign_in_needed", || {
        !provider.sign_ins().is_empty()
    }));
    let before = stored(&core, id);
    assert_eq!(pin_of(&before), None);

    // The person signs in again: a new password, the server's certificate trusted this time, and
    // the host written another way (brackets, a different case: the same server).
    let pin = CertFingerprint::of_der(b"the server's own certificate").to_string();
    let again = block(core.add_account(
        found("User1@Example.org", "[127.0.0.1]", 1, "nobody"),
        "A new label".into(),
        Some(pin.clone()),
        false,
        password_cred("the new password"),
    ))
    .unwrap();
    assert_eq!(
        again,
        AddedAccount { id, updated: true },
        "the account keeps its id"
    );

    let accounts = core.list_accounts().unwrap();
    assert_eq!(accounts.len(), 1, "no second account");
    assert_eq!(accounts[0].id, id);
    assert_eq!(accounts[0].label, before.label, "the label is the person's");
    assert_eq!(accounts[0].email, before.email);
    // Out of the needs-sign-in state: the account runs again with the new credential.
    assert_eq!(accounts[0].status, AccountStatus::Ok);
    // The new settings are stored (host as typed, the pin), and nothing secret with them.
    let after = stored(&core, id);
    assert_eq!(after.config["imap"]["host"], "[127.0.0.1]");
    assert_eq!(pin_of(&after), Some(pin));
    assert!(!after.config.to_string().contains("new password"));
    assert_eq!(after.position, before.position);
    assert_eq!(after.created_at, before.created_at);
    // The credential came with the call: the provider was asked only at open, and told only once.
    assert_eq!(provider.asked(), vec![id]);
    assert_eq!(provider.sign_ins(), vec![id]);
}

/// Signs in for `alice@example.org` on the dead server with `pin`, as the app does after `verify`.
fn sign_in(
    core: &Core,
    pin: Option<&String>,
    accept_changed_certificate: bool,
) -> Result<AddedAccount, CoreError> {
    block(core.add_account(
        same_server("alice@example.org"),
        "Alice".into(),
        pin.cloned(),
        accept_changed_certificate,
        password_cred("x"),
    ))
}

fn fingerprint(name: &str) -> String {
    CertFingerprint::of_der(name.as_bytes()).to_string()
}

/// A core with `alice@example.org` stored with a pin (a self-signed server the person trusted).
fn core_with_pinned_account() -> (TempDb, Arc<Core>, i64, String) {
    let db = TempDb::new();
    let core = db.open(&MemProvider::new());
    let pin = fingerprint("certificate A");
    let added = sign_in(&core, Some(&pin), false).unwrap();
    assert!(!added.updated);
    assert_eq!(pin_of(&stored(&core, added.id)), Some(pin.clone()));
    (db, core, added.id, pin)
}

/// The re-add must not swap in another certificate behind the person's back: an interceptor with
/// certificate B would be trusted for good by a plain "trust this certificate" prompt, without the
/// warning that this is not the certificate trusted before. (This replaces the test
/// `a_stored_pin_follows_the_latest_sign_in`, which asserted exactly that behaviour.)
#[test]
fn a_different_pin_is_refused_without_the_acknowledgement() {
    let (_db, core, id, pin) = core_with_pinned_account();
    let before = stored(&core, id);
    let other = fingerprint("certificate B");
    assert_eq!(
        sign_in(&core, Some(&other), false),
        Err(CoreError::CertificateChanged)
    );
    assert_eq!(stored(&core, id), before, "nothing was changed");
    assert_eq!(pin_of(&stored(&core, id)), Some(pin));
    assert_eq!(core.list_accounts().unwrap().len(), 1);
}

/// The second way in: a re-add with no pin (the interceptor's certificate chains to a trusted
/// authority) must not drop the pin silently either.
#[test]
fn no_pin_is_refused_without_the_acknowledgement_when_one_is_stored() {
    let (_db, core, id, pin) = core_with_pinned_account();
    let before = stored(&core, id);
    assert_eq!(
        sign_in(&core, None, false),
        Err(CoreError::CertificateChanged)
    );
    assert_eq!(stored(&core, id), before);
    assert_eq!(pin_of(&stored(&core, id)), Some(pin));
}

#[test]
fn with_the_acknowledgement_the_pin_is_replaced_or_dropped() {
    let (_db, core, id, _pin) = core_with_pinned_account();
    let other = fingerprint("certificate B");
    let added = sign_in(&core, Some(&other), true).unwrap();
    assert_eq!(added, AddedAccount { id, updated: true });
    assert_eq!(pin_of(&stored(&core, id)), Some(other.clone()));
    // Confirmed again later, with the certificate now chaining to an authority: no pin.
    assert_eq!(sign_in(&core, None, true).unwrap().id, id);
    assert_eq!(pin_of(&stored(&core, id)), None);
    assert_eq!(core.list_accounts().unwrap().len(), 1);
}

#[test]
fn the_same_pin_as_stored_is_accepted_without_the_acknowledgement() {
    let (_db, core, id, pin) = core_with_pinned_account();
    // The pin `verify` returned for the stored account's certificate (any spelling of the hex).
    let added = sign_in(&core, Some(&pin.to_lowercase()), false).unwrap();
    assert_eq!(added, AddedAccount { id, updated: true });
    assert_eq!(pin_of(&stored(&core, id)), Some(pin));
}

#[test]
fn the_acknowledgement_changes_nothing_when_no_pin_is_stored_or_for_a_new_account() {
    let db = TempDb::new();
    let core = db.open(&MemProvider::new());
    let pin = fingerprint("certificate A");
    // A new account with the flag: just added.
    let added = sign_in(&core, None, true).unwrap();
    assert!(!added.updated);
    // No stored pin: a pin may be added with or without the flag.
    assert_eq!(sign_in(&core, Some(&pin), false).unwrap().id, added.id);
    assert_eq!(pin_of(&stored(&core, added.id)), Some(pin));
}

/// The app cannot store the new password: it undoes the update, and the account is as it was,
/// settings and pin included.
#[test]
fn an_update_can_be_rolled_back() {
    let (_db, core, id, pin) = core_with_pinned_account();
    let before = stored(&core, id);
    let other = fingerprint("certificate B");
    let added = sign_in(&core, Some(&other), true).unwrap();
    assert!(added.updated);
    assert_eq!(pin_of(&stored(&core, id)), Some(other));

    block(core.rollback_account_update(id)).unwrap();
    assert_eq!(stored(&core, id), before);
    assert_eq!(pin_of(&stored(&core, id)), Some(pin));
    assert_eq!(core.list_accounts().unwrap().len(), 1);
    // Nothing more to undo: a second call, an unknown id and a new account do nothing.
    block(core.rollback_account_update(id)).unwrap();
    block(core.rollback_account_update(999)).unwrap();
    assert_eq!(stored(&core, id), before);
}

#[test]
fn a_new_account_has_nothing_to_roll_back_and_removing_forgets_the_update() {
    let db = TempDb::new();
    let core = db.open(&MemProvider::new());
    let added = sign_in(&core, None, false).unwrap();
    block(core.rollback_account_update(added.id)).unwrap();
    assert_eq!(core.list_accounts().unwrap().len(), 1);
    assert!(sign_in(&core, None, false).unwrap().updated);
    block(core.remove_account(added.id)).unwrap();
    block(core.rollback_account_update(added.id)).unwrap();
    assert!(core.list_accounts().unwrap().is_empty());
}

#[test]
fn another_server_for_a_stored_address_is_an_error_and_changes_nothing() {
    let (_db, _provider, core, id) = core_with_account_needing_sign_in();
    let before = stored(&core, id);
    for (what, discovered) in [
        (
            "another host",
            found("user1@example.org", "localhost", 1, "nobody"),
        ),
        (
            "another port",
            found("user1@example.org", "127.0.0.1", 2, "nobody"),
        ),
        (
            "another user name",
            found("user1@example.org", "127.0.0.1", 1, "somebody"),
        ),
        (
            "a user name in other case",
            found("USER1@example.org", "127.0.0.1", 1, "Nobody"),
        ),
    ] {
        assert_eq!(
            block(core.add_account(
                discovered,
                "x".into(),
                None,
                false,
                password_cred("another password")
            )),
            Err(CoreError::AccountExists),
            "{what}"
        );
        assert_eq!(core.list_accounts().unwrap().len(), 1, "{what}");
        assert_eq!(
            stored(&core, id),
            before,
            "{what}: the account is untouched"
        );
        assert_eq!(
            core.list_accounts().unwrap()[0].status,
            AccountStatus::NeedsSignIn,
            "{what}: still waiting for its own sign-in"
        );
    }
}

#[test]
fn settings_this_version_cannot_read_are_never_overwritten() {
    let db = TempDb::new();
    let newer = serde_json::json!({ "version": 99, "something": "a newer app wrote this" });
    let id = {
        let d = Db::open(&db.path, None).unwrap();
        accounts::insert(
            d.connection(),
            &NewAccount {
                label: "Newer".into(),
                email: "user1@example.org".into(),
                protocol: Protocol::Imap,
                provider: "generic".into(),
                config: newer.clone(),
                color: None,
                position: 0,
            },
            0,
        )
        .unwrap()
    };
    let core = db.open(&MemProvider::new());
    assert_eq!(
        block(core.add_account(
            same_server("user1@example.org"),
            "x".into(),
            None,
            false,
            password_cred("x")
        )),
        Err(CoreError::AccountExists)
    );
    assert_eq!(stored(&core, id).config, newer);
    assert_eq!(core.list_accounts().unwrap().len(), 1);
}

#[test]
fn the_sample_mailbox_is_never_the_account_that_is_updated() {
    let db = TempDb::new();
    let core = db.open(&MemProvider::new());
    let demo = block(core.add_demo_account()).unwrap();
    assert!(demo.is_demo);
    assert_eq!(demo.email, DEMO_ADDRESS);
    let demo_row = stored(&core, demo.id);

    // A real account that happens to use the sample address is a new account, whatever its server.
    let real = block(core.add_account(
        same_server(DEMO_ADDRESS),
        "Real".into(),
        None,
        false,
        password_cred("x"),
    ))
    .unwrap()
    .id;
    assert_ne!(real, demo.id);
    let accounts = core.list_accounts().unwrap();
    assert_eq!(accounts.len(), 2);
    assert!(accounts.iter().find(|a| a.id == demo.id).unwrap().is_demo);
    assert!(!accounts.iter().find(|a| a.id == real).unwrap().is_demo);
    assert_eq!(
        stored(&core, demo.id),
        demo_row,
        "the sample mailbox is untouched"
    );

    // Adding it again finds the real one, not the sample mailbox, and a different server is refused
    // because of the real one.
    assert_eq!(
        block(core.add_account(
            same_server(DEMO_ADDRESS),
            "Real".into(),
            None,
            false,
            password_cred("y")
        )),
        Ok(AddedAccount {
            id: real,
            updated: true
        })
    );
    assert_eq!(
        block(core.add_account(
            found(DEMO_ADDRESS, "localhost", 1, "nobody"),
            "Real".into(),
            None,
            false,
            password_cred("y")
        )),
        Err(CoreError::AccountExists)
    );
    assert_eq!(core.list_accounts().unwrap().len(), 2);
    assert_eq!(stored(&core, demo.id), demo_row);
}

#[test]
fn updating_an_account_uses_up_no_id_and_two_at_once_make_one_account() {
    let db = TempDb::new();
    let core = db.open(&MemProvider::new());
    let add = |email: &str| {
        core.add_account(
            same_server(email),
            "x".into(),
            None,
            false,
            password_cred("x"),
        )
    };
    // Two sign-ins for one new address at the same moment (the screen was recreated, say): one
    // adds it, the other updates it, and each is told which it was.
    let (a, b) = block(async { tokio::join!(add("alice@example.org"), add("alice@example.org")) });
    let (a, b) = (a.unwrap(), b.unwrap());
    assert_eq!(a.id, b.id);
    assert_eq!(
        [a.updated, b.updated].iter().filter(|u| **u).count(),
        1,
        "exactly one of them added the account"
    );
    assert_eq!(core.list_accounts().unwrap().len(), 1);
    // Ids are still handed out one by one: the updates did not use any.
    let again = block(add("alice@example.org")).unwrap();
    assert_eq!(
        again,
        AddedAccount {
            id: a.id,
            updated: true
        }
    );
    let next = block(add("bob@example.org")).unwrap();
    assert_eq!(
        next,
        AddedAccount {
            id: a.id + 1,
            updated: false
        }
    );
}

/// The address is compared with full Unicode lower-casing, not only ASCII.
#[test]
fn the_address_is_compared_with_unicode_lower_casing() {
    let db = TempDb::new();
    let core = db.open(&MemProvider::new());
    let add = |email: &str| {
        block(core.add_account(
            same_server(email),
            "x".into(),
            None,
            false,
            password_cred("x"),
        ))
        .unwrap()
    };
    let first = add("Élodie@Example.org");
    assert!(!first.updated);
    // The same address in other case, in letters outside ASCII too, and with spaces around it.
    assert_eq!(
        add(" élodie@example.ORG "),
        AddedAccount {
            id: first.id,
            updated: true
        }
    );
    assert_eq!(add("ÉLODIE@EXAMPLE.ORG").id, first.id);
    assert_eq!(core.list_accounts().unwrap().len(), 1);
    // Another address is another account.
    assert!(!add("elodie@example.org").updated);
    assert_eq!(core.list_accounts().unwrap().len(), 2);
}

/// Any difference in the servers is another account: security, the SMTP server, its port. A
/// sign-in again can never repoint where mail is read or sent.
#[test]
fn a_different_security_mode_or_smtp_server_is_another_server() {
    let db = TempDb::new();
    let core = db.open(&MemProvider::new());
    let smtp = |host: &str, port: u16| ServerSettings {
        host: host.into(),
        port,
        security: SecurityMode::Tls,
        username: "nobody".into(),
    };
    let with = |imap_security: SecurityMode, smtp: Option<ServerSettings>| {
        let mut d = same_server("alice@example.org");
        d.imap.as_mut().unwrap().security = imap_security;
        d.smtp = smtp;
        d
    };
    let add =
        |d: FfiDiscovered| block(core.add_account(d, "x".into(), None, false, password_cred("x")));
    let first = add(with(SecurityMode::Tls, Some(smtp("127.0.0.1", 2)))).unwrap();
    let before = stored(&core, first.id);
    for (what, d) in [
        (
            "another IMAP security mode",
            with(SecurityMode::StartTls, Some(smtp("127.0.0.1", 2))),
        ),
        (
            "another SMTP host",
            with(SecurityMode::Tls, Some(smtp("localhost", 2))),
        ),
        (
            "another SMTP port",
            with(SecurityMode::Tls, Some(smtp("127.0.0.1", 3))),
        ),
        ("no SMTP server", with(SecurityMode::Tls, None)),
    ] {
        assert_eq!(add(d), Err(CoreError::AccountExists), "{what}");
        assert_eq!(stored(&core, first.id), before, "{what}: untouched");
    }
    // The same SMTP server written another way is the same server.
    let same = add(with(SecurityMode::Tls, Some(smtp("[127.0.0.1]", 2)))).unwrap();
    assert_eq!(
        same,
        AddedAccount {
            id: first.id,
            updated: true
        }
    );
    // And an account stored without SMTP does not take one.
    let bare = {
        let mut d = same_server("bob@example.org");
        d.smtp = None;
        add(d).unwrap()
    };
    let mut with_smtp = same_server("bob@example.org");
    with_smtp.smtp = Some(smtp("127.0.0.1", 2));
    assert_eq!(add(with_smtp), Err(CoreError::AccountExists));
    assert!(!bare.updated);
}
