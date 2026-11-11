// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Account discovery, end to end, with no real network.
//!
//! The name service and the web are replaced by recording fakes, so these tests can say exactly
//! which names and URLs discovery asked for. The mail server is real: the in-process IMAP server
//! from `croft-test-support`, reached the way a person's own server would be, through SRV records
//! (the same flow against GreenMail and Dovecot is in `tests/discovery_containers.rs`).
#![allow(clippy::unwrap_used, clippy::expect_used)]

use croft_protocols::discovery::testing::{FakeHttp, FakeResolver};
use croft_protocols::discovery::*;
use croft_protocols::{probe_certificate, CertFingerprint, Credential, Security};
use croft_test_support::{ServerOptions, TestServer, TlsMode};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc::unbounded_channel;

struct Net {
    dns: Arc<FakeResolver>,
    http: Arc<FakeHttp>,
}

impl Net {
    fn new() -> Self {
        Self {
            dns: Arc::new(FakeResolver::new()),
            http: Arc::new(FakeHttp::new()),
        }
    }

    fn discoverer(&self) -> Discoverer {
        Discoverer::new(self.dns.clone(), self.http.clone())
    }
}

fn srv(port: u16, target: &str) -> Vec<SrvRecord> {
    vec![SrvRecord {
        priority: 0,
        weight: 1,
        port,
        target: target.to_string(),
    }]
}

fn autoconfig_xml(host: &str, port: u16, socket: &str) -> String {
    format!(
        r#"<?xml version="1.0"?>
<clientConfig version="1.1"><emailProvider id="x">
<incomingServer type="imap"><hostname>{host}</hostname><port>{port}</port>
<socketType>{socket}</socketType><username>%EMAILLOCALPART%</username>
<authentication>password-cleartext</authentication></incomingServer>
<outgoingServer type="smtp"><hostname>{host}</hostname><port>587</port>
<socketType>STARTTLS</socketType><username>%EMAILADDRESS%</username></outgoingServer>
</emailProvider></clientConfig>"#
    )
}

const SUB: &str = "https://autoconfig.example.org/mail/config-v1.1.xml";
const WELL_KNOWN: &str = "https://example.org/.well-known/autoconfig/mail/config-v1.1.xml";
const JMAP: &str = "https://example.org/.well-known/jmap";

// ------------------------------------------------------------------------------------------
// Which answer wins
// ------------------------------------------------------------------------------------------

#[tokio::test]
async fn an_earlier_source_beats_a_later_one_even_if_the_later_answers_first() {
    let net = Net::new();
    // Autoconfig is slow (300 ms); SRV and JMAP answer at once.
    net.http.set_delay(SUB, Duration::from_millis(300));
    net.http.set_body(
        SUB,
        autoconfig_xml("auto.example.org", 993, "SSL").as_bytes(),
    );
    net.dns
        .set_srv("_imaps._tcp.example.org", srv(993, "srv.example.org"));
    net.http
        .set_redirect(JMAP, 307, "https://example.org/jmap/session");

    let found = net.discoverer().discover("ann@example.org").await.unwrap();
    assert_eq!(found.source, Source::Autoconfig);
    assert_eq!(found.imap.unwrap().host, "auto.example.org");
}

#[tokio::test]
async fn the_order_is_subdomain_then_well_known_then_srv_then_jmap() {
    let net = Net::new();
    net.http.set_body(
        SUB,
        autoconfig_xml("one.example.org", 993, "SSL").as_bytes(),
    );
    net.http.set_body(
        WELL_KNOWN,
        autoconfig_xml("two.example.org", 993, "SSL").as_bytes(),
    );
    net.dns
        .set_srv("_imaps._tcp.example.org", srv(993, "three.example.org"));
    net.http
        .set_redirect(JMAP, 307, "https://four.example.org/jmap/session");

    let d = net.discoverer();
    let host = |s: DiscoveredSettings| s.imap.map(|i| i.host);
    assert_eq!(
        host(d.discover("a@example.org").await.unwrap()).as_deref(),
        Some("one.example.org")
    );
    net.http.set_status(SUB, 404);
    assert_eq!(
        host(d.discover("a@example.org").await.unwrap()).as_deref(),
        Some("two.example.org")
    );
    net.http.set_status(WELL_KNOWN, 404);
    let s = d.discover("a@example.org").await.unwrap();
    assert_eq!(
        (s.source, host(s).as_deref()),
        (Source::Srv, Some("three.example.org"))
    );
    net.dns.set_srv("_imaps._tcp.example.org", vec![]);
    let s = d.discover("a@example.org").await.unwrap();
    assert_eq!(s.source, Source::JmapWellKnown);
    assert!(s.imap.is_none());
    assert_eq!(
        s.jmap.as_deref(),
        Some("https://four.example.org/jmap/session")
    );
}

