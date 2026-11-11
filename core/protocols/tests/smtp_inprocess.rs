// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The SMTP sender against the in-process fake server (`croft-test-support`).
#![allow(clippy::unwrap_used, clippy::expect_used)]

use croft_protocols::smtp::MAYBE_SENT_REASON;
use croft_protocols::*;
use croft_test_support::{SmtpMisbehavior, SmtpOptions, SmtpServer, Stage, TlsMode};
use std::time::Duration;

const MSG: &[u8] = b"From: a@example.org\r\nTo: b@example.org\r\nSubject: hi\r\n\r\nHello.\r\n";

fn config(server: &SmtpServer, security: SmtpSecurity) -> SmtpConfig {
    SmtpConfig::new(server.host(), server.port(), security, server.username())
        .with_timeout(Duration::from_secs(5))
}

fn plain(server: &SmtpServer) -> SmtpSender {
    SmtpSender::new(
        config(server, SmtpSecurity::PlaintextForTestsOnly),
        Credential::password(server.password()),
    )
    .unwrap()
}

fn pinned(server: &SmtpServer, security: SmtpSecurity) -> SmtpSender {
    let fp = CertFingerprint::of_der(&server.certificate_der().unwrap());
    SmtpSender::new(
        config(server, security).with_pinned_cert(fp),
        Credential::password(server.password()),
    )
    .unwrap()
}

async fn with_options(opts: SmtpOptions) -> SmtpServer {
    SmtpServer::start(opts).await
}

#[tokio::test]
async fn sends_a_message_with_the_right_envelope() {
    let server = SmtpServer::start_default().await;
    let receipt = plain(&server)
        .send("a@example.org", &["b@example.org", "c@example.org"], MSG)
        .await
        .unwrap();
    assert_eq!(receipt.recipients, 2);
    let got = server.accepted();
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].mail_from, "a@example.org");
    assert_eq!(got[0].rcpt_to, ["b@example.org", "c@example.org"]);
    assert_eq!(got[0].data, MSG);
    assert_eq!(got[0].authenticated_as.as_deref(), Some("alice"));
    assert_eq!(server.connection_count(), 1);
    assert_eq!(server.commands().last().map(String::as_str), Some("QUIT"));
}

#[tokio::test]
async fn lines_are_normalised_and_dots_stuffed() {
    let server = SmtpServer::start_default().await;
    let raw = b"Subject: x\n\n.leading dot\n.\nlone cr\rend\r\n..two";
    plain(&server)
        .send("a@example.org", &["b@example.org"], raw)
        .await
        .unwrap();
    let got = server.accepted();
    // What the server receives, after it removes the stuffing, is the message with CRLF ends and
    // a final line end, and the lone "." line did not end the message early.
    assert_eq!(
        got[0].data,
        b"Subject: x\r\n\r\n.leading dot\r\n.\r\nlone cr\r\nend\r\n..two\r\n"
    );
}

#[tokio::test]
async fn eight_bit_and_utf8_addresses_use_the_extensions() {
    let server = SmtpServer::start_default().await;
    plain(&server)
        .send(
            "jörg@example.org",
            &["b@example.org"],
            "Grüße\r\n".as_bytes(),
        )
        .await
        .unwrap();
    let cmds = server.commands();
    let mail = cmds.iter().find(|c| c.starts_with("MAIL")).unwrap();
    assert!(
        mail.contains("BODY=8BITMIME") && mail.contains("SMTPUTF8"),
        "{mail}"
    );

    let server = with_options(SmtpOptions {
        eight_bit: false,
        smtputf8: false,
        ..SmtpOptions::default()
    })
    .await;
    let err = plain(&server)
        .send("a@example.org", &["b@example.org"], "Grüße\r\n".as_bytes())
        .await
        .unwrap_err();
    assert!(matches!(err, BackendError::Permanent(_)), "{err:?}");
    let err = plain(&server)
        .send("jörg@example.org", &["b@example.org"], MSG)
        .await
        .unwrap_err();
    assert!(matches!(err, BackendError::Permanent(_)), "{err:?}");
    assert!(server.accepted().is_empty());
}

