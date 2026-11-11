// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Nothing the adapter does may put a password, an OAuth token or mail content into a log record.
//!
//! A logger is installed that accepts every level and keeps every record. Then the adapter signs
//! in (with a marker password), lists folders, reads headers, moves a message, fails a sign-in and
//! fails a command, all against the in-process server, and no record may contain a marker.
//!
//! This file is its own test binary, so the global logger belongs to it alone. Run it in the
//! release profile too: `cargo test --release -p croft-protocols --test no_secret_logging`.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use croft_protocols::*;
use croft_test_support::{sample_message, ServerOptions, TestServer};
use std::sync::Mutex;
use std::time::Duration;

const PASSWORD: &str = "MARKER-pw-7f3a91c2";
const TOKEN: &str = "MARKER-token-5d0e44b8";
const SUBJECT: &str = "MARKER-subject-91ab33";

struct Capture(Mutex<Vec<String>>);

impl log::Log for Capture {
    fn enabled(&self, _: &log::Metadata) -> bool {
        true
    }
    fn log(&self, record: &log::Record) {
        self.0.lock().unwrap().push(format!(
            "[{}] {}: {}",
            record.level(),
            record.target(),
            record.args()
        ));
    }
    fn flush(&self) {}
}

static CAPTURE: Capture = Capture(Mutex::new(Vec::new()));

fn config(server: &TestServer) -> ImapConfig {
    ImapConfig::new(
        server.host(),
        server.port(),
        Security::PlaintextForTestsOnly,
        server.username(),
    )
    .with_timeout(Duration::from_secs(5))
}

#[tokio::test]
async fn no_log_record_contains_the_password_the_token_or_mail_content() {
    log::set_logger(&CAPTURE).expect("this test binary installs the only logger");
    // Ask for everything. What is actually compiled in is capped by `log::STATIC_MAX_LEVEL`.
    log::set_max_level(log::LevelFilter::Trace);

    let server = TestServer::start(ServerOptions {
        password: PASSWORD.into(),
        ..ServerOptions::default()
    })
    .await;
    server.add_message(
        "INBOX",
        &sample_message("marker@x", SUBJECT, "Ann Example <ann@example.org>"),
    );

    // A good sign-in with the marker password, then ordinary traffic that carries mail content.
    let b = ImapBackend::new(config(&server), Credential::password(PASSWORD)).unwrap();
    b.list_folders().await.unwrap();
    let snapshot = b.snapshot("INBOX").await.unwrap();
    let uids: Vec<u32> = snapshot.messages.keys().copied().collect();
    let headers = b.fetch_headers("INBOX", &uids).await.unwrap();
    assert_eq!(headers[0].subject, SUBJECT);
    b.move_to("INBOX", &uids, "Trash").await.unwrap();
    b.close().await;

    // A failed password-based sign-in and a failed token sign-in.
    let wrong = ImapBackend::new(config(&server), Credential::password(TOKEN)).unwrap();
    assert!(matches!(
        wrong.list_folders().await,
        Err(BackendError::Auth(_))
    ));
    let oauth = ImapBackend::new(
        config(&server),
        Credential::oauth2(server.username(), TOKEN),
    )
    .unwrap();
    assert!(matches!(
        oauth.list_folders().await,
        Err(BackendError::Auth(_))
    ));
    // A refused command.
    let b = ImapBackend::new(config(&server), Credential::password(PASSWORD)).unwrap();
    assert!(b.snapshot("No such folder").await.is_err());

    let records = CAPTURE.0.lock().unwrap().clone();
    for record in &records {
        for marker in [PASSWORD, TOKEN, SUBJECT, "ann@example.org", "marker@x"] {
            assert!(
                !record.contains(marker),
                "a log record contains `{marker}`: {record}"
            );
        }
    }
    // Belt and braces: the level the build allows is never above Debug.
    assert!(log::STATIC_MAX_LEVEL <= log::LevelFilter::Debug);
}
