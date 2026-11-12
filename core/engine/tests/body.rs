// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! `EngineHandle::fetch_body` over the sample mailbox: fetched once, parsed, sanitised and stored
//! (remote images blocked, attachments listed), then available offline; never marks the message
//! read; scoped by account; and its errors carry no message content.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::sync::Arc;

use common::create_account;
use croft_engine::{BodyError, Engine, EngineConfig, EngineHandle, ManualClock};
use croft_protocols::demo::INBOX;
use croft_protocols::{
    BackendFuture, Capabilities, FlagChange, FolderStatus, MoveOutcome, RawMessage, RemoteFolder,
    RemoteHeader, RemoteSnapshot, Uid,
};
use croft_protocols::{DemoBackend, MailBackend};
use croft_storage::messages::{self, Message};
use croft_storage::{folders, reading, Db};

const START_MS: i64 = 1_794_398_400_000;

struct Rig {
    _dir: tempfile::TempDir,
    _engine: Engine,
    backend: Arc<DemoBackend>,
    ui: EngineHandle,
    account: i64,
}

async fn rig() -> Rig {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("croft.db");
    let account = create_account(&Db::open(&path, None).unwrap(), "robin@example.org");
    let engine = Engine::open(
        Db::open(&path, None).unwrap(),
        Arc::new(ManualClock::new(START_MS)),
        EngineConfig::default(),
    )
    .unwrap();
    let backend = Arc::new(DemoBackend::new(START_MS));
    engine.attach_account(account, backend.clone()).unwrap();
    let ui = engine.handle();
    ui.run_once(account, true).await.unwrap();
    Rig {
        _dir: dir,
        _engine: engine,
        backend,
        ui,
        account,
    }
}

fn inbox(r: &Rig) -> Vec<Message> {
    r.ui.read(|db| {
        let f = folders::find_by_path(db.connection(), r.account, INBOX)
            .unwrap()
            .unwrap();
        messages::list_by_folder(db.connection(), r.account, f.id, i64::MAX, 500, 0).unwrap()
    })
}

/// The sample message with an attachment, and the one with a remote image.
fn pick(r: &Rig) -> (Message, Message) {
    let mut with_attachment = None;
    let mut with_image = None;
    for m in inbox(r) {
        let raw = r
            .backend
            .message_source(INBOX, u32::try_from(m.uid).unwrap())
            .unwrap_or_default()
            .to_lowercase();
        if raw.contains("content-disposition: attachment") {
            with_attachment = Some(m.clone());
        }
        if raw.contains("<img") && raw.contains("https://") {
            with_image = Some(m);
        }
    }
    (with_attachment.unwrap(), with_image.unwrap())
}

#[tokio::test]
async fn a_body_is_fetched_once_stored_and_then_read_offline() {
    let r = rig().await;
    let (attached, imaged) = pick(&r);
    assert_eq!(attached.body_state, "none");
    r.ui.fetch_body(r.account, attached.id).await.unwrap();

    let (body, files, row) = r.ui.read(|db| {
        let c = db.connection();
        (
            reading::body(c, r.account, attached.id).unwrap().unwrap(),
            reading::attachments(c, r.account, attached.id).unwrap(),
            messages::get(c, r.account, attached.id).unwrap().unwrap(),
        )
    });
    assert!(body.text.is_some() || body.html_clean.is_some());
    assert!(!files.is_empty());
    assert!(files.iter().all(|f| !f.file_name.contains('/')));
    assert_eq!(row.body_state, "full");
    assert!(row.has_attachments);
    assert!(!row.snippet.is_empty());

    // Remote images are blocked in what is stored.
    r.ui.fetch_body(r.account, imaged.id).await.unwrap();
    let body = r.ui.read(|db| {
        reading::body(db.connection(), r.account, imaged.id)
            .unwrap()
            .unwrap()
    });
    let html = body.html_clean.unwrap();
    assert!(body.remote_blocked >= 1, "{html}");
    assert!(!html.contains("src=\"https://"), "{html}");

    // Offline, a stored body is still there; an unstored one says why.
    r.ui.set_online(r.account, false);
    r.ui.fetch_body(r.account, attached.id).await.unwrap();
    let unstored = inbox(&r)
        .into_iter()
        .find(|m| m.body_state == "none")
        .unwrap();
    assert!(matches!(
        r.ui.fetch_body(r.account, unstored.id).await,
        Err(BodyError::Offline)
    ));
}

