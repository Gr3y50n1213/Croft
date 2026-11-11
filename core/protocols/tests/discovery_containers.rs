// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Account discovery against the real servers in containers: GreenMail (found through an
//! autoconfig file) and Dovecot (found through SRV records, with implicit TLS and with STARTTLS).
//! The name service and the web are still fakes, serving records that point at `127.0.0.1`; the
//! mail servers are real.
//!
//! Needs the containers running, like `tests/integration.rs`:
//!
//! ```text
//! docker compose -f test-servers/docker-compose.yml up -d --wait greenmail dovecot
//! cd core && cargo test -p croft-protocols --features integration --test discovery_containers
//! docker compose -f test-servers/docker-compose.yml down
//! ```
//!
//! The host ports can be changed with `CROFT_TEST_GREENMAIL_IMAPS_PORT`,
//! `CROFT_TEST_DOVECOT_IMAPS_PORT` and `CROFT_TEST_DOVECOT_STARTTLS_PORT`, for running a second
//! copy of the servers next to someone else's (`docker compose -p other-name ...` with remapped
//! ports).
//!
//! Both servers use self-signed certificates, so each test walks the whole certificate flow:
//! refused with a fingerprint, trusted once, signed in. The third server of "three test
//! servers" is the in-process one in `tests/discovery.rs`.
#![cfg(feature = "integration")]
#![allow(clippy::unwrap_used, clippy::expect_used)]

use base64::Engine;
use croft_protocols::discovery::testing::{FakeHttp, FakeResolver};
use croft_protocols::discovery::*;
use croft_protocols::{CertFingerprint, Credential, Security};
use std::sync::Arc;
use std::time::Duration;

/// A host port, from the environment when set. The defaults are the ones in
/// `test-servers/docker-compose.yml`; the variables exist so that a second copy of the servers,
/// started under another compose project name with other host ports, can run at the same time as
/// someone else's.
fn port(variable: &str, default: u16) -> u16 {
    std::env::var(variable)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

fn greenmail_imaps() -> u16 {
    port("CROFT_TEST_GREENMAIL_IMAPS_PORT", 3993)
}

fn dovecot_imaps() -> u16 {
    port("CROFT_TEST_DOVECOT_IMAPS_PORT", 31993)
}

fn dovecot_starttls() -> u16 {
    port("CROFT_TEST_DOVECOT_STARTTLS_PORT", 31143)
}

fn quick() -> VerifyOptions {
    VerifyOptions {
        timeout: Duration::from_secs(20),
        ..VerifyOptions::default()
    }
}

fn password(p: &str) -> Credential {
    Credential::Password(p.to_string())
}

/// SHA-256 fingerprint of the PEM certificate at `path`.
fn fingerprint_of_pem(path: &str) -> CertFingerprint {
    let pem = std::fs::read_to_string(path).unwrap();
    let b64: String = pem.lines().filter(|l| !l.starts_with("-----")).collect();
    let der = base64::engine::general_purpose::STANDARD
        .decode(b64)
        .unwrap();
    CertFingerprint::of_der(&der)
}

fn dovecot_cert() -> CertFingerprint {
    fingerprint_of_pem(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../test-servers/dovecot/ssl/tls.crt"
    ))
}

fn srv(port: u16) -> Vec<SrvRecord> {
    vec![SrvRecord {
        priority: 10,
        weight: 1,
        port,
        target: "localhost".into(),
    }]
}

/// The certificate flow every self-signed server goes through.
async fn flow(settings: &DiscoveredSettings, good: &str, bad: &str) -> CertFingerprint {
    // No pin: refused with the fingerprint, before the password is sent.
    let err = verify(settings, password(good), quick()).await.unwrap_err();
    let VerifyError::UntrustedCertificate { fingerprint } = err.clone() else {
        panic!("expected an untrusted certificate, got {err:?}");
    };
    assert_ne!(err.message(), VerifyError::WrongPassword.message());

    // Trusted once: the retry pins the fingerprint and signs in.
    let pinned = VerifyOptions {
        pinned_cert: Some(fingerprint),
        ..quick()
    };
    let ok = verify(settings, password(good), pinned).await.unwrap();
    assert_eq!(ok.imap.pinned_cert, Some(fingerprint));
    assert!(ok.capabilities.uidplus);

    // A wrong password on the pinned connection is "wrong password" and nothing else.
    let wrong = verify(settings, password(bad), pinned).await.unwrap_err();
    assert_eq!(wrong, VerifyError::WrongPassword);
    assert_ne!(
        wrong.message(),
        VerifyError::UntrustedCertificate { fingerprint }.message()
    );
    fingerprint
}

#[tokio::test]
async fn greenmail_found_through_an_autoconfig_file() {
    let dns = Arc::new(FakeResolver::new());
    let http = Arc::new(FakeHttp::new());
    // %EMAILLOCALPART% makes the login name "test", which is GreenMail's user (password "test").
    let xml = format!(
        r#"<?xml version="1.0"?>
<clientConfig version="1.1"><emailProvider id="greenmail.test">
<incomingServer type="imap"><hostname>127.0.0.1</hostname><port>{}</port>
<socketType>SSL</socketType><username>%EMAILLOCALPART%</username>
<authentication>password-cleartext</authentication></incomingServer>
</emailProvider></clientConfig>"#,
        greenmail_imaps()
    );
    http.set_body(
        "https://autoconfig.greenmail.test/mail/config-v1.1.xml",
        xml.as_bytes(),
    );
    let settings = Discoverer::new(dns.clone(), http.clone())
        .discover("test@greenmail.test")
        .await
        .unwrap();
    assert_eq!(settings.source, Source::Autoconfig);
    let imap = settings.imap.clone().unwrap();
    assert_eq!(
        (imap.host.as_str(), imap.port, imap.security),
        ("127.0.0.1", greenmail_imaps(), Security::Tls)
    );
    assert_eq!(imap.username, "test");

    let fingerprint = flow(&settings, "test", "not-the-password").await;
    // GreenMail makes a new certificate at every start, so only its shape can be checked.
    assert_eq!(fingerprint.as_bytes().len(), 32);
}

#[tokio::test]
async fn dovecot_found_through_srv_records_with_implicit_tls() {
    let dns = Arc::new(FakeResolver::new());
    let http = Arc::new(FakeHttp::new());
    dns.set_srv("_imaps._tcp.dovecot.test", srv(dovecot_imaps()));
    let settings = Discoverer::new(dns.clone(), http.clone())
        .discover("anyone@dovecot.test")
        .await
        .unwrap();
    assert_eq!(settings.source, Source::Srv);
    let imap = settings.imap.clone().unwrap();
    assert_eq!(
        (imap.host.as_str(), imap.port, imap.security),
        ("localhost", dovecot_imaps(), Security::Tls)
    );
    // Dovecot's certificate is the fixed test certificate, so the fingerprint is known.
    let fingerprint = flow(&settings, "testpass", "wrong").await;
    assert_eq!(fingerprint, dovecot_cert());
}

#[tokio::test]
async fn dovecot_found_through_srv_records_with_starttls() {
    let dns = Arc::new(FakeResolver::new());
    let http = Arc::new(FakeHttp::new());
    dns.set_srv("_imap._tcp.dovecot.test", srv(dovecot_starttls()));
    let settings = Discoverer::new(dns.clone(), http.clone())
        .discover("anyone@dovecot.test")
        .await
        .unwrap();
    assert_eq!(settings.imap.as_ref().unwrap().security, Security::StartTls);
    let fingerprint = flow(&settings, "testpass", "wrong").await;
    assert_eq!(fingerprint, dovecot_cert());
}