#[tokio::test]
async fn every_sign_in_mechanism_works() {
    for mech in ["PLAIN", "LOGIN", "XOAUTH2"] {
        let server = with_options(SmtpOptions {
            mechanisms: vec![mech],
            ..SmtpOptions::default()
        })
        .await;
        let cred = if mech == "XOAUTH2" {
            Credential::oauth2(server.username(), server.password())
        } else {
            Credential::password(server.password())
        };
        let sender =
            SmtpSender::new(config(&server, SmtpSecurity::PlaintextForTestsOnly), cred).unwrap();
        sender
            .send("a@example.org", &["b@example.org"], MSG)
            .await
            .unwrap_or_else(|e| panic!("{mech}: {e:?}"));
        assert_eq!(server.accepted().len(), 1, "{mech}");
        // The password is never in the command log.
        assert!(
            server.commands().iter().all(|c| !c.contains("horse")),
            "{mech}"
        );
    }
}

#[tokio::test]
async fn wrong_credentials_are_an_auth_error_for_every_mechanism() {
    for mech in ["PLAIN", "LOGIN", "XOAUTH2"] {
        let server = with_options(SmtpOptions {
            mechanisms: vec![mech],
            ..SmtpOptions::default()
        })
        .await;
        let cred = if mech == "XOAUTH2" {
            Credential::oauth2(server.username(), "expired-token")
        } else {
            Credential::password("wrong")
        };
        let sender =
            SmtpSender::new(config(&server, SmtpSecurity::PlaintextForTestsOnly), cred).unwrap();
        let err = sender
            .send("a@example.org", &["b@example.org"], MSG)
            .await
            .unwrap_err();
        assert!(matches!(err, BackendError::Auth(_)), "{mech}: {err:?}");
        assert!(server.accepted().is_empty(), "{mech}");
        // After the failure the sender did not carry on to MAIL FROM.
        assert!(
            !server.commands().iter().any(|c| c.starts_with("MAIL")),
            "{mech}"
        );
    }
}

#[tokio::test]
async fn a_server_with_no_usable_sign_in_method_is_an_auth_error() {
    let server = with_options(SmtpOptions {
        mechanisms: vec!["PLAIN"],
        ..SmtpOptions::default()
    })
    .await;
    let sender = SmtpSender::new(
        config(&server, SmtpSecurity::PlaintextForTestsOnly),
        Credential::oauth2("alice", "token"),
    )
    .unwrap();
    let err = sender
        .send("a@example.org", &["b@example.org"], MSG)
        .await
        .unwrap_err();
    assert!(matches!(err, BackendError::Auth(_)), "{err:?}");
}

#[tokio::test]
async fn hanging_up_during_sign_in_is_an_auth_error_not_a_retry() {
    let server = SmtpServer::start_default().await;
    server.set_misbehavior(SmtpMisbehavior::HangUpOnAuth);
    let err = plain(&server)
        .send("a@example.org", &["b@example.org"], MSG)
        .await
        .unwrap_err();
    assert!(matches!(err, BackendError::Auth(_)), "{err:?}");
}

#[tokio::test]
async fn temporary_answers_are_transient_at_every_stage() {
    for (stage, code) in [
        (Stage::Auth, 454),
        (Stage::MailFrom, 451),
        (Stage::RcptTo, 450),
        (Stage::Data, 451),
        (Stage::EndOfData, 452),
    ] {
        let server = SmtpServer::start_default().await;
        server.set_reply(stage, code, "try again with secret@example.org");
        let err = plain(&server)
            .send("a@example.org", &["b@example.org"], MSG)
            .await
            .unwrap_err();
        assert!(
            matches!(err, BackendError::Transient(_)),
            "{stage:?}: {err:?}"
        );
        assert!(!format!("{err:?}").contains("secret@"), "{stage:?}");
        assert!(server.accepted().is_empty(), "{stage:?}");
    }
}

#[tokio::test]
async fn refusals_are_permanent_with_a_fixed_reason() {
    for (stage, code) in [
        (Stage::MailFrom, 550),
        (Stage::RcptTo, 550),
        (Stage::Data, 554),
        (Stage::EndOfData, 554),
    ] {
        let server = SmtpServer::start_default().await;
        server.set_reply(stage, code, "no such user b@example.org");
        let err = plain(&server)
            .send("a@example.org", &["b@example.org"], MSG)
            .await
            .unwrap_err();
        let BackendError::Permanent(reason) = &err else {
            panic!("{stage:?}: {err:?}");
        };
        assert!(
            !reason.contains('@') && !reason.contains("no such"),
            "{reason}"
        );
        assert!(server.accepted().is_empty(), "{stage:?}");
    }
}