#[tokio::test]
async fn fetching_never_marks_read_and_is_scoped_by_account() {
    let r = rig().await;
    let unread = inbox(&r).into_iter().find(|m| !m.is_seen).unwrap();
    r.ui.fetch_body(r.account, unread.id).await.unwrap();
    let snap = r.backend.snapshot(INBOX).await.unwrap();
    assert!(!snap.messages[&u32::try_from(unread.uid).unwrap()].seen);
    assert!(!r
        .ui
        .read(|db| messages::get(db.connection(), r.account, unread.id)
            .unwrap()
            .unwrap()
            .is_seen));

    assert!(matches!(
        r.ui.fetch_body(r.account + 1, unread.id).await,
        Err(BodyError::NoSuchMessage)
    ));
    assert!(matches!(
        r.ui.fetch_body(r.account, 999_999).await,
        Err(BodyError::NoSuchMessage)
    ));
}

#[tokio::test]
async fn a_message_gone_from_the_server_says_so() {
    let r = rig().await;
    let m = inbox(&r)[0].clone();
    r.backend
        .delete_forever(INBOX, &[u32::try_from(m.uid).unwrap()])
        .await
        .unwrap();
    assert!(matches!(
        r.ui.fetch_body(r.account, m.id).await,
        Err(BodyError::NotOnServer)
    ));
}

#[test]
fn the_reader_and_the_protocol_layer_agree_on_the_size_limit() {
    assert_eq!(
        croft_reader::MAX_MESSAGE_BYTES,
        croft_protocols::MAX_MESSAGE_BYTES
    );
}

#[test]
fn body_errors_are_fixed_wording() {
    for e in [
        BodyError::NoSuchMessage,
        BodyError::Offline,
        BodyError::NeedsSignIn,
        BodyError::NotOnServer,
        BodyError::TooLarge { size: 9 },
        BodyError::Unreadable,
    ] {
        let text = e.to_string();
        assert!(!text.contains('@') && !text.contains('<'), "{text}");
    }
}

/// The sample mailbox, counting body fetches and making each take a little while.
struct Counting {
    inner: Arc<DemoBackend>,
    fetches: std::sync::atomic::AtomicUsize,
    /// Served instead of the sample message, when set.
    raw: Option<Vec<u8>>,
}

impl MailBackend for Counting {
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
    fn fetch_body<'a>(&'a self, folder: &'a str, uid: Uid) -> BackendFuture<'a, RawMessage> {
        self.fetches
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Box::pin(async move {
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            match &self.raw {
                Some(raw) => Ok(RawMessage::Found(raw.clone())),
                None => self.inner.fetch_body(folder, uid).await,
            }
        })
    }
}

/// An engine over the sample mailbox through [`Counting`], serving `raw` for every body.
async fn counting_rig(
    raw: Option<Vec<u8>>,
) -> (tempfile::TempDir, Engine, Arc<Counting>, EngineHandle, i64) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("croft.db");
    let account = create_account(&Db::open(&path, None).unwrap(), "robin@example.org");
    let engine = Engine::open(
        Db::open(&path, None).unwrap(),
        Arc::new(ManualClock::new(START_MS)),
        EngineConfig::default(),
    )
    .unwrap();
    let backend = Arc::new(Counting {
        inner: Arc::new(DemoBackend::new(START_MS)),
        fetches: std::sync::atomic::AtomicUsize::new(0),
        raw,
    });
    engine.attach_account(account, backend.clone()).unwrap();
    let ui = engine.handle();
    ui.run_once(account, true).await.unwrap();
    (dir, engine, backend, ui, account)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn two_loads_of_one_message_fetch_and_sanitise_it_once() {
    // Round 3 review L3: the second waits for the first, then finds the body stored.
    let (_dir, _engine, backend, ui, account) = counting_rig(None).await;
    let m = ui.read(|db| {
        let f = folders::find_by_path(db.connection(), account, INBOX)
            .unwrap()
            .unwrap();
        messages::list_by_folder(db.connection(), account, f.id, i64::MAX, 500, 0).unwrap()
    })[0]
        .clone();
    let (a, b) = tokio::join!(ui.fetch_body(account, m.id), ui.fetch_body(account, m.id));
    a.unwrap();
    b.unwrap();
    assert_eq!(
        backend.fetches.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "fetched once"
    );
    // Other messages are not held up by it: two different ones fetch both.
    let others = ui.read(|db| {
        let f = folders::find_by_path(db.connection(), account, INBOX)
            .unwrap()
            .unwrap();
        messages::list_by_folder(db.connection(), account, f.id, i64::MAX, 500, 0).unwrap()
    });
    let (c, d) = tokio::join!(
        ui.fetch_body(account, others[1].id),
        ui.fetch_body(account, others[2].id)
    );
    c.unwrap();
    d.unwrap();
    assert_eq!(backend.fetches.load(std::sync::atomic::Ordering::SeqCst), 3);
}

