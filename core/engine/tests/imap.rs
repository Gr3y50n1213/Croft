// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The engine with the real IMAP adapter (`ImapBackend`) against the in-process IMAP server from
//! `croft-test-support` (plaintext on 127.0.0.1). Driven step by step with a manual clock; the
//! network is real (loopback), so these use real time only for I/O.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use common::create_account;
use croft_engine::{AccountId, Engine, EngineConfig, EngineHandle, ManualClock};
use croft_protocols::{
    BackendFuture, Capabilities, Credential, FlagChange, FolderStatus, ImapBackend, ImapConfig,
    MailBackend, MoveOutcome, RemoteFolder, RemoteHeader, RemoteSnapshot, Security, Uid,
};
use croft_storage::messages::Message;
use croft_storage::{folders, messages, Db};
use croft_test_support::{sample_message, Caps, ServerOptions, TestServer};

fn imap(server: &TestServer) -> ImapBackend {
    let config = ImapConfig::new(
        server.host(),
        server.port(),
        Security::PlaintextForTestsOnly,
        server.username(),
    )
    .with_timeout(Duration::from_secs(5));
    ImapBackend::new(config, Credential::Password(server.password())).unwrap()
}

fn msg(id: &str) -> String {
    sample_message(
        id,
        &format!("Subject of {id}"),
        "Ann Example <ann@example.org>",
    )
}

/// The real adapter, except that the first `find_by_message_id` answers "not found" when
/// `miss_once` is set (a server whose search has not caught up with a move yet). Every other call,
/// and every later lookup, goes to the server.
struct Adapter {
    inner: ImapBackend,
    miss_once: AtomicBool,
}

impl MailBackend for Adapter {
    fn capabilities(&self) -> BackendFuture<'_, Capabilities> {
        self.inner.capabilities()
    }
    fn list_folders(&self) -> BackendFuture<'_, Vec<RemoteFolder>> {
        self.inner.list_folders()
    }
    fn folder_status<'a>(&'a self, folder: &'a str) -> BackendFuture<'a, FolderStatus> {
        self.inner.folder_status(folder)
    }
    fn snapshot<'a>(&'a self, folder: &'a str) -> BackendFuture<'a, RemoteSnapshot> {
        self.inner.snapshot(folder)
    }
    fn fetch_headers<'a>(
        &'a self,
        folder: &'a str,
        uids: &'a [Uid],
    ) -> BackendFuture<'a, Vec<RemoteHeader>> {
        self.inner.fetch_headers(folder, uids)
    }
    fn find_by_message_id<'a>(
        &'a self,
        folder: &'a str,
        message_id: &'a str,
    ) -> BackendFuture<'a, Option<Uid>> {
        if self.miss_once.swap(false, Ordering::SeqCst) {
            return Box::pin(async { Ok(None) });
        }
        self.inner.find_by_message_id(folder, message_id)
    }
    fn set_flag<'a>(
        &'a self,
        folder: &'a str,
        uids: &'a [Uid],
        change: FlagChange,
    ) -> BackendFuture<'a, ()> {
        self.inner.set_flag(folder, uids, change)
    }
    fn move_to<'a>(
        &'a self,
        folder: &'a str,
        uids: &'a [Uid],
        dest: &'a str,
    ) -> BackendFuture<'a, MoveOutcome> {
        self.inner.move_to(folder, uids, dest)
    }
    fn delete_forever<'a>(&'a self, folder: &'a str, uids: &'a [Uid]) -> BackendFuture<'a, ()> {
        self.inner.delete_forever(folder, uids)
    }
}

struct ImapRig {
    server: TestServer,
    _dir: tempfile::TempDir,
    path: std::path::PathBuf,
    clock: ManualClock,
    engine: Option<Engine>,
    ui: EngineHandle,
    account: AccountId,
    miss_first_lookup: bool,
}