#[tokio::test]
async fn a_failing_earlier_probe_does_not_hold_up_a_later_success() {
    let net = Net::new();
    net.http.set_error(SUB, FetchError::Connect);
    net.http.set_status(WELL_KNOWN, 500);
    net.dns
        .set_srv("_imaps._tcp.example.org", srv(993, "srv.example.org"));
    let started = std::time::Instant::now();
    let s = net.discoverer().discover("a@example.org").await.unwrap();
    assert_eq!(s.source, Source::Srv);
    assert!(started.elapsed() < Duration::from_secs(2));
}

#[tokio::test]
async fn when_time_runs_out_a_finished_later_probe_can_still_win() {
    let net = Net::new();
    net.http.set_delay(SUB, Duration::from_secs(5)); // never finishes in time
    net.dns
        .set_srv("_imaps._tcp.example.org", srv(993, "srv.example.org"));
    let limits = Limits {
        overall_timeout: Duration::from_millis(300),
        ..Limits::default()
    };
    let s = net
        .discoverer()
        .with_limits(limits)
        .discover("a@example.org")
        .await
        .unwrap();
    assert_eq!(s.source, Source::Srv);
}

#[tokio::test]
async fn a_slow_request_times_out_on_its_own_limit() {
    let net = Net::new();
    net.http.set_delay(SUB, Duration::from_secs(5));
    let limits = Limits {
        request_timeout: Duration::from_millis(200),
        ..Limits::default()
    };
    let err = net
        .discoverer()
        .with_limits(limits)
        .discover("a@example.org")
        .await
        .unwrap_err();
    let DiscoveryError::NothingFound { reasons } = err else {
        panic!("{err:?}")
    };
    assert_eq!(
        reasons[0],
        (Probe::AutoconfigSubdomain, ProbeFailure::TimedOut)
    );
}

#[tokio::test]
async fn smtp_and_auth_hints_come_with_the_winning_source() {
    let net = Net::new();
    net.http.set_body(
        SUB,
        autoconfig_xml("mail.example.org", 143, "STARTTLS").as_bytes(),
    );
    let s = net.discoverer().discover("Ann@example.org").await.unwrap();
    let imap = s.imap.unwrap();
    assert_eq!((imap.port, imap.security), (143, Security::StartTls));
    assert_eq!(imap.username, "Ann");
    let smtp = s.smtp.unwrap();
    assert_eq!((smtp.port, smtp.security), (587, Security::StartTls));
    assert_eq!(smtp.username, "Ann@example.org");
    assert_eq!((s.provider, s.auth_hint), (None, AuthHint::Password));
}

// ------------------------------------------------------------------------------------------
// The provider table
// ------------------------------------------------------------------------------------------

