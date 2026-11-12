// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! OAuth (T21) against the fake token endpoint and the in-process IMAP server: code exchange,
//! refresh with rotation, `invalid_grant` as `Auth`, a slow endpoint as `Transient`, and XOAUTH2
//! with a good and an expired token. No network. Every token is made up.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use croft_protocols::oauth::token::{OAuthClient, TokenEndpoint};
use croft_protocols::oauth::{OAuthError, OAuthProvider, Pkce};
use croft_protocols::*;
use croft_test_support::oauth::FakeTokenEndpoint;
use croft_test_support::TestServer;
use std::time::Duration;

fn client() -> OAuthClient {
    OAuthClient {
        client_id: "made-up-client".into(),
        client_secret: None,
    }
}

async fn endpoint() -> (FakeTokenEndpoint, TokenEndpoint) {
    let fake = FakeTokenEndpoint::start().await;
    let ep = TokenEndpoint::loopback(OAuthProvider::Google, &fake.url()).unwrap();
    (fake, ep)
}

#[tokio::test]
async fn a_code_is_exchanged_with_the_verifier_and_no_secret() {
    let (fake, ep) = endpoint().await;
    let pkce = Pkce::generate().unwrap();
    fake.add_code("code-1", &pkce.challenge, "refresh-0");
    let t = ep
        .exchange_code(&client(), "code-1", "http://127.0.0.1:1", &pkce.verifier, 0)
        .await
        .unwrap();
    assert_eq!(&*t.access_token, "access-1");
    assert_eq!(
        t.refresh_token.as_deref().map(String::as_str),
        Some("refresh-0")
    );
    let sent = fake.requests().join("\n");
    assert!(sent.contains("code_verifier="));
    assert!(
        !sent.contains("client_secret"),
        "a public client sends no secret"
    );

    // A code works once; a wrong verifier never.
    assert_eq!(
        ep.exchange_code(&client(), "code-1", "http://127.0.0.1:1", &pkce.verifier, 0)
            .await
            .unwrap_err(),
        OAuthError::InvalidGrant
    );
    fake.add_code(
        "code-2",
        &croft_protocols::oauth::challenge("the-right-one"),
        "refresh-x",
    );
    assert_eq!(
        ep.exchange_code(&client(), "code-2", "http://127.0.0.1:1", "a-wrong-one", 0)
            .await
            .unwrap_err(),
        OAuthError::InvalidGrant
    );
}

#[tokio::test]
async fn a_refresh_rotates_the_refresh_token_and_a_revoked_one_is_auth() {
    let (fake, ep) = endpoint().await;
    fake.add_refresh_token("refresh-0");
    fake.set_rotation(true);
    let t = ep.refresh(&client(), "refresh-0", 0).await.unwrap();
    let next = t.refresh_token.clone().expect("rotated");
    assert_ne!(next.as_str(), "refresh-0");
    // The old one is gone; the new one works.
    let old = ep.refresh(&client(), "refresh-0", 0).await.unwrap_err();
    assert_eq!(old, OAuthError::InvalidGrant);
    assert!(matches!(BackendError::from(old), BackendError::Auth(_)));
    ep.refresh(&client(), &next, 0).await.unwrap();
}

#[tokio::test]
async fn a_slow_endpoint_is_transient() {
    let (fake, ep) = endpoint().await;
    fake.add_refresh_token("refresh-0");
    fake.set_slow(Some(Duration::from_secs(6)));
    let e = ep.refresh(&client(), "refresh-0", 0).await.unwrap_err();
    assert_eq!(e, OAuthError::Transient);
    assert!(matches!(BackendError::from(e), BackendError::Transient(_)));
}

#[tokio::test]
async fn a_client_secret_is_sent_only_when_the_person_gave_one() {
    let (fake, ep) = endpoint().await;
    fake.add_refresh_token("refresh-0");
    let with_secret = OAuthClient {
        client_id: "made-up-client".into(),
        client_secret: Some(zeroize::Zeroizing::new("made-up-secret".into())),
    };
    ep.refresh(&with_secret, "refresh-0", 0).await.unwrap();
    assert!(fake.requests()[0].contains("client_secret=made-up-secret"));
}

fn imap(server: &TestServer, token: &str) -> ImapBackend {
    ImapBackend::new(
        ImapConfig::new(
            server.host(),
            server.port(),
            Security::PlaintextForTestsOnly,
            server.username(),
        )
        .with_timeout(Duration::from_secs(5)),
        Credential::oauth2(server.username(), token),
    )
    .unwrap()
}

#[tokio::test]
async fn imap_accepts_a_valid_access_token_and_refuses_an_expired_one() {
    let server = TestServer::start_default().await;
    server.set_password("access-1");
    imap(&server, "access-1").list_folders().await.unwrap();
    // The token expired: the server now wants another.
    server.set_password("access-2");
    assert!(matches!(
        imap(&server, "access-1").list_folders().await,
        Err(BackendError::Auth(_))
    ));
    imap(&server, "access-2").list_folders().await.unwrap();
}