impl ImapRig {
    async fn new(caps: Caps, miss_first_lookup: bool) -> Self {
        let server = TestServer::start(ServerOptions {
            caps,
            ..ServerOptions::default()
        })
        .await;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("croft.db");
        let db = Db::open(&path, None).unwrap();
        let account = create_account(&db, "alice@example.org");
        drop(db);
        let clock = ManualClock::new(1_000_000);
        let (engine, ui) = Self::start(&server, &path, &clock, account, miss_first_lookup);
        Self {
            server,
            _dir: dir,
            path,
            clock,
            engine: Some(engine),
            ui,
            account,
            miss_first_lookup,
        }
    }

    fn start(
        server: &TestServer,
        path: &std::path::Path,
        clock: &ManualClock,
        account: AccountId,
        miss_first_lookup: bool,
    ) -> (Engine, EngineHandle) {
        let engine = Engine::open(
            Db::open(path, None).unwrap(),
            Arc::new(clock.clone()),
            EngineConfig::default(),
        )
        .unwrap();
        let backend = Adapter {
            inner: imap(server),
            miss_once: AtomicBool::new(miss_first_lookup),
        };
        engine.attach_account(account, Arc::new(backend)).unwrap();
        let ui = engine.handle();
        (engine, ui)
    }

    /// The app is killed: a new engine (and a new connection) on the same database file.
    fn kill_and_restart(&mut self) {
        drop(self.engine.take());
        let (engine, ui) = Self::start(
            &self.server,
            &self.path,
            &self.clock,
            self.account,
            self.miss_first_lookup,
        );
        self.ui = ui;
        self.engine = Some(engine);
    }

    async fn run(&self) {
        self.ui.run_once(self.account, false).await.unwrap();
    }

    async fn sync(&self) {
        self.ui.run_once(self.account, true).await.unwrap();
    }

    fn row(&self, folder: &str, uid: u32) -> Option<Message> {
        let account = self.account;
        self.ui.read(|db| {
            let f = folders::find_by_path(db.connection(), account, folder).unwrap()?;
            messages::find_by_uid(db.connection(), account, f.id, uid).unwrap()
        })
    }

    fn server_flags(&self, folder: &str, id: &str) -> Option<Vec<String>> {
        let want = format!("<{id}>");
        self.server
            .messages(folder)
            .into_iter()
            .find(|m| m.raw.contains(&want))
            .map(|m| m.flags.into_iter().collect())
    }

    fn server_uid(&self, folder: &str, id: &str) -> Option<u32> {
        let want = format!("<{id}>");
        self.server
            .messages(folder)
            .into_iter()
            .find(|m| m.raw.contains(&want))
            .map(|m| m.uid)
    }

    fn stores(&self) -> usize {
        self.server
            .commands()
            .iter()
            .filter(|c| c.to_ascii_uppercase().starts_with("UID STORE"))
            .count()
    }
}

#[tokio::test]
async fn mark_read_kill_restart_reaches_the_imap_server_exactly_once() {
    let mut rig = ImapRig::new(Caps::all(), false).await;
    rig.server.add_message("INBOX", &msg("a@x"));
    rig.sync().await;
    let id = rig.row("INBOX", 1).unwrap().id;
    rig.ui.mark_seen(rig.account, id, true).unwrap();
    rig.kill_and_restart();
    assert!(rig.row("INBOX", 1).unwrap().is_seen);
    rig.server.clear_commands();

    rig.clock.advance(1_500);
    rig.run().await;
    assert_eq!(rig.stores(), 1);
    assert_eq!(
        rig.server_flags("INBOX", "a@x").unwrap(),
        vec!["\\Seen".to_owned()]
    );
    rig.kill_and_restart();
    rig.clock.advance(60_000);
    rig.sync().await;
    rig.run().await;
    assert_eq!(rig.stores(), 1, "exactly once");
    let row = rig.row("INBOX", 1).unwrap();
    assert!(row.is_seen && row.pending_ops == 0);
}