#[tokio::test]
async fn provider_domains_are_answered_without_any_network_traffic() {
    let cases = [
        (
            "a@gmail.com",
            Provider::Gmail,
            "imap.gmail.com",
            AuthHint::OAuth2OrAppPassword,
        ),
        (
            "a@googlemail.com",
            Provider::Gmail,
            "imap.gmail.com",
            AuthHint::OAuth2OrAppPassword,
        ),
        (
            "a@outlook.com",
            Provider::Outlook,
            "outlook.office365.com",
            AuthHint::OAuth2,
        ),
        (
            "a@hotmail.com",
            Provider::Outlook,
            "outlook.office365.com",
            AuthHint::OAuth2,
        ),
        (
            "a@live.com",
            Provider::Outlook,
            "outlook.office365.com",
            AuthHint::OAuth2,
        ),
        (
            "a@icloud.com",
            Provider::ICloud,
            "imap.mail.me.com",
            AuthHint::AppPassword,
        ),
        (
            "a@me.com",
            Provider::ICloud,
            "imap.mail.me.com",
            AuthHint::AppPassword,
        ),
        (
            "a@mac.com",
            Provider::ICloud,
            "imap.mail.me.com",
            AuthHint::AppPassword,
        ),
        (
            "a@fastmail.com",
            Provider::Fastmail,
            "imap.fastmail.com",
            AuthHint::AppPassword,
        ),
        (
            "a@FastMail.FM",
            Provider::Fastmail,
            "imap.fastmail.com",
            AuthHint::AppPassword,
        ),
    ];
    for (address, provider, host, hint) in cases {
        let net = Net::new();
        let s = net.discoverer().discover(address).await.unwrap();
        assert_eq!(s.source, Source::ProviderTable, "{address}");
        assert_eq!(s.provider, Some(provider), "{address}");
        assert_eq!(s.imap.as_ref().unwrap().host, host, "{address}");
        assert_eq!(s.imap.as_ref().unwrap().security, Security::Tls);
        assert_eq!(s.auth_hint, hint, "{address}");
        assert!(s.smtp.is_some());
        assert!(net.http.requests().is_empty(), "{address}");
        assert!(net.dns.srv_queries().is_empty(), "{address}");
        assert!(net.dns.ip_queries().is_empty(), "{address}");
    }
    let net = Net::new();
    let f = net.discoverer().discover("a@fastmail.com").await.unwrap();
    assert_eq!(
        f.jmap.as_deref(),
        Some("https://api.fastmail.com/jmap/session")
    );
    let smtp = f.smtp.unwrap();
    assert_eq!(
        (smtp.host.as_str(), smtp.port, smtp.security),
        ("smtp.fastmail.com", 465, Security::Tls)
    );
}

// ------------------------------------------------------------------------------------------
// Privacy: exactly what is asked
// ------------------------------------------------------------------------------------------

#[tokio::test]
async fn a_search_that_finds_nothing_asks_only_the_persons_own_domain() {
    let net = Net::new();
    let err = net
        .discoverer()
        .discover("zq7local@example.org")
        .await
        .unwrap_err();
    assert!(matches!(err, DiscoveryError::NothingFound { .. }));

    let mut requests = net.http.requests();
    requests.sort();
    assert_eq!(
        requests,
        [
            "https://autoconfig.example.org/mail/config-v1.1.xml",
            "https://example.org/.well-known/autoconfig/mail/config-v1.1.xml",
            "https://example.org/.well-known/jmap",
        ]
    );
    let mut srv_names = net.dns.srv_queries();
    srv_names.sort();
    assert_eq!(
        srv_names,
        [
            "_imap._tcp.example.org",
            "_imaps._tcp.example.org",
            "_jmap._tcp.example.org",
            "_submission._tcp.example.org",
            "_submissions._tcp.example.org",
        ]
    );
    // Nothing else, and the local part is in none of it.
    let everything = [requests, srv_names, net.dns.ip_queries()]
        .concat()
        .join("\n");
    assert!(!everything.contains("zq7local"));
    assert!(!everything.contains('?') && !everything.contains("emailaddress"));
    for forbidden in ["thunderbird", "mozilla", "mx", "http://"] {
        assert!(
            !everything.to_lowercase().contains(forbidden),
            "{forbidden} in {everything}"
        );
    }
}

