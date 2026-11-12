// This Source Code Form is subject to the terms of the GNU Affero General Public License,
// version 3 or later. See LICENSE-AGPL-3.0 at the repository root.

//! Tests of the sign-in API through the real router, with a software passkey
//! (`webauthn-authenticator-rs`, made-up keys) and a clock the test moves. The one thing these
//! cannot cover is real authenticators in real browsers (reported as unverified).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::BTreeSet;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use axum::Router;
use base64::Engine;
use http_body_util::BodyExt;
use serde_json::{json, Value};
use tower::ServiceExt;
use webauthn_authenticator_rs::softpasskey::SoftPasskey;
use webauthn_authenticator_rs::WebauthnAuthenticator;
use webauthn_rs::prelude::{CreationChallengeResponse, RequestChallengeResponse, Url};

use super::*;
use crate::auth::{ACCESS_TTL_MS, ENROL_TTL_MS, LOCK_MS};
use crate::http::{router, App};
use crate::store::Role;

const ORIGIN: &str = "https://companion.test";
const T0: i64 = 1_800_000_000_000;

struct H {
    app: App,
    router: Router,
    clock: Arc<AtomicI64>,
    master: Key,
    _mail_dir: tempfile::TempDir,
}

fn h() -> H {
    let master = Key::generate().unwrap();
    let store = Store::in_memory().unwrap();
    store.check_master_key(&master).unwrap();
    let clock = Arc::new(AtomicI64::new(T0));
    let c = clock.clone();
    let auth = Auth::new(
        ORIGIN,
        master.clone(),
        Vec::new(),
        Arc::new(move || c.load(Ordering::SeqCst)),
    )
    .unwrap();
    // With users' mail too, so the cross-user test reaches each user's own core.
    let mail_dir = tempfile::tempdir().unwrap();
    let app =
        App::new(store, auth).with_mail(crate::mail::MailCores::new(mail_dir.path().to_path_buf()));
    H {
        router: router(app.clone()),
        app,
        clock,
        master,
        _mail_dir: mail_dir,
    }
}

/// A person: their software passkey and, once signed in, their tokens.
struct Person {
    id: i64,
    name: String,
    key: SoftPasskey,
    totp_secret: Vec<u8>,
    recovery_codes: Vec<String>,
}

#[derive(Clone)]
struct Signed {
    access: String,
    refresh: String,
}

impl H {
    fn now(&self) -> i64 {
        self.clock.load(Ordering::SeqCst)
    }

    fn advance(&self, ms: i64) {
        self.clock.fetch_add(ms, Ordering::SeqCst);
    }

    fn store(&self) -> std::sync::MutexGuard<'_, Store> {
        self.app.store.lock().unwrap()
    }

    async fn call_from(
        &self,
        ip: &str,
        method: Method,
        path: &str,
        token: Option<&str>,
        body: Option<Value>,
    ) -> (StatusCode, Value) {
        let mut req = Request::builder().method(method).uri(path);
        if let Some(t) = token {
            req = req.header("authorization", format!("Bearer {t}"));
        }
        let mut req = match body {
            Some(b) => req
                .header("content-type", "application/json")
                .body(Body::from(b.to_string()))
                .unwrap(),
            None => req.body(Body::empty()).unwrap(),
        };
        req.extensions_mut()
            .insert(Peer(format!("{ip}:40000").parse().unwrap()));
        let res = self.router.clone().oneshot(req).await.unwrap();
        let status = res.status();
        let bytes = res.into_body().collect().await.unwrap().to_bytes();
        let v = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        (status, v)
    }

    /// From an address of its own each time, so the per-address limits stay out of the way of
    /// tests about something else.
    async fn call(
        &self,
        method: Method,
        path: &str,
        token: Option<&str>,
        body: Option<Value>,
    ) -> (StatusCode, Value) {
        static N: AtomicI64 = AtomicI64::new(1);
        let n = N.fetch_add(1, Ordering::SeqCst);
        let ip = format!("10.{}.{}.{}", (n >> 16) & 255, (n >> 8) & 255, n & 255);
        self.call_from(&ip, method, path, token, body).await
    }

    fn add_user(&self, name: &str, role: Role) -> i64 {
        self.store().add_user(&self.master, name, role).unwrap().id
    }

    fn enrolment(&self, user_id: i64) -> String {
        let now = self.now();
        self.store()
            .create_enrolment(user_id, None, now)
            .unwrap()
            .to_string()
    }

    /// Registers `key` with an enrolment link, as the page does.
    async fn enrol_with(
        &self,
        token: &str,
        key: &mut SoftPasskey,
        origin: &str,
    ) -> (StatusCode, Value) {
        let (s, start) = self
            .call(
                Method::POST,
                "/api/enrol/start",
                None,
                Some(json!({ "token": token })),
            )
            .await;
        assert_eq!(s, StatusCode::OK, "{start}");
        let ccr: CreationChallengeResponse =
            serde_json::from_value(start["options"].clone()).unwrap();
        let cred = key
            .do_registration(Url::parse(origin).unwrap(), ccr)
            .unwrap();
        self.call(
            Method::POST,
            "/api/enrol/finish",
            None,
            Some(json!({ "token": token, "ceremony": start["ceremony"], "credential": cred })),
        )
        .await
    }

    async fn person(&self, name: &str, role: Role) -> Person {
        let id = self.add_user(name, role);
        let token = self.enrolment(id);
        let mut key = SoftPasskey::new(true);
        let (s, done) = self.enrol_with(&token, &mut key, ORIGIN).await;
        assert_eq!(s, StatusCode::OK, "{done}");
        let secret = done["totp"]["secret"].as_str().unwrap();
        Person {
            id,
            name: name.to_owned(),
            key,
            totp_secret: base32_decode(secret),
            recovery_codes: done["recovery_codes"]
                .as_array()
                .unwrap()
                .iter()
                .map(|c| c.as_str().unwrap().to_owned())
                .collect(),
        }
    }

    /// A passkey assertion for a fresh named sign-in ceremony: (ceremony, credential).
    async fn assertion(&self, p: &mut Person) -> (Value, Value) {
        let (s, start) = self
            .call(
                Method::POST,
                "/api/sign-in/passkey/start",
                None,
                Some(json!({ "user": p.name })),
            )
            .await;
        assert_eq!(s, StatusCode::OK, "{start}");
        let rcr: RequestChallengeResponse =
            serde_json::from_value(start["options"].clone()).unwrap();
        let cred = p
            .key
            .do_authentication(Url::parse(ORIGIN).unwrap(), rcr)
            .unwrap();
        (
            start["ceremony"].clone(),
            serde_json::to_value(cred).unwrap(),
        )
    }

    async fn finish(&self, ceremony: &Value, credential: &Value) -> (StatusCode, Value) {
        self.call(
            Method::POST,
            "/api/sign-in/passkey/finish",
            None,
            Some(json!({ "ceremony": ceremony, "credential": credential, "device": "Test phone" })),
        )
        .await
    }

    async fn sign_in(&self, p: &mut Person) -> Signed {
        let (c, cred) = self.assertion(p).await;
        let (s, v) = self.finish(&c, &cred).await;
        assert_eq!(s, StatusCode::OK, "{v}");
        assert_eq!(v["recovery"], false);
        signed(&v)
    }

    fn totp_now(&self, p: &Person) -> String {
        let step = crate::totp::step_of(u64::try_from(self.now() / 1000).unwrap());
        format!("{:06}", crate::totp::code_at(&p.totp_secret, step))
    }

    async fn recover(&self, p: &Person, totp: &str, code: &str) -> (StatusCode, Value) {
        self.call(
            Method::POST,
            "/api/sign-in/recovery",
            None,
            Some(json!({ "user": p.name, "totp": totp, "recovery_code": code })),
        )
        .await
    }

    async fn status(&self, method: Method, path: &str, token: Option<&str>) -> StatusCode {
        self.call(method, path, token, None).await.0
    }
}