/// Read, archive and star `a@x`; Archive already holds `z@x` so the copy gets UID 2 there.
async fn read_archive_star(rig: &ImapRig) {
    rig.server.add_message("INBOX", &msg("a@x"));
    rig.server.add_message("Archive", &msg("z@x"));
    rig.sync().await;
    let id = rig.row("INBOX", 1).unwrap().id;
    rig.ui.mark_seen(rig.account, id, true).unwrap();
    rig.ui.move_to(rig.account, id, "Archive").unwrap();
    rig.ui.set_flagged(rig.account, id, true).unwrap();
    rig.clock.advance(5_000);
    rig.run().await;
}

fn assert_star_landed_on_the_moved_copy(rig: &ImapRig) {
    let mut flags = rig.server_flags("Archive", "a@x").expect("moved");
    flags.sort();
    assert_eq!(flags, vec!["\\Flagged".to_owned(), "\\Seen".to_owned()]);
    assert!(rig.server_flags("INBOX", "a@x").is_none());
    assert!(rig.server_flags("Archive", "z@x").unwrap().is_empty());
    let uid = rig.server_uid("Archive", "a@x").unwrap();
    let row = rig.row("Archive", uid).unwrap();
    assert_eq!(row.message_id_hdr.as_deref(), Some("<a@x>"));
    assert!(row.is_seen && row.is_flagged && !row.local_hidden && row.pending_ops == 0);
    assert!(rig.row("INBOX", 1).is_none());
}

#[tokio::test]
async fn read_archive_star_with_copyuid_on_the_imap_server() {
    let rig = ImapRig::new(Caps::all(), false).await;
    read_archive_star(&rig).await;
    assert_star_landed_on_the_moved_copy(&rig);
    assert!(rig
        .server
        .commands()
        .iter()
        .any(|c| c.to_ascii_uppercase().starts_with("UID MOVE")));
}

#[tokio::test]
async fn read_archive_star_without_uidplus_on_the_imap_server_finds_the_copy_by_message_id() {
    // No MOVE, no UIDPLUS: the adapter copies, and the copy's UID comes from a SEARCH.
    let rig = ImapRig::new(Caps::bare(), false).await;
    read_archive_star(&rig).await;
    assert_star_landed_on_the_moved_copy(&rig);
    assert!(rig
        .server
        .commands()
        .iter()
        .any(|c| c.to_ascii_uppercase().contains("SEARCH")));
}

#[tokio::test]
async fn read_archive_star_without_uidplus_and_a_uidvalidity_change_on_the_imap_server() {
    let rig = ImapRig::new(Caps::bare(), true).await;
    read_archive_star(&rig).await;
    // The first lookup missed: the star waits. Then Archive gets a new UIDVALIDITY.
    assert!(rig.server_flags("Archive", "a@x").is_some());
    assert!(!rig
        .server_flags("Archive", "a@x")
        .unwrap()
        .contains(&"\\Flagged".to_owned()));
    rig.server.change_uid_validity("Archive", 777);
    rig.sync().await;
    assert_star_landed_on_the_moved_copy(&rig);
    let account = rig.account;
    let validity = rig.ui.read(|db| {
        folders::find_by_path(db.connection(), account, "Archive")
            .unwrap()
            .unwrap()
            .uid_validity
    });
    assert_eq!(validity, Some(777));
}

#[tokio::test]
async fn an_archive_queued_before_the_inbox_was_renumbered_moves_the_right_message() {
    let rig = ImapRig::new(Caps::all(), false).await;
    let x = rig.server.add_message("INBOX", &msg("x@x"));
    rig.server.add_message("INBOX", &msg("a@x"));
    rig.sync().await;
    let a = rig.row("INBOX", 2).unwrap().id;
    rig.ui.move_to(rig.account, a, "Archive").unwrap();
    // During the Undo window another client deletes x@x and the folder is recreated: a@x is
    // UID 1 now.
    rig.server.remove_messages("INBOX", &[x]);
    rig.server.change_uid_validity("INBOX", 4242);
    assert_eq!(rig.server_uid("INBOX", "a@x"), Some(1));
    rig.clock.advance(5_000);
    rig.run().await;
    assert!(rig.server_flags("Archive", "a@x").is_some());
    assert!(rig.server.messages("INBOX").is_empty());
}