#[tokio::test]
async fn every_name_and_url_ends_in_the_persons_domain_or_a_subdomain_of_it() {
    // Autoconfig answers and points at other hosts for the servers; only the domain is asked.
    let net = Net::new();
    net.http.set_body(
        SUB,
        autoconfig_xml("imap.hosting.example", 993, "SSL").as_bytes(),
    );
    net.dns
        .set_srv("_imaps._tcp.example.org", srv(993, "imap.hosting.example"));
    net.discoverer()
        .discover("zq7local@example.org")
        .await
        .unwrap();
    for url in net.http.requests() {
        let parsed = HttpsUrl::parse(&url).unwrap();
        assert!(parsed.is_within("example.org"), "{url}");
    }
    for name in net
        .dns
        .srv_queries()
        .into_iter()
        .chain(net.dns.ip_queries())
    {
        assert!(name.ends_with(".example.org"), "{name}");
    }
}

#[tokio::test]
async fn redirects_are_followed_only_within_the_domain_over_https() {
    let net = Net::new();
    // autoconfig.example.org -> example.org (same domain: allowed) -> the file.
    net.http
        .set_redirect(SUB, 301, "https://www.example.org/auto/config.xml");
    net.http.set_redirect(
        "https://www.example.org/auto/config.xml",
        302,
        "/final/config.xml",
    );
    net.http.set_body(
        "https://www.example.org/final/config.xml",
        autoconfig_xml("mail.example.org", 993, "SSL").as_bytes(),
    );
    let s = net.discoverer().discover("a@example.org").await.unwrap();
    assert_eq!(s.source, Source::Autoconfig);
    assert!(net
        .http
        .requests()
        .contains(&"https://www.example.org/final/config.xml".to_string()));

    // Off-domain, plain HTTP and look-alike hosts are not requested.
    for target in [
        "https://evil.example.net/config.xml",
        "https://example.org.evil.net/config.xml",
        "https://notexample.org/config.xml",
        "http://example.org/config.xml",
        "https://user:pw@example.org/config.xml",
        "//evil.example.net/config.xml",
        "relative/config.xml",
        "ftp://example.org/config.xml",
    ] {
        let net = Net::new();
        net.http.set_redirect(SUB, 302, target);
        net.http.set_body(
            target,
            autoconfig_xml("x.example.org", 993, "SSL").as_bytes(),
        );
        net.http.set_body(
            "https://evil.example.net/config.xml",
            autoconfig_xml("x.example.org", 993, "SSL").as_bytes(),
        );
        let err = net
            .discoverer()
            .discover("a@example.org")
            .await
            .unwrap_err();
        let DiscoveryError::NothingFound { reasons } = err else {
            panic!("{err:?}")
        };
        assert_eq!(
            reasons[0],
            (Probe::AutoconfigSubdomain, ProbeFailure::BadRedirect),
            "{target}"
        );
        let asked = net.http.requests();
        assert!(
            !asked
                .iter()
                .any(|u| u.contains("evil") || u.contains("notexample") || u.starts_with("http:")),
            "{target}: {asked:?}"
        );
    }
}

#[tokio::test]
async fn at_most_three_redirects_are_followed() {
    let net = Net::new();
    net.http.set_redirect(SUB, 301, "https://a.example.org/1");
    net.http
        .set_redirect("https://a.example.org/1", 301, "https://a.example.org/2");
    net.http
        .set_redirect("https://a.example.org/2", 301, "https://a.example.org/3");
    net.http.set_body(
        "https://a.example.org/3",
        autoconfig_xml("mail.example.org", 993, "SSL").as_bytes(),
    );
    // Three redirects: allowed.
    assert!(net.discoverer().discover("a@example.org").await.is_ok());
    // A fourth: refused.
    net.http
        .set_redirect("https://a.example.org/3", 301, "https://a.example.org/4");
    net.http.set_body(
        "https://a.example.org/4",
        autoconfig_xml("mail.example.org", 993, "SSL").as_bytes(),
    );
    assert!(net.discoverer().discover("a@example.org").await.is_err());
    assert!(!net.http.requests().iter().any(|u| u.ends_with("/4")));
}

