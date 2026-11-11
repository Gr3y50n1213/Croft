// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Tests that need no network: opening, errors, discovery against fakes, the message list and
//! the actions on rows put in the database directly.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use croft_protocols::discovery::testing::{FakeHttp, FakeResolver};
use croft_protocols::discovery::Discoverer;
use croft_protocols::{Flags, RemoteFolder, RemoteHeader, SpecialUse};
use croft_storage::{folders, messages, Db};

use super::*;
use crate::core::discover_with;

fn folder(path: &str, special_use: SpecialUse) -> RemoteFolder {
    RemoteFolder {
        path: path.to_owned(),
        delimiter: Some('/'),
        special_use,
        selectable: true,
    }
}

fn header(uid: u32, date_ms: i64) -> RemoteHeader {
    RemoteHeader {
        uid,
        message_id: Some(format!("<m{uid}@example.org>")),
        references: vec![],
        subject: format!("Subject {uid}"),
        from_name: Some("Sam Rivera".into()),
        from_addr: Some("sam@example.org".into()),
        date_ms,
        size: 1200,
        flags: Flags::default(),
    }
}

#[test]
fn the_version_is_4() {
    assert_eq!(FFI_API_VERSION, 4);
    assert_eq!(ffi_api_version(), 4);
    assert_eq!(croft_version(), env!("CARGO_PKG_VERSION"));
}

#[test]
fn open_with_no_key_and_reopen() {
    let db = TempDb::new();
    let provider = MemProvider::new();
    {
        let core = db.open(&provider);
        assert!(core.list_accounts().unwrap().is_empty());
        assert!(core.list_folders(1).unwrap().is_empty());
    }
    // Reopening the same file (the first Core is gone) sees the same, empty, database.
    let core = db.open(&provider);
    assert!(core.list_accounts().unwrap().is_empty());
    assert!(provider.asked().is_empty());
}

#[test]
fn a_key_of_the_wrong_length_is_a_typed_error() {
    let db = TempDb::new();
    let err = Core::open(
        db.path.display().to_string(),
        Some(vec![1, 2, 3]),
        MemProvider::new(),
    )
    .err()
    .unwrap();
    // Which of the two depends on the build: a build without SQLCipher refuses any key.
    assert!(
        err == CoreError::storage(StorageReason::InvalidKey)
            || err == CoreError::storage(StorageReason::EncryptionUnavailable),
        "{err:?}"
    );
}

#[test]
fn a_non_database_file_is_a_storage_error_without_the_path() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("jane-doe-mail.db");
    std::fs::write(&path, vec![b'x'; 4096]).unwrap();
    let err = Core::open(path.display().to_string(), None, MemProvider::new())
        .err()
        .unwrap();
    let text = format!("{err} {err:?}");
    assert_eq!(
        err,
        CoreError::storage(StorageReason::NotADatabase),
        "{text}"
    );
    assert!(!text.contains("jane-doe"), "{text}");
}

#[test]
fn a_newer_schema_is_refused() {
    let db = TempDb::new();
    {
        let d = Db::open(&db.path, None).unwrap();
        d.connection()
            .pragma_update(None, "user_version", 9999)
            .unwrap();
    }
    let err = Core::open(db.path.display().to_string(), None, MemProvider::new())
        .err()
        .unwrap();
    assert_eq!(err, CoreError::SchemaTooNew);
}

#[test]
fn stored_accounts_start_at_open_and_a_missing_credential_means_sign_in_needed() {
    let db = TempDb::new();
    let id = {
        let d = Db::open(&db.path, None).unwrap();
        insert_account(&d, 1)
    };
    let provider = MemProvider::new();
    let core = db.open(&provider);
    // The provider was asked for this account's credential and had none.
    assert_eq!(provider.asked(), vec![id]);
    let accounts = core.list_accounts().unwrap();
    assert_eq!(accounts.len(), 1);
    assert_eq!(accounts[0].status, AccountStatus::NeedsSignIn);
    // `sign_in_needed` runs on a blocking thread: wait for it.
    block(wait_for(&core, "sign_in_needed", || {
        !provider.sign_ins().is_empty()
    }));
    assert_eq!(provider.sign_ins(), vec![id]);
}

