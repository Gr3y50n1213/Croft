// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! `EngineHandle::search_server` against the in-process IMAP server: hits already on the device
//! come back as their rows, hits that are not get their headers stored first, and a server that
//! cannot search, an offline device and another account's folders are handled. No network.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::sync::Arc;
use std::time::Duration;

use common::create_account;
use croft_engine::{Engine, EngineConfig, ManualClock, SearchError};
use croft_protocols::{Credential, ImapBackend, ImapConfig, Security, ServerQuery};
use croft_storage::{folders, messages, Db};
use croft_test_support::{sample_message, Misbehavior, TestServer};

fn imap(server: &TestServer) -> ImapBackend {
    let config = ImapConfig::new(
        server.host(),
        server.port(),
        Security::PlaintextForTestsOnly,
        server.username(),
    )
    .with_timeout(Duration::from_secs(5));
    ImapBackend::new(config, Credential::password(server.password())).unwrap()
}

fn text(t: &str) -> ServerQuery {
    ServerQuery {
        text: vec![t.into()],
        ..ServerQuery::default()
    }
}

#[tokio::test]
async fn hits_come_back_as_rows_and_new_ones_are_stored_first() {
    let server = TestServer::start_default().await;
    server.add_message(
        "INBOX",
        &sample_message("a@x", "Lunch plans", "Ann <ann@example.org>"),
    );
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("croft.db");
    let account = create_account(&Db::open(&path, None).unwrap(), "me@example.org");
    let engine = Engine::open(
        Db::open(&path, None).unwrap(),
        Arc::new(ManualClock::new(1_000_000)),
        EngineConfig::default(),
    )
    .unwrap();
    engine
        .attach_account(account, Arc::new(imap(&server)))
        .unwrap();
    let ui = engine.handle();
    ui.run_once(account, true).await.unwrap();
    let inbox = ui.read(|db| {
        folders::find_by_path(db.connection(), account, "INBOX")
            .unwrap()
            .unwrap()
            .id
    });
    // A message the device has not synced yet.
    let late = server.add_message(
        "INBOX",
        &sample_message("b@x", "Lunch again", "Ben <ben@example.org>"),
    );
    assert!(ui
        .read(|db| messages::find_by_uid(db.connection(), account, inbox, late).unwrap())
        .is_none());

    let ids = ui
        .search_server(account, &[inbox], &text("lunch"))
        .await
        .unwrap();
    assert_eq!(ids.len(), 2, "both hits");
    let stored = ui
        .read(|db| messages::find_by_uid(db.connection(), account, inbox, late).unwrap())
        .expect("the late hit is now a row");
    assert_eq!(ids[0], stored.id, "newest first");
    assert_eq!(stored.subject, "Lunch again");

    // Another account's folder ids are ignored, and an empty query sends nothing.
    assert!(ui
        .search_server(account, &[inbox + 1000], &text("lunch"))
        .await
        .unwrap_err()
        .to_string()
        .contains("cannot search"));
    assert!(ui
        .search_server(account, &[inbox], &ServerQuery::default())
        .await
        .unwrap()
        .is_empty());

    // A server without text search: a typed answer, not a failure.
    server.set_misbehavior(Misbehavior::NoTextSearch);
    assert!(matches!(
        ui.search_server(account, &[inbox], &text("lunch")).await,
        Err(SearchError::Unsupported)
    ));

    // Offline: nothing is sent.
    ui.set_online(account, false);
    assert!(matches!(
        ui.search_server(account, &[inbox], &text("lunch")).await,
        Err(SearchError::Offline)
    ));
}
