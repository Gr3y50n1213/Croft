// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The sample mailbox through the engine: a full sync, every action, a send, and syncs after them,
//! all on a runtime **without an I/O driver**.
//!
//! Why this shape of proof: the engine never creates a connection itself. It only calls the
//! `MailBackend` and `MailSubmitter` it is given, and only the IMAP, JMAP and SMTP adapters open
//! sockets (all through tokio). There is no connector or dialer to inject, so instead every step
//! runs on a tokio runtime built without `enable_io`: a tokio TCP or UDP socket panics there ("IO
//! is disabled"), so a passing test means no tokio socket was opened. This runtime does **not**
//! catch a `std::net` socket or a name lookup (`tokio::net::lookup_host` runs on the blocking pool,
//! and `TcpStream::connect` with a host name resolves it before it panics). Those are covered by the
//! demo module's own test, which fails if its code names a socket or lookup API at all.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::sync::Arc;

use common::create_account;
use croft_engine::{
    resume_demo, AccountId, Clock, Engine, EngineConfig, EngineHandle, ManualClock, NewMessage,
};
use croft_protocols::demo::{ARCHIVE, INBOX, SENT, TRASH};
use croft_protocols::{DemoBackend, MailBackend};
use croft_storage::messages::{self, Message};
use croft_storage::{folders, Db};

const START_MS: i64 = 1_794_398_400_000;

/// A runtime with a timer and **no I/O driver**.
fn no_network_runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .unwrap()
}

struct Rig {
    dir: tempfile::TempDir,
    clock: ManualClock,
    backend: Arc<DemoBackend>,
    engine: Option<Engine>,
    ui: EngineHandle,
    account: AccountId,
}

/// Opens the engine (with an Outbox) on the database file and attaches the sample mailbox with
/// the engine's sample-mailbox submitter.
fn start(
    path: &std::path::Path,
    clock: &ManualClock,
    account: AccountId,
    backend: Arc<DemoBackend>,
) -> (Engine, EngineHandle) {
    let engine = Engine::open_with_outbox(
        Db::open(path, None).unwrap(),
        Db::open(path, None).unwrap(),
        Arc::new(clock.clone()),
        EngineConfig::default(),
    )
    .unwrap();
    let ui = engine.handle();
    engine
        .attach_account_with_submitter(
            account,
            backend.clone(),
            ui.demo_submitter(account, backend),
        )
        .unwrap();
    (engine, ui)
}

impl Rig {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("croft.db");
        let account = create_account(&Db::open(&path, None).unwrap(), "robin@example.org");
        let clock = ManualClock::new(START_MS);
        let backend = Arc::new(DemoBackend::new(START_MS));
        let (engine, ui) = start(&path, &clock, account, backend.clone());
        Self {
            dir,
            clock,
            backend,
            engine: Some(engine),
            ui,
            account,
        }
    }

    /// The app is killed and started again: the engine is dropped without a shutdown, and the
    /// sample mailbox is rebuilt from what the device stored, as the FFI does at `Core::open`.
    fn kill_and_restart(&mut self) {
        drop(self.engine.take());
        let path = self.dir.path().join("croft.db");
        let backend = Arc::new(
            resume_demo(
                &Db::open(&path, None).unwrap(),
                self.account,
                self.clock.now_ms(),
            )
            .unwrap(),
        );
        let (engine, ui) = start(&path, &self.clock, self.account, backend.clone());
        self.backend = backend;
        self.ui = ui;
        self.engine = Some(engine);
    }

    async fn sync(&self) {
        self.ui.run_once(self.account, true).await.unwrap();
    }

    /// Visible rows of a folder, newest first.
    fn rows(&self, path: &str) -> Vec<Message> {
        let account = self.account;
        self.ui.read(|db| {
            let f = folders::find_by_path(db.connection(), account, path)
                .unwrap()
                .unwrap();
            messages::list_by_folder(db.connection(), account, f.id, i64::MAX, 500, 0).unwrap()
        })
    }

    fn by_id(&self, path: &str, id: &str) -> Message {
        self.rows(path)
            .into_iter()
            .find(|m| m.message_id_hdr.as_deref() == Some(id))
            .unwrap_or_else(|| panic!("{id} not in {path}"))
    }
}

fn id(local: &str) -> String {
    format!("<{local}@sample.example.org>")
}