#[tokio::test]
async fn the_jmap_probe_records_a_redirect_without_following_it() {
    let net = Net::new();
    net.http
        .set_redirect(JMAP, 301, "https://api.provider.example/jmap/session");
    let s = net.discoverer().discover("a@example.org").await.unwrap();
    assert_eq!(
        s.jmap.as_deref(),
        Some("https://api.provider.example/jmap/session")
    );
    assert!(!net
        .http
        .requests()
        .iter()
        .any(|u| u.contains("provider.example")));
    // A session document served in place is accepted too; a redirect to plain HTTP is not.
    let net = Net::new();
    net.http.set_body(
        JMAP,
        br#"{"capabilities":{"urn:ietf:params:jmap:core":{}}}"#,
    );
    let s = net.discoverer().discover("a@example.org").await.unwrap();
    assert_eq!(s.jmap.as_deref(), Some(JMAP));
    let net = Net::new();
    net.http.set_redirect(JMAP, 301, "http://example.org/jmap");
    assert!(net.discoverer().discover("a@example.org").await.is_err());
    let net = Net::new();
    net.http.set_body(JMAP, b"<html>not jmap</html>");
    assert!(net.discoverer().discover("a@example.org").await.is_err());
}

// ------------------------------------------------------------------------------------------
// Failures say why
// ------------------------------------------------------------------------------------------

#[tokio::test]
async fn each_probe_reports_its_own_reason() {
    let net = Net::new();
    net.http.set_error(SUB, FetchError::Tls);
    net.http.set_body(WELL_KNOWN, b"<html>an error page</html>");
    net.dns
        .fail_srv("_imaps._tcp.example.org", ResolveError::Failed);
    net.http.set_status(JMAP, 404);
    let err = net
        .discoverer()
        .discover("a@example.org")
        .await
        .unwrap_err();
    let DiscoveryError::NothingFound { reasons } = &err else {
        panic!("{err:?}")
    };
    assert_eq!(
        reasons,
        &[
            (Probe::AutoconfigSubdomain, ProbeFailure::CertificateProblem),
            (Probe::AutoconfigWellKnown, ProbeFailure::Unusable),
            (Probe::Srv, ProbeFailure::Unreachable),
            (Probe::JmapWellKnown, ProbeFailure::NotPublished),
        ]
    );
    assert!(err.message().contains("enter the server details yourself"));
}

#[tokio::test]
async fn an_autoconfig_host_name_with_the_local_part_is_unusable_and_never_looked_up() {
    let net = Net::new();
    for host in ["%EMAILLOCALPART%.evil.example", "imap.%EMAILADDRESS%"] {
        net.http
            .set_body(SUB, autoconfig_xml(host, 993, "SSL").as_bytes());
        net.http
            .set_body(WELL_KNOWN, autoconfig_xml(host, 993, "SSL").as_bytes());
        let err = net
            .discoverer()
            .discover("alice@example.org")
            .await
            .unwrap_err();
        let DiscoveryError::NothingFound { reasons } = err else {
            panic!()
        };
        assert_eq!(
            reasons[0],
            (Probe::AutoconfigSubdomain, ProbeFailure::Unusable)
        );
        assert_eq!(
            reasons[1],
            (Probe::AutoconfigWellKnown, ProbeFailure::Unusable)
        );
    }
    // Nothing with the local part in it was ever asked of DNS or HTTP.
    let asked: Vec<String> = [
        net.dns.srv_queries(),
        net.dns.ip_queries(),
        net.http.requests(),
    ]
    .concat();
    assert!(
        asked
            .iter()
            .all(|q| !q.to_lowercase().contains("alice") && !q.contains("evil")),
        "{asked:?}"
    );
    // The domain placeholder is still allowed in a host name.
    net.http.set_body(
        SUB,
        autoconfig_xml("imap.%EMAILDOMAIN%", 993, "SSL").as_bytes(),
    );
    let found = net
        .discoverer()
        .discover("alice@example.org")
        .await
        .unwrap();
    assert_eq!(found.imap.unwrap().host, "imap.example.org");
}