#[tokio::test]
async fn a_body_whose_html_was_pending_is_rendered_on_the_next_open() {
    // Round 3 review M1/H6: a busy sanitiser or a missed deadline stores the text with
    // `html_pending`, never `too_complex`; the next fetch renders the HTML from the stored
    // message (offline) and keeps the attachment rows.
    let r = rig().await;
    let (_, imaged) = pick(&r);
    r.ui.fetch_body(r.account, imaged.id).await.unwrap();
    let rendered = r.ui.read(|db| {
        reading::body(db.connection(), r.account, imaged.id)
            .unwrap()
            .unwrap()
    });
    assert!(rendered.html_clean.is_some() && !rendered.html_pending);
    // As if the first attempt had not run: HTML missing, pending.
    r.ui.read(|db| {
        db.connection()
            .execute(
                "UPDATE message_body SET html_clean = NULL, html_pending = 1, truncated = 1
                  WHERE message_id = ?1",
                [imaged.id],
            )
            .unwrap();
    });
    let attachments =
        r.ui.read(|db| reading::attachments(db.connection(), r.account, imaged.id).unwrap());
    r.ui.set_online(r.account, false);
    r.ui.fetch_body(r.account, imaged.id).await.unwrap();
    let again = r.ui.read(|db| {
        reading::body(db.connection(), r.account, imaged.id)
            .unwrap()
            .unwrap()
    });
    assert!(!again.html_pending && !again.too_complex);
    assert_eq!(again.html_clean, rendered.html_clean);
    assert_eq!(again.remote_blocked, rendered.remote_blocked);
    assert_eq!(
        r.ui.read(|db| reading::attachments(db.connection(), r.account, imaged.id).unwrap()),
        attachments
    );
}

#[tokio::test]
async fn html_past_the_work_budget_is_marked_and_never_parsed_again() {
    // Round 3 review M1: the budget's verdict is deterministic, so it is stored; later opens show
    // the text without parsing again.
    let bomb = format!(
        "<div><font title=\"{}\"></div>{}",
        "X".repeat(50_000),
        "<p>x</p>".repeat(20_000)
    );
    let raw = format!(
        "From: x@example.org\r\nSubject: Bomb\r\nContent-Type: text/html\r\n\r\n{bomb}\r\n"
    )
    .into_bytes();
    let (_dir, _engine, backend, ui, account) = counting_rig(Some(raw)).await;
    let m = ui.read(|db| {
        let f = folders::find_by_path(db.connection(), account, INBOX)
            .unwrap()
            .unwrap();
        messages::list_by_folder(db.connection(), account, f.id, i64::MAX, 500, 0).unwrap()
    })[0]
        .clone();
    ui.fetch_body(account, m.id).await.unwrap();
    let stored = ui.read(|db| {
        reading::body(db.connection(), account, m.id)
            .unwrap()
            .unwrap()
    });
    assert!(stored.too_complex && !stored.html_pending && stored.html_clean.is_none());
    assert!(stored.truncated && stored.text.is_some());
    // The next open reads the stored body: no fetch, no parse (a pending body would be parsed
    // again; a marked one never is).
    ui.fetch_body(account, m.id).await.unwrap();
    assert_eq!(backend.fetches.load(std::sync::atomic::Ordering::SeqCst), 1);
    assert!(!ui.read(|db| reading::html_pending(db.connection(), account, m.id).unwrap()));
}