#[test]
fn signed_in_without_a_credential_is_a_typed_error_and_with_one_resumes() {
    let db = TempDb::new();
    let id = {
        let d = Db::open(&db.path, None).unwrap();
        insert_account(&d, 1)
    };
    let provider = MemProvider::new();
    let core = db.open(&provider);
    assert_eq!(block(core.signed_in(id)), Err(CoreError::NoCredential));
    assert_eq!(
        core.list_accounts().unwrap()[0].status,
        AccountStatus::NeedsSignIn
    );
    assert_eq!(block(core.signed_in(999)), Err(CoreError::NoSuchAccount));

    provider.store(id, password_cred("anything"));
    block(core.signed_in(id)).unwrap();
    assert_eq!(core.list_accounts().unwrap()[0].status, AccountStatus::Ok);
}

#[test]
fn unusable_stored_settings_show_as_an_error_not_a_crash() {
    let db = TempDb::new();
    {
        let d = Db::open(&db.path, None).unwrap();
        let id = insert_account(&d, 1);
        d.connection()
            .execute(
                "UPDATE account SET config_json = '{\"version\": 99}' WHERE id = ?1",
                [id],
            )
            .unwrap();
    }
    let provider = MemProvider::new();
    let core = db.open(&provider);
    assert_eq!(
        core.list_accounts().unwrap()[0].status,
        AccountStatus::Error
    );
    assert!(provider.asked().is_empty());
}

#[test]
fn network_state_shows_on_the_status_chip_and_in_the_change_log() {
    let db = TempDb::new();
    {
        let d = Db::open(&db.path, None).unwrap();
        insert_account(&d, 1);
    }
    let provider = MemProvider::new();
    provider.store(1, password_cred("x"));
    let core = db.open(&provider);
    core.set_network_available(false).unwrap();
    assert_eq!(
        core.list_accounts().unwrap()[0].status,
        AccountStatus::Offline
    );
    let batch = block(core.next_change(0)).unwrap();
    assert!(batch.accounts.contains(&1));
    core.set_network_available(true).unwrap();
    assert_eq!(core.list_accounts().unwrap()[0].status, AccountStatus::Ok);
}

// ---- discovery against fakes ----

fn net_for(server_port: u16) -> (Arc<FakeResolver>, Arc<FakeHttp>) {
    let http = Arc::new(FakeHttp::new());
    let xml = format!(
        r#"<?xml version="1.0"?>
<clientConfig version="1.1"><emailProvider id="example.org">
<incomingServer type="imap"><hostname>127.0.0.1</hostname><port>{server_port}</port>
<socketType>SSL</socketType><authentication>password-cleartext</authentication>
<username>%EMAILLOCALPART%</username></incomingServer>
</emailProvider></clientConfig>"#
    );
    http.set_body(
        "https://autoconfig.example.org/mail/config-v1.1.xml",
        xml.as_bytes(),
    );
    (Arc::new(FakeResolver::new()), http)
}

#[derive(Default)]
struct Probes(Mutex<Vec<(DiscoveryProbe, ProbeProgress)>>);

impl DiscoveryObserver for Probes {
    fn on_probe(&self, probe: DiscoveryProbe, state: ProbeProgress) -> Result<(), ProviderError> {
        self.0.lock().unwrap().push((probe, state));
        Ok(())
    }
}