#[tokio::test]
async fn a_message_that_is_too_large_is_refused_up_front_or_by_the_server() {
    // The server states a limit: Croft does not even start.
    let server = with_options(SmtpOptions {
        size_limit: Some(100),
        ..SmtpOptions::default()
    })
    .await;
    let big = vec![b'x'; 1000];
    let err = plain(&server)
        .send("a@example.org", &["b@example.org"], &big)
        .await
        .unwrap_err();
    assert!(matches!(err, BackendError::Permanent(_)), "{err:?}");
    assert!(!server.commands().iter().any(|c| c == "DATA"));

    // The server answers 552 after the body.
    let server = SmtpServer::start_default().await;
    server.set_reply(Stage::EndOfData, 552, "too big");
    let err = plain(&server)
        .send("a@example.org", &["b@example.org"], MSG)
        .await
        .unwrap_err();
    assert!(matches!(err, BackendError::Permanent(_)), "{err:?}");
}

#[tokio::test]
async fn one_refused_recipient_sends_nothing_to_anyone() {
    let server = SmtpServer::start_default().await;
    server.set_rcpt_reply("bad@example.org", 550, "no such user");
    let err = plain(&server)
        .send(
            "a@example.org",
            &["good@example.org", "bad@example.org", "also@example.org"],
            MSG,
        )
        .await
        .unwrap_err();
    assert!(matches!(err, BackendError::Permanent(_)), "{err:?}");
    assert!(server.accepted().is_empty());
    assert!(
        !server.commands().iter().any(|c| c == "DATA"),
        "DATA must not be issued when a recipient was refused: {:?}",
        server.commands()
    );
    // A temporary refusal of one recipient is retryable, and also sends nothing.
    server.set_rcpt_reply("bad@example.org", 450, "mailbox busy");
    let err = plain(&server)
        .send(
            "a@example.org",
            &["good@example.org", "bad@example.org"],
            MSG,
        )
        .await
        .unwrap_err();
    assert!(matches!(err, BackendError::Transient(_)), "{err:?}");
    assert!(server.accepted().is_empty());
}

#[tokio::test]
async fn authentication_codes_after_sign_in_pause_the_account() {
    for code in [530, 534, 535] {
        let server = SmtpServer::start_default().await;
        server.set_reply(Stage::MailFrom, code, "go away");
        let err = plain(&server)
            .send("a@example.org", &["b@example.org"], MSG)
            .await
            .unwrap_err();
        assert!(matches!(err, BackendError::Auth(_)), "{code}: {err:?}");
    }
}

#[tokio::test]
async fn nothing_is_listening_is_transient() {
    let port = {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap().port()
    };
    let sender = SmtpSender::new(
        SmtpConfig::new("127.0.0.1", port, SmtpSecurity::PlaintextForTestsOnly, "u")
            .with_timeout(Duration::from_secs(2)),
        Credential::password("pw"),
    )
    .unwrap();
    let err = sender
        .send("a@example.org", &["b@example.org"], MSG)
        .await
        .unwrap_err();
    assert!(matches!(err, BackendError::Transient(_)), "{err:?}");
}

#[tokio::test]
async fn a_refused_greeting_is_transient() {
    let server = SmtpServer::start_default().await;
    server.set_misbehavior(SmtpMisbehavior::RefuseGreeting);
    let err = plain(&server)
        .send("a@example.org", &["b@example.org"], MSG)
        .await
        .unwrap_err();
    assert!(matches!(err, BackendError::Transient(_)), "{err:?}");
}

#[tokio::test]
async fn a_server_that_never_answers_is_transient_before_the_body() {
    // A listener that accepts and says nothing: the greeting never comes.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let _hold = tokio::spawn(async move {
        let mut keep = Vec::new();
        while let Ok((s, _)) = listener.accept().await {
            keep.push(s);
        }
    });
    let sender = SmtpSender::new(
        SmtpConfig::new("127.0.0.1", port, SmtpSecurity::PlaintextForTestsOnly, "u")
            .with_timeout(Duration::from_millis(300)),
        Credential::password("pw"),
    )
    .unwrap();
    let err = sender
        .send("a@example.org", &["b@example.org"], MSG)
        .await
        .unwrap_err();
    assert!(matches!(err, BackendError::Transient(_)), "{err:?}");
}

