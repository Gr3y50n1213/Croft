// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The SMTP sender must not put a password, an OAuth token, an address or message content into a
//! log record, or into an error string, on any path (success, wrong password, refusal, drop).
//!
//! This file is its own test binary, so the global logger belongs to it alone.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use croft_protocols::*;
use croft_test_support::{SmtpMisbehavior, SmtpOptions, SmtpServer, Stage};
use std::sync::Mutex;
use std::time::Duration;

const PASSWORD: &str = "MARKER-pw-7f3a91c2";
const TOKEN: &str = "MARKER-token-5d0e44b8";
const BODY: &str = "MARKER-body-91ab33";
const RCPT: &str = "marker-rcpt-33ce10@example.org";
const SENDER: &str = "marker-from-8821ab@example.org";

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

const MARKERS: [&str; 5] = [PASSWORD, TOKEN, BODY, "marker-rcpt", "marker-from"];

fn assert_clean(what: &str, text: &str) {
    for m in MARKERS {
        assert!(!text.contains(m), "{what} contains {m}: {text}");
    }
}

#[tokio::test]
async fn no_log_record_or_error_contains_a_secret_an_address_or_content() {
    log::set_logger(&CAPTURE).expect("this test binary installs the only logger");
    log::set_max_level(log::LevelFilter::Trace);

    let server = SmtpServer::start(SmtpOptions {
        password: PASSWORD.into(),
        ..SmtpOptions::default()
    })
    .await;
    let message = format!("Subject: s\r\n\r\n{BODY}\r\n");
    let cfg = SmtpConfig::new(
        server.host(),
        server.port(),
        SmtpSecurity::PlaintextForTestsOnly,
        server.username(),
    )
    .with_timeout(Duration::from_secs(5))
    .with_deadline(Duration::from_millis(800));
    let good = SmtpSender::new(cfg.clone(), Credential::password(PASSWORD)).unwrap();
    let wrong = SmtpSender::new(cfg.clone(), Credential::password(TOKEN)).unwrap();
    let oauth_wrong = SmtpSender::new(cfg, Credential::oauth2(server.username(), TOKEN)).unwrap();

    let mut errors = Vec::new();
    good.send(SENDER, &[RCPT], message.as_bytes())
        .await
        .unwrap();
    errors.push(wrong.send(SENDER, &[RCPT], message.as_bytes()).await);
    errors.push(oauth_wrong.send(SENDER, &[RCPT], message.as_bytes()).await);
    server.set_reply(Stage::RcptTo, 550, RCPT);
    errors.push(good.send(SENDER, &[RCPT], message.as_bytes()).await);
    server.clear_reply(Stage::RcptTo);
    server.set_reply(Stage::EndOfData, 451, BODY);
    errors.push(good.send(SENDER, &[RCPT], message.as_bytes()).await);
    server.clear_reply(Stage::EndOfData);
    server.set_misbehavior(SmtpMisbehavior::AcceptThenDrop);
    errors.push(good.send(SENDER, &[RCPT], message.as_bytes()).await);
    server.set_misbehavior(SmtpMisbehavior::HangAfterData);
    errors.push(good.send(SENDER, &[RCPT], message.as_bytes()).await);
    errors.push(good.send("bad\r\n", &[RCPT], message.as_bytes()).await);

    for e in &errors {
        let err = e.as_ref().unwrap_err();
        assert_clean("error", &format!("{err} {err:?}"));
    }
    assert_clean("debug of the sender", &format!("{good:?} {oauth_wrong:?}"));
    for record in CAPTURE.0.lock().unwrap().iter() {
        assert_clean("log record", record);
    }
}