#[test]
fn discover_finds_settings_and_reports_every_probe_before_it_returns() {
    let (dns, http) = net_for(14993);
    let probes = Arc::new(Probes::default());
    let found = block(discover_with(
        Discoverer::new(dns, http),
        "alice@example.org".into(),
        Some(probes.clone()),
    ))
    .unwrap();
    let imap = found.imap.clone().unwrap();
    assert_eq!(
        (
            imap.host.as_str(),
            imap.port,
            imap.security,
            imap.username.as_str()
        ),
        ("127.0.0.1", 14993, SecurityMode::Tls, "alice")
    );
    assert_eq!(found.email, "alice@example.org");
    assert_eq!(found.source, DiscoverySource::Autoconfig);
    assert_eq!(found.provider, None);

    let seen = probes.0.lock().unwrap().clone();
    // The provider table was skipped, the winner was found, and the later ones skipped.
    assert!(seen.contains(&(DiscoveryProbe::ProviderTable, ProbeProgress::Skipped)));
    assert!(seen.contains(&(DiscoveryProbe::AutoconfigSubdomain, ProbeProgress::Found)));
    assert!(seen.contains(&(DiscoveryProbe::Srv, ProbeProgress::Skipped)));
}

#[test]
fn discover_knows_a_provider_without_asking_the_network() {
    let (dns, http) = net_for(1);
    let found = block(discover_with(
        Discoverer::new(dns, http.clone()),
        "someone@gmail.com".into(),
        None,
    ))
    .unwrap();
    assert_eq!(found.provider, Some(KnownProvider::Gmail));
    assert_eq!(found.source, DiscoverySource::ProviderTable);
    assert!(http.requests().is_empty());
}

#[test]
fn discover_errors_are_typed_and_fieldless() {
    let (dns, http) = net_for(1);
    let nothing = block(discover_with(
        Discoverer::new(dns, http),
        "bob@nowhere.example".into(),
        None,
    ))
    .unwrap_err();
    assert_eq!(nothing, DiscoverError::NothingFound);

    // Through the exported method: an invalid address is refused before any network is touched.
    let db = TempDb::new();
    let core = db.open(&MemProvider::new());
    assert_eq!(
        block(core.discover("not an address".into(), None)).unwrap_err(),
        DiscoverError::InvalidAddress
    );
    assert_eq!(
        discover_error_message_key(DiscoverError::InvalidAddress),
        "discover.invalid_address"
    );
    assert!(!discover_error_default_message(DiscoverError::Offline).is_empty());
}

// ---- callbacks into the app ----

/// A provider whose every callback fails, the way a Kotlin exception arrives.
struct Failing;

impl CredentialProvider for Failing {
    fn credential(&self, _: i64) -> Result<Option<Arc<FfiCredential>>, ProviderError> {
        Err(ProviderError::Failed)
    }
    fn sign_in_needed(&self, _: i64) -> Result<(), ProviderError> {
        Err(ProviderError::Failed)
    }
}

#[test]
fn a_provider_that_fails_means_no_credential_and_nothing_panics() {
    let db = TempDb::new();
    let id = {
        let d = Db::open(&db.path, None).unwrap();
        insert_account(&d, 1)
    };
    let core = Core::open(db.path.display().to_string(), None, Arc::new(Failing)).unwrap();
    assert_eq!(
        core.list_accounts().unwrap()[0].status,
        AccountStatus::NeedsSignIn
    );
    assert_eq!(block(core.signed_in(id)), Err(CoreError::NoCredential));
    // The failing `sign_in_needed` ran on a blocking thread and was ignored; the core still works.
    core.set_network_available(false).unwrap();
    assert!(block(core.next_change(0)).unwrap().seq > 0);
}

#[test]
fn a_credential_that_is_not_utf8_is_no_credential() {
    let db = TempDb::new();
    let id = {
        let d = Db::open(&db.path, None).unwrap();
        insert_account(&d, 1)
    };
    let provider = MemProvider::new();
    provider.store(id, FfiCredential::password(vec![0xff, 0xfe, 0x80]));
    let core = db.open(&provider);
    assert_eq!(
        core.list_accounts().unwrap()[0].status,
        AccountStatus::NeedsSignIn
    );
    assert_eq!(block(core.signed_in(id)), Err(CoreError::NoCredential));
    // `verify` refuses it too, before any network is touched.
    let found = discovered_for_dead_port();
    let bad = FfiCredential::oauth2("u".into(), vec![0xc0, 0x00]);
    assert_eq!(
        block(core.verify(found, bad, None)),
        Err(VerifyFailure::WrongPassword)
    );
}

