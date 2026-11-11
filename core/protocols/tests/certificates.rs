// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Certificate handling in `verify` and `check_certificate`, against an in-process IMAP server
//! whose certificate this file controls: a private authority (trusted for this test process
//! through `SSL_CERT_FILE`, which the platform root store reads), expired and wrong-name leaves,
//! and self-signed ones.
//!
//! The server logs every command it receives (a `LOGIN` is logged as `LOGIN ***`), so the tests
//! can prove that no credential is sent before the certificate is accepted.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use croft_protocols::discovery::*;
use croft_protocols::{
    check_certificate, CertFingerprint, CertificateProblem, Credential, Security,
};
use rcgen::{
    BasicConstraints, CertificateParams, DnType, ExtendedKeyUsagePurpose, IsCa, Issuer, KeyPair,
    KeyUsagePurpose,
};
use std::sync::{Arc, Mutex, Once, OnceLock};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;
use tokio_rustls::rustls::crypto::ring;
use tokio_rustls::rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use tokio_rustls::rustls::ServerConfig;
use tokio_rustls::TlsAcceptor;

const PASSWORD: &str = "correct horse";

static CA: OnceLock<Issuer<'static, KeyPair>> = OnceLock::new();
static CA_DIR: OnceLock<tempfile::TempDir> = OnceLock::new();
static INIT: Once = Once::new();

/// Makes the private authority and tells the platform root store about it. Every test calls this
/// first, so the variable is set before anything reads the roots (they are read once per process).
fn private_ca() -> &'static Issuer<'static, KeyPair> {
    INIT.call_once(|| {
        let mut params = CertificateParams::new(Vec::<String>::new()).unwrap();
        params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
        params
            .distinguished_name
            .push(DnType::CommonName, "Croft test authority");
        let key = KeyPair::generate().unwrap();
        let cert = params.self_signed(&key).unwrap();
        // The directory lives in a static, so it stays until the test process ends.
        let dir = CA_DIR.get_or_init(|| tempfile::tempdir().unwrap());
        let path = dir.path().join("croft-test-ca.pem");
        std::fs::write(&path, cert.pem()).unwrap();
        // Edition 2021: safe. Done once, before any test reads the root store.
        std::env::set_var("SSL_CERT_FILE", &path);
        let _ = CA.set(Issuer::new(params, key));
    });
    CA.get().unwrap()
}

#[derive(Clone, Copy, PartialEq)]
enum Leaf {
    /// Signed by the private authority, valid, for `localhost`.
    Good,
    /// Signed by the private authority, valid for `localhost` until 2001.
    Expired,
    /// Signed by the private authority, valid, for `elsewhere.test`.
    WrongName,
    /// Signed by itself, for `localhost`.
    SelfSigned,
}

struct Identity {
    chain: Vec<CertificateDer<'static>>,
    key: PrivateKeyDer<'static>,
}

impl Identity {
    fn new(kind: Leaf) -> Identity {
        let ca = private_ca();
        let name = if kind == Leaf::WrongName {
            "elsewhere.test"
        } else {
            "localhost"
        };
        let mut params = CertificateParams::new(vec![name.to_string()]).unwrap();
        params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
        if kind == Leaf::Expired {
            params.not_before = rcgen::date_time_ymd(2000, 1, 1);
            params.not_after = rcgen::date_time_ymd(2001, 1, 1);
        }
        let key = KeyPair::generate().unwrap();
        let cert = if kind == Leaf::SelfSigned {
            params.self_signed(&key).unwrap()
        } else {
            params.signed_by(&key, ca).unwrap()
        };
        Identity {
            chain: vec![CertificateDer::from(cert.der().to_vec())],
            key: PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key.serialize_der())),
        }
    }

    fn fingerprint(&self) -> CertFingerprint {
        CertFingerprint::of_der(self.chain[0].as_ref())
    }
}

/// How the server answers once a client is connected.
#[derive(Clone, Copy, PartialEq)]
enum Behaviour {
    /// Accepts the password, answers `CAPABILITY`.
    Normal,
    /// Behaves for the first connection (the certificate check) and answers every later one with
    /// a fatal TLS alert, so signing in fails with a `Permanent` error from the adapter.
    AlertsAfterTheFirstConnection,
    /// Answers a failed `LOGIN`, closes the connection, and stops listening, so later
    /// connections are refused.
    VanishesAfterAFailedLogin,
}

struct Server {
    port: u16,
    log: Arc<Mutex<Vec<String>>>,
}

impl Server {
    fn commands(&self) -> Vec<String> {
        self.log.lock().unwrap().clone()
    }

    fn saw_sign_in(&self) -> bool {
        self.commands()
            .iter()
            .any(|c| c.starts_with("LOGIN") || c.starts_with("AUTHENTICATE"))
    }
}

