// This Source Code Form is subject to the terms of the GNU Affero General Public License,
// version 3 or later. See LICENSE-AGPL-3.0 at the repository root.

//! Tests of OIDC sign-in (T25) through the real router, against a fake provider on this machine
//! (made-up keys, discovery, a key set and a token endpoint that checks the client's credentials
//! and the PKCE verifier). The browser's part is played by the test: it reads the provider URL the
//! Companion sends it to, has the fake provider mint a code for it, and follows the redirects.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::HashMap;
use std::sync::atomic::{AtomicI64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use axum::body::Body;
use axum::extract::{Form, State as AxState};
use axum::http::{Method, Request, StatusCode};
use axum::routing::{get, post};
use axum::Router;
use base64::Engine;
use http_body_util::BodyExt;
use serde_json::{json, Value};
use tower::ServiceExt;

use super::*;
use crate::config::OidcSettings;
use crate::http::{router, App};
use crate::jwt::tests::TestSigner;
use crate::oidc::{s256, Provider, FLOW_TTL_MS, HANDOVER_TTL_MS};
use crate::store::Role;

const ORIGIN: &str = "https://companion.test";
const T0: i64 = 1_800_000_000_000;
const SECRET: &str = "made-up-client-secret";

/// How the fake provider misbehaves, for one test.
#[derive(Default, Clone)]
struct Knobs {
    /// Claims to change in the next ID token (null removes one).
    claims: Value,
    /// A header to use instead of `{"alg": "ES256", "kid": "k1"}`.
    header: Option<Value>,
    /// Sign with `alg: none` and an empty signature.
    unsigned: bool,
    /// The `issuer` the discovery document gives, instead of the real one.
    discovery_issuer: Option<String>,
    /// The token endpoint the discovery document gives, instead of the real one.
    token_endpoint: Option<String>,
    /// `code_challenge_methods_supported` in the discovery document.
    pkce_methods: Option<Value>,
    /// Sign with HS256, using the public key's JWK text as the HMAC secret (algorithm confusion).
    hmac_forgery: bool,
}

struct Fake {
    issuer: String,
    redirect_uri: String,
    signer: Mutex<TestSigner>,
    /// Every ID token issued and code minted (the secrets test looks for them).
    issued: Mutex<Vec<String>>,
    discovery_fetches: AtomicUsize,
    knobs: Mutex<Knobs>,
    /// code -> (nonce, PKCE challenge)
    codes: Mutex<HashMap<String, (String, String)>>,
    jwks_fetches: AtomicUsize,
    clock_s: Arc<AtomicI64>,
}

fn b64(b: &[u8]) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(b)
}

async fn discovery(AxState(f): AxState<Arc<Fake>>) -> axum::Json<Value> {
    f.discovery_fetches.fetch_add(1, Ordering::SeqCst);
    let k = f.knobs.lock().unwrap().clone();
    let mut doc = json!({
        "issuer": k.discovery_issuer.unwrap_or_else(|| f.issuer.clone()),
        "authorization_endpoint": format!("{}/authorize", f.issuer),
        "token_endpoint": k.token_endpoint.unwrap_or_else(|| format!("{}/token", f.issuer)),
        "jwks_uri": format!("{}/jwks", f.issuer),
        "token_endpoint_auth_methods_supported": ["client_secret_basic"],
    });
    if let Some(m) = k.pkce_methods {
        doc["code_challenge_methods_supported"] = m;
    }
    axum::Json(doc)
}

async fn jwks(AxState(f): AxState<Arc<Fake>>) -> axum::Json<Value> {
    f.jwks_fetches.fetch_add(1, Ordering::SeqCst);
    axum::Json(json!({ "keys": [f.signer.lock().unwrap().jwk("k1")] }))
}

