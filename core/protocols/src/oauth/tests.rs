// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The pure part of OAuth (T21). Every token, code and secret below is made up for the tests.

use super::*;

#[test]
fn the_pkce_challenge_matches_rfc_7636_appendix_b() {
    let p = Pkce::from_verifier("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk");
    assert_eq!(p.challenge, "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM");
    assert_eq!(format!("{p:?}"), "Pkce(***)");
}

#[test]
fn a_generated_verifier_is_43_unreserved_characters_and_never_repeats() {
    let a = Pkce::generate().unwrap();
    let b = Pkce::generate().unwrap();
    assert_eq!(a.verifier.len(), 43);
    assert!(a
        .verifier
        .bytes()
        .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_'));
    assert_ne!(*a.verifier, *b.verifier);
    assert_eq!(a.challenge, challenge(&a.verifier));
}

fn url_for(provider: OAuthProvider) -> String {
    authorization_url(&AuthRequest {
        provider,
        client_id: "1234-made-up.apps.example",
        redirect_uri: "http://127.0.0.1:41234",
        state: "st4te",
        challenge: "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM",
        login_hint: Some("ann@example.org"),
    })
}

#[test]
fn the_google_url_has_the_mail_scope_s256_the_redirect_and_no_secret() {
    let u = url_for(OAuthProvider::Google);
    assert!(u.starts_with("https://accounts.google.com/o/oauth2/v2/auth?response_type=code&"));
    assert!(u.contains("&scope=https%3A%2F%2Fmail.google.com%2F&"));
    assert!(u.contains("&code_challenge=E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM&"));
    assert!(u.contains("&code_challenge_method=S256"));
    assert!(u.contains("&redirect_uri=http%3A%2F%2F127.0.0.1%3A41234&"));
    assert!(u.contains("&state=st4te&"));
    assert!(u.contains("&login_hint=ann%40example.org"));
    assert!(!u.contains("secret"));
}

#[test]
fn the_microsoft_url_asks_for_imap_smtp_and_a_refresh_token() {
    let u = url_for(OAuthProvider::Microsoft);
    assert!(u.starts_with("https://login.microsoftonline.com/common/oauth2/v2.0/authorize?"));
    assert!(u.contains("IMAP.AccessAsUser.All"));
    assert!(u.contains("SMTP.Send"));
    assert!(u.contains("offline_access"));
    assert!(u.contains("code_challenge_method=S256"));
    assert!(!u.contains("secret"));
}

const REDIRECT: &str = "com.example.croft:/oauth2redirect";

#[test]
fn a_good_redirect_gives_the_code() {
    let code = parse_redirect(
        "com.example.croft:/oauth2redirect?state=st4te&code=4%2F0Ab-made-up&scope=x",
        REDIRECT,
        "st4te",
    )
    .unwrap();
    assert_eq!(&*code, "4/0Ab-made-up");
}

#[test]
fn a_bad_redirect_is_refused_with_the_right_error() {
    let check = |url: &str| {
        parse_redirect(url, REDIRECT, "st4te")
            .map(|_| ())
            .unwrap_err()
    };
    assert_eq!(
        check("com.example.croft:/oauth2redirect?state=other&code=abc"),
        OAuthError::StateMismatch
    );
    assert_eq!(
        check("com.example.croft:/oauth2redirect?code=abc"),
        OAuthError::StateMismatch
    );
    assert_eq!(
        check("com.example.croft:/oauth2redirect?state=st4te"),
        OAuthError::MissingCode
    );
    assert_eq!(
        check("com.example.croft:/oauth2redirect?state=st4te&code="),
        OAuthError::MissingCode
    );
    assert_eq!(
        check("com.example.croft:/oauth2redirect?state=st4te&error=access_denied"),
        OAuthError::AccessDenied
    );
    assert_eq!(
        check("com.example.croft:/oauth2redirect?state=st4te&error=invalid_client&error_description=Bad+client+ann%40example.org"),
        OAuthError::Refused("invalid_client".into())
    );
    assert_eq!(
        check("https://evil.example/oauth2redirect?state=st4te&code=abc"),
        OAuthError::WrongRedirect
    );
    assert_eq!(
        check("com.example.croft:/oauth2redirect?state=st4te&code=%zz"),
        OAuthError::Malformed
    );
}

#[test]
fn an_error_with_the_wrong_state_is_a_state_mismatch_not_a_message_to_show() {
    // A forged "access denied" from another flow must not cancel this one.
    assert_eq!(
        parse_redirect(
            "com.example.croft:/oauth2redirect?state=old&error=access_denied",
            REDIRECT,
            "st4te"
        )
        .map(|_| ())
        .unwrap_err(),
        OAuthError::StateMismatch
    );
}

#[test]
fn a_provider_error_code_is_cut_to_a_short_safe_word() {
    let e = parse_redirect(
        &format!(
            "com.example.croft:/oauth2redirect?state=st4te&error={}%3Cscript%3E",
            "x".repeat(100)
        ),
        REDIRECT,
        "st4te",
    )
    .unwrap_err();
    assert_eq!(e, OAuthError::Refused("x".repeat(40)));
}

#[test]
fn the_token_bodies_carry_the_verifier_and_only_the_secret_that_was_given() {
    let b = code_exchange_body("cid", None, "c/1", "http://127.0.0.1:5", "verif");
    assert_eq!(
        &*b,
        "grant_type=authorization_code&code=c%2F1&redirect_uri=http%3A%2F%2F127.0.0.1%3A5&client_id=cid&code_verifier=verif"
    );
    let b = code_exchange_body("cid", Some("made-up-secret"), "c", "r", "v");
    assert!(b.ends_with("&client_secret=made-up-secret"));
    let r = refresh_body(OAuthProvider::Google, "cid", None, "1//rt");
    assert_eq!(
        &*r,
        "grant_type=refresh_token&refresh_token=1%2F%2Frt&client_id=cid"
    );
    let r = refresh_body(OAuthProvider::Microsoft, "cid", None, "rt");
    assert!(r.contains("&scope=https%3A%2F%2Foutlook.office.com%2FIMAP.AccessAsUser.All"));
}

#[test]
fn a_token_answer_is_read_and_errors_are_typed() {
    let t = parse_token_answer(
        200,
        br#"{"access_token":"ya29.made-up","expires_in":3599,"refresh_token":"1//made-up","token_type":"Bearer"}"#,
        1_000,
    )
    .unwrap();
    assert_eq!(&*t.access_token, "ya29.made-up");
    assert_eq!(
        t.refresh_token.as_deref().map(String::as_str),
        Some("1//made-up")
    );
    assert_eq!(t.expires_at_ms, 1_000 + 3_599_000);
    let shown = format!("{t:?}");
    assert!(!shown.contains("made-up"), "{shown}");

    let no_rotation = parse_token_answer(200, br#"{"access_token":"a"}"#, 0).unwrap();
    assert!(no_rotation.refresh_token.is_none());
    assert_eq!(no_rotation.expires_at_ms, 3_600_000);

    assert_eq!(
        parse_token_answer(400, br#"{"error":"invalid_grant","error_description":"Token has been expired or revoked."}"#, 0).unwrap_err(),
        OAuthError::InvalidGrant
    );
    assert_eq!(
        parse_token_answer(401, br#"{"error":"invalid_client"}"#, 0).unwrap_err(),
        OAuthError::Refused("invalid_client".into())
    );
    assert_eq!(
        parse_token_answer(503, b"", 0).unwrap_err(),
        OAuthError::Transient
    );
    assert_eq!(
        parse_token_answer(429, b"", 0).unwrap_err(),
        OAuthError::Transient
    );
    assert_eq!(
        parse_token_answer(200, b"<html>", 0).unwrap_err(),
        OAuthError::Malformed
    );
    assert_eq!(
        parse_token_answer(200, br#"{"token_type":"Bearer"}"#, 0).unwrap_err(),
        OAuthError::Malformed
    );
}

#[test]
fn invalid_grant_is_auth_and_a_slow_endpoint_is_transient() {
    assert!(matches!(
        BackendError::from(OAuthError::InvalidGrant),
        BackendError::Auth(_)
    ));
    assert!(matches!(
        BackendError::from(OAuthError::Transient),
        BackendError::Transient(_)
    ));
    assert!(matches!(
        BackendError::from(OAuthError::Refused("invalid_client".into())),
        BackendError::Permanent(_)
    ));
}

#[test]
fn the_xoauth2_string_has_the_documented_shape() {
    assert_eq!(
        &*xoauth2_string("ann@example.org", "ya29.made-up"),
        "user=ann@example.org\u{1}auth=Bearer ya29.made-up\u{1}\u{1}"
    );
}

#[test]
fn only_a_loopback_token_endpoint_can_replace_the_providers() {
    assert!(
        token::TokenEndpoint::loopback(OAuthProvider::Google, "http://127.0.0.1:9/token").is_some()
    );
    assert!(
        token::TokenEndpoint::loopback(OAuthProvider::Google, "http://evil.example/token")
            .is_none()
    );
    assert!(
        token::TokenEndpoint::loopback(OAuthProvider::Google, "https://127.0.0.1:9/token")
            .is_none()
    );
    let real = token::TokenEndpoint::for_provider(OAuthProvider::Microsoft).unwrap();
    assert!(format!("{real:?}").contains("login.microsoftonline.com"));
    for p in [OAuthProvider::Google, OAuthProvider::Microsoft] {
        assert!(p
            .token_url()
            .starts_with(&format!("https://{}/", p.token_host())));
        assert_eq!(OAuthProvider::from_key(p.key()), Some(p));
    }
}
