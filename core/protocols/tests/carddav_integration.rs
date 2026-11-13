// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! T38a: the CardDAV client against a real Stalwart server in a container (Stalwart serves
//! CardDAV beside JMAP). Only with `--features integration`, and with Stalwart running and set up
//! (see `jmap_integration.rs` and `test-servers/stalwart/setup.sh`). Plain HTTP on localhost
//! (`CROFT_TEST_STALWART_HTTP_PORT`, default 8080), which the client allows only for loopback.
#![cfg(feature = "integration")]
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use croft_protocols::carddav::{CardDav, CardDavConfig, WriteOutcome};
use croft_protocols::{BackendError, Credential};
use zeroize::Zeroizing;

const ALICE: &str = "alice@example.org";
const ALICE_PW: &str = "croft-alice-purple-orbit-7421";

fn client(password: &str) -> CardDav {
    let port = std::env::var("CROFT_TEST_STALWART_HTTP_PORT")
        .ok()
        .and_then(|v| v.parse::<u16>().ok())
        .unwrap_or(8080);
    CardDav::new(
        &CardDavConfig {
            base_url: format!("http://127.0.0.1:{port}"),
            username: ALICE.into(),
            pinned_cert: None,
            timeout: Duration::from_secs(10),
            deadline: Duration::from_secs(30),
        },
        &Credential::Password(Zeroizing::new(password.into())),
    )
    .unwrap()
}

#[tokio::test]
async fn stalwart_carddav_discovery_sync_and_conditional_writes() {
    let dav = client(ALICE_PW);
    let books = dav.discover().await.unwrap();
    assert!(!books.is_empty(), "Stalwart offers an address book");
    let book = books[0].path.clone();
    let start = dav.changes(&book, None).await.unwrap();
    let token = dav.discover().await.unwrap()[0].sync_token.clone();

    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let uid = format!("croft-test-{nanos}");
    let path = format!("{}/{uid}.vcf", book.trim_end_matches('/'));
    let card = format!(
        "BEGIN:VCARD\r\nVERSION:4.0\r\nUID:{uid}\r\nFN:Croft Test\r\nEMAIL:test@example.org\r\nEND:VCARD\r\n"
    );
    assert_eq!(
        dav.put(&path, &card, None).await.unwrap(),
        WriteOutcome::Done
    );
    assert_eq!(
        dav.put(&path, &card, None).await.unwrap(),
        WriteOutcome::Conflict,
        "never an overwrite on create"
    );

    let after = match &token {
        Some(t) => dav.changes(&book, Some(t)).await.unwrap(),
        None => dav.changes(&book, None).await.unwrap(),
    };
    let mine = after
        .changed
        .iter()
        .find(|r| r.path == path)
        .expect("listed")
        .clone();
    let fetched = dav.fetch(&book, std::slice::from_ref(&path)).await.unwrap();
    assert!(fetched[0].text.contains("FN:Croft Test"));

    let edited = card.replace("Croft Test", "Croft Edited");
    assert_eq!(
        dav.put(&path, &edited, Some("\"not-the-etag\""))
            .await
            .unwrap(),
        WriteOutcome::Conflict
    );
    assert_eq!(
        dav.put(&path, &edited, Some(&mine.etag)).await.unwrap(),
        WriteOutcome::Done
    );
    let now = dav.changes(&book, None).await.unwrap();
    let etag = now
        .changed
        .iter()
        .find(|r| r.path == path)
        .unwrap()
        .etag
        .clone();
    assert_eq!(dav.delete(&path, &etag).await.unwrap(), WriteOutcome::Done);
    let end = dav.changes(&book, None).await.unwrap();
    assert_eq!(end.changed.len(), start.changed.len());

    let err = client("wrong-made-up-password")
        .discover()
        .await
        .unwrap_err();
    assert!(matches!(err, BackendError::Auth(_)), "{err:?}");
}