fn signed(v: &Value) -> Signed {
    Signed {
        access: v["access_token"].as_str().unwrap().to_owned(),
        refresh: v["refresh_token"].as_str().unwrap().to_owned(),
    }
}

fn base32_decode(s: &str) -> Vec<u8> {
    const A: &str = "ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
    let (mut buf, mut bits, mut out) = (0u32, 0, Vec::new());
    for c in s.chars() {
        buf = (buf << 5) | A.find(c).unwrap() as u32;
        bits += 5;
        if bits >= 8 {
            bits -= 8;
            out.push((buf >> bits) as u8);
        }
    }
    out
}

// ---- Enrolment and passkeys ----

#[tokio::test]
async fn enrolment_registers_a_passkey_once_and_the_passkey_signs_in() {
    let h = h();
    let id = h.add_user("ann", Role::User);
    let token = h.enrolment(id);
    let mut key = SoftPasskey::new(true);
    let (s, done) = h.enrol_with(&token, &mut key, ORIGIN).await;
    assert_eq!(s, StatusCode::OK, "{done}");
    assert_eq!(done["recovery_codes"].as_array().unwrap().len(), 10);
    assert!(done["totp"]["uri"]
        .as_str()
        .unwrap()
        .starts_with("otpauth://totp/Croft%20Companion:ann?secret="));
    let qr = done["totp"]["qr"].as_array().unwrap();
    assert!(qr.len() >= 21 && qr.iter().all(|r| r.as_str().unwrap().len() == qr.len()));

    // The link is spent.
    let (s, _) = h
        .call(
            Method::POST,
            "/api/enrol/start",
            None,
            Some(json!({ "token": token })),
        )
        .await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);

    let mut ann = Person {
        id,
        name: "ann".into(),
        key,
        totp_secret: Vec::new(),
        recovery_codes: Vec::new(),
    };
    let t = h.sign_in(&mut ann).await;
    let (s, me) = h.call(Method::GET, "/api/me", Some(&t.access), None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(me["name"], "ann");
    assert_eq!(me["role"], "user");
}

#[tokio::test]
async fn an_enrolment_link_expires_after_a_day_and_a_new_one_replaces_the_old() {
    let h = h();
    let id = h.add_user("ann", Role::User);
    let old = h.enrolment(id);
    let new = h.enrolment(id);
    let start = |t: String| {
        let h = &h;
        async move {
            h.call(
                Method::POST,
                "/api/enrol/start",
                None,
                Some(json!({ "token": t })),
            )
            .await
            .0
        }
    };
    assert_eq!(start(old).await, StatusCode::UNAUTHORIZED, "replaced");
    assert_eq!(start(new.clone()).await, StatusCode::OK);
    h.advance(ENROL_TTL_MS);
    assert_eq!(start(new).await, StatusCode::UNAUTHORIZED, "expired");
}

#[tokio::test]
async fn the_enrolment_finish_needs_the_same_link_as_the_start() {
    let h = h();
    let ann = h.add_user("ann", Role::User);
    let bob = h.add_user("bob", Role::User);
    let ann_link = h.enrolment(ann);
    let bob_link = h.enrolment(bob);
    let (_, start) = h
        .call(
            Method::POST,
            "/api/enrol/start",
            None,
            Some(json!({ "token": ann_link })),
        )
        .await;
    let ccr: CreationChallengeResponse = serde_json::from_value(start["options"].clone()).unwrap();
    let mut key = SoftPasskey::new(true);
    let cred = key
        .do_registration(Url::parse(ORIGIN).unwrap(), ccr)
        .unwrap();
    let (s, _) = h
        .call(
            Method::POST,
            "/api/enrol/finish",
            None,
            Some(json!({ "token": bob_link, "ceremony": start["ceremony"], "credential": cred })),
        )
        .await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
    assert!(h.store().passkeys(ann).unwrap().is_empty());
    assert!(h.store().passkeys(bob).unwrap().is_empty());
}

#[tokio::test]
async fn a_passkey_made_for_another_origin_is_refused() {
    let h = h();
    let id = h.add_user("ann", Role::User);
    let token = h.enrolment(id);
    let mut key = SoftPasskey::new(true);
    // A subdomain passes the authenticator's own check (the RP id is a suffix) but is not the
    // Companion's origin.
    let (s, _) = h
        .enrol_with(&token, &mut key, "https://evil.companion.test")
        .await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
    assert!(h.store().passkeys(id).unwrap().is_empty());
}

