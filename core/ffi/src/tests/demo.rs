// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The sample mailbox through the exported surface: add it (once), list it, act on it, sync, close
//! and reopen, remove it. The credential provider panics if it is ever asked: the sample mailbox
//! has no credential and no server adapter, and every step here runs with the device offline.

use croft_storage::Db;

use super::*;

/// A provider that must never be called: asking for a credential is the first step of building a
/// server adapter for an account.
struct NeverAsked;

impl CredentialProvider for NeverAsked {
    fn credential(&self, _: i64) -> Result<Option<Arc<FfiCredential>>, ProviderError> {
        panic!("the sample mailbox asked for a credential");
    }

    fn sign_in_needed(&self, _: i64) -> Result<(), ProviderError> {
        panic!("the sample mailbox asked for a sign-in");
    }
}

fn open(db: &TempDb) -> Arc<Core> {
    Core::open(db.path.display().to_string(), None, Arc::new(NeverAsked)).unwrap()
}

fn folder_id(core: &Core, account: i64, path: &str) -> i64 {
    core.list_folders(account)
        .unwrap()
        .into_iter()
        .find(|f| f.path == path)
        .unwrap_or_else(|| panic!("no folder {path}"))
        .id
}

fn rows(core: &Core, account: i64, path: &str) -> Vec<MessageRow> {
    let Some(f) = core
        .list_folders(account)
        .unwrap()
        .into_iter()
        .find(|f| f.path == path)
    else {
        return Vec::new();
    };
    core.list_messages(
        MessageScope::Folder {
            account,
            folder_id: f.id,
        },
        None,
        500,
    )
    .unwrap()
    .rows
}

fn row(core: &Core, account: i64, path: &str, subject: &str) -> Option<MessageRow> {
    rows(core, account, path)
        .into_iter()
        .find(|r| r.subject == subject)
}

const BOOK_CLUB: &str = "Book club: vote for next month's pick";
const FERRY: &str = "Your booking for Friday's 08:15 sailing";
const POTTERY: &str = "Photos from the pottery workshop";

/// Waits for the first sync of the sample mailbox.
fn wait_for_sample_inbox(core: &Core, account: i64) {
    block(wait_for(core, "sample inbox", || {
        rows(core, account, "INBOX").len() == 10 && rows(core, account, "Archive").len() == 3
    }));
}

#[test]
fn the_sample_mailbox_is_added_once_without_a_credential() {
    let db = TempDb::new();
    let core = open(&db);
    core.set_network_available(false).unwrap();
    let info = block(core.add_demo_account()).unwrap();
    assert!(info.is_demo);
    assert_eq!(info.label, "Sample mail");
    assert!(info.email.ends_with("@example.org"));
    assert_eq!(
        info.status,
        AccountStatus::Ok,
        "offline does not concern it"
    );
    // Adding it again returns the same account.
    let again = block(core.add_demo_account()).unwrap();
    assert_eq!(again.id, info.id);
    let all = core.list_accounts().unwrap();
    assert_eq!(all.len(), 1);
    // (Not the whole record: the first sync may set `last_synced_ms` in between.)
    assert_eq!((all[0].id, all[0].is_demo), (again.id, true));
    // Nothing to sign in to.
    block(core.signed_in(info.id)).unwrap();
    core.sync_now(info.id).unwrap();
}

#[test]
fn a_real_account_is_not_marked_demo() {
    let db = TempDb::new();
    {
        let d = Db::open(&db.path, None).unwrap();
        insert_account(&d, 1);
    }
    let provider = MemProvider::new();
    let core = db.open(&provider);
    let all = core.list_accounts().unwrap();
    assert_eq!(all.len(), 1);
    assert!(!all[0].is_demo);
}

#[test]
fn folders_and_messages_are_listed() {
    let db = TempDb::new();
    let core = open(&db);
    let account = block(core.add_demo_account()).unwrap().id;
    wait_for_sample_inbox(&core, account);
    let mut paths: Vec<String> = core
        .list_folders(account)
        .unwrap()
        .into_iter()
        .map(|f| f.path)
        .collect();
    paths.sort();
    assert_eq!(paths, ["Archive", "Drafts", "INBOX", "Sent", "Trash"]);
    let specials: Vec<String> = core
        .list_folders(account)
        .unwrap()
        .into_iter()
        .map(|f| f.special_use)
        .collect();
    for s in ["inbox", "sent", "drafts", "trash", "archive"] {
        assert!(specials.iter().any(|x| x == s), "{s}");
    }
    block(wait_for(&core, "all folders", || {
        rows(&core, account, "Sent").len() == 3
            && rows(&core, account, "Drafts").len() == 2
            && rows(&core, account, "Trash").len() == 2
    }));
    let inbox = rows(&core, account, "INBOX");
    assert_eq!(inbox[0].subject, "Welcome to the sample mailbox");
    assert!(inbox.iter().any(|r| !r.seen));
    assert!(inbox.iter().any(|r| r.thread_count >= 3), "a long thread");
    for r in &inbox {
        let addr = r.sender_address.as_deref().unwrap();
        assert!(
            addr.ends_with("@example.org") || addr.ends_with("@example.com"),
            "{addr}"
        );
    }
    // The unified Inbox shows it too.
    let unified = core
        .list_messages(MessageScope::UnifiedInbox, None, 500)
        .unwrap();
    assert_eq!(unified.rows.len(), 10);
}