async fn token(
    AxState(f): AxState<Arc<Fake>>,
    headers: axum::http::HeaderMap,
    Form(form): Form<HashMap<String, String>>,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    let basic = format!(
        "Basic {}",
        base64::engine::general_purpose::STANDARD.encode(format!("croft:{SECRET}"))
    );
    if headers.get("authorization").and_then(|v| v.to_str().ok()) != Some(basic.as_str()) {
        return (StatusCode::UNAUTHORIZED, "{\"error\":\"invalid_client\"}").into_response();
    }
    let code = form.get("code").cloned().unwrap_or_default();
    let Some((nonce, challenge)) = f.codes.lock().unwrap().remove(&code) else {
        return (StatusCode::BAD_REQUEST, "{\"error\":\"invalid_grant\"}").into_response();
    };
    let verifier = form.get("code_verifier").cloned().unwrap_or_default();
    if s256(&verifier) != challenge || form.get("redirect_uri") != Some(&f.redirect_uri) {
        return (StatusCode::BAD_REQUEST, "{\"error\":\"invalid_grant\"}").into_response();
    }
    let k = f.knobs.lock().unwrap().clone();
    let now = f.clock_s.load(Ordering::SeqCst);
    let mut claims = json!({
        "iss": f.issuer, "sub": "made-up-sub-ann", "aud": "croft",
        "exp": now + 300, "iat": now, "nonce": nonce,
        "preferred_username": "ann", "groups": ["mail"],
    });
    if let Some(extra) = k.claims.as_object() {
        for (key, v) in extra {
            if v.is_null() {
                claims.as_object_mut().unwrap().remove(key);
            } else {
                claims[key] = v.clone();
            }
        }
    }
    let signer = f.signer.lock().unwrap();
    let id_token = if k.hmac_forgery {
        let header = b64(json!({"alg": "HS256", "kid": "k1"}).to_string().as_bytes());
        let input = format!("{header}.{}", b64(claims.to_string().as_bytes()));
        let secret = signer.jwk("k1").to_string();
        let key = ring::hmac::Key::new(ring::hmac::HMAC_SHA256, secret.as_bytes());
        format!(
            "{input}.{}",
            b64(ring::hmac::sign(&key, input.as_bytes()).as_ref())
        )
    } else if k.unsigned {
        format!(
            "{}.{}.",
            b64(json!({"alg": "none"}).to_string().as_bytes()),
            b64(claims.to_string().as_bytes())
        )
    } else {
        let header = k
            .header
            .unwrap_or_else(|| json!({"alg": signer.alg(), "kid": "k1"}));
        signer.sign(&header, &claims)
    };
    drop(signer);
    f.issued.lock().unwrap().push(id_token.clone());
    axum::Json(json!({"access_token": "made-up", "token_type": "Bearer", "id_token": id_token}))
        .into_response()
}

struct H {
    app: App,
    router: Router,
    clock: Arc<AtomicI64>,
    master: Key,
    fake: Arc<Fake>,
}

async fn h_with(create_users: bool) -> H {
    h_full(create_users, ORIGIN).await
}

async fn h_full(create_users: bool, public_url: &str) -> H {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let issuer = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
    let clock = Arc::new(AtomicI64::new(T0));
    let clock_s = Arc::new(AtomicI64::new(T0 / 1000));
    let fake = Arc::new(Fake {
        issuer: issuer.clone(),
        redirect_uri: format!("{public_url}/oidc/callback"),
        signer: Mutex::new(TestSigner::es256()),
        issued: Mutex::new(Vec::new()),
        discovery_fetches: AtomicUsize::new(0),
        knobs: Mutex::new(Knobs::default()),
        codes: Mutex::new(HashMap::new()),
        jwks_fetches: AtomicUsize::new(0),
        clock_s: clock_s.clone(),
    });
    let provider_app = Router::new()
        .route("/.well-known/openid-configuration", get(discovery))
        .route("/jwks", get(jwks))
        .route("/token", post(token))
        .with_state(fake.clone());
    tokio::spawn(async move { axum::serve(listener, provider_app).await });

    let master = Key::generate().unwrap();
    let store = Store::in_memory().unwrap();
    store.check_master_key(&master).unwrap();
    let c = clock.clone();
    let settings = OidcSettings {
        issuer,
        client_id: "croft".into(),
        client_secret_file: "/nonexistent".into(),
        name: "Test ID".into(),
        create_users,
        required_group: create_users.then(|| "mail".to_owned()),
        groups_claim: "groups".into(),
    };
    let auth = Auth::new(
        ORIGIN,
        master.clone(),
        Vec::new(),
        Arc::new(move || c.load(Ordering::SeqCst)),
    )
    .unwrap()
    .with_oidc(Provider::for_tests(settings, SECRET, public_url));
    let app = App::new(store, auth);
    H {
        router: router(app.clone()),
        app,
        clock,
        master,
        fake,
    }
}

/// What the app keeps while the browser is away.
struct AppSide {
    verifier: String,
    challenge: String,
}

fn app_side() -> AppSide {
    let verifier = crate::auth::random_token().unwrap().to_string();
    AppSide {
        challenge: s256(&verifier),
        verifier,
    }
}

const APP_REDIRECT: &str = "https://companion.test/app/signed-in";

fn query_of(url: &str) -> HashMap<String, String> {
    let q = url.split_once('?').map(|(_, q)| q).unwrap_or("");
    q.split('&')
        .filter_map(|kv| kv.split_once('='))
        .map(|(k, v)| {
            let v = percent_decode(v);
            (k.to_owned(), v)
        })
        .collect()
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            out.push(u8::from_str_radix(&s[i + 1..i + 3], 16).unwrap());
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8(out).unwrap()
}

impl H {
    fn now(&self) -> i64 {
        self.clock.load(Ordering::SeqCst)
    }

    fn advance(&self, ms: i64) {
        self.clock.fetch_add(ms, Ordering::SeqCst);
        self.fake.clock_s.store(self.now() / 1000, Ordering::SeqCst);
    }