#[tokio::test]
async fn a_replayed_assertion_and_a_counter_going_backwards_are_refused() {
    let h = h();
    let mut ann = h.person("ann", Role::User).await;
    // Replay: the same answer to the same ceremony a second time.
    let (c, cred) = h.assertion(&mut ann).await;
    assert_eq!(h.finish(&c, &cred).await.0, StatusCode::OK);
    assert_eq!(h.finish(&c, &cred).await.0, StatusCode::UNAUTHORIZED);
    // The same answer to a new ceremony (another challenge).
    let (c2, _) = h.assertion(&mut ann).await;
    assert_eq!(h.finish(&c2, &cred).await.0, StatusCode::UNAUTHORIZED);
    // A counter going backwards: an older assertion, held back, arrives after a newer one.
    let (c_old, old) = h.assertion(&mut ann).await;
    let (c_new, new) = h.assertion(&mut ann).await;
    assert_eq!(h.finish(&c_new, &new).await.0, StatusCode::OK);
    assert_eq!(h.finish(&c_old, &old).await.0, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn a_discoverable_passkey_signs_in_without_a_name() {
    let h = h();
    let mut ann = h.person("ann", Role::User).await;
    let (s, start) = h
        .call(
            Method::POST,
            "/api/sign-in/passkey/start",
            None,
            Some(json!({})),
        )
        .await;
    assert_eq!(s, StatusCode::OK);
    assert!(start["options"]["publicKey"]["allowCredentials"]
        .as_array()
        .is_none_or(Vec::is_empty));
    // The software authenticator cannot hold discoverable credentials, so the test plays that
    // part: it is told which credential to use, and adds the user handle a discoverable
    // credential returns (the handle is not signed, which is why the server checks that the
    // credential belongs to that user).
    let (cred_id, handle) = {
        let mut st = h.store();
        let (_, pk) = st.passkeys(ann.id).unwrap().remove(0);
        (pk.cred_id().clone(), st.user_handle(ann.id).unwrap())
    };
    let mut options = start["options"].clone();
    options["publicKey"]["allowCredentials"] =
        json!([{ "type": "public-key", "id": b64url(cred_id.as_slice()) }]);
    let rcr: RequestChallengeResponse = serde_json::from_value(options).unwrap();
    let cred = ann
        .key
        .do_authentication(Url::parse(ORIGIN).unwrap(), rcr)
        .unwrap();
    let mut cred = serde_json::to_value(cred).unwrap();
    let b64 = |b: &[u8]| base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(b);
    // Claiming to be another user with Ann's credential is refused, and spends the ceremony.
    let bob = h.person("bob", Role::User).await;
    let bob_handle = h.store().user_handle(bob.id).unwrap();
    let mut forged = cred.clone();
    forged["response"]["userHandle"] = json!(b64(bob_handle.as_bytes()));
    assert_eq!(
        h.finish(&start["ceremony"], &forged).await.0,
        StatusCode::UNAUTHORIZED
    );

    let (s, start) = h
        .call(
            Method::POST,
            "/api/sign-in/passkey/start",
            None,
            Some(json!({})),
        )
        .await;
    assert_eq!(s, StatusCode::OK);
    let mut options = start["options"].clone();
    options["publicKey"]["allowCredentials"] =
        json!([{ "type": "public-key", "id": b64url(cred_id.as_slice()) }]);
    let rcr: RequestChallengeResponse = serde_json::from_value(options).unwrap();
    cred = serde_json::to_value(
        ann.key
            .do_authentication(Url::parse(ORIGIN).unwrap(), rcr)
            .unwrap(),
    )
    .unwrap();
    cred["response"]["userHandle"] = json!(b64(handle.as_bytes()));
    let (s, v) = h.finish(&start["ceremony"], &cred).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let (_, me) = h
        .call(
            Method::GET,
            "/api/me",
            Some(v["access_token"].as_str().unwrap()),
            None,
        )
        .await;
    assert_eq!(me["name"], "ann");
}

#[tokio::test]
async fn a_known_name_lists_its_credentials_and_an_unknown_one_gets_a_discoverable_challenge() {
    // Telling the two apart is accepted (DECISIONS); this pins what each gets.
    let h = h();
    h.person("ann", Role::User).await;
    let start = |user: &'static str| {
        let h = &h;
        async move {
            h.call(
                Method::POST,
                "/api/sign-in/passkey/start",
                None,
                Some(json!({ "user": user })),
            )
            .await
        }
    };
    let (s, known) = start("ann").await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(
        known["options"]["publicKey"]["allowCredentials"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert!(known["options"].get("mediation").is_none());
    let (s, unknown) = start("nobody").await;
    assert_eq!(s, StatusCode::OK);
    assert!(unknown["options"]["publicKey"]["challenge"].is_string());
    assert!(unknown["options"]["publicKey"]["allowCredentials"]
        .as_array()
        .is_none_or(Vec::is_empty));
}

#[tokio::test]
async fn a_signed_in_user_adds_and_removes_passkeys_but_never_the_last() {
    let h = h();
    let mut ann = h.person("ann", Role::User).await;
    let t = h.sign_in(&mut ann).await;
    let (s, start) = h
        .call(Method::POST, "/api/passkeys/start", Some(&t.access), None)
        .await;
    assert_eq!(s, StatusCode::OK);
    let mut second = SoftPasskey::new(true);
    let ccr: CreationChallengeResponse = serde_json::from_value(start["options"].clone()).unwrap();
    let cred = second
        .do_registration(Url::parse(ORIGIN).unwrap(), ccr)
        .unwrap();
    let (s, _) = h
        .call(
            Method::POST,
            "/api/passkeys/finish",
            Some(&t.access),
            Some(json!({ "ceremony": start["ceremony"], "credential": cred })),
        )
        .await;
    assert_eq!(s, StatusCode::OK);
    let (_, list) = h
        .call(Method::GET, "/api/passkeys", Some(&t.access), None)
        .await;
    let ids: Vec<i64> = list["passkeys"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["id"].as_i64().unwrap())
        .collect();
    assert_eq!(ids.len(), 2);
    let path = |id: i64| format!("/api/passkeys/{id}");
    assert_eq!(
        h.status(Method::DELETE, &path(ids[0]), Some(&t.access))
            .await,
        StatusCode::OK
    );
    assert_eq!(
        h.status(Method::DELETE, &path(ids[1]), Some(&t.access))
            .await,
        StatusCode::CONFLICT
    );
}

// ---- TOTP and recovery ----

#[tokio::test]
async fn totp_with_a_recovery_code_gives_a_session_that_can_only_add_a_passkey() {
    let h = h();
    let ann = h.person("ann", Role::User).await;
    let code = ann.recovery_codes[0].clone();
    // TOTP alone, or with a wrong recovery code, never signs in.
    let (s, _) = h
        .recover(&ann, &h.totp_now(&ann), "AAAA-BBBB-CCCC-DDDD")
        .await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
    let (s, _) = h.recover(&ann, "000000", &code).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
    // Both right: a recovery session.
    let (s, v) = h
        .recover(&ann, &h.totp_now(&ann), &code.to_lowercase())
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["recovery"], true);
    let t = signed(&v);
    assert_eq!(
        h.status(Method::GET, "/api/me", Some(&t.access)).await,
        StatusCode::OK
    );
    assert_eq!(
        h.status(Method::GET, "/api/sessions", Some(&t.access))
            .await,
        StatusCode::FORBIDDEN
    );
    // The same code and the same TOTP step are not accepted again.
    h.advance(1000);
    let (s, _) = h
        .recover(&ann, &h.totp_now(&ann), &ann.recovery_codes[1])
        .await;
    assert_eq!(s, StatusCode::UNAUTHORIZED, "same TOTP step");
    h.advance(30_000);
    let (s, _) = h.recover(&ann, &h.totp_now(&ann), &code).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED, "spent recovery code");
    // Registering a passkey makes it a full session.
    let (_, start) = h
        .call(Method::POST, "/api/passkeys/start", Some(&t.access), None)
        .await;
    let mut new_key = SoftPasskey::new(true);
    let ccr: CreationChallengeResponse = serde_json::from_value(start["options"].clone()).unwrap();
    let cred = new_key
        .do_registration(Url::parse(ORIGIN).unwrap(), ccr)
        .unwrap();
    let (s, _) = h
        .call(
            Method::POST,
            "/api/passkeys/finish",
            Some(&t.access),
            Some(json!({ "ceremony": start["ceremony"], "credential": cred })),
        )
        .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(
        h.status(Method::GET, "/api/sessions", Some(&t.access))
            .await,
        StatusCode::OK
    );
}

#[tokio::test]
async fn re_enrolment_replaces_every_factor_and_signs_out_every_device() {
    let h = h();
    let mut ann = h.person("ann", Role::User).await;
    let t = h.sign_in(&mut ann).await;
    let token = h.enrolment(ann.id);
    let mut new_key = SoftPasskey::new(true);
    let (s, done) = h.enrol_with(&token, &mut new_key, ORIGIN).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(
        h.status(Method::GET, "/api/me", Some(&t.access)).await,
        StatusCode::UNAUTHORIZED
    );
    // The old passkey and the old recovery codes no longer work.
    let (s, start) = h
        .call(
            Method::POST,
            "/api/sign-in/passkey/start",
            None,
            Some(json!({ "user": "ann" })),
        )
        .await;
    assert_eq!(s, StatusCode::OK);
    let rcr: RequestChallengeResponse = serde_json::from_value(start["options"].clone()).unwrap();
    assert!(
        ann.key
            .do_authentication(Url::parse(ORIGIN).unwrap(), rcr)
            .is_err(),
        "not offered"
    );
    ann.totp_secret = base32_decode(done["totp"]["secret"].as_str().unwrap());
    let (s, _) = h
        .recover(&ann, &h.totp_now(&ann), &ann.recovery_codes[0])
        .await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
}

// ---- Tokens ----

#[tokio::test]
async fn access_tokens_expire_and_refresh_tokens_rotate() {
    let h = h();
    let mut ann = h.person("ann", Role::User).await;
    let t = h.sign_in(&mut ann).await;
    h.advance(ACCESS_TTL_MS);
    assert_eq!(
        h.status(Method::GET, "/api/me", Some(&t.access)).await,
        StatusCode::UNAUTHORIZED
    );
    let (s, v) = h
        .call(
            Method::POST,
            "/api/token/refresh",
            None,
            Some(json!({ "refresh_token": t.refresh })),
        )
        .await;
    assert_eq!(s, StatusCode::OK);
    let t2 = signed(&v);
    assert_ne!(t2.refresh, t.refresh);
    assert_eq!(
        h.status(Method::GET, "/api/me", Some(&t2.access)).await,
        StatusCode::OK
    );
    // The new refresh token works once too.
    let (s, v) = h
        .call(
            Method::POST,
            "/api/token/refresh",
            None,
            Some(json!({ "refresh_token": t2.refresh })),
        )
        .await;
    assert_eq!(s, StatusCode::OK);
    let t3 = signed(&v);
    assert_eq!(
        h.status(Method::GET, "/api/me", Some(&t3.access)).await,
        StatusCode::OK
    );
}

#[tokio::test]
async fn a_reused_refresh_token_revokes_the_whole_sign_in_and_is_audited() {
    let h = h();
    let mut ann = h.person("ann", Role::User).await;
    let other_device = h.sign_in(&mut ann).await;
    let t = h.sign_in(&mut ann).await;
    let (_, v) = h
        .call(
            Method::POST,
            "/api/token/refresh",
            None,
            Some(json!({ "refresh_token": t.refresh })),
        )
        .await;
    let t2 = signed(&v);
    // Someone replays the spent token: the family is revoked.
    let (s, _) = h
        .call(
            Method::POST,
            "/api/token/refresh",
            None,
            Some(json!({ "refresh_token": t.refresh })),
        )
        .await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
    assert_eq!(
        h.status(Method::GET, "/api/me", Some(&t2.access)).await,
        StatusCode::UNAUTHORIZED
    );
    let (s, _) = h
        .call(
            Method::POST,
            "/api/token/refresh",
            None,
            Some(json!({ "refresh_token": t2.refresh })),
        )
        .await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
    // Only that sign-in: the other device is untouched.
    let (s, audit) = h
        .call(Method::GET, "/api/audit", Some(&other_device.access), None)
        .await;
    assert_eq!(s, StatusCode::OK);
    assert!(audit["entries"]
        .as_array()
        .unwrap()
        .iter()
        .any(|e| e["action"] == "token.reuse" && e["outcome"] == "revoked"));
}

#[tokio::test]
async fn a_device_is_signed_out_remotely_and_a_disabled_user_is_refused_at_once() {
    let h = h();
    let mut admin = h.person("root", Role::Admin).await;
    let mut ann = h.person("ann", Role::User).await;
    let phone = h.sign_in(&mut ann).await;
    let laptop = h.sign_in(&mut ann).await;
    let (_, list) = h
        .call(Method::GET, "/api/sessions", Some(&laptop.access), None)
        .await;
    let sessions = list["sessions"].as_array().unwrap();
    assert_eq!(sessions.len(), 2);
    let phone_id = sessions.iter().find(|s| s["current"] == false).unwrap()["id"]
        .as_i64()
        .unwrap();
    assert_eq!(
        h.status(
            Method::DELETE,
            &format!("/api/sessions/{phone_id}"),
            Some(&laptop.access)
        )
        .await,
        StatusCode::OK
    );
    assert_eq!(
        h.status(Method::GET, "/api/me", Some(&phone.access)).await,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        h.status(Method::GET, "/api/me", Some(&laptop.access)).await,
        StatusCode::OK
    );

    let a = h.sign_in(&mut admin).await;
    let (held_c, held) = h.assertion(&mut ann).await;
    let (s, _) = h
        .call(
            Method::POST,
            &format!("/api/admin/users/{}/disable", ann.id),
            Some(&a.access),
            None,
        )
        .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(
        h.status(Method::GET, "/api/me", Some(&laptop.access)).await,
        StatusCode::UNAUTHORIZED
    );
    let (s, _) = h
        .call(
            Method::POST,
            "/api/token/refresh",
            None,
            Some(json!({ "refresh_token": laptop.refresh })),
        )
        .await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
    assert_eq!(
        h.finish(&held_c, &held).await.0,
        StatusCode::UNAUTHORIZED,
        "disabled between start and finish"
    );
    // The admin cannot disable themselves.
    let (s, _) = h
        .call(
            Method::POST,
            &format!("/api/admin/users/{}/disable", admin.id),
            Some(&a.access),
            None,
        )
        .await;
    assert_eq!(s, StatusCode::CONFLICT);
    // Re-enabled, ann signs in again; the admin's actions are in her audit log.
    let (s, _) = h
        .call(
            Method::POST,
            &format!("/api/admin/users/{}/enable", ann.id),
            Some(&a.access),
            None,
        )
        .await;
    assert_eq!(s, StatusCode::OK);
    let t = h.sign_in(&mut ann).await;
    let (_, audit) = h
        .call(Method::GET, "/api/audit", Some(&t.access), None)
        .await;
    let by_admin: Vec<&str> = audit["entries"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["by_admin"] == admin.id)
        .map(|e| e["action"].as_str().unwrap())
        .collect();
    assert_eq!(by_admin, ["user.enable", "user.disable"]);
}

// ---- Limits ----

#[tokio::test]
async fn ten_failures_lock_the_user_for_fifteen_minutes_and_it_is_audited() {
    let h = h();
    let mut ann = h.person("ann", Role::User).await;
    for i in 0..9 {
        let (s, _) = h.recover(&ann, "000000", &format!("WRONG-{i}")).await;
        assert_eq!(s, StatusCode::UNAUTHORIZED);
    }
    let (s, v) = h.recover(&ann, "000000", "WRONG").await;
    assert_eq!(s, StatusCode::LOCKED, "the tenth failure locks");
    assert_eq!(v["until_ms"], h.now() + LOCK_MS);
    // Locked: even the right recovery answers are refused, but her passkey still works (the lock
    // guards only the guessable path; review finding: anyone could lock her out of her passkey).
    let (s, _) = h
        .recover(&ann, &h.totp_now(&ann), &ann.recovery_codes[0])
        .await;
    assert_eq!(s, StatusCode::LOCKED);
    let (c, cred) = h.assertion(&mut ann).await;
    assert_eq!(h.finish(&c, &cred).await.0, StatusCode::OK);
    h.advance(LOCK_MS);
    let t = h.sign_in(&mut ann).await;
    let (_, audit) = h
        .call(Method::GET, "/api/audit", Some(&t.access), None)
        .await;
    assert!(audit["entries"]
        .as_array()
        .unwrap()
        .iter()
        .any(|e| e["action"] == "sign_in.lock" && e["outcome"] == "locked"));
}

#[tokio::test]
async fn a_burst_from_one_address_is_rate_limited_with_retry_after() {
    let h = h();
    let mut limited = 0;
    for _ in 0..40 {
        let (s, v) = h
            .call_from(
                "198.51.100.9",
                Method::POST,
                "/api/sign-in/recovery",
                None,
                Some(json!({ "user": "nobody", "totp": "000000", "recovery_code": "x" })),
            )
            .await;
        if s == StatusCode::TOO_MANY_REQUESTS {
            assert_eq!(v["error"], "rate_limited");
            limited += 1;
        }
    }
    assert!(limited >= 20, "{limited}");
    // Another address is not affected.
    let (s, _) = h
        .call_from(
            "198.51.100.10",
            Method::POST,
            "/api/sign-in/recovery",
            None,
            Some(json!({ "user": "nobody", "totp": "000000", "recovery_code": "x" })),
        )
        .await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
}

#[test]
fn the_client_address_comes_from_a_trusted_proxy_only() {
    let auth = Auth::new(
        ORIGIN,
        Key::generate().unwrap(),
        vec!["172.18.0.2".parse().unwrap()],
        Auth::system_clock(),
    )
    .unwrap();
    let mut headers = HeaderMap::new();
    headers.insert("x-forwarded-for", "1.2.3.4, 203.0.113.5".parse().unwrap());
    let proxy = Some(Peer("172.18.0.2:5000".parse().unwrap()));
    let direct = Some(Peer("198.51.100.1:5000".parse().unwrap()));
    assert_eq!(auth.client(proxy, &headers).to_string(), "203.0.113.5");
    assert_eq!(auth.client(direct, &headers).to_string(), "198.51.100.1");
    assert_eq!(
        auth.client(proxy, &HeaderMap::new()).to_string(),
        "172.18.0.2"
    );
}

// ---- Every route: access and isolation ----

/// Each route that is not public refuses a caller without a token; routes for full sessions
/// refuse a recovery session; admin routes refuse a user. Then user A calls every route that
/// takes an id with user B's ids and is refused, and B's data is untouched.
#[tokio::test]
async fn every_route_checks_who_calls_and_no_user_reaches_another_users_data() {
    let h = h();
    let mut admin = h.person("root", Role::Admin).await;
    let mut a = h.person("ann", Role::User).await;
    let mut b = h.person("bob", Role::User).await;
    let a_t = h.sign_in(&mut a).await;
    let b_t = h.sign_in(&mut b).await;
    let admin_t = h.sign_in(&mut admin).await;
    let (_, rv) = h.recover(&a, &h.totp_now(&a), &a.recovery_codes[0]).await;
    let a_recovery = signed(&rv);

    let b_passkey = h.store().passkey_list(b.id).unwrap()[0].id;
    let b_session = h.store().sessions(b.id).unwrap()[0].id;

    let mut id_routes = BTreeSet::new();
    for r in routes() {
        let path = r.path.replace("{id}", &b.id.to_string());
        let m = r.method.clone();
        match r.access {
            Access::Public => continue,
            Access::Signed => {}
            Access::Full => assert_eq!(
                h.status(m.clone(), &path, Some(&a_recovery.access)).await,
                StatusCode::FORBIDDEN,
                "{} {} with a recovery session",
                r.method,
                r.path
            ),
            Access::Admin => {
                assert_eq!(
                    h.status(m.clone(), &path, Some(&a_t.access)).await,
                    StatusCode::FORBIDDEN,
                    "{} {} as a user",
                    r.method,
                    r.path
                );
                assert_eq!(
                    h.status(m.clone(), &path, Some(&a_recovery.access)).await,
                    StatusCode::FORBIDDEN
                );
            }
        }
        assert_eq!(
            h.status(m.clone(), &path, None).await,
            StatusCode::UNAUTHORIZED,
            "{} {} without a token",
            r.method,
            r.path
        );
        assert_eq!(
            h.status(m, &path, Some("not-a-token")).await,
            StatusCode::UNAUTHORIZED
        );
        if r.path.contains("{id}") {
            id_routes.insert((r.method.to_string(), r.path));
        }
    }

    // Routes that take an id, called by A with B's ids.
    let mut covered = BTreeSet::new();
    let passkey = format!("/api/passkeys/{b_passkey}");
    assert_eq!(
        h.status(Method::DELETE, &passkey, Some(&a_t.access)).await,
        StatusCode::NOT_FOUND
    );
    covered.insert(("DELETE".to_owned(), "/api/passkeys/{id}"));
    let session = format!("/api/sessions/{b_session}");
    assert_eq!(
        h.status(Method::DELETE, &session, Some(&a_t.access)).await,
        StatusCode::NOT_FOUND
    );
    covered.insert(("DELETE".to_owned(), "/api/sessions/{id}"));
    let p = format!("/api/admin/users/{}/oidc/unlink", b.id);
    assert_eq!(
        h.status(Method::POST, &p, Some(&a_t.access)).await,
        StatusCode::FORBIDDEN
    );
    covered.insert(("POST".to_owned(), "/api/admin/users/{id}/oidc/unlink"));
    // Mail accounts live in each user's own core, so another user's account id names nothing
    // here (A has none); the mail tests check it with accounts on both sides.
    assert_eq!(
        h.status(Method::DELETE, "/api/accounts/1", Some(&a_t.access))
            .await,
        StatusCode::NOT_FOUND
    );
    covered.insert(("DELETE".to_owned(), "/api/accounts/{id}"));
    for action in ["disable", "enable"] {
        let p = format!("/api/admin/users/{}/{action}", b.id);
        assert_eq!(
            h.status(Method::POST, &p, Some(&a_t.access)).await,
            StatusCode::FORBIDDEN
        );
        covered.insert((
            "POST".to_owned(),
            match action {
                "disable" => "/api/admin/users/{id}/disable",
                _ => "/api/admin/users/{id}/enable",
            },
        ));
    }
    assert_eq!(
        covered, id_routes,
        "a route with an id is missing from this test"
    );

    // B's ceremony cannot be finished by A.
    let (_, start) = h
        .call(Method::POST, "/api/passkeys/start", Some(&b_t.access), None)
        .await;
    let ccr: CreationChallengeResponse = serde_json::from_value(start["options"].clone()).unwrap();
    let mut key = SoftPasskey::new(true);
    let cred = key
        .do_registration(Url::parse(ORIGIN).unwrap(), ccr)
        .unwrap();
    let (s, _) = h
        .call(
            Method::POST,
            "/api/passkeys/finish",
            Some(&a_t.access),
            Some(json!({ "ceremony": start["ceremony"], "credential": cred })),
        )
        .await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
    assert_eq!(h.store().passkey_list(a.id).unwrap().len(), 1);
    assert_eq!(h.store().passkey_list(b.id).unwrap().len(), 1);

    // Lists show only the caller's own rows.
    let (_, s) = h
        .call(Method::GET, "/api/sessions", Some(&a_t.access), None)
        .await;
    assert!(s["sessions"]
        .as_array()
        .unwrap()
        .iter()
        .all(|x| x["id"] != b_session));
    let (_, p) = h
        .call(Method::GET, "/api/passkeys", Some(&a_t.access), None)
        .await;
    assert!(p["passkeys"]
        .as_array()
        .unwrap()
        .iter()
        .all(|x| x["id"] != b_passkey));
    let (_, au) = h
        .call(Method::GET, "/api/audit", Some(&a_t.access), None)
        .await;
    assert!(au["entries"]
        .as_array()
        .unwrap()
        .iter()
        .all(|e| e["user_id"] == a.id));

    // B is untouched; the admin sees everyone but nothing of B's beyond the list and the log.
    assert_eq!(
        h.status(Method::GET, "/api/me", Some(&b_t.access)).await,
        StatusCode::OK
    );
    let (s, users) = h
        .call(Method::GET, "/api/admin/users", Some(&admin_t.access), None)
        .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(users["users"].as_array().unwrap().len(), 3);
    let (s, _) = h
        .call(Method::GET, "/api/admin/audit", Some(&admin_t.access), None)
        .await;
    assert_eq!(s, StatusCode::OK);
    // An admin's own token is still only theirs: B's session and passkey are not the admin's.
    assert_eq!(
        h.status(Method::DELETE, &session, Some(&admin_t.access))
            .await,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        h.status(Method::DELETE, &passkey, Some(&admin_t.access))
            .await,
        StatusCode::NOT_FOUND
    );
}

// ---- Nothing secret is kept or said ----

#[tokio::test]
async fn no_token_code_or_secret_is_stored_in_the_clear_or_audited() {
    let h = h();
    let mut ann = h.person("ann", Role::Admin).await;
    let t = h.sign_in(&mut ann).await;
    let (_, v) = h
        .call(
            Method::POST,
            "/api/token/refresh",
            None,
            Some(json!({ "refresh_token": t.refresh })),
        )
        .await;
    let t2 = signed(&v);
    let link_token = h.enrolment(ann.id);
    let mut secrets: Vec<Vec<u8>> = vec![
        t.access.into_bytes(),
        t.refresh.into_bytes(),
        t2.access.into_bytes(),
        t2.refresh.into_bytes(),
        link_token.into_bytes(),
        ann.totp_secret.clone(),
        crate::totp::base32(&ann.totp_secret).as_bytes().to_vec(),
    ];
    for c in &ann.recovery_codes {
        secrets.push(c.clone().into_bytes());
        secrets.push(c.replace('-', "").into_bytes());
    }
    let store = h.store();
    let tables = store.tables_and_user_scope().unwrap();
    for (table, _) in tables {
        let mut stmt = store
            .conn
            .prepare(&format!("SELECT * FROM {table}"))
            .unwrap();
        let n = stmt.column_count();
        let mut rows = stmt.query([]).unwrap();
        while let Some(row) = rows.next().unwrap() {
            for i in 0..n {
                let bytes: Vec<u8> = match row.get_ref(i).unwrap() {
                    rusqlite::types::ValueRef::Text(t) | rusqlite::types::ValueRef::Blob(t) => {
                        t.to_vec()
                    }
                    _ => continue,
                };
                for s in &secrets {
                    assert!(
                        !bytes.windows(s.len()).any(|w| w == &s[..]),
                        "a secret is in the clear in {table}"
                    );
                }
            }
        }
    }
    // Audit actions are fixed words.
    for e in store.audit_entries(None, 1000).unwrap() {
        assert!(
            e.action
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b == b'.' || b == b'_'),
            "{}",
            e.action
        );
    }
}

