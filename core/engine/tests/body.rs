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
