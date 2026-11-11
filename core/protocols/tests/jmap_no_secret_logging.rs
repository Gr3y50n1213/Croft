// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The JMAP adapter must not put a password, an OAuth token, an address or message content into a
//! log record, or into an error string or `Debug` output, on any path (success, wrong password,
//! refused submission, lost answer, server errors).
//!
//! This file is its own test binary, so the global logger belongs to it alone.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use croft_protocols::*;
use croft_test_support::JmapServer;
use std::sync::{Arc, Mutex};
use std::time::Duration;

const PASSWORD: &str = "MARKER-pw-7f3a91c2";
const TOKEN: &str = "MARKER-token-5d0e44b8";
const BODY: &str = "MARKER-body-91ab33";
const RCPT: &str = "marker-rcpt-33ce10@example.org";
const USER: &str = "marker-user-8821ab@example.org";
const SUBJECT: &str = "MARKER-subject-6c20f1";

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

const MARKERS: [&str; 6] = [PASSWORD, TOKEN, BODY, "marker-rcpt", "marker-user", SUBJECT];

fn assert_clean(what: &str, text: &str) {
    for m in MARKERS {
        assert!(!text.contains(m), "{what} contains {m}: {text}");
    }
}

#[tokio::test]
async fn no_log_record_or_error_contains_a_secret_an_address_or_content() {
    log::set_logger(&CAPTURE).expect("this test binary installs the only logger");
    log::set_max_level(log::LevelFilter::Trace);

    let server = JmapServer::start().await;
    server.set_credentials(USER, PASSWORD);
    server.set_bearer_token(TOKEN);
    let raw = format!(
        "From: Ann <ann@example.org>\r\nTo: <{RCPT}>\r\nSubject: {SUBJECT}\r\n\
         Date: Mon, 02 Mar 2026 09:30:00 +0000\r\nMessage-ID: <m1@example.org>\r\n\r\n{BODY}\r\n"
    );
    server.add_message("inbox", &raw, &[]);
    let cfg = || JmapConfig::new(server.session_url(), USER).with_timeout(Duration::from_secs(3));
    let make = |credential: Credential| {
        JmapBackend::new(cfg(), credential, Arc::new(MemoryIdStore::new())).unwrap()
    };

    let mut seen_errors: Vec<String> = Vec::new();
    let mut note = |what: &str, e: BackendError| {
        let text = format!("{e:?} / {e}");
        assert_clean(what, &text);
        seen_errors.push(text);
    };

    // Debug output of everything that holds a credential.
    let good = make(Credential::password(PASSWORD));
    assert_clean("Debug of the backend", &format!("{good:?}"));
    assert_clean("Debug of the config", &format!("{:?}", cfg()));
    assert_clean(
        "Debug of the credentials",
        &format!(
            "{:?} {:?}",
            Credential::password(PASSWORD),
            Credential::oauth2(USER, TOKEN)
        ),
    );
    let sender = JmapSubmitter::new(cfg(), Credential::password(PASSWORD)).unwrap();
    assert_clean("Debug of the submitter", &format!("{sender:?}"));

    // Success paths, with message content going through.
    good.capabilities().await.unwrap();
    good.list_folders().await.unwrap();
    let snap = good.snapshot("Inbox").await.unwrap();
    let uids: Vec<Uid> = snap.messages.keys().copied().collect();
    good.fetch_headers("Inbox", &uids).await.unwrap();
    good.find_by_message_id("Inbox", "<m1@example.org>")
        .await
        .unwrap();
    good.set_flag("Inbox", &uids, FlagChange::Seen(true))
        .await
        .unwrap();
    good.move_to("Inbox", &uids, "Archive").await.unwrap();
    let bearer = make(Credential::oauth2(USER, TOKEN));
    bearer.list_folders().await.unwrap();

    // Failure paths.
    let wrong_pw = make(Credential::password(format!("{PASSWORD}-wrong")));
    note("wrong password", wrong_pw.list_folders().await.unwrap_err());
    let wrong_token = make(Credential::oauth2(USER, format!("{TOKEN}-wrong")));
    note(
        "wrong token",
        wrong_token.snapshot("Inbox").await.unwrap_err(),
    );
    note(
        "no such folder",
        good.snapshot("MARKER-folder").await.unwrap_err(),
    );
    server.set_api_status(Some(503));
    note("server error", good.list_folders().await.unwrap_err());
    server.set_api_status(None);

    // Sending: success, refusal (the server's own error text names the user), a lost answer.
    let mime = format!("From: <{USER}>\r\nTo: <{RCPT}>\r\nSubject: {SUBJECT}\r\n\r\n{BODY}\r\n");
    sender.send(USER, &[RCPT], mime.as_bytes()).await.unwrap();
    server.set_submission_error(Some("forbiddenFrom"));
    note(
        "refused",
        sender
            .send(USER, &[RCPT], mime.as_bytes())
            .await
            .unwrap_err(),
    );
    server.set_submission_error(None);
    server.drop_answer_for(Some("EmailSubmission/set"));
    note(
        "lost answer",
        sender
            .send(USER, &[RCPT], mime.as_bytes())
            .await
            .unwrap_err(),
    );
    note(
        "bad address",
        sender
            .send(USER, &["marker-rcpt bad@example.org"], mime.as_bytes())
            .await
            .unwrap_err(),
    );
    let wrong_sender = JmapSubmitter::new(cfg(), Credential::password("wrong")).unwrap();
    note(
        "sender sign-in",
        wrong_sender
            .send(USER, &[RCPT], mime.as_bytes())
            .await
            .unwrap_err(),
    );

    assert!(seen_errors.len() >= 8);
    let records = CAPTURE.0.lock().unwrap().clone();
    for r in &records {
        assert_clean("a log record", r);
    }
}