// ---- the duplicate-send rule ----------------------------------------------------------------

#[tokio::test]
async fn a_server_that_takes_the_message_and_drops_the_connection_is_never_retried() {
    let server = SmtpServer::start_default().await;
    server.set_misbehavior(SmtpMisbehavior::AcceptThenDrop);
    let err = plain(&server)
        .send("a@example.org", &["b@example.org"], MSG)
        .await
        .unwrap_err();
    assert_eq!(err, BackendError::Permanent(MAYBE_SENT_REASON.into()));
    // The server really did keep it: this is exactly why the answer must not be Transient.
    assert_eq!(server.accepted().len(), 1);
}

#[tokio::test]
async fn silence_after_the_body_is_treated_the_same_way() {
    let server = SmtpServer::start_default().await;
    server.set_misbehavior(SmtpMisbehavior::HangAfterData);
    let sender = SmtpSender::new(
        config(&server, SmtpSecurity::PlaintextForTestsOnly)
            .with_deadline(Duration::from_millis(600)),
        Credential::password(server.password()),
    )
    .unwrap();
    let err = sender
        .send("a@example.org", &["b@example.org"], MSG)
        .await
        .unwrap_err();
    assert_eq!(err, BackendError::Permanent(MAYBE_SENT_REASON.into()));

    server.set_misbehavior(SmtpMisbehavior::DropWithoutAccepting);
    let err = plain(&server)
        .send("a@example.org", &["b@example.org"], MSG)
        .await
        .unwrap_err();
    assert_eq!(err, BackendError::Permanent(MAYBE_SENT_REASON.into()));
}

// ---- TLS ----------------------------------------------------------------------------------

#[tokio::test]
async fn implicit_tls_with_a_pinned_certificate_sends() {
    let server = with_options(SmtpOptions {
        tls: TlsMode::Implicit,
        ..SmtpOptions::default()
    })
    .await;
    pinned(&server, SmtpSecurity::Tls)
        .send("a@example.org", &["b@example.org"], MSG)
        .await
        .unwrap();
    let got = server.accepted();
    assert_eq!(got.len(), 1);
    assert!(got[0].over_tls);
}

#[tokio::test]
async fn an_untrusted_certificate_is_an_auth_error_and_nothing_secret_is_sent() {
    for (tls, security) in [
        (TlsMode::Implicit, SmtpSecurity::Tls),
        (TlsMode::StartTls, SmtpSecurity::StartTls),
    ] {
        let server = with_options(SmtpOptions {
            tls,
            ..SmtpOptions::default()
        })
        .await;
        // No pin: the self-signed certificate does not chain to a trusted root.
        let sender = SmtpSender::new(
            config(&server, security),
            Credential::password(server.password()),
        )
        .unwrap();
        let err = sender
            .send("a@example.org", &["b@example.org"], MSG)
            .await
            .unwrap_err();
        assert!(matches!(err, BackendError::Auth(_)), "{tls:?}: {err:?}");
        assert!(
            !server.commands().iter().any(|c| c.starts_with("AUTH")),
            "{tls:?}: the password must not be offered over an unverified channel"
        );
        // A pin for some other certificate is refused the same way.
        let wrong = CertFingerprint::of_der(b"some other certificate");
        let sender = SmtpSender::new(
            config(&server, security).with_pinned_cert(wrong),
            Credential::password(server.password()),
        )
        .unwrap();
        let err = sender
            .send("a@example.org", &["b@example.org"], MSG)
            .await
            .unwrap_err();
        assert!(matches!(err, BackendError::Auth(_)), "{tls:?}: {err:?}");
        assert!(server.accepted().is_empty());
    }
}

#[tokio::test]
async fn starttls_upgrades_before_signing_in() {
    let server = with_options(SmtpOptions {
        tls: TlsMode::StartTls,
        ..SmtpOptions::default()
    })
    .await;
    pinned(&server, SmtpSecurity::StartTls)
        .send("a@example.org", &["b@example.org"], MSG)
        .await
        .unwrap();
    let cmds = server.commands();
    let tls_at = cmds.iter().position(|c| c == "STARTTLS").unwrap();
    let auth_at = cmds.iter().position(|c| c.starts_with("AUTH")).unwrap();
    assert!(tls_at < auth_at, "{cmds:?}");
    assert!(server.accepted()[0].over_tls);
}