/// An implicit-TLS IMAP server presenting `identity`.
async fn serve(identity: Identity, behaviour: Behaviour) -> Server {
    let cfg = ServerConfig::builder_with_provider(Arc::new(ring::default_provider()))
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(identity.chain, identity.key)
        .unwrap();
    let acceptor = TlsAcceptor::from(Arc::new(cfg));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let log = Arc::new(Mutex::new(Vec::new()));
    let seen = log.clone();
    let stop = Arc::new(tokio::sync::Notify::new());
    let stopper = stop.clone();
    tokio::spawn(async move {
        let mut accepted = 0usize;
        loop {
            let mut tcp = tokio::select! {
                accepted = listener.accept() => match accepted {
                    Ok((tcp, _)) => tcp,
                    Err(_) => break,
                },
                _ = stop.notified() => break,
            };
            accepted += 1;
            if behaviour == Behaviour::AlertsAfterTheFirstConnection && accepted > 1 {
                // A TLS record: alert, fatal, handshake_failure.
                let _ = tcp
                    .write_all(&[0x15, 0x03, 0x03, 0x00, 0x02, 0x02, 0x28])
                    .await;
                continue;
            }
            let acceptor = acceptor.clone();
            let log = seen.clone();
            let stopper = stopper.clone();
            tokio::spawn(async move {
                let Ok(tls) = acceptor.accept(tcp).await else {
                    return;
                };
                let (reader, mut writer) = tokio::io::split(tls);
                let mut reader = BufReader::new(reader);
                let _ = writer.write_all(b"* OK ready\r\n").await;
                let mut line = String::new();
                while reader.read_line(&mut line).await.unwrap_or(0) > 0 {
                    let mut parts = line.trim_end().splitn(3, ' ');
                    let tag = parts.next().unwrap_or("x").to_string();
                    let command = parts.next().unwrap_or("").to_ascii_uppercase();
                    let args = parts.next().unwrap_or("").to_string();
                    let reply = match command.as_str() {
                        "LOGIN" => {
                            log.lock().unwrap().push("LOGIN ***".into());
                            if args.contains(PASSWORD) {
                                format!("{tag} OK done\r\n")
                            } else if behaviour == Behaviour::VanishesAfterAFailedLogin {
                                // Stop listening first, then answer and hang up.
                                stopper.notify_one();
                                tokio::time::sleep(Duration::from_millis(100)).await;
                                let _ = writer
                                    .write_all(
                                        format!("{tag} NO [AUTHENTICATIONFAILED] nope\r\n")
                                            .as_bytes(),
                                    )
                                    .await;
                                return;
                            } else {
                                format!("{tag} NO [AUTHENTICATIONFAILED] nope\r\n")
                            }
                        }
                        "CAPABILITY" => {
                            log.lock().unwrap().push(command.clone());
                            format!("* CAPABILITY IMAP4rev1 MOVE UIDPLUS\r\n{tag} OK done\r\n")
                        }
                        "LOGOUT" => {
                            log.lock().unwrap().push(command.clone());
                            format!("* BYE\r\n{tag} OK bye\r\n")
                        }
                        _ => {
                            log.lock().unwrap().push(command.clone());
                            format!("{tag} OK\r\n")
                        }
                    };
                    let _ = writer.write_all(reply.as_bytes()).await;
                    line.clear();
                }
            });
        }
    });
    Server { port, log }
}

fn settings(port: u16, provider: Option<Provider>) -> DiscoveredSettings {
    DiscoveredSettings {
        imap: Some(ImapServer {
            host: "localhost".into(),
            port,
            security: Security::Tls,
            username: "alice@certs.test".into(),
        }),
        smtp: None,
        jmap: None,
        source: if provider.is_some() {
            Source::ProviderTable
        } else {
            Source::Srv
        },
        provider,
        auth_hint: AuthHint::Password,
    }
}

fn options(pin: Option<CertFingerprint>) -> VerifyOptions {
    VerifyOptions {
        pinned_cert: pin,
        timeout: Duration::from_secs(5),
    }
}

fn password(p: &str) -> Credential {
    Credential::password(p)
}

#[tokio::test]
async fn a_certificate_from_a_trusted_private_authority_needs_no_pin() {
    private_ca();
    let server = serve(Identity::new(Leaf::Good), Behaviour::Normal).await;
    let ok = verify(
        &settings(server.port, None),
        password(PASSWORD),
        options(None),
    )
    .await
    .unwrap();
    assert_eq!(ok.imap.pinned_cert, None);
    assert!(server.saw_sign_in());
}

