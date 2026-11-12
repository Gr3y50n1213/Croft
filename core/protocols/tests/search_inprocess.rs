// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Server search (`MailBackend::search`) over IMAP against the in-process server. No network.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use croft_protocols::*;
use croft_test_support::{Misbehavior, TestServer};
use std::time::Duration;

fn backend(server: &TestServer) -> ImapBackend {
    ImapBackend::new(
        ImapConfig::new(
            server.host(),
            server.port(),
            Security::PlaintextForTestsOnly,
            server.username(),
        )
        .with_timeout(Duration::from_secs(5)),
        Credential::password(server.password()),
    )
    .unwrap()
}

fn mail(from: &str, to: &str, subject: &str, body: &str) -> String {
    format!("From: {from}\r\nTo: {to}\r\nSubject: {subject}\r\nMessage-ID: <{subject}@x>\r\n\r\n{body}\r\n")
}

fn found(o: SearchOutcome) -> Vec<Uid> {
    match o {
        SearchOutcome::Found(u) => u,
        SearchOutcome::Unsupported => panic!("unsupported"),
    }
}

#[tokio::test]
async fn from_subject_and_body_text_find_the_right_uids_newest_first() {
    let server = TestServer::start_default().await;
    let a = server.add_message(
        "INBOX",
        &mail("ann@example.org", "me@example.org", "Lunch", "see you"),
    );
    let b = server.add_message(
        "INBOX",
        &mail(
            "ben@example.org",
            "me@example.org",
            "Report",
            "lunch is at noon",
        ),
    );
    let c = server.add_message(
        "INBOX",
        &mail("ann@example.org", "dee@example.org", "Plans", "nothing"),
    );
    let be = backend(&server);
    let q = |f: fn(&mut ServerQuery)| {
        let mut q = ServerQuery::default();
        f(&mut q);
        q
    };
    let text = q(|q| q.text = vec!["lunch".into()]);
    assert_eq!(found(be.search("INBOX", &text).await.unwrap()), vec![b, a]);
    let from = q(|q| q.from = vec!["ann@".into()]);
    assert_eq!(found(be.search("INBOX", &from).await.unwrap()), vec![c, a]);
    let subject = q(|q| q.subject = vec!["report".into()]);
    assert_eq!(found(be.search("INBOX", &subject).await.unwrap()), vec![b]);
    let to = q(|q| q.to = vec!["dee@".into()]);
    assert_eq!(found(be.search("INBOX", &to).await.unwrap()), vec![c]);
    let both = q(|q| {
        q.from = vec!["ann".into()];
        q.text = vec!["see you".into()];
    });
    assert_eq!(found(be.search("INBOX", &both).await.unwrap()), vec![a]);
    let none = q(|q| q.text = vec!["zebra".into()]);
    assert_eq!(
        found(be.search("INBOX", &none).await.unwrap()),
        Vec::<Uid>::new()
    );
    assert_eq!(
        found(be.search("INBOX", &ServerQuery::default()).await.unwrap()),
        Vec::<Uid>::new(),
        "an empty query sends nothing"
    );
}

#[tokio::test]
async fn flags_narrow_the_search() {
    let server = TestServer::start_default().await;
    let read = server.add_message_with_flags(
        "INBOX",
        &mail("ann@example.org", "me@example.org", "One", "budget"),
        &["\\Seen"],
    );
    let unread_starred = server.add_message_with_flags(
        "INBOX",
        &mail("ann@example.org", "me@example.org", "Two", "budget"),
        &["\\Flagged"],
    );
    let be = backend(&server);
    let mut q = ServerQuery {
        text: vec!["budget".into()],
        ..ServerQuery::default()
    };
    assert_eq!(
        found(be.search("INBOX", &q).await.unwrap()),
        vec![unread_starred, read]
    );
    q.unread = true;
    assert_eq!(
        found(be.search("INBOX", &q).await.unwrap()),
        vec![unread_starred]
    );
    q.unread = false;
    q.starred = true;
    assert_eq!(
        found(be.search("INBOX", &q).await.unwrap()),
        vec![unread_starred]
    );
}

#[tokio::test]
async fn a_server_without_text_search_is_unsupported_not_an_error() {
    let server = TestServer::start_default().await;
    server.add_message(
        "INBOX",
        &mail("ann@example.org", "me@example.org", "Lunch", "x"),
    );
    server.set_misbehavior(Misbehavior::NoTextSearch);
    let be = backend(&server);
    let q = ServerQuery {
        text: vec!["lunch".into()],
        ..ServerQuery::default()
    };
    assert_eq!(
        be.search("INBOX", &q).await.unwrap(),
        SearchOutcome::Unsupported
    );
    // The connection is still fine afterwards.
    assert!(be.folder_status("INBOX").await.is_ok());
}

#[tokio::test]
async fn a_word_this_adapter_cannot_send_is_unsupported_and_nothing_is_sent() {
    let server = TestServer::start_default().await;
    let be = backend(&server);
    let q = ServerQuery {
        text: vec!["caf\u{e9}".into()],
        ..ServerQuery::default()
    };
    assert_eq!(
        be.search("INBOX", &q).await.unwrap(),
        SearchOutcome::Unsupported
    );
}