#[tokio::test]
async fn errors_say_nothing_about_which_part_was_wrong() {
    let h = h();
    let ann = h.person("ann", Role::User).await;
    let (s1, v1) = h.recover(&ann, &h.totp_now(&ann), "WRONG").await;
    let (s2, v2) = h
        .call(
            Method::POST,
            "/api/sign-in/recovery",
            None,
            Some(json!({ "user": "nobody", "totp": "123456", "recovery_code": "x" })),
        )
        .await;
    assert_eq!((s1, &v1), (s2, &v2));
    assert_eq!(v1, json!({ "error": "invalid" }));
    let (s, v) = h
        .call(
            Method::POST,
            "/api/sign-in/recovery",
            None,
            Some(json!({ "user": 5 })),
        )
        .await;
    assert_eq!(
        (s, v),
        (StatusCode::BAD_REQUEST, json!({ "error": "bad_request" }))
    );
}

#[tokio::test]
async fn the_enrolment_page_runs_only_its_own_script() {
    let h = h();
    for path in ["/enrol", "/enrol.js", "/enrol.css"] {
        let req = Request::get(path).body(Body::empty()).unwrap();
        let res = h.router.clone().oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let csp = res.headers()["content-security-policy"].to_str().unwrap();
        assert_eq!(csp, crate::enrol_page::PAGE_CSP);
        assert!(csp.contains("script-src 'self'") && !csp.contains("unsafe"));
    }
    let html = include_str!("../web/enrol.html");
    for tag in html.split("<script").skip(1) {
        assert!(tag.trim_start().starts_with("src="), "no inline script");
    }
    let js = include_str!("../web/enrol.js");
    assert!(!js.contains("innerHTML") && !js.contains("eval("));
    // The API keeps the strict default.
    let req = Request::get("/api/me").body(Body::empty()).unwrap();
    let res = h.router.clone().oneshot(req).await.unwrap();
    assert!(res.headers()["content-security-policy"]
        .to_str()
        .unwrap()
        .starts_with("default-src 'none'"));
}