    fn knobs(&self, k: Knobs) {
        *self.fake.knobs.lock().unwrap() = k;
    }

    fn store(&self) -> std::sync::MutexGuard<'_, Store> {
        self.app.store.lock().unwrap()
    }

    async fn raw(
        &self,
        method: Method,
        path: &str,
        token: Option<&str>,
        body: Option<Value>,
    ) -> (StatusCode, Option<String>, Value) {
        static N: AtomicI64 = AtomicI64::new(1);
        let n = N.fetch_add(1, Ordering::SeqCst);
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
        let ip = format!("10.9.{}.{}:4000", (n >> 8) & 255, n & 255);
        req.extensions_mut().insert(Peer(ip.parse().unwrap()));
        let res = self.router.clone().oneshot(req).await.unwrap();
        let status = res.status();
        let location = res
            .headers()
            .get("location")
            .map(|v| v.to_str().unwrap().to_owned());
        let bytes = res.into_body().collect().await.unwrap().to_bytes();
        (
            status,
            location,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }

    fn user(&self, name: &str) -> i64 {
        self.store()
            .add_user(&self.master, name, Role::User)
            .unwrap()
            .id
    }

    fn link(&self, user_id: i64, subject: &str) {
        let issuer = self.fake.issuer.clone();
        let now = self.now();
        self.store()
            .link_identity(user_id, &issuer, subject, now)
            .unwrap();
    }

    /// A signed-in session without a passkey (the store's own sign-in), for the link tests.
    fn session(&self, user_id: i64) -> String {
        let now = self.now();
        self.store()
            .start_session(user_id, "test", false, now)
            .unwrap()
            .access
            .to_string()
    }

    /// The browser's trip: from the provider URL to the Companion's callback, with a code the
    /// fake provider minted for that flow. Returns the callback's answer.
    async fn browse(&self, provider_url: &str) -> (StatusCode, Option<String>) {
        let q = query_of(provider_url);
        assert_eq!(q["code_challenge_method"], "S256");
        assert_eq!(q["response_type"], "code");
        let code = crate::auth::random_token().unwrap().to_string();
        self.fake.issued.lock().unwrap().push(code.clone());
        self.fake.codes.lock().unwrap().insert(
            code.clone(),
            (q["nonce"].clone(), q["code_challenge"].clone()),
        );
        let path = format!("/oidc/callback?state={}&code={code}", q["state"]);
        let (s, loc, _) = self.raw(Method::GET, &path, None, None).await;
        (s, loc)
    }

    async fn start(&self, app: &AppSide) -> String {
        let path = format!(
            "/api/sign-in/oidc/start?app_challenge={}&app_redirect={}",
            app.challenge,
            crate::oidc::with_query("", &[("x", APP_REDIRECT)]).trim_start_matches("?x=")
        );
        let (s, loc, v) = self.raw(Method::GET, &path, None, None).await;
        assert_eq!(s, StatusCode::SEE_OTHER, "{v}");
        loc.unwrap()
    }

    /// A whole sign-in up to the app's redirect: the query the app receives.
    async fn sign_in_redirect(&self, app: &AppSide) -> HashMap<String, String> {
        let url = self.start(app).await;
        let (s, loc) = self.browse(&url).await;
        assert_eq!(s, StatusCode::SEE_OTHER);
        let loc = loc.unwrap();
        assert!(loc.starts_with(APP_REDIRECT), "{loc}");
        query_of(&loc)
    }

    async fn exchange(&self, code: &str, verifier: &str) -> (StatusCode, Value) {
        let (s, _, v) = self
            .raw(
                Method::POST,
                "/api/sign-in/oidc/exchange",
                None,
                Some(json!({"code": code, "verifier": verifier, "device": "Web"})),
            )
            .await;
        (s, v)
    }
}