#[test]
fn a_full_sync_every_action_and_a_send_need_no_network_and_stay_after_later_syncs() {
    no_network_runtime().block_on(async {
        let r = Rig::new();
        r.sync().await;
        assert_eq!(r.rows(INBOX).len(), 10);
        assert_eq!(r.rows(SENT).len(), 3);
        assert_eq!(r.rows(TRASH).len(), 2);
        assert_eq!(r.rows(ARCHIVE).len(), 3);

        // Read, star, archive, move, delete forever: every action of the handle.
        let welcome = r.by_id(INBOX, &id("welcome"));
        assert!(!welcome.is_seen);
        r.ui.mark_seen(r.account, welcome.id, true).unwrap();
        let bookclub = r.by_id(INBOX, &id("bookclub"));
        r.ui.set_flagged(r.account, bookclub.id, true).unwrap();
        let ferry = r.by_id(INBOX, &id("ferry"));
        r.ui.move_to(r.account, ferry.id, ARCHIVE).unwrap();
        let pottery = r.by_id(INBOX, &id("pottery"));
        r.ui.move_to(r.account, pottery.id, TRASH).unwrap();
        let sale = r.by_id(TRASH, &id("tools-sale"));
        r.ui.delete_forever(r.account, sale.id).unwrap();

        // A message written in the sample mailbox.
        let mime = "Message-ID: <reply-1@example.org>\r\n\
                    Date: Wed, 11 Nov 2026 12:00:00 +0000\r\n\
                    From: Robin Sample <robin@example.org>\r\n\
                    To: Lena Hoffmann <lena@example.org>\r\n\
                    Subject: Re: Book club: vote for next month's pick\r\n\
                    In-Reply-To: <bookclub@sample.example.org>\r\n\
                    \r\n\
                    Number 2, please.\r\n";
        r.ui.send(
            NewMessage {
                account: r.account,
                identity: None,
                draft: None,
                envelope_from: "robin@example.org".into(),
                recipients: vec!["lena@example.org".into()],
            },
            mime.as_bytes().to_vec(),
        )
        .unwrap();

        // Past every wait (1.5 s for flags, the Undo window for moves and the send): everything
        // is confirmed by the sample "server".
        r.clock.advance(60_000);
        r.sync().await;
        r.sync().await;
        assert!(
            r.ui.outbox(r.account).is_empty(),
            "nothing waits to be sent"
        );

        let check = |r: &Rig| {
            assert!(r.by_id(INBOX, &id("welcome")).is_seen);
            assert!(r.by_id(INBOX, &id("bookclub")).is_flagged);
            assert_eq!(r.rows(INBOX).len(), 8);
            r.by_id(ARCHIVE, &id("ferry"));
            r.by_id(TRASH, &id("pottery"));
            assert!(r
                .rows(TRASH)
                .iter()
                .all(|m| m.message_id_hdr.as_deref() != Some(id("tools-sale").as_str())));
            let sent = r.by_id(SENT, "<reply-1@example.org>");
            assert!(sent.is_seen);
            for path in [INBOX, ARCHIVE, TRASH, SENT] {
                assert!(
                    r.rows(path).iter().all(|m| m.pending_ops == 0),
                    "{path}: every change is confirmed"
                );
            }
        };
        check(&r);

        // Later syncs change nothing the person did.
        r.clock.advance(15 * 60_000);
        r.sync().await;
        check(&r);

        // The sample "server" agrees with the device.
        let snap = r.backend.snapshot(INBOX).await.unwrap();
        let welcome = r.by_id(INBOX, &id("welcome"));
        assert!(snap.messages[&u32::try_from(welcome.uid).unwrap()].seen);
    });
}

const REPLY_MIME: &str = "Message-ID: <reply-2@example.org>\r\n\
                          Date: Wed, 11 Nov 2026 12:00:00 +0000\r\n\
                          From: Robin Sample <robin@example.org>\r\n\
                          To: Maya Okafor <maya@example.org>\r\n\
                          Subject: Re: Weekend hike on Saturday?\r\n\
                          \r\n\
                          See you at 8:30.\r\n";

fn envelope(account: AccountId) -> NewMessage {
    NewMessage {
        account,
        identity: None,
        draft: None,
        envelope_from: "robin@example.org".into(),
        recipients: vec!["maya@example.org".into()],
    }
}

/// A message sent from the sample mailbox is stored in the local Sent folder before the Outbox
/// lets go of it, so an app killed before the next sync still has it after the restart (the
/// in-memory "server" is rebuilt from the device).
#[test]
fn a_sent_message_is_in_sent_after_a_restart_before_any_sync() {
    no_network_runtime().block_on(async {
        let mut r = Rig::new();
        r.sync().await;
        r.ui.send(envelope(r.account), REPLY_MIME.as_bytes().to_vec())
            .unwrap();
        r.clock.advance(60_000);
        // Operations and the Outbox only: no sync.
        r.ui.run_once(r.account, false).await.unwrap();
        assert!(r.ui.outbox(r.account).is_empty(), "sent and filed");
        let stored = r.by_id(SENT, "<reply-2@example.org>");
        assert!(stored.is_seen);

        r.kill_and_restart();
        assert_eq!(r.by_id(SENT, "<reply-2@example.org>").uid, stored.uid);
        let in_memory = r.backend.snapshot(SENT).await.unwrap();
        assert!(in_memory
            .messages
            .contains_key(&u32::try_from(stored.uid).unwrap()));
        r.sync().await;
        let copies = r
            .rows(SENT)
            .into_iter()
            .filter(|m| m.message_id_hdr.as_deref() == Some("<reply-2@example.org>"))
            .count();
        assert_eq!(copies, 1, "kept, and once");
        assert_eq!(r.rows(SENT).len(), 4);
    });
}

/// The same when the Sent folder was never synced: the copy creates it on the device.
#[test]
fn a_message_sent_before_the_first_sync_is_kept_too() {
    no_network_runtime().block_on(async {
        let mut r = Rig::new();
        r.ui.send(envelope(r.account), REPLY_MIME.as_bytes().to_vec())
            .unwrap();
        r.clock.advance(60_000);
        r.ui.run_once(r.account, false).await.unwrap();
        assert!(r.ui.outbox(r.account).is_empty());
        r.by_id(SENT, "<reply-2@example.org>");
        r.kill_and_restart();
        r.sync().await;
        r.by_id(SENT, "<reply-2@example.org>");
        assert_eq!(r.rows(SENT).len(), 4, "the built-in three and the new one");
    });
}