#[tokio::test]
async fn starttls_never_falls_back_to_plaintext() {
    // The server offers no STARTTLS at all.
    let server = SmtpServer::start_default().await;
    let sender = SmtpSender::new(
        config(&server, SmtpSecurity::StartTls),
        Credential::password(server.password()),
    )
    .unwrap();
    let err = sender
        .send("a@example.org", &["b@example.org"], MSG)
        .await
        .unwrap_err();
    assert!(matches!(err, BackendError::Permanent(_)), "{err:?}");
    assert!(!server.commands().iter().any(|c| c.starts_with("AUTH")));
    assert!(server.accepted().is_empty());
}

#[tokio::test]
async fn plaintext_injected_after_starttls_is_refused() {
    let server = with_options(SmtpOptions {
        tls: TlsMode::StartTls,
        ..SmtpOptions::default()
    })
    .await;
    server.set_misbehavior(SmtpMisbehavior::InjectAfterStartTls);
    let err = pinned(&server, SmtpSecurity::StartTls)
        .send("a@example.org", &["b@example.org"], MSG)
        .await
        .unwrap_err();
    assert!(matches!(err, BackendError::Auth(_)), "{err:?}");
    assert!(server.accepted().is_empty());
}

// ---- input checks -------------------------------------------------------------------------

#[tokio::test]
async fn bad_addresses_never_reach_the_network() {
    let server = SmtpServer::start_default().await;
    let sender = plain(&server);
    for (from, to) in [
        ("a@example.org", "b@example.org\r\nDATA"),
        ("a@example.org\r\nRSET", "b@example.org"),
        ("a@example.org", "not-an-address"),
        ("a@example.org", "<b@example.org>"),
    ] {
        let err = sender.send(from, &[to], MSG).await.unwrap_err();
        assert!(matches!(err, BackendError::Permanent(_)), "{from:?} {to:?}");
    }
    let none: [&str; 0] = [];
    assert!(matches!(
        sender.send("a@example.org", &none, MSG).await,
        Err(BackendError::Permanent(_))
    ));
    assert_eq!(server.connection_count(), 0);
}

#[tokio::test]
async fn a_message_containing_an_end_of_data_line_cannot_inject_commands() {
    let server = SmtpServer::start_default().await;
    let evil = b"Subject: x\r\n\r\nbody\r\n.\r\nMAIL FROM:<evil@example.org>\r\nRCPT TO:<victim@example.org>\r\nDATA\r\nspam\r\n.\r\n";
    plain(&server)
        .send("a@example.org", &["b@example.org"], evil)
        .await
        .unwrap();
    let got = server.accepted();
    assert_eq!(
        got.len(),
        1,
        "the injected part must not become a second message"
    );
    assert_eq!(got[0].rcpt_to, ["b@example.org"]);
    assert!(!server.commands().iter().any(|c| c.contains("evil@")));
}

#[tokio::test]
async fn the_certificate_probe_reads_the_fingerprint_and_sends_no_credential() {
    for (tls, security) in [
        (TlsMode::Implicit, SmtpSecurity::Tls),
        (TlsMode::StartTls, SmtpSecurity::StartTls),
    ] {
        let server = with_options(SmtpOptions {
            tls,
            ..SmtpOptions::default()
        })
        .await;
        let seen = probe_smtp_certificate(&config(&server, security))
            .await
            .unwrap();
        assert_eq!(
            seen,
            CertFingerprint::of_der(&server.certificate_der().unwrap())
        );
        assert!(
            !server
                .commands()
                .iter()
                .any(|c| c.starts_with("AUTH") || c.starts_with("MAIL")),
            "{tls:?}: {:?}",
            server.commands()
        );
        // Probing trusts nothing: the same settings still refuse the certificate when sending.
        let sender = SmtpSender::new(config(&server, security), Credential::password("x")).unwrap();
        assert!(matches!(
            sender.send("a@example.org", &["b@example.org"], MSG).await,
            Err(BackendError::Auth(_))
        ));
    }
    let server = SmtpServer::start_default().await;
    assert!(
        probe_smtp_certificate(&config(&server, SmtpSecurity::PlaintextForTestsOnly))
            .await
            .is_err()
    );
}