#[tokio::test]
async fn an_oversized_file_and_an_unencrypted_only_file_are_named() {
    let net = Net::new();
    net.http.set_body(SUB, &vec![b' '; 64 * 1024 + 1]);
    net.http.set_body(
        WELL_KNOWN,
        autoconfig_xml("mail.example.org", 143, "plain").as_bytes(),
    );
    let err = net
        .discoverer()
        .discover("a@example.org")
        .await
        .unwrap_err();
    let DiscoveryError::NothingFound { reasons } = err else {
        panic!()
    };
    assert_eq!(reasons[0].1, ProbeFailure::TooLarge);
    assert_eq!(reasons[1].1, ProbeFailure::OnlyUnencrypted);
}

#[tokio::test]
async fn no_network_at_all_is_reported_as_offline() {
    let net = Net::new();
    net.http.set_error(SUB, FetchError::Dns);
    net.http.set_error(WELL_KNOWN, FetchError::TimedOut);
    net.http.set_error(JMAP, FetchError::Connect);
    for name in ["_imaps", "_imap", "_submissions", "_submission", "_jmap"] {
        net.dns
            .fail_srv(&format!("{name}._tcp.example.org"), ResolveError::TimedOut);
    }
    let err = net
        .discoverer()
        .discover("a@example.org")
        .await
        .unwrap_err();
    assert!(matches!(err, DiscoveryError::Offline { .. }), "{err:?}");
    assert_ne!(
        err.message(),
        DiscoveryError::NothingFound { reasons: vec![] }.message()
    );
}

#[tokio::test]
async fn bad_addresses_are_refused_before_anything_is_asked() {
    let net = Net::new();
    for bad in [
        "",
        "nobody",
        "a@",
        "@example.org",
        "a@localhost",
        "a b@example.org",
    ] {
        let err = net.discoverer().discover(bad).await.unwrap_err();
        assert_eq!(err, DiscoveryError::InvalidAddress, "{bad:?}");
    }
    assert!(net.http.requests().is_empty() && net.dns.srv_queries().is_empty());
}

#[tokio::test]
async fn progress_updates_end_with_the_final_state_of_every_probe() {
    let net = Net::new();
    net.http.set_body(
        WELL_KNOWN,
        autoconfig_xml("mail.example.org", 993, "SSL").as_bytes(),
    );
    let (tx, mut rx) = unbounded_channel();
    let found = net
        .discoverer()
        .with_progress(tx)
        .discover("a@example.org")
        .await
        .unwrap();
    assert_eq!(found.source, Source::Autoconfig);
    let mut last = std::collections::HashMap::new();
    while let Ok(update) = rx.try_recv() {
        last.insert(update.probe, update.state);
    }
    assert_eq!(last[&Probe::ProviderTable], ProbeState::Skipped);
    assert_eq!(
        last[&Probe::AutoconfigSubdomain],
        ProbeState::Failed(ProbeFailure::NotPublished)
    );
    assert_eq!(last[&Probe::AutoconfigWellKnown], ProbeState::Found);
    assert_eq!(last[&Probe::Srv], ProbeState::Skipped);
    assert_eq!(last[&Probe::JmapWellKnown], ProbeState::Skipped);
}

// ------------------------------------------------------------------------------------------
// Verify, against the in-process IMAP server reached through SRV
// ------------------------------------------------------------------------------------------

const DOMAIN_USER: &str = "alice@srv.test";

async fn server(tls: TlsMode) -> TestServer {
    TestServer::start(ServerOptions {
        username: DOMAIN_USER.into(),
        password: "correct horse".into(),
        tls,
        ..ServerOptions::default()
    })
    .await
}

/// Runs the find phase for `alice@srv.test` with the given SRV record name pointing at `server`.
async fn found_via_srv(service: &str, server: &TestServer) -> DiscoveredSettings {
    let net = Net::new();
    net.dns.set_srv(
        &format!("{service}._tcp.srv.test"),
        srv(server.port(), "localhost"),
    );
    let s = net.discoverer().discover(DOMAIN_USER).await.unwrap();
    assert_eq!(s.source, Source::Srv);
    s
}

fn password(p: &str) -> Credential {
    Credential::password(p)
}

fn quick() -> VerifyOptions {
    VerifyOptions {
        timeout: Duration::from_secs(5),
        ..VerifyOptions::default()
    }
}