// ---- Regression tests for the independent review's findings ----

/// H1: an admin could get a user's enrolment link from the API, register their own passkey and
/// sign in as the user. Re-enrolment is now the server operator's (`user enrol`); the API has no
/// route that hands out a link.
#[tokio::test]
async fn no_api_route_lets_an_admin_get_a_users_enrolment_link() {
    let h = h();
    let mut root = h.person("root", Role::Admin).await;
    let bob = h.person("bob", Role::User).await;
    let t = h.sign_in(&mut root).await;
    let s = h
        .status(
            Method::POST,
            &format!("/api/admin/users/{}/enrol", bob.id),
            Some(&t.access),
        )
        .await;
    assert!(
        s == StatusCode::NOT_FOUND || s == StatusCode::METHOD_NOT_ALLOWED,
        "{s}"
    );
    for r in routes() {
        assert!(
            !r.path.contains("enrol") || r.access == Access::Public,
            "{}",
            r.path
        );
    }
}

/// M2: naming a user and sending someone else's credential ten times locked the user out of
/// their own passkey.
#[tokio::test]
async fn failed_passkey_answers_never_lock_the_named_user() {
    let h = h();
    let mut ann = h.person("ann", Role::User).await;
    let mut mallory = h.person("mallory", Role::User).await;
    let (_, junk) = h.assertion(&mut mallory).await;
    for i in 0..12 {
        let ip = format!("203.0.113.{i}");
        let (s, start) = h
            .call_from(
                &ip,
                Method::POST,
                "/api/sign-in/passkey/start",
                None,
                Some(json!({"user": "ann"})),
            )
            .await;
        assert_eq!(s, StatusCode::OK);
        let (s, _) = h
            .call_from(
                &ip,
                Method::POST,
                "/api/sign-in/passkey/finish",
                None,
                Some(json!({"ceremony": start["ceremony"], "credential": junk})),
            )
            .await;
        assert_eq!(s, StatusCode::UNAUTHORIZED);
    }
    assert_eq!(h.store().locked_until(ann.id, h.now()).unwrap(), None);
    let (c, cred) = h.assertion(&mut ann).await;
    assert_eq!(h.finish(&c, &cred).await.0, StatusCode::OK);
    // Still audited for her.
    let failures = h
        .store()
        .audit_entries(Some(ann.id), 100)
        .unwrap()
        .iter()
        .filter(|e| e.action == "sign_in" && e.outcome == "failed")
        .count();
    assert_eq!(failures, 12);
}

