// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! T38a: the CardDAV client against the in-process fake: discovery, sync with and without a
//! token, multiget, conditional writes, conflicts, and a server that points elsewhere.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::time::Duration;

use croft_protocols::carddav::{CardDav, CardDavConfig, WriteOutcome};
use croft_protocols::Credential;
use croft_test_support::carddav::{DavServer, BOOK, DAV_PASSWORD, DAV_USER};
use zeroize::Zeroizing;

fn card(uid: &str, name: &str) -> String {
    format!("BEGIN:VCARD\r\nVERSION:4.0\r\nUID:{uid}\r\nFN:{name}\r\nEND:VCARD\r\n")
}

fn client(server: &DavServer, password: &str) -> CardDav {
    CardDav::new(
        &CardDavConfig {
            base_url: server.origin(),
            username: DAV_USER.into(),
            pinned_cert: None,
            timeout: Duration::from_secs(5),
            deadline: Duration::from_secs(10),
        },
        &Credential::Password(Zeroizing::new(password.into())),
    )
    .unwrap()
}

#[tokio::test]
async fn discovery_finds_the_address_book_through_the_well_known_redirect() {
    let server = DavServer::start().await;
    let books = client(&server, DAV_PASSWORD).discover().await.unwrap();
    assert_eq!(books.len(), 1);
    assert_eq!(books[0].path, BOOK);
    assert_eq!(books[0].name, "Contacts");
    assert_eq!(books[0].sync_token.as_deref(), Some("tok-0"));
    let paths: Vec<String> = server.requests().into_iter().map(|r| r.path).collect();
    assert_eq!(
        paths,
        [
            "/.well-known/carddav",
            "/dav/",
            "/dav/principals/alice/",
            "/dav/books/alice/"
        ]
    );
    // A wrong password is a sign-in failure, not "no contacts".
    let err = client(&server, "wrong").discover().await.unwrap_err();
    assert!(
        matches!(err, croft_protocols::BackendError::Auth(_)),
        "{err:?}"
    );
}

#[tokio::test]
async fn a_principal_on_another_server_is_never_contacted() {
    let server = DavServer::start().await;
    server.set_principal_href("https://evil.example/principal/");
    // Treated as a server without CardDAV: no address books, and nothing sent anywhere else.
    let books = client(&server, DAV_PASSWORD).discover().await.unwrap();
    assert!(books.is_empty());
    assert!(server
        .requests()
        .iter()
        .all(|r| !r.path.contains("principal/")));
}

#[tokio::test]
async fn sync_lists_changes_since_a_token_and_falls_back_to_a_full_listing() {
    let server = DavServer::start().await;
    server.put_card("a.vcf", &card("a", "Ann"));
    server.put_card("b.vcf", &card("b", "Bo"));
    let dav = client(&server, DAV_PASSWORD);
    let first = dav.changes(BOOK, None).await.unwrap();
    assert!(first.full);
    assert_eq!(first.changed.len(), 2);
    let token = dav.discover().await.unwrap()[0].sync_token.clone().unwrap();

    server.put_card("c.vcf", &card("c", "Cy"));
    server.remove_card("a.vcf");
    let delta = dav.changes(BOOK, Some(&token)).await.unwrap();
    assert!(!delta.full);
    assert_eq!(
        delta
            .changed
            .iter()
            .map(|c| c.path.as_str())
            .collect::<Vec<_>>(),
        [format!("{BOOK}c.vcf")]
    );
    assert_eq!(delta.removed, [format!("{BOOK}a.vcf")]);
    assert!(delta.sync_token.is_some());

    // An unknown token, or a server without sync: every card again.
    let full = dav.changes(BOOK, Some("tok-999")).await.unwrap();
    assert!(full.full && full.changed.len() == 2);
    server.set_sync(false);
    let full = dav.changes(BOOK, Some(&token)).await.unwrap();
    assert!(full.full && full.changed.len() == 2);

    let fetched = dav
        .fetch(
            BOOK,
            &full
                .changed
                .iter()
                .map(|c| c.path.clone())
                .collect::<Vec<_>>(),
        )
        .await
        .unwrap();
    assert_eq!(fetched.len(), 2);
    assert!(fetched.iter().any(|c| c.text.contains("FN:Bo")));
    assert!(fetched.iter().all(|c| c.etag.starts_with('"')));
}

#[tokio::test]
async fn writes_are_conditional_on_the_etag() {
    let server = DavServer::start().await;
    let dav = client(&server, DAV_PASSWORD);
    let path = format!("{BOOK}new.vcf");
    assert_eq!(
        dav.put(&path, &card("n", "New"), None).await.unwrap(),
        WriteOutcome::Done
    );
    // Creating again over an existing card is a conflict, never an overwrite.
    assert_eq!(
        dav.put(&path, &card("n", "Other"), None).await.unwrap(),
        WriteOutcome::Conflict
    );
    let etag = dav.changes(BOOK, None).await.unwrap().changed[0]
        .etag
        .clone();
    // Someone else changes it: our update with the old etag is refused.
    server.put_card("new.vcf", &card("n", "Theirs"));
    assert_eq!(
        dav.put(&path, &card("n", "Mine"), Some(&etag))
            .await
            .unwrap(),
        WriteOutcome::Conflict
    );
    assert!(server.cards()[0].1.contains("Theirs"));
    let etag = dav.changes(BOOK, None).await.unwrap().changed[0]
        .etag
        .clone();
    assert_eq!(
        dav.put(&path, &card("n", "Mine"), Some(&etag))
            .await
            .unwrap(),
        WriteOutcome::Done
    );
    assert_eq!(
        dav.delete(&path, &etag).await.unwrap(),
        WriteOutcome::Conflict,
        "the etag moved on"
    );
    let etag = dav.changes(BOOK, None).await.unwrap().changed[0]
        .etag
        .clone();
    assert_eq!(dav.delete(&path, &etag).await.unwrap(), WriteOutcome::Done);
    assert_eq!(dav.delete(&path, &etag).await.unwrap(), WriteOutcome::Gone);
    assert!(server.cards().is_empty());
    // The conditions went as headers; the credential never shows in what the server recorded.
    let puts: Vec<_> = server
        .requests()
        .into_iter()
        .filter(|r| r.method == "PUT")
        .collect();
    assert!(puts[0]
        .headers
        .iter()
        .any(|(k, v)| k == "if-none-match" && v == "*"));
    assert!(puts[2].headers.iter().any(|(k, _)| k == "if-match"));
}
