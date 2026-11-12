// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! T26: with an address policy set (as the Companion sets one), every protocol refuses a server
//! whose address the policy forbids, after the name lookup, so a name that points at the machine
//! itself (`localhost`) is refused like the address. Its own test binary: the policy is for the
//! whole process. The OIDC fetch, for the admin's own provider, is the one exception.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;
use std::time::Duration;

use croft_protocols::netpolicy;

#[tokio::test]
async fn a_forbidden_address_is_refused_by_every_protocol_and_an_allowed_one_is_reached() {
    // A listener that would answer, so a refusal can only come from the policy.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        loop {
            if listener.accept().await.is_err() {
                break;
            }
        }
    });
    netpolicy::set_address_policy(Arc::new(|ip| !netpolicy::is_internal(ip)));

    for host in ["127.0.0.1", "localhost"] {
        // JMAP (and the HTTPS client the Companion's OIDC uses).
        let mut cfg =
            croft_protocols::JmapConfig::new(format!("https://{host}:{port}/jmap"), "ann");
        cfg.timeout = Duration::from_secs(2);
        let err = croft_protocols::jmap::probe_certificate(&cfg)
            .await
            .unwrap_err();
        assert!(
            format!("{err:?}").contains("not allowed"),
            "{host}: {err:?}"
        );
        // IMAP.
        let mut imap =
            croft_protocols::ImapConfig::new(host, port, croft_protocols::Security::Tls, "ann");
        imap.timeout = Duration::from_secs(2);
        let err = croft_protocols::probe_certificate(&imap).await.unwrap_err();
        assert!(
            format!("{err:?}").contains("not allowed"),
            "{host}: {err:?}"
        );
        // SMTP.
        let mut smtp =
            croft_protocols::SmtpConfig::new(host, port, croft_protocols::SmtpSecurity::Tls, "ann");
        smtp.timeout = Duration::from_secs(2);
        let err = croft_protocols::probe_smtp_certificate(&smtp)
            .await
            .unwrap_err();
        assert!(
            format!("{err:?}").contains("not allowed"),
            "{host}: {err:?}"
        );
    }

    // A push endpoint is a client's choice (T28): the policy applies.
    let err = croft_protocols::oauth::fetch::post_push(
        &format!("https://127.0.0.1:{port}/push"),
        "vapid t=x, k=y",
        &[("TTL", "60")],
        b"x",
    )
    .await
    .unwrap_err();
    assert_eq!(err, croft_protocols::oauth::fetch::FetchError::Refused);

    // The Companion's OIDC provider is its admin's choice: the fetch it uses is not restricted, so a
    // provider on the same network keeps working.
    let http = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let http_port = http.local_addr().unwrap().port();
    tokio::spawn(async move {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        if let Ok((mut s, _)) = http.accept().await {
            let mut buf = [0u8; 2048];
            let _ = s.read(&mut buf).await;
            let _ = s
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n{}")
                .await;
        }
    });
    let ok = croft_protocols::oauth::fetch::get(&format!("http://127.0.0.1:{http_port}/"), 100)
        .await
        .unwrap();
    assert_eq!(ok.status, 200);

    // Allowed again: the connection is made (and fails later, at TLS, for another reason).
    netpolicy::set_address_policy(Arc::new(|_| true));
    let mut cfg = croft_protocols::JmapConfig::new(format!("https://127.0.0.1:{port}/jmap"), "ann");
    cfg.timeout = Duration::from_secs(2);
    let err = croft_protocols::jmap::probe_certificate(&cfg)
        .await
        .unwrap_err();
    assert!(!format!("{err:?}").contains("not allowed"), "{err:?}");
}