/// M1: one signed-in user filled the ceremony table and blocked every sign-in for five minutes.
#[tokio::test]
async fn one_user_cannot_crowd_out_other_peoples_ceremonies() {
    let h = h();
    let mut ann = h.person("ann", Role::User).await;
    let mut bob = h.person("bob", Role::User).await;
    let t = h.sign_in(&mut ann).await;
    // Bob starts first, then Ann floods (from many addresses, past the per-address limit).
    let (c, cred) = h.assertion(&mut bob).await;
    for _ in 0..200 {
        let _ = h
            .call(Method::POST, "/api/passkeys/start", Some(&t.access), None)
            .await;
    }
    assert!(h.app.auth.ceremonies().len() <= 2 * crate::ceremony::PER_OWNER);
    assert_eq!(
        h.finish(&c, &cred).await.0,
        StatusCode::OK,
        "Bob's ceremony survived"
    );
    // The signed-in start is rate limited per address too.
    let mut limited = false;
    for _ in 0..40 {
        let (s, _) = h
            .call_from(
                "198.51.100.77",
                Method::POST,
                "/api/passkeys/start",
                Some(&t.access),
                None,
            )
            .await;
        limited |= s == StatusCode::TOO_MANY_REQUESTS;
    }
    assert!(limited);
}

