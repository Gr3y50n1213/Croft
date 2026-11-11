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
        password_cred("the new password"),
    ))
    .unwrap();
    assert_eq!(again, id, "the account keeps its id");

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

#[test]
fn a_stored_pin_follows_the_latest_sign_in() {
    let db = TempDb::new();
    let core = db.open(&MemProvider::new());
    let first = CertFingerprint::of_der(b"first").to_string();
    let second = CertFingerprint::of_der(b"second").to_string();
    let add = |pin: Option<&String>| {
        block(core.add_account(
            same_server("alice@example.org"),
            "Alice".into(),
            pin.cloned(),
            password_cred("x"),
        ))
        .unwrap()
    };
    let id = add(Some(&first));
    assert_eq!(pin_of(&stored(&core, id)), Some(first));
    // Signed in again without a pin (the server has a proper certificate now): none is kept.
    assert_eq!(add(None), id);
    assert_eq!(pin_of(&stored(&core, id)), None);
    assert_eq!(add(Some(&second)), id);
    assert_eq!(pin_of(&stored(&core, id)), Some(second));
    assert_eq!(core.list_accounts().unwrap().len(), 1);
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
        password_cred("x"),
    ))
    .unwrap();
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
            password_cred("y")
        )),
        Ok(real)
    );
    assert_eq!(
        block(core.add_account(
            found(DEMO_ADDRESS, "localhost", 1, "nobody"),
            "Real".into(),
            None,
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
    let add =
        |email: &str| core.add_account(same_server(email), "x".into(), None, password_cred("x"));
    // Two sign-ins for one new address at the same moment (the screen was recreated, say).
    let (a, b) = block(async { tokio::join!(add("alice@example.org"), add("alice@example.org")) });
    let (a, b) = (a.unwrap(), b.unwrap());
    assert_eq!(a, b);
    assert_eq!(core.list_accounts().unwrap().len(), 1);
    // Ids are still handed out one by one: the updates did not use any.
    assert_eq!(block(add("alice@example.org")).unwrap(), a);
    let next = block(add("bob@example.org")).unwrap();
    assert_eq!(next, a + 1);
}
