// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The sample mailbox through the engine: a full sync, every action, a send, and syncs after them,
//! all on a runtime **without an I/O driver**.
//!
//! Why this shape of proof: the engine never creates a connection itself. It only calls the
//! `MailBackend` and `MailSubmitter` it is given, and only the IMAP, JMAP and SMTP adapters open
//! sockets (all through tokio). There is no connector or dialer to inject, so instead every step
//! runs on a tokio runtime built without `enable_io`: any TCP, UDP or DNS call made through tokio
//! panics there ("IO is disabled"), so a passing test means none was made. The demo module's own
//! test checks that its source names no socket API at all (`std::net` included).

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::sync::Arc;

use common::create_account;
use croft_engine::{AccountId, Engine, EngineConfig, EngineHandle, ManualClock, NewMessage};
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
    _dir: tempfile::TempDir,
    clock: ManualClock,
    backend: Arc<DemoBackend>,
    _engine: Engine,
    ui: EngineHandle,
    account: AccountId,
}

impl Rig {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("croft.db");
        let db = Db::open(&path, None).unwrap();
        let account = create_account(&db, "robin@example.org");
        let clock = ManualClock::new(START_MS);
        let engine = Engine::open_with_outbox(
            db,
            Db::open(&path, None).unwrap(),
            Arc::new(clock.clone()),
            EngineConfig::default(),
        )
        .unwrap();
        let backend = Arc::new(DemoBackend::new(START_MS));
        engine
            .attach_account_with_submitter(account, backend.clone(), Arc::new(backend.submitter()))
            .unwrap();
        let ui = engine.handle();
        Self {
            _dir: dir,
            clock,
            backend,
            _engine: engine,
            ui,
            account,
        }
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