/// Whether the server has received a sign-in command (the log shows `LOGIN ***`, never the
/// password).
fn signed_in(server: &TestServer) -> bool {
    server
        .commands()
        .iter()
        .any(|c| c.starts_with("LOGIN") || c.starts_with("AUTHENTICATE"))
}

/// The whole certificate flow for one server: untrusted, then trusted once, then signed in.
async fn self_signed_flow(service: &str, mode: TlsMode, expected: Security) {
    let srv = server(mode).await;
    let settings = found_via_srv(service, &srv).await;
    assert_eq!(settings.imap.as_ref().unwrap().security, expected);

    // 1. No pin: refused before anything secret is sent, with the fingerprint.
    let err = verify(&settings, password("correct horse"), quick())
        .await
        .unwrap_err();
    assert!(!signed_in(&srv), "{:?}", srv.commands());
    let VerifyError::UntrustedCertificate { fingerprint } = err.clone() else {
        panic!("{err:?}")
    };
    assert_eq!(
        fingerprint,
        CertFingerprint::of_der(&srv.certificate_der().unwrap())
    );
    assert!(err.message().contains("self-signed"));
    // The same fingerprint the account-setup screen gets from probe_certificate.
    let probed = probe_certificate(&settings.imap.as_ref().unwrap().to_config())
        .await
        .unwrap();
    assert_eq!(probed, fingerprint);

    // 2. The person trusts it once: the retry pins it and signs in.
    let options = VerifyOptions {
        pinned_cert: Some(fingerprint),
        ..quick()
    };
    let ok = verify(&settings, password("correct horse"), options)
        .await
        .unwrap();
    assert!(signed_in(&srv));
    assert_eq!(ok.imap.pinned_cert, Some(fingerprint));
    assert!(ok.capabilities.move_cmd);

    // 3. A wrong password on the pinned connection is its own error.
    let err = verify(&settings, password("wrong"), options)
        .await
        .unwrap_err();
    assert_eq!(err, VerifyError::WrongPassword);
}

#[tokio::test]
async fn srv_with_implicit_tls_untrusted_certificate_then_pinned_success() {
    self_signed_flow("_imaps", TlsMode::Implicit, Security::Tls).await;
}

#[tokio::test]
async fn srv_with_starttls_untrusted_certificate_then_pinned_success() {
    self_signed_flow("_imap", TlsMode::StartTls, Security::StartTls).await;
}

#[tokio::test]
async fn wrong_password_and_untrusted_certificate_have_their_own_messages() {
    let srv = server(TlsMode::Implicit).await;
    let settings = found_via_srv("_imaps", &srv).await;
    let pin = CertFingerprint::of_der(&srv.certificate_der().unwrap());
    let wrong = verify(
        &settings,
        password("nope"),
        VerifyOptions {
            pinned_cert: Some(pin),
            ..quick()
        },
    )
    .await
    .unwrap_err();
    let untrusted = verify(&settings, password("correct horse"), quick())
        .await
        .unwrap_err();
    assert_eq!(wrong, VerifyError::WrongPassword);
    assert!(matches!(
        untrusted,
        VerifyError::UntrustedCertificate { .. }
    ));
    assert_ne!(wrong.message(), untrusted.message());
    assert!(wrong.message().contains("password"));
    assert!(untrusted.message().contains("certificate"));
    // Neither message carries the address or the password.
    for m in [wrong.message(), untrusted.message()] {
        assert!(!m.contains("alice") && !m.contains("srv.test") && !m.contains("nope"));
    }
}

#[tokio::test]
async fn a_provider_domain_with_a_self_signed_certificate_is_interception() {
    // A server standing in for the provider presents a self-signed certificate, as a hostile
    // network would. Trusting it is never offered, and no credential reaches the server.
    for (service, mode) in [("_imaps", TlsMode::Implicit), ("_imap", TlsMode::StartTls)] {
        let srv = server(mode).await;
        let mut settings = found_via_srv(service, &srv).await;
        settings.provider = Some(Provider::Gmail);
        let err = verify(&settings, password("correct horse"), quick())
            .await
            .unwrap_err();
        assert_eq!(err, VerifyError::ConnectionIntercepted);
        assert!(!signed_in(&srv), "{:?}", srv.commands());
        assert!(!err.message().contains("alice") && !err.message().contains("srv.test"));
        // The same settings without the provider are a question for the person.
        settings.provider = None;
        let err = verify(&settings, password("correct horse"), quick())
            .await
            .unwrap_err();
        assert!(matches!(err, VerifyError::UntrustedCertificate { .. }));
    }
}