fn discovered_for_dead_port() -> FfiDiscovered {
    FfiDiscovered {
        email: "alice@example.org".into(),
        imap: Some(ServerSettings {
            host: "127.0.0.1".into(),
            port: 1,
            security: SecurityMode::Tls,
            username: "alice".into(),
        }),
        smtp: None,
        jmap: None,
        source: DiscoverySource::Autoconfig,
        provider: None,
        auth_hint: AuthKind::Password,
    }
}

/// An observer that sleeps in every callback and counts them.
struct Slow {
    delay: Duration,
    calls: AtomicUsize,
}

impl Slow {
    fn new(delay: Duration) -> Arc<Self> {
        Arc::new(Self {
            delay,
            calls: AtomicUsize::new(0),
        })
    }
}

impl DiscoveryObserver for Slow {
    fn on_probe(&self, _: DiscoveryProbe, _: ProbeProgress) -> Result<(), ProviderError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        std::thread::sleep(self.delay);
        Err(ProviderError::Failed)
    }
}

#[test]
fn a_slow_observer_does_not_delay_next_change_or_sync() {
    let db = TempDb::new();
    {
        let d = Db::open(&db.path, None).unwrap();
        insert_account(&d, 1);
    }
    let provider = MemProvider::new();
    provider.store(1, password_cred("x"));
    let core = db.open(&provider);

    // Two discoveries whose observers sleep 2 s per call: with the callbacks on the runtime's
    // two workers, both workers would be stuck.
    let observers = [
        Slow::new(Duration::from_secs(2)),
        Slow::new(Duration::from_secs(2)),
    ];
    let running: Vec<_> = observers
        .iter()
        .map(|o| {
            let (dns, http) = net_for(14993);
            let (core, o) = (core.clone(), o.clone());
            rt().spawn(async move {
                core.discover_using(
                    Discoverer::new(dns, http),
                    "alice@example.org".into(),
                    Some(o),
                )
                .await
            })
        })
        .collect();
    // Wait until both observers are inside a callback.
    let began = std::time::Instant::now();
    while observers
        .iter()
        .any(|o| o.calls.load(Ordering::SeqCst) == 0)
    {
        assert!(
            began.elapsed() < Duration::from_secs(10),
            "no probe reported"
        );
        std::thread::sleep(Duration::from_millis(10));
    }

    let started = std::time::Instant::now();
    core.set_network_available(false).unwrap();
    core.sync_now(1).unwrap();
    let batch = block_for(Duration::from_secs(1), core.next_change(0))
        .expect("next_change was held up by a slow observer")
        .unwrap();
    assert!(batch.seq > 0);
    assert!(
        block_for(Duration::from_secs(1), core.signed_in(1)).is_some(),
        "signed_in was held up by a slow observer"
    );
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "{:?}",
        started.elapsed()
    );

    // Cancelling the calls ends them without waiting for the observers.
    for r in running {
        r.abort();
    }
}