#[tokio::test]
async fn a_linked_person_signs_in_through_the_provider() {
    let h = h_with(false).await;
    let ann = h.user("ann");
    h.link(ann, "made-up-sub-ann");
    let (s, _, info) = h.raw(Method::GET, "/api/oidc", None, None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(info, json!({"enabled": true, "name": "Test ID"}));
    let app = app_side();
    let back = h.sign_in_redirect(&app).await;
    let (s, v) = h.exchange(&back["code"], &app.verifier).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["recovery"], false);
    let token = v["access_token"].as_str().unwrap();
    let (s, _, me) = h.raw(Method::GET, "/api/me", Some(token), None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(me["name"], "ann");
    // The hand-over code works once.
    assert_eq!(
        h.exchange(&back["code"], &app.verifier).await.0,
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn the_hand_over_needs_the_apps_verifier_and_expires() {
    let h = h_with(false).await;
    let ann = h.user("ann");
    h.link(ann, "made-up-sub-ann");
    // Login CSRF: someone else's app started the flow; the code lands in this browser, whose app
    // does not hold that verifier.
    let theirs = app_side();
    let back = h.sign_in_redirect(&theirs).await;
    assert_eq!(
        h.exchange(&back["code"], &app_side().verifier).await.0,
        StatusCode::UNAUTHORIZED
    );
    // Spent by the wrong try.
    assert_eq!(
        h.exchange(&back["code"], &theirs.verifier).await.0,
        StatusCode::UNAUTHORIZED
    );
    // Expired.
    let app = app_side();
    let back = h.sign_in_redirect(&app).await;
    h.advance(HANDOVER_TTL_MS);
    assert_eq!(
        h.exchange(&back["code"], &app.verifier).await.0,
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn a_replayed_unknown_or_expired_state_is_refused() {
    let h = h_with(false).await;
    let ann = h.user("ann");
    h.link(ann, "made-up-sub-ann");
    let app = app_side();
    let url = h.start(&app).await;
    let (s, _) = h.browse(&url).await;
    assert_eq!(s, StatusCode::SEE_OTHER);
    assert_eq!(
        h.browse(&url).await.0,
        StatusCode::UNAUTHORIZED,
        "replayed state"
    );
    let (s, _, _) = h
        .raw(
            Method::GET,
            "/oidc/callback?state=made-up&code=x",
            None,
            None,
        )
        .await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
    let url = h.start(&app).await;
    h.advance(FLOW_TTL_MS);
    assert_eq!(
        h.browse(&url).await.0,
        StatusCode::UNAUTHORIZED,
        "expired state"
    );
}

#[tokio::test]
async fn every_bad_id_token_is_refused() {
    let h = h_with(false).await;
    let ann = h.user("ann");
    h.link(ann, "made-up-sub-ann");
    let cases: Vec<(&str, Knobs)> = vec![
        (
            "iss",
            Knobs {
                claims: json!({"iss": "https://elsewhere.example"}),
                ..Knobs::default()
            },
        ),
        (
            "aud",
            Knobs {
                claims: json!({"aud": "another-client"}),
                ..Knobs::default()
            },
        ),
        (
            "azp missing",
            Knobs {
                claims: json!({"aud": ["croft", "other"]}),
                ..Knobs::default()
            },
        ),
        (
            "azp wrong",
            Knobs {
                claims: json!({"azp": "other"}),
                ..Knobs::default()
            },
        ),
        (
            "expired",
            Knobs {
                claims: json!({"exp": T0 / 1000 - 120}),
                ..Knobs::default()
            },
        ),
        (
            "not yet valid",
            Knobs {
                claims: json!({"nbf": T0 / 1000 + 3600}),
                ..Knobs::default()
            },
        ),
        (
            "issued in the future",
            Knobs {
                claims: json!({"iat": T0 / 1000 + 3600}),
                ..Knobs::default()
            },
        ),
        (
            "wrong nonce",
            Knobs {
                claims: json!({"nonce": "made-up-other"}),
                ..Knobs::default()
            },
        ),
        (
            "no nonce",
            Knobs {
                claims: json!({"nonce": null}),
                ..Knobs::default()
            },
        ),
        (
            "alg none",
            Knobs {
                unsigned: true,
                ..Knobs::default()
            },
        ),
        (
            "unknown kid",
            Knobs {
                header: Some(json!({"alg": "ES256", "kid": "k2"})),
                ..Knobs::default()
            },
        ),
        (
            "HS256 keyed with the public key",
            Knobs {
                hmac_forgery: true,
                ..Knobs::default()
            },
        ),
    ];
    for (name, knobs) in cases {
        h.knobs(knobs);
        let back = h.sign_in_redirect(&app_side()).await;
        assert_eq!(
            back.get("error").map(String::as_str),
            Some("invalid"),
            "{name}"
        );
        assert!(!back.contains_key("code"), "{name}");
    }
    h.knobs(Knobs::default());
    let back = h.sign_in_redirect(&app_side()).await;
    assert!(back.contains_key("code"), "a good token still works");
}

#[tokio::test]
async fn an_unknown_key_id_refetches_the_key_set_at_most_once_a_minute() {
    let h = h_with(false).await;
    let ann = h.user("ann");
    h.link(ann, "made-up-sub-ann");
    let _ = h.sign_in_redirect(&app_side()).await;
    let first = h.fake.jwks_fetches.load(Ordering::SeqCst);
    assert_eq!(first, 1);
    h.knobs(Knobs {
        header: Some(json!({"alg": "ES256", "kid": "k2"})),
        ..Knobs::default()
    });
    h.advance(61_000);
    for _ in 0..5 {
        let _ = h.sign_in_redirect(&app_side()).await;
    }
    assert_eq!(
        h.fake.jwks_fetches.load(Ordering::SeqCst),
        2,
        "one refetch, not five"
    );
}

#[tokio::test]
async fn a_discovery_document_with_another_issuer_or_an_http_endpoint_is_refused() {
    let h = h_with(false).await;
    let app = app_side();
    let path = format!(
        "/api/sign-in/oidc/start?app_challenge={}&app_redirect=https%3A%2F%2Fcompanion.test%2Fapp",
        app.challenge
    );
    h.knobs(Knobs {
        discovery_issuer: Some("https://evil.example".into()),
        ..Knobs::default()
    });
    let (s, loc, _) = h.raw(Method::GET, &path, None, None).await;
    assert_eq!((s, loc), (StatusCode::BAD_GATEWAY, None));
    h.knobs(Knobs {
        token_endpoint: Some("http://id.example/token".into()),
        ..Knobs::default()
    });
    let (s, _, _) = h.raw(Method::GET, &path, None, None).await;
    assert_eq!(s, StatusCode::BAD_GATEWAY);
}

#[tokio::test]
async fn the_apps_redirect_must_stay_on_the_companion() {
    let h = h_with(false).await;
    let app = app_side();
    for bad in [
        "https://evil.example/app",
        "https://companion.test.evil.example/app",
        "https://companion.test//evil.example/app",
        "https://companion.test@evil.example/app",
        "https://companion.test/app#x",
        "https://companion.test/a\\b",
        "http://companion.test/app",
    ] {
        let path = format!(
            "/api/sign-in/oidc/start?app_challenge={}&app_redirect={}",
            app.challenge,
            crate::oidc::with_query("", &[("x", bad)]).trim_start_matches("?x=")
        );
        let (s, loc, _) = h.raw(Method::GET, &path, None, None).await;
        assert_eq!((s, loc), (StatusCode::UNAUTHORIZED, None), "{bad}");
    }
    // A challenge that is not S256-shaped is refused too.
    let path = "/api/sign-in/oidc/start?app_challenge=short&app_redirect=https%3A%2F%2Fcompanion.test%2Fapp";
    assert_eq!(
        h.raw(Method::GET, path, None, None).await.0,
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn an_unknown_person_is_refused_unless_users_are_created_for_the_group() {
    let h = h_with(false).await;
    let back = h.sign_in_redirect(&app_side()).await;
    assert_eq!(back.get("error").map(String::as_str), Some("not_linked"));
    assert!(h.store().users().unwrap().is_empty());
    assert!(h
        .store()
        .audit_entries(None, 10)
        .unwrap()
        .iter()
        .any(|e| e.action == "sign_in" && e.outcome == "failed"));

    let h = h_with(true).await;
    h.knobs(Knobs {
        claims: json!({"groups": ["other"]}),
        ..Knobs::default()
    });
    let back = h.sign_in_redirect(&app_side()).await;
    assert_eq!(
        back.get("error").map(String::as_str),
        Some("not_linked"),
        "not in the group"
    );
    h.knobs(Knobs::default());
    let app = app_side();
    let back = h.sign_in_redirect(&app).await;
    let (s, v) = h.exchange(&back["code"], &app.verifier).await;
    assert_eq!(s, StatusCode::OK);
    let (_, _, me) = h
        .raw(
            Method::GET,
            "/api/me",
            Some(v["access_token"].as_str().unwrap()),
            None,
        )
        .await;
    assert_eq!(
        (me["name"].as_str(), me["role"].as_str()),
        (Some("ann"), Some("user"))
    );
    // The second time, the same person is the same user.
    let back = h.sign_in_redirect(&app_side()).await;
    assert!(back.contains_key("code"));
    assert_eq!(h.store().users().unwrap().len(), 1);
}

#[tokio::test]
async fn a_disabled_user_is_refused_after_the_provider_says_yes() {
    let h = h_with(false).await;
    let ann = h.user("ann");
    h.link(ann, "made-up-sub-ann");
    let root = h
        .store()
        .add_user(&h.master, "root", Role::Admin)
        .unwrap()
        .id;
    let now = h.now();
    h.store().set_disabled(root, ann, true, now).unwrap();
    let back = h.sign_in_redirect(&app_side()).await;
    assert_eq!(back.get("error").map(String::as_str), Some("not_linked"));
    // And a code handed over just before the disabling does not sign in either.
    let now = h.now();
    h.store().set_disabled(root, ann, false, now).unwrap();
    let app = app_side();
    let back = h.sign_in_redirect(&app).await;
    let now = h.now();
    h.store().set_disabled(root, ann, true, now).unwrap();
    assert_eq!(
        h.exchange(&back["code"], &app.verifier).await.0,
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn a_signed_in_user_links_their_identity_and_nobody_else_can_finish_it() {
    let h = h_with(false).await;
    let ann = h.user("ann");
    let bob = h.user("bob");
    let ann_t = h.session(ann);
    let bob_t = h.session(bob);
    let app = app_side();
    let (s, _, v) = h
        .raw(
            Method::POST,
            "/api/oidc/link/start",
            Some(&ann_t),
            Some(json!({"app_challenge": app.challenge, "app_redirect": APP_REDIRECT})),
        )
        .await;
    assert_eq!(s, StatusCode::OK);
    let (s, loc) = h.browse(v["url"].as_str().unwrap()).await;
    assert_eq!(s, StatusCode::SEE_OTHER);
    let code = query_of(&loc.unwrap())["code"].clone();
    // Bob cannot finish Ann's link, even with her verifier; the code is spent by the try.
    let (s, _, _) = h
        .raw(
            Method::POST,
            "/api/oidc/link/finish",
            Some(&bob_t),
            Some(json!({"code": code, "verifier": app.verifier})),
        )
        .await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
    assert_eq!(
        h.store()
            .user_for_identity(&h.fake.issuer, "made-up-sub-ann")
            .unwrap(),
        None
    );
    // A link code cannot be used to sign in either.
    let app = app_side();
    let (_, _, v) = h
        .raw(
            Method::POST,
            "/api/oidc/link/start",
            Some(&ann_t),
            Some(json!({"app_challenge": app.challenge, "app_redirect": APP_REDIRECT})),
        )
        .await;
    let (_, loc) = h.browse(v["url"].as_str().unwrap()).await;
    let code = query_of(&loc.unwrap())["code"].clone();
    assert_eq!(
        h.exchange(&code, &app.verifier).await.0,
        StatusCode::UNAUTHORIZED
    );
    // Ann finishes her own.
    let app = app_side();
    let (_, _, v) = h
        .raw(
            Method::POST,
            "/api/oidc/link/start",
            Some(&ann_t),
            Some(json!({"app_challenge": app.challenge, "app_redirect": APP_REDIRECT})),
        )
        .await;
    let (_, loc) = h.browse(v["url"].as_str().unwrap()).await;
    let code = query_of(&loc.unwrap())["code"].clone();
    let (s, _, _) = h
        .raw(
            Method::POST,
            "/api/oidc/link/finish",
            Some(&ann_t),
            Some(json!({"code": code, "verifier": app.verifier})),
        )
        .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(
        h.store()
            .user_for_identity(&h.fake.issuer, "made-up-sub-ann")
            .unwrap(),
        Some(ann)
    );
    // Bob's identity cannot be the same person; unlinking is his own business or an admin's.
    assert_eq!(
        h.raw(Method::DELETE, "/api/oidc/link", Some(&bob_t), None)
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        h.raw(Method::DELETE, "/api/oidc/link", Some(&ann_t), None)
            .await
            .0,
        StatusCode::OK
    );
    assert_eq!(
        h.store()
            .user_for_identity(&h.fake.issuer, "made-up-sub-ann")
            .unwrap(),
        None
    );
}

#[tokio::test]
async fn an_admin_unlinks_a_users_identity_and_it_is_audited() {
    let h = h_with(false).await;
    let ann = h.user("ann");
    h.link(ann, "made-up-sub-ann");
    let root = h
        .store()
        .add_user(&h.master, "root", Role::Admin)
        .unwrap()
        .id;
    let root_t = h.session(root);
    let path = format!("/api/admin/users/{ann}/oidc/unlink");
    assert_eq!(
        h.raw(Method::POST, &path, Some(&root_t), None).await.0,
        StatusCode::OK
    );
    assert_eq!(
        h.raw(Method::POST, &path, Some(&root_t), None).await.0,
        StatusCode::NOT_FOUND
    );
    let entries = h.store().audit_entries(Some(ann), 10).unwrap();
    assert!(entries
        .iter()
        .any(|e| e.action == "oidc.unlink" && e.actor_id == Some(root)));
}

#[tokio::test]
async fn without_oidc_the_routes_say_so() {
    // The T24 harness has no provider.
    let master = Key::generate().unwrap();
    let auth = Auth::new(ORIGIN, master, Vec::new(), Auth::system_clock()).unwrap();
    let r = router(App::new(Store::in_memory().unwrap(), auth));
    let res = r
        .clone()
        .oneshot(Request::get("/api/oidc").body(Body::empty()).unwrap())
        .await
        .unwrap();
    let body = res.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(
        serde_json::from_slice::<Value>(&body).unwrap(),
        json!({"enabled": false})
    );
    let res = r
        .oneshot(
            Request::get("/api/sign-in/oidc/start?app_challenge=x&app_redirect=y")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn no_code_secret_or_token_is_stored_or_audited() {
    let h = h_with(true).await;
    let app = app_side();
    let back = h.sign_in_redirect(&app).await;
    let (_, v) = h.exchange(&back["code"], &app.verifier).await;
    let mut secrets: Vec<String> = vec![
        SECRET.into(),
        back["code"].clone(),
        app.verifier.clone(),
        v["access_token"].as_str().unwrap().into(),
        v["refresh_token"].as_str().unwrap().into(),
    ];
    secrets.extend(h.fake.issued.lock().unwrap().iter().cloned());
    secrets.retain(|s| s.len() > 8);
    let store = h.store();
    for (table, _) in store.tables_and_user_scope().unwrap() {
        let mut stmt = store
            .conn
            .prepare(&format!("SELECT * FROM {table}"))
            .unwrap();
        let n = stmt.column_count();
        let mut rows = stmt.query([]).unwrap();
        while let Some(row) = rows.next().unwrap() {
            for i in 0..n {
                if let rusqlite::types::ValueRef::Text(t) | rusqlite::types::ValueRef::Blob(t) =
                    row.get_ref(i).unwrap()
                {
                    for s in &secrets {
                        assert!(
                            !t.windows(s.len()).any(|w| w == s.as_bytes()),
                            "a secret in {table}"
                        );
                    }
                }
            }
        }
    }
    // The provider's Debug never shows the secret.
    let dbg = format!("{:?}", h.app.auth.oidc.as_ref().unwrap());
    assert!(!dbg.contains(SECRET));
}

// ---- Regression tests for the independent review's findings ----

/// M1: with a `public_url` that has a path, dot segments (`/../`, `%2e%2e`) took the browser,
/// and the hand-over code, out of the Companion's path.
#[tokio::test]
async fn an_app_redirect_with_dot_segments_or_another_path_is_refused() {
    let h = h_full(false, "https://companion.test/croft").await;
    let p = h.app.auth.oidc.clone().unwrap();
    for bad in [
        "https://companion.test/croft/../mallory/",
        "https://companion.test/croft/%2e%2e/mallory/",
        "https://companion.test/croft/.%2E/mallory/",
        "https://companion.test/croft/a/../../mallory/",
        "https://companion.test/croft/./app",
        "https://companion.test/croft%2f..%2fmallory",
        "https://companion.test/croftx/app",
        "https://companion.test/other",
        "https://companion.test/croft",
        "https://companion.test:443/croft/app",
        "https://COMPANION.test/croft/app",
        "https://companion.test/croft//evil.example/",
    ] {
        assert!(!p.app_redirect_ok(bad), "{bad}");
    }
    for good in [
        "https://companion.test/croft/app",
        "https://companion.test/croft/app/signed-in?x=1",
    ] {
        assert!(p.app_redirect_ok(good), "{good}");
    }
    // And end to end: the whole flow under a path still works.
    let ann = h.user("ann");
    h.link(ann, "made-up-sub-ann");
    let app = app_side();
    let path = format!(
        "/api/sign-in/oidc/start?app_challenge={}&app_redirect=https%3A%2F%2Fcompanion.test%2Fcroft%2Fapp",
        app.challenge
    );
    let (s, loc, _) = h.raw(Method::GET, &path, None, None).await;
    assert_eq!(s, StatusCode::SEE_OTHER);
    let (_, back) = h.browse(&loc.unwrap()).await;
    let back = back.unwrap();
    assert!(
        back.starts_with("https://companion.test/croft/app?code="),
        "{back}"
    );
}

async fn enrol_with_a_passkey(h: &H, user_id: i64) {
    use webauthn_authenticator_rs::softpasskey::SoftPasskey;
    use webauthn_authenticator_rs::WebauthnAuthenticator;
    let now = h.now();
    let token = h
        .store()
        .create_enrolment(user_id, None, now)
        .unwrap()
        .to_string();
    let (s, _, start) = h
        .raw(
            Method::POST,
            "/api/enrol/start",
            None,
            Some(json!({"token": token})),
        )
        .await;
    assert_eq!(s, StatusCode::OK);
    let ccr: webauthn_rs::prelude::CreationChallengeResponse =
        serde_json::from_value(start["options"].clone()).unwrap();
    let cred = SoftPasskey::new(true)
        .do_registration(webauthn_rs::prelude::Url::parse(ORIGIN).unwrap(), ccr)
        .unwrap();
    let (s, _, _) = h
        .raw(
            Method::POST,
            "/api/enrol/finish",
            None,
            Some(json!({"token": token, "ceremony": start["ceremony"], "credential": cred})),
        )
        .await;
    assert_eq!(s, StatusCode::OK);
}

/// M2: an identity linked with a stolen session still signed in after re-enrolment, the
/// documented way to cut off a lost device.
#[tokio::test]
async fn re_enrolment_removes_a_linked_identity() {
    let h = h_with(false).await;
    let ann = h.user("ann");
    let stolen = h.session(ann);
    // The thief links their own provider account.
    h.knobs(Knobs {
        claims: json!({"sub": "made-up-sub-mallory"}),
        ..Knobs::default()
    });
    let app = app_side();
    let (_, _, v) = h
        .raw(
            Method::POST,
            "/api/oidc/link/start",
            Some(&stolen),
            Some(json!({"app_challenge": app.challenge, "app_redirect": APP_REDIRECT})),
        )
        .await;
    let (_, loc) = h.browse(v["url"].as_str().unwrap()).await;
    let code = query_of(&loc.unwrap())["code"].clone();
    let (s, _, _) = h
        .raw(
            Method::POST,
            "/api/oidc/link/finish",
            Some(&stolen),
            Some(json!({"code": code, "verifier": app.verifier})),
        )
        .await;
    assert_eq!(s, StatusCode::OK);
    // It is visible to Ann and to the admin.
    let (_, _, me) = h.raw(Method::GET, "/api/me", Some(&stolen), None).await;
    assert_eq!(me["oidc_linked"], true);
    // The admin re-enrols Ann: every way in is replaced, the planted identity included.
    enrol_with_a_passkey(&h, ann).await;
    assert_eq!(
        h.raw(Method::GET, "/api/me", Some(&stolen), None).await.0,
        StatusCode::UNAUTHORIZED
    );
    let back = h.sign_in_redirect(&app_side()).await;
    assert_eq!(back.get("error").map(String::as_str), Some("not_linked"));
    assert!(h
        .store()
        .audit_entries(Some(ann), 50)
        .unwrap()
        .iter()
        .any(|e| e.action == "oidc.unlink"));
}

/// M2: linking needs a recent sign-in, not just any live session.
#[tokio::test]
async fn linking_needs_a_recent_sign_in() {
    let h = h_with(false).await;
    let ann = h.user("ann");
    let t = h.session(ann);
    h.advance(crate::api::oidc_api::LINK_FRESH_MS + 1);
    let app = app_side();
    let (s, _, _) = h
        .raw(
            Method::POST,
            "/api/oidc/link/start",
            Some(&t),
            Some(json!({"app_challenge": app.challenge, "app_redirect": APP_REDIRECT})),
        )
        .await;
    assert_eq!(s, StatusCode::FORBIDDEN);
}

/// Low: a key rotated under the same key id was never picked up.
#[tokio::test]
async fn a_key_rotated_under_the_same_id_is_picked_up() {
    let h = h_with(false).await;
    let ann = h.user("ann");
    h.link(ann, "made-up-sub-ann");
    assert!(h.sign_in_redirect(&app_side()).await.contains_key("code"));
    *h.fake.signer.lock().unwrap() = TestSigner::es256();
    h.advance(61_000);
    let back = h.sign_in_redirect(&app_side()).await;
    assert!(back.contains_key("code"), "{back:?}");
    assert_eq!(h.fake.jwks_fetches.load(Ordering::SeqCst), 2);
}

/// Low: a provider that lists its PKCE methods without S256 is refused; a provider that is down
/// is not asked again on every start.
#[tokio::test]
async fn pkce_must_be_s256_and_a_failed_discovery_waits_before_the_next_try() {
    let h = h_with(false).await;
    let path = format!(
        "/api/sign-in/oidc/start?app_challenge={}&app_redirect=https%3A%2F%2Fcompanion.test%2Fapp",
        app_side().challenge
    );
    h.knobs(Knobs {
        pkce_methods: Some(json!(["plain"])),
        ..Knobs::default()
    });
    assert_eq!(
        h.raw(Method::GET, &path, None, None).await.0,
        StatusCode::BAD_GATEWAY
    );
    let fetched = h.fake.discovery_fetches.load(Ordering::SeqCst);
    h.knobs(Knobs::default());
    assert_eq!(
        h.raw(Method::GET, &path, None, None).await.0,
        StatusCode::BAD_GATEWAY,
        "waits"
    );
    assert_eq!(h.fake.discovery_fetches.load(Ordering::SeqCst), fetched);
    h.advance(31_000);
    assert_eq!(
        h.raw(Method::GET, &path, None, None).await.0,
        StatusCode::SEE_OTHER
    );
    h.knobs(Knobs {
        pkce_methods: Some(json!(["S256", "plain"])),
        ..Knobs::default()
    });
    assert_eq!(
        h.raw(Method::GET, &path, None, None).await.0,
        StatusCode::SEE_OTHER
    );
}

/// Low: a new user's name may not differ from an existing one only by case.
#[tokio::test]
async fn a_created_name_never_differs_from_another_only_by_case() {
    let h = h_with(true).await;
    h.store().add_user(&h.master, "Ann", Role::Admin).unwrap();
    let app = app_side();
    let back = h.sign_in_redirect(&app).await;
    let (_, v) = h.exchange(&back["code"], &app.verifier).await;
    let (_, _, me) = h
        .raw(
            Method::GET,
            "/api/me",
            Some(v["access_token"].as_str().unwrap()),
            None,
        )
        .await;
    assert_eq!(
        (me["name"].as_str(), me["role"].as_str()),
        (Some("ann-2"), Some("user"))
    );
}