#[tokio::test]
async fn a_pinned_server_that_changes_certificate_is_reported_as_changed() {
    let srv = server(TlsMode::Implicit).await;
    let settings = found_via_srv("_imaps", &srv).await;
    let other = CertFingerprint::of_der(b"some other certificate");
    let err = verify(
        &settings,
        password("correct horse"),
        VerifyOptions {
            pinned_cert: Some(other),
            ..quick()
        },
    )
    .await
    .unwrap_err();
    let VerifyError::CertificateChanged { fingerprint } = err.clone() else {
        panic!("{err:?}")
    };
    assert_eq!(
        fingerprint,
        CertFingerprint::of_der(&srv.certificate_der().unwrap())
    );
    assert_ne!(err.message(), VerifyError::WrongPassword.message());
}

#[tokio::test]
async fn a_server_without_tls_and_a_closed_port_are_different_errors() {
    // The listener speaks plain IMAP on a port published as implicit TLS.
    let plain = server(TlsMode::None).await;
    let settings = found_via_srv("_imaps", &plain).await;
    let err = verify(
        &settings,
        password("correct horse"),
        VerifyOptions {
            timeout: Duration::from_secs(2),
            ..VerifyOptions::default()
        },
    )
    .await
    .unwrap_err();
    assert_eq!(err, VerifyError::NoSecureConnection);

    // Nothing listens on the port.
    let closed = {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap().port()
    };
    let net = Net::new();
    net.dns
        .set_srv("_imaps._tcp.srv.test", srv(closed, "localhost"));
    let settings = net.discoverer().discover(DOMAIN_USER).await.unwrap();
    let err = verify(&settings, password("x"), quick()).await.unwrap_err();
    assert_eq!(err, VerifyError::CannotConnect);
    assert_ne!(err.message(), VerifyError::NoSecureConnection.message());
}

#[tokio::test]
async fn a_provider_that_rejects_a_normal_password_gets_its_own_explanation() {
    // A server standing in for the provider: the settings say Gmail, the sign-in is refused.
    let srv = server(TlsMode::Implicit).await;
    let mut settings = found_via_srv("_imaps", &srv).await;
    let pin = CertFingerprint::of_der(&srv.certificate_der().unwrap());
    let options = VerifyOptions {
        pinned_cert: Some(pin),
        ..quick()
    };
    let mut messages = std::collections::HashSet::new();
    for provider in [
        Provider::Gmail,
        Provider::Outlook,
        Provider::ICloud,
        Provider::Fastmail,
    ] {
        settings.provider = Some(provider);
        let err = verify(&settings, password("my normal password"), options)
            .await
            .unwrap_err();
        assert_eq!(err, VerifyError::NeedsAppPasswordOrOAuth(provider));
        assert_ne!(err.message(), VerifyError::WrongPassword.message());
        assert!(messages.insert(err.message()));
    }
    assert!(messages.iter().any(|m| m.contains("app password")));
    // The right password still works for them.
    let ok = verify(&settings, password("correct horse"), options).await;
    assert!(ok.is_ok());
}

#[tokio::test]
async fn jmap_only_settings_cannot_be_verified_over_imap_yet() {
    let net = Net::new();
    net.http.set_redirect(
        "https://srv.test/.well-known/jmap",
        307,
        "https://srv.test/jmap/session",
    );
    let settings = net.discoverer().discover(DOMAIN_USER).await.unwrap();
    let err = verify(&settings, password("x"), quick()).await.unwrap_err();
    assert_eq!(err, VerifyError::JmapOnly);
}