#[tokio::test]
async fn an_expired_certificate_is_invalid_and_nothing_is_offered_to_trust() {
    private_ca();
    let identity = Identity::new(Leaf::Expired);
    let server = serve(identity, Behaviour::Normal).await;
    let err = verify(
        &settings(server.port, None),
        password(PASSWORD),
        options(None),
    )
    .await
    .unwrap_err();
    assert_eq!(err, VerifyError::CertificateInvalid);
    assert!(!server.saw_sign_in(), "{:?}", server.commands());
    assert!(!err.message().contains("self-signed"));
}

#[tokio::test]
async fn a_certificate_for_another_name_is_invalid_and_nothing_is_offered_to_trust() {
    private_ca();
    let server = serve(Identity::new(Leaf::WrongName), Behaviour::Normal).await;
    let err = verify(
        &settings(server.port, None),
        password(PASSWORD),
        options(None),
    )
    .await
    .unwrap_err();
    assert_eq!(err, VerifyError::CertificateInvalid);
    assert!(!server.saw_sign_in(), "{:?}", server.commands());
}

#[tokio::test]
async fn the_check_names_each_problem_without_sending_anything() {
    private_ca();
    let config = |port: u16| settings(port, None).imap.unwrap().to_config();
    let expired = serve(Identity::new(Leaf::Expired), Behaviour::Normal).await;
    assert_eq!(
        check_certificate(&config(expired.port)).await,
        Err(CertificateProblem::Expired)
    );
    let wrong_name = serve(Identity::new(Leaf::WrongName), Behaviour::Normal).await;
    assert_eq!(
        check_certificate(&config(wrong_name.port)).await,
        Err(CertificateProblem::WrongName)
    );
    let own = Identity::new(Leaf::SelfSigned);
    let fingerprint = own.fingerprint();
    let self_signed = serve(own, Behaviour::Normal).await;
    assert_eq!(
        check_certificate(&config(self_signed.port)).await,
        Err(CertificateProblem::UnknownIssuer { fingerprint })
    );
    let good_identity = Identity::new(Leaf::Good);
    let good_fingerprint = good_identity.fingerprint();
    let good = serve(good_identity, Behaviour::Normal).await;
    assert_eq!(check_certificate(&config(good.port)).await, Ok(()));
    // The pin decides when there is one: the right one passes, another one is a mismatch.
    assert_eq!(
        check_certificate(&config(self_signed.port).with_pinned_cert(fingerprint)).await,
        Ok(())
    );
    let other = CertFingerprint::of_der(b"another certificate");
    assert_eq!(
        check_certificate(&config(good.port).with_pinned_cert(other)).await,
        Err(CertificateProblem::PinMismatch {
            fingerprint: good_fingerprint
        })
    );
    for server in [&expired, &wrong_name, &self_signed, &good] {
        assert!(!server.saw_sign_in());
    }
}

#[tokio::test]
async fn a_self_signed_provider_certificate_is_interception_not_a_question() {
    private_ca();
    let server = serve(Identity::new(Leaf::SelfSigned), Behaviour::Normal).await;
    let mut messages = std::collections::HashSet::new();
    for provider in [
        Provider::Gmail,
        Provider::Outlook,
        Provider::ICloud,
        Provider::Fastmail,
    ] {
        let err = verify(
            &settings(server.port, Some(provider)),
            password(PASSWORD),
            options(None),
        )
        .await
        .unwrap_err();
        assert_eq!(err, VerifyError::ConnectionIntercepted);
        assert!(err.message().contains("intercepted"));
        assert!(err.message().contains("password was not sent"));
        messages.insert(err.message());
    }
    assert_eq!(messages.len(), 1);
    assert!(!server.saw_sign_in(), "{:?}", server.commands());
}

#[tokio::test]
async fn every_certificate_problem_on_a_provider_domain_is_interception() {
    private_ca();
    let provider = Some(Provider::Gmail);
    for kind in [Leaf::Expired, Leaf::WrongName] {
        let server = serve(Identity::new(kind), Behaviour::Normal).await;
        let err = verify(
            &settings(server.port, provider),
            password(PASSWORD),
            options(None),
        )
        .await
        .unwrap_err();
        assert_eq!(err, VerifyError::ConnectionIntercepted);
        assert!(!server.saw_sign_in());
    }
    // Also when a pin is set and the certificate is another one.
    let server = serve(Identity::new(Leaf::SelfSigned), Behaviour::Normal).await;
    let err = verify(
        &settings(server.port, provider),
        password(PASSWORD),
        options(Some(CertFingerprint::of_der(b"old certificate"))),
    )
    .await
    .unwrap_err();
    assert_eq!(err, VerifyError::ConnectionIntercepted);
    assert!(!server.saw_sign_in());
}