/// L/M1: a concurrent burst of recovery guesses got far more than ten checks before the lock.
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn a_concurrent_burst_gets_at_most_ten_checks() {
    let h = Arc::new(h());
    let ann = h.person("ann", Role::User).await;
    let mut tasks = Vec::new();
    for i in 0..60 {
        let h = h.clone();
        tasks.push(tokio::spawn(async move {
            h.call(
                Method::POST,
                "/api/sign-in/recovery",
                None,
                Some(
                    json!({"user": "ann", "totp": "000000", "recovery_code": format!("WRONG-{i}")}),
                ),
            )
            .await
            .0
        }));
    }
    let mut statuses = Vec::new();
    for t in tasks {
        statuses.push(t.await.unwrap());
    }
    let checked = h
        .store()
        .audit_entries(Some(ann.id), 1000)
        .unwrap()
        .iter()
        .filter(|e| e.action == "sign_in" && e.outcome == "failed")
        .count();
    assert_eq!(checked, crate::auth::LOCK_AFTER as usize, "{statuses:?}");
    assert_eq!(
        statuses
            .iter()
            .filter(|s| **s == StatusCode::UNAUTHORIZED)
            .count(),
        9
    );
    assert_eq!(
        statuses
            .iter()
            .filter(|s| **s == StatusCode::LOCKED)
            .count(),
        51
    );
}