#[test]
fn nothing_is_delivered_after_discover_is_cancelled() {
    let db = TempDb::new();
    let core = db.open(&MemProvider::new());
    let observer = Slow::new(Duration::from_millis(300));
    let (dns, http) = net_for(14993);
    let call = {
        let (core, o) = (core.clone(), observer.clone());
        rt().spawn(async move {
            core.discover_using(
                Discoverer::new(dns, http),
                "alice@example.org".into(),
                Some(o),
            )
            .await
        })
    };
    let began = std::time::Instant::now();
    while observer.calls.load(Ordering::SeqCst) == 0 {
        assert!(
            began.elapsed() < Duration::from_secs(10),
            "no probe reported"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    call.abort();
    // The callback that was already running finishes; then nothing more arrives, although more
    // updates were queued.
    std::thread::sleep(Duration::from_millis(600));
    let after_cancel = observer.calls.load(Ordering::SeqCst);
    std::thread::sleep(Duration::from_millis(1200));
    assert_eq!(observer.calls.load(Ordering::SeqCst), after_cancel);
    assert!(after_cancel <= 2, "{after_cancel} callbacks ran");
}

// ---- the message list ----

/// Two accounts with an inbox each plus an Archive for the first; `n` messages in the first
/// inbox with dates 1000, 2000, ... (the last two share a date to prove the id tie-break).
struct ListRig {
    db: TempDb,
    core: Arc<Core>,
    a1: i64,
    a2: i64,
    inbox1: i64,
    inbox2: i64,
    archive1: i64,
    other1: i64,
}

fn list_rig(n: u32) -> ListRig {
    let db = TempDb::new();
    let (a1, a2, inbox1, inbox2, archive1, other1) = {
        let d = Db::open(&db.path, None).unwrap();
        let a1 = insert_account(&d, 1);
        let a2 = insert_account(&d, 2);
        let c = d.connection();
        let inbox1 = folders::upsert(c, a1, &folder("INBOX", SpecialUse::Inbox)).unwrap();
        let inbox2 = folders::upsert(c, a2, &folder("INBOX", SpecialUse::Inbox)).unwrap();
        let archive1 = folders::upsert(c, a1, &folder("Archive", SpecialUse::Archive)).unwrap();
        let other1 = folders::upsert(c, a1, &folder("Projects", SpecialUse::Other)).unwrap();
        for uid in 1..=n {
            let date = if uid == n {
                1000 * i64::from(n - 1)
            } else {
                1000 * i64::from(uid)
            };
            messages::upsert(c, a1, inbox1, &header(uid, date)).unwrap();
        }
        (a1, a2, inbox1, inbox2, archive1, other1)
    };
    ListRig {
        core: db.open(&MemProvider::new()),
        db,
        a1,
        a2,
        inbox1,
        inbox2,
        archive1,
        other1,
    }
}

#[test]
fn list_messages_pages_newest_first_without_repeats_or_gaps() {
    let rig = list_rig(7);
    let scope = MessageScope::Folder {
        account: rig.a1,
        folder_id: rig.inbox1,
    };
    let all = rig.core.list_messages(scope, None, 100).unwrap();
    assert_eq!(all.rows.len(), 7);
    assert_eq!(all.next, None);
    // Newest first; the two rows with the same date come in id order, highest id first.
    let dates: Vec<i64> = all.rows.iter().map(|r| r.date_ms).collect();
    assert!(dates.windows(2).all(|w| w[0] >= w[1]), "{dates:?}");
    let ids: Vec<i64> = all.rows.iter().map(|r| r.id).collect();
    assert_eq!(all.rows[0].date_ms, all.rows[1].date_ms);
    assert!(all.rows[0].id > all.rows[1].id);

    // Walk it in pages of 3.
    let mut walked = Vec::new();
    let mut before = None;
    let mut pages = 0;
    loop {
        let page = rig.core.list_messages(scope, before, 3).unwrap();
        pages += 1;
        walked.extend(page.rows.iter().map(|r| r.id));
        match page.next {
            Some(c) => before = Some(c),
            None => break,
        }
    }
    assert_eq!(pages, 3);
    assert_eq!(walked, ids);

    // A page that ends exactly at the end has no `next`.
    assert_eq!(rig.core.list_messages(scope, None, 7).unwrap().next, None);
    assert!(rig
        .core
        .list_messages(scope, None, 6)
        .unwrap()
        .next
        .is_some());
    assert!(rig
        .core
        .list_messages(scope, None, 0)
        .unwrap()
        .rows
        .is_empty());

    // A cursor stays valid when newer mail arrives in between: no repeat, no skip.
    let first = rig.core.list_messages(scope, None, 3).unwrap();
    {
        let d = Db::open(&rig.db.path, None).unwrap();
        messages::upsert(d.connection(), rig.a1, rig.inbox1, &header(50, 999_999)).unwrap();
    }
    let second = rig.core.list_messages(scope, first.next, 3).unwrap();
    let first_ids: Vec<i64> = first.rows.iter().map(|r| r.id).collect();
    assert!(second.rows.iter().all(|r| !first_ids.contains(&r.id)));
    assert_eq!(second.rows.len(), 3);
}

#[test]
fn rows_are_shaped_for_the_list() {
    let rig = list_rig(2);
    let page = rig
        .core
        .list_messages(
            MessageScope::Folder {
                account: rig.a1,
                folder_id: rig.inbox1,
            },
            None,
            10,
        )
        .unwrap();
    let r = &page.rows[0];
    assert_eq!((r.account, r.folder), (rig.a1, rig.inbox1));
    assert_eq!(r.sender_name.as_deref(), Some("Sam Rivera"));
    assert_eq!(r.sender_address.as_deref(), Some("sam@example.org"));
    assert!(r.subject.starts_with("Subject"));
    assert!(!r.seen && !r.flagged && !r.has_attachments && !r.pending);
}

#[test]
fn snoozed_and_hidden_rows_are_left_out_and_a_foreign_folder_id_shows_nothing() {
    let rig = list_rig(4);
    let scope = MessageScope::Folder {
        account: rig.a1,
        folder_id: rig.inbox1,
    };
    {
        let d = Db::open(&rig.db.path, None).unwrap();
        let c = d.connection();
        // Snoozed far into the future, snoozed in the past (visible again), and hidden.
        c.execute(
            "UPDATE message SET snoozed_until = 99999999999999 WHERE uid = 1",
            [],
        )
        .unwrap();
        c.execute("UPDATE message SET snoozed_until = 5 WHERE uid = 2", [])
            .unwrap();
        c.execute("UPDATE message SET local_hidden = 1 WHERE uid = 3", [])
            .unwrap();
    }
    let rows = rig.core.list_messages(scope, None, 10).unwrap().rows;
    let subjects: Vec<&str> = rows.iter().map(|r| r.subject.as_str()).collect();
    assert_eq!(rows.len(), 2, "{subjects:?}");
    assert!(!subjects.contains(&"Subject 1") && !subjects.contains(&"Subject 3"));

    // The folder id of account 1 asked for as account 2: nothing.
    let wrong = MessageScope::Folder {
        account: rig.a2,
        folder_id: rig.inbox1,
    };
    assert!(rig
        .core
        .list_messages(wrong, None, 10)
        .unwrap()
        .rows
        .is_empty());
}

#[test]
fn the_unified_inbox_merges_every_accounts_inbox_only() {
    let rig = list_rig(3);
    {
        let d = Db::open(&rig.db.path, None).unwrap();
        let c = d.connection();
        // Account 2's inbox: one message in the middle of the dates, one newest.
        messages::upsert(c, rig.a2, rig.inbox2, &header(1, 1500)).unwrap();
        messages::upsert(c, rig.a2, rig.inbox2, &header(2, 9000)).unwrap();
        // Not an inbox: never shown in the unified list.
        messages::upsert(c, rig.a1, rig.archive1, &header(1, 8000)).unwrap();
        messages::upsert(c, rig.a1, rig.other1, &header(1, 8500)).unwrap();
    }
    let page = rig
        .core
        .list_messages(MessageScope::UnifiedInbox, None, 100)
        .unwrap();
    assert_eq!(page.rows.len(), 5);
    let dates: Vec<i64> = page.rows.iter().map(|r| r.date_ms).collect();
    assert_eq!(dates, vec![9000, 2000, 2000, 1500, 1000]);
    let accounts: Vec<i64> = page.rows.iter().map(|r| r.account).collect();
    assert_eq!(accounts, vec![rig.a2, rig.a1, rig.a1, rig.a2, rig.a1]);

    // Paging through it in twos gives the same order.
    let mut walked = Vec::new();
    let mut before = None;
    loop {
        let p = rig
            .core
            .list_messages(MessageScope::UnifiedInbox, before, 2)
            .unwrap();
        walked.extend(p.rows.iter().map(|r| r.id));
        match p.next {
            Some(c) => before = Some(c),
            None => break,
        }
    }
    assert_eq!(walked, page.rows.iter().map(|r| r.id).collect::<Vec<_>>());
}

// ---- actions ----

fn row_ids(rig: &ListRig) -> Vec<i64> {
    rig.core
        .list_messages(
            MessageScope::Folder {
                account: rig.a1,
                folder_id: rig.inbox1,
            },
            None,
            100,
        )
        .unwrap()
        .rows
        .iter()
        .map(|r| r.id)
        .collect()
}

#[test]
fn archive_and_trash_find_the_special_folder_or_say_which_is_missing() {
    let rig = list_rig(3);
    let ids = row_ids(&rig);
    // The account has an Archive but no Trash.
    assert_eq!(rig.core.trash(vec![ids[0]]), Err(CoreError::NoTrashFolder));
    assert_eq!(row_ids(&rig).len(), 3, "nothing was changed");

    // Account 2 has neither.
    let other = {
        let d = Db::open(&rig.db.path, None).unwrap();
        messages::upsert(d.connection(), rig.a2, rig.inbox2, &header(1, 5)).unwrap()
    };
    // Mixed batch: the missing Archive of account 2 stops the whole call before anything moves.
    assert_eq!(
        rig.core.archive(vec![ids[0], other]),
        Err(CoreError::NoArchiveFolder)
    );
    assert_eq!(row_ids(&rig).len(), 3);

    let ops = rig.core.archive(vec![ids[0], ids[1]]).unwrap();
    assert_eq!(ops.len(), 2);
    assert_eq!(
        row_ids(&rig),
        vec![ids[2]],
        "the moved rows are hidden at once"
    );

    // Undo puts a row back and is then too late a second time.
    assert!(rig.core.undo(ops[0]).unwrap());
    assert!(!rig.core.undo(ops[0]).unwrap());
    assert_eq!(row_ids(&rig).len(), 2);
}

#[test]
fn actions_refuse_unknown_ids_and_folders_of_other_accounts() {
    let rig = list_rig(2);
    let ids = row_ids(&rig);
    assert_eq!(
        rig.core.mark_seen(vec![ids[0], 424_242], true),
        Err(CoreError::NoSuchMessage)
    );
    assert_eq!(
        rig.core.delete_forever(vec![424_242]),
        Err(CoreError::NoSuchMessage)
    );
    assert_eq!(
        rig.core.set_flagged(vec![424_242], true),
        Err(CoreError::NoSuchMessage)
    );
    // A folder of account 2 is not a destination for a message of account 1.
    assert_eq!(
        rig.core.move_to(ids.clone(), rig.inbox2),
        Err(CoreError::NoSuchFolder)
    );
    assert_eq!(
        rig.core.move_to(ids.clone(), 999_999),
        Err(CoreError::NoSuchFolder)
    );
    assert_eq!(row_ids(&rig).len(), 2, "nothing was changed");

    // A folder of the same account is.
    let ops = rig.core.move_to(vec![ids[0]], rig.other1).unwrap();
    assert_eq!(ops.len(), 1);
    // Already there: nothing to do, no error.
    assert!(rig
        .core
        .move_to(vec![ids[1]], rig.inbox1)
        .unwrap()
        .is_empty());
    // Operation ids that do not exist are "too late", not an error.
    assert!(!rig.core.undo(9_999_999).unwrap());
}

#[test]
fn mark_seen_and_its_opposite_cancel_out_in_the_local_rows() {
    let rig = list_rig(2);
    let ids = row_ids(&rig);
    let seen = |rig: &ListRig| {
        rig.core
            .list_messages(
                MessageScope::Folder {
                    account: rig.a1,
                    folder_id: rig.inbox1,
                },
                None,
                10,
            )
            .unwrap()
            .rows
            .iter()
            .map(|r| (r.seen, r.pending))
            .collect::<Vec<_>>()
    };
    rig.core.mark_seen(vec![ids[0]], true).unwrap();
    assert!(
        seen(&rig).contains(&(true, true)),
        "applied locally at once, marked pending"
    );
    rig.core.mark_seen(vec![ids[0]], false).unwrap();
    assert!(seen(&rig).iter().all(|&(s, _)| !s));
}

#[test]
fn removing_an_account_deletes_its_rows_and_is_reported() {
    let rig = list_rig(3);
    let seq = block(rig.core.next_change(0)).unwrap().seq;
    block(rig.core.remove_account(rig.a1)).unwrap();
    assert_eq!(rig.core.list_accounts().unwrap().len(), 1);
    assert!(rig.core.list_folders(rig.a1).unwrap().is_empty());
    assert!(rig
        .core
        .list_messages(
            MessageScope::Folder {
                account: rig.a1,
                folder_id: rig.inbox1
            },
            None,
            10
        )
        .unwrap()
        .rows
        .is_empty());
    let batch = block(rig.core.next_change(seq)).unwrap();
    assert!(batch.accounts.contains(&rig.a1));
    assert_eq!(
        block(rig.core.remove_account(rig.a1)),
        Err(CoreError::NoSuchAccount)
    );
}

#[test]
fn add_account_refuses_unusable_input_before_touching_the_database() {
    let db = TempDb::new();
    let core = db.open(&MemProvider::new());
    let good = ServerSettings {
        host: "mail.example.org".into(),
        port: 993,
        security: SecurityMode::Tls,
        username: "alice".into(),
    };
    let base = FfiDiscovered {
        email: "alice@example.org".into(),
        imap: Some(good.clone()),
        smtp: None,
        jmap: None,
        source: DiscoverySource::Autoconfig,
        provider: None,
        auth_hint: AuthKind::Password,
    };
    let jmap_only = FfiDiscovered {
        imap: None,
        jmap: Some("https://example.org/.well-known/jmap".into()),
        ..base.clone()
    };
    assert_eq!(
        block(core.add_account(jmap_only, "x".into(), None, password_cred("x"))),
        Err(CoreError::UnsupportedServer)
    );
    assert_eq!(
        block(core.add_account(
            base.clone(),
            "x".into(),
            Some("zz".into()),
            password_cred("x")
        )),
        Err(CoreError::BadFingerprint)
    );
    let no_host = FfiDiscovered {
        imap: Some(ServerSettings {
            host: "  ".into(),
            ..good
        }),
        ..base.clone()
    };
    assert_eq!(
        block(core.add_account(no_host, "x".into(), None, password_cred("x"))),
        Err(CoreError::BadSettings)
    );
    // A credential that is not UTF-8 is refused with a typed error, and nothing is written.
    assert_eq!(
        block(core.add_account(
            base,
            "x".into(),
            None,
            FfiCredential::password(vec![0xff, 0xfe])
        )),
        Err(CoreError::NoCredential)
    );
    assert!(core.list_accounts().unwrap().is_empty());
}

#[test]
fn a_removed_accounts_id_is_not_given_to_the_next_account() {
    let db = TempDb::new();
    let core = db.open(&MemProvider::new());
    let found = FfiDiscovered {
        email: "alice@example.org".into(),
        imap: Some(ServerSettings {
            host: "127.0.0.1".into(),
            port: 1,
            security: SecurityMode::Tls,
            username: "alice".into(),
        }),
        smtp: None,
        jmap: None,
        source: DiscoverySource::Autoconfig,
        provider: None,
        auth_hint: AuthKind::Password,
    };
    let add = |name: &str| {
        block(core.add_account(found.clone(), name.into(), None, password_cred("x"))).unwrap()
    };
    assert_eq!(add("one"), 1);
    assert_eq!(add("two"), 2);
    block(core.remove_account(2)).unwrap();
    assert_eq!(add("three"), 3);
}