#[tokio::test]
async fn self_hosted_self_signed_asks_once_then_signs_in_with_the_pin() {
    private_ca();
    let identity = Identity::new(Leaf::SelfSigned);
    let fingerprint = identity.fingerprint();
    let server = serve(identity, Behaviour::Normal).await;
    let found = settings(server.port, None);
    let err = verify(&found, password(PASSWORD), options(None))
        .await
        .unwrap_err();
    assert_eq!(err, VerifyError::UntrustedCertificate { fingerprint });
    assert!(!server.saw_sign_in());
    let message = err.message();
    assert!(message.contains("expected only if you"));
    assert!(message.contains("do not trust it"));
    let ok = verify(&found, password(PASSWORD), options(Some(fingerprint)))
        .await
        .unwrap();
    assert_eq!(ok.imap.pinned_cert, Some(fingerprint));
    assert!(server.saw_sign_in());
}

#[tokio::test]
async fn an_unchanged_pin_and_a_later_permanent_failure_is_a_server_problem_not_a_changed_certificate(
) {
    private_ca();
    let identity = Identity::new(Leaf::SelfSigned);
    let fingerprint = identity.fingerprint();
    let server = serve(identity, Behaviour::AlertsAfterTheFirstConnection).await;
    let err = verify(
        &settings(server.port, None),
        password(PASSWORD),
        options(Some(fingerprint)),
    )
    .await
    .unwrap_err();
    assert_eq!(err, VerifyError::ServerProblem);
    assert!(!server.saw_sign_in());
    for other in [
        VerifyError::CertificateChanged { fingerprint },
        VerifyError::UntrustedCertificate { fingerprint },
        VerifyError::CertificateInvalid,
        VerifyError::WrongPassword,
    ] {
        assert_ne!(err.message(), other.message());
    }
}

#[tokio::test]
async fn a_different_certificate_than_the_pin_is_reported_as_changed() {
    private_ca();
    let identity = Identity::new(Leaf::SelfSigned);
    let presented = identity.fingerprint();
    let server = serve(identity, Behaviour::Normal).await;
    let pin = CertFingerprint::of_der(b"the certificate trusted last year");
    assert_ne!(pin, presented);
    let err = verify(
        &settings(server.port, None),
        password(PASSWORD),
        options(Some(pin)),
    )
    .await
    .unwrap_err();
    assert_eq!(
        err,
        VerifyError::CertificateChanged {
            fingerprint: presented
        }
    );
    assert!(!server.saw_sign_in());
}

#[tokio::test]
async fn a_wrong_password_behind_a_valid_pin_is_a_wrong_password() {
    private_ca();
    let identity = Identity::new(Leaf::SelfSigned);
    let fingerprint = identity.fingerprint();
    let server = serve(identity, Behaviour::Normal).await;
    let err = verify(
        &settings(server.port, None),
        password("not the password"),
        options(Some(fingerprint)),
    )
    .await
    .unwrap_err();
    assert_eq!(err, VerifyError::WrongPassword);
    assert!(server.saw_sign_in());
}

#[tokio::test]
async fn a_dropped_re_check_after_a_refused_sign_in_stays_a_wrong_password() {
    private_ca();
    let server = serve(
        Identity::new(Leaf::Good),
        Behaviour::VanishesAfterAFailedLogin,
    )
    .await;
    let err = verify(
        &settings(server.port, None),
        password("not the password"),
        options(None),
    )
    .await
    .unwrap_err();
    assert_eq!(err, VerifyError::WrongPassword);
}

#[tokio::test]
async fn a_pin_is_ignored_on_a_provider_domain() {
    private_ca();
    let identity = Identity::new(Leaf::SelfSigned);
    let fingerprint = identity.fingerprint();
    let server = serve(identity, Behaviour::Normal).await;
    let err = verify(
        &settings(server.port, Some(Provider::Fastmail)),
        password(PASSWORD),
        options(Some(fingerprint)),
    )
    .await
    .unwrap_err();
    assert_eq!(err, VerifyError::ConnectionIntercepted);
    assert!(!server.saw_sign_in(), "{:?}", server.commands());
}

#[tokio::test]
async fn a_provider_that_rejects_a_normal_password_gets_its_own_explanation() {
    private_ca();
    let server = serve(Identity::new(Leaf::Good), Behaviour::Normal).await;
    let mut messages = std::collections::HashSet::new();
    for provider in [
        Provider::Gmail,
        Provider::Outlook,
        Provider::ICloud,
        Provider::Fastmail,
    ] {
        let found = settings(server.port, Some(provider));
        let err = verify(&found, password("my normal password"), options(None))
            .await
            .unwrap_err();
        assert_eq!(err, VerifyError::NeedsAppPasswordOrOAuth(provider));
        assert_ne!(err.message(), VerifyError::WrongPassword.message());
        assert!(messages.insert(err.message()));
        // The right password still works.
        let ok = verify(&found, password(PASSWORD), options(None)).await;
        assert!(ok.is_ok());
    }
    assert!(messages.iter().any(|m| m.contains("app password")));
}