/// L4 and L5: a recovery session cannot be refreshed (it lives as long as its access token).
#[tokio::test]
async fn a_recovery_session_cannot_be_refreshed() {
    let h = h();
    let ann = h.person("ann", Role::User).await;
    let (_, v) = h
        .recover(&ann, &h.totp_now(&ann), &ann.recovery_codes[0])
        .await;
    assert_eq!(v["recovery"], true);
    let (s, _) = h
        .call(
            Method::POST,
            "/api/token/refresh",
            None,
            Some(json!({"refresh_token": v["refresh_token"]})),
        )
        .await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
    assert_eq!(
        h.status(
            Method::GET,
            "/api/me",
            Some(v["access_token"].as_str().unwrap())
        )
        .await,
        StatusCode::OK
    );
}

/// L1 and L2: a proxy seen as a mapped IPv6 address is recognised, and an `X-Forwarded-For`
/// entry that does not parse stops the walk (the client wrote what lies before it).
#[test]
fn mapped_proxy_addresses_and_unparsable_hops_are_handled() {
    let auth = Auth::new(
        ORIGIN,
        Key::generate().unwrap(),
        vec!["172.18.0.2".parse().unwrap()],
        Auth::system_clock(),
    )
    .unwrap();
    let mut headers = HeaderMap::new();
    headers.insert("x-forwarded-for", "203.0.113.5".parse().unwrap());
    let mapped = Some(Peer("[::ffff:172.18.0.2]:5000".parse().unwrap()));
    assert_eq!(auth.client(mapped, &headers).to_string(), "203.0.113.5");
    let proxy = Some(Peer("172.18.0.2:5000".parse().unwrap()));
    headers.insert(
        "x-forwarded-for",
        "6.6.6.6, 203.0.113.5:4711".parse().unwrap(),
    );
    assert_eq!(auth.client(proxy, &headers).to_string(), "172.18.0.2");
    let direct_mapped = Some(Peer("[::ffff:198.51.100.1]:5000".parse().unwrap()));
    assert_eq!(
        auth.client(direct_mapped, &headers).to_string(),
        "198.51.100.1"
    );
}

/// Spec gap: a wrong origin is refused at sign-in too, not only at registration.
#[tokio::test]
async fn a_sign_in_answer_made_for_another_origin_is_refused() {
    let h = h();
    let mut ann = h.person("ann", Role::User).await;
    let (_, start) = h
        .call(
            Method::POST,
            "/api/sign-in/passkey/start",
            None,
            Some(json!({"user": "ann"})),
        )
        .await;
    let rcr: RequestChallengeResponse = serde_json::from_value(start["options"].clone()).unwrap();
    let cred = ann
        .key
        .do_authentication(Url::parse("https://evil.companion.test").unwrap(), rcr)
        .unwrap();
    let (s, _) = h
        .finish(&start["ceremony"], &serde_json::to_value(cred).unwrap())
        .await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
}

/// Spec gap: identifiers in a request body. Ann's valid answer to a ceremony started for Bob's
/// name signs nobody in.
#[tokio::test]
async fn an_answer_for_a_ceremony_named_for_another_user_is_refused() {
    let h = h();
    let mut ann = h.person("ann", Role::User).await;
    h.person("bob", Role::User).await;
    let ann_cred_id = h.store().passkeys(ann.id).unwrap()[0].1.cred_id().clone();
    let (_, start) = h
        .call(
            Method::POST,
            "/api/sign-in/passkey/start",
            None,
            Some(json!({"user": "bob"})),
        )
        .await;
    let mut options = start["options"].clone();
    options["publicKey"]["allowCredentials"] =
        json!([{ "type": "public-key", "id": b64url(ann_cred_id.as_slice()) }]);
    let rcr: RequestChallengeResponse = serde_json::from_value(options).unwrap();
    let cred = ann
        .key
        .do_authentication(Url::parse(ORIGIN).unwrap(), rcr)
        .unwrap();
    let (s, _) = h
        .finish(&start["ceremony"], &serde_json::to_value(cred).unwrap())
        .await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
}

#[test]
fn device_names_lose_control_and_invisible_formatting_characters() {
    assert_eq!(
        device_name(Some("Ann\u{202E}enohp\u{200B}\n".into())),
        "Annenohp"
    );
    assert_eq!(device_name(Some("  ".into())), "Unnamed device");
    assert_eq!(device_name(Some("x".repeat(100))).len(), MAX_DEVICE);
}