#[test]
fn star_and_move_stay_after_a_sync_and_after_a_restart() {
    let db = TempDb::new();
    let account = {
        let core = open(&db);
        core.set_network_available(false).unwrap();
        core.set_undo_window_ms(0);
        let account = block(core.add_demo_account()).unwrap().id;
        wait_for_sample_inbox(&core, account);

        let book = row(&core, account, "INBOX", BOOK_CLUB).unwrap();
        assert!(!book.flagged && !book.seen);
        core.set_flagged(vec![book.id], true).unwrap();
        core.mark_seen(vec![book.id], true).unwrap();
        let ferry = row(&core, account, "INBOX", FERRY).unwrap();
        core.archive(vec![ferry.id]).unwrap();
        let pottery = row(&core, account, "INBOX", POTTERY).unwrap();
        core.move_to(vec![pottery.id], folder_id(&core, account, "Trash"))
            .unwrap();

        // Confirmed by the sample "server" (no network: the device is offline).
        block(wait_for(&core, "changes confirmed", || {
            let book = row(&core, account, "INBOX", BOOK_CLUB);
            book.is_some_and(|b| b.flagged && b.seen && !b.pending)
                && row(&core, account, "Archive", FERRY).is_some_and(|r| !r.pending)
                && row(&core, account, "Trash", POTTERY).is_some_and(|r| !r.pending)
        }));

        // A sync keeps them.
        core.sync_now(account).unwrap();
        std::thread::sleep(Duration::from_millis(300));
        block(wait_for(&core, "after sync", || {
            core.list_accounts().unwrap()[0].last_synced_ms.is_some()
        }));
        assert!(row(&core, account, "INBOX", BOOK_CLUB).unwrap().flagged);
        assert!(row(&core, account, "INBOX", FERRY).is_none());
        assert!(row(&core, account, "Archive", FERRY).is_some());
        assert_eq!(rows(&core, account, "INBOX").len(), 8);
        account
    };

    // The app restarts: the sample mailbox is rebuilt from the device, and its first sync
    // (which runs at start) undoes nothing.
    let core = open(&db);
    core.set_network_available(false).unwrap();
    let info = &core.list_accounts().unwrap()[0];
    assert!(info.is_demo);
    assert_eq!(info.status, AccountStatus::Ok);
    core.sync_now(account).unwrap();
    let started = std::time::Instant::now();
    while started.elapsed() < Duration::from_secs(2) {
        let book = row(&core, account, "INBOX", BOOK_CLUB).unwrap();
        assert!(book.flagged && book.seen, "star and read kept");
        assert!(row(&core, account, "INBOX", FERRY).is_none());
        assert!(row(&core, account, "Archive", FERRY).is_some());
        assert!(row(&core, account, "Trash", POTTERY).is_some());
        assert_eq!(rows(&core, account, "INBOX").len(), 8);
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn removing_the_sample_mailbox_deletes_all_its_rows() {
    let db = TempDb::new();
    let account = {
        let core = open(&db);
        let account = block(core.add_demo_account()).unwrap().id;
        wait_for_sample_inbox(&core, account);
        // One change still waiting in the queue when it is removed.
        core.set_undo_window_ms(60_000);
        let ferry = row(&core, account, "INBOX", FERRY).unwrap();
        core.archive(vec![ferry.id]).unwrap();
        block(core.remove_account(account)).unwrap();
        assert!(core.list_accounts().unwrap().is_empty());
        assert!(core.list_folders(account).unwrap().is_empty());
        assert_eq!(
            block(core.remove_account(account)).unwrap_err(),
            CoreError::NoSuchAccount
        );
        assert_eq!(
            core.sync_now(account).unwrap_err(),
            CoreError::NoSuchAccount
        );
        // It can be added again (a new account id).
        let again = block(core.add_demo_account()).unwrap();
        assert_ne!(again.id, account);
        block(core.remove_account(again.id)).unwrap();
        account
    };
    let d = Db::open(&db.path, None).unwrap();
    let conn = d.connection();
    let count = |table: &str| -> i64 {
        conn.query_row(
            &format!("SELECT COUNT(*) FROM {table} WHERE account_id >= ?1"),
            [account],
            |r| r.get(0),
        )
        .unwrap()
    };
    for table in [
        "folder",
        "message",
        "op_queue",
        "op_awaiting_uid",
        "outbox",
        "identity",
    ] {
        assert_eq!(count(table), 0, "{table}");
    }
    let accounts: i64 = conn
        .query_row("SELECT COUNT(*) FROM account", [], |r| r.get(0))
        .unwrap();
    assert_eq!(accounts, 0);
}
