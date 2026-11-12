// This Source Code Form is subject to the terms of the GNU Affero General Public License,
// version 3 or later. See LICENSE-AGPL-3.0 at the repository root.

//! The OIDC routes (T25), a part of [`super`] (the route table lists them with their access).
//!
//! Sign-in: the app makes a PKCE verifier, sends its S256 challenge and its redirect to
//! `/api/sign-in/oidc/start`, the browser goes to the provider and back to `/oidc/callback`, which
//! sends it on to the app's redirect with a one-time `code` (or an `error`); the app redeems the
//! code with its verifier at `/api/sign-in/oidc/exchange` for T24 tokens. Linking works the same
//! way, started and finished by a signed-in app. A failed check counts for the address (never
//! toward a user's lock: nothing here can be guessed) and is audited.

use axum::extract::{Query, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use serde_json::json;
use zeroize::Zeroizing;

use super::{
    body, device_name, failed, limit, start_session, with_store, Admin, ApiError, ApiResult, Full,
    MaybePeer,
};
use crate::ceremony::Owner;
use crate::http::App;
use crate::limits;
use crate::oidc::{with_query, OidcError, Outcome, Provider, Purpose};
use crate::store::StoreError;

/// How recent a sign-in must be to link an identity.
pub const LINK_FRESH_MS: i64 = 10 * 60 * 1000;

fn provider(app: &App) -> Result<std::sync::Arc<Provider>, ApiError> {
    app.auth.oidc.clone().ok_or(ApiError::NotFound)
}

fn see_other(location: &str) -> Response {
    match HeaderValue::from_str(location) {
        Ok(v) => {
            let mut res = StatusCode::SEE_OTHER.into_response();
            res.headers_mut().insert(header::LOCATION, v);
            res
        }
        Err(_) => ApiError::Server.into_response(),
    }
}

fn oidc_error(e: &OidcError) -> ApiError {
    match e {
        OidcError::Unreachable | OidcError::Discovery(_) => ApiError::Unavailable,
        OidcError::Server => ApiError::Server,
        _ => ApiError::Invalid,
    }
}

/// `GET /api/oidc`: whether OIDC sign-in is offered, and the button's name.
pub async fn info(State(app): State<App>) -> ApiResult {
    Ok(Json(match &app.auth.oidc {
        Some(p) => json!({"enabled": true, "name": p.settings().name}),
        None => json!({"enabled": false}),
    })
    .into_response())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StartQuery {
    app_challenge: String,
    app_redirect: String,
}

/// `GET /api/sign-in/oidc/start?app_challenge=..&app_redirect=..`: sends the browser to the
/// provider.
pub async fn sign_in_start(
    State(app): State<App>,
    MaybePeer(peer): MaybePeer,
    headers: HeaderMap,
    q: Result<Query<StartQuery>, axum::extract::rejection::QueryRejection>,
) -> ApiResult {
    let ip = limit(&app, peer, &headers)?;
    let Query(q) = q.map_err(|_| ApiError::BadRequest)?;
    let p = provider(&app)?;
    let url = p
        .start(
            Owner::Address(limits::who(ip)),
            Purpose::SignIn,
            &q.app_challenge,
            &q.app_redirect,
            app.auth.now(),
        )
        .await
        .map_err(|e| oidc_error(&e))?;
    Ok(see_other(&url))
}

#[derive(Deserialize)]
pub struct CallbackQuery {
    state: Option<String>,
    code: Option<String>,
    error: Option<String>,
    // The provider may add others (`session_state`, `iss`); they are not used.
}

/// `GET /oidc/callback`: the provider sends the browser here; it goes on to the app's redirect
/// with a one-time `code`, or with `error` (`provider`, `unreachable`, `invalid`, `not_linked`).
pub async fn callback(
    State(app): State<App>,
    MaybePeer(peer): MaybePeer,
    headers: HeaderMap,
    q: Result<Query<CallbackQuery>, axum::extract::rejection::QueryRejection>,
) -> ApiResult {
    let ip = limit(&app, peer, &headers)?;
    let Query(q) = q.map_err(|_| ApiError::BadRequest)?;
    let p = provider(&app)?;
    let now = app.auth.now();
    let Some(flow) = q.state.as_deref().and_then(|s| p.take_flow(s, now)) else {
        failed(&app, ip);
        let _ = with_store(&app, move |s| s.record_failure(None, now, false)).await;
        return Err(ApiError::Invalid);
    };
    let back = |pairs: &[(&str, &str)]| see_other(&with_query(flow.app_redirect(), pairs));
    if q.error.is_some() {
        let _ = with_store(&app, move |s| s.record_failure(None, now, false)).await;
        return Ok(back(&[("error", "provider")]));
    }
    let Some(code) = q.code.map(Zeroizing::new) else {
        return Ok(back(&[("error", "provider")]));
    };
    let identity = match p.finish(&flow, &code, now).await {
        Ok(id) => id,
        Err(e) => {
            failed(&app, ip);
            let _ = with_store(&app, move |s| s.record_failure(None, now, false)).await;
            let word = if e == OidcError::Unreachable {
                "unreachable"
            } else {
                "invalid"
            };
            return Ok(back(&[("error", word)]));
        }
    };
    let outcome = match flow.purpose().clone() {
        Purpose::Link {
            user_id,
            session_id,
        } => Some(Outcome::Link {
            user_id,
            session_id,
            issuer: identity.issuer,
            subject: identity.subject,
        }),
        Purpose::SignIn => {
            let settings = p.settings().clone();
            let auth_ = app.auth.clone();
            with_store(&app, move |s| {
                if let Some(id) = s.user_for_identity(&identity.issuer, &identity.subject)? {
                    let disabled = s.user_by_id(id)?.is_none_or(|u| u.disabled);
                    if disabled {
                        s.record_failure(Some(id), now, false)?;
                        return Ok(None);
                    }
                    return Ok(Some(Outcome::SignIn { user_id: id }));
                }
                let member = settings
                    .required_group
                    .as_ref()
                    .is_some_and(|g| identity.groups.contains(g));
                if settings.create_users && member {
                    let id = s.create_oidc_user(
                        &auth_.master,
                        identity.preferred_username.as_deref(),
                        &identity.issuer,
                        &identity.subject,
                        now,
                    )?;
                    return Ok(Some(Outcome::SignIn { user_id: id }));
                }
                s.record_failure(None, now, false)?;
                Ok::<_, StoreError>(None)
            })
            .await??
        }
    };
    let Some(outcome) = outcome else {
        failed(&app, ip);
        return Ok(back(&[("error", "not_linked")]));
    };
    let handover = p
        .handover(&flow, outcome, now)
        .map_err(|e| oidc_error(&e))?;
    Ok(back(&[("code", handover.as_str())]))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Exchange {
    code: String,
    verifier: String,
    device: Option<String>,
}

/// `POST /api/sign-in/oidc/exchange`: the hand-over code and the app's verifier for tokens.
pub async fn exchange(
    State(app): State<App>,
    MaybePeer(peer): MaybePeer,
    headers: HeaderMap,
    req: Result<Json<Exchange>, axum::extract::rejection::JsonRejection>,
) -> ApiResult {
    let ip = limit(&app, peer, &headers)?;
    let req = body(req)?;
    let p = provider(&app)?;
    let (code, verifier) = (Zeroizing::new(req.code), Zeroizing::new(req.verifier));
    match p.redeem(&code, &verifier, app.auth.now()) {
        Ok(Outcome::SignIn { user_id }) => {
            let res = start_session(&app, user_id, device_name(req.device), false).await;
            if res.is_ok() {
                // Marks the sign-in as made through the provider.
                let _ =
                    with_store(&app, move |s| s.audit(Some(user_id), "sign_in.oidc", "ok")).await;
            }
            res
        }
        _ => {
            failed(&app, ip);
            Err(ApiError::Invalid)
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LinkStart {
    app_challenge: String,
    app_redirect: String,
}

/// `POST /api/oidc/link/start`: the provider URL the signed-in app sends the browser to.
pub async fn link_start(
    State(app): State<App>,
    MaybePeer(peer): MaybePeer,
    headers: HeaderMap,
    Full(c): Full,
    req: Result<Json<LinkStart>, axum::extract::rejection::JsonRejection>,
) -> ApiResult {
    limit(&app, peer, &headers)?;
    let req = body(req)?;
    let p = provider(&app)?;
    let now = app.auth.now();
    // Linking adds a way in, so it needs a sign-in from the last minutes, not just any live
    // session (T25 review: a stolen session could plant the thief's identity).
    let started = with_store(&app, move |s| s.session_started(c.user_id, c.session_id)).await??;
    if started.is_none_or(|t| now - t > LINK_FRESH_MS) {
        return Err(ApiError::Forbidden);
    }
    let url = p
        .start(
            Owner::User(c.user_id),
            Purpose::Link {
                user_id: c.user_id,
                session_id: c.session_id,
            },
            &req.app_challenge,
            &req.app_redirect,
            app.auth.now(),
        )
        .await
        .map_err(|e| oidc_error(&e))?;
    Ok(Json(json!({ "url": url })).into_response())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LinkFinish {
    code: String,
    verifier: String,
}

/// `POST /api/oidc/link/finish`: links the identity, if the code was made for this user's own
/// session and the verifier matches.
pub async fn link_finish(
    State(app): State<App>,
    Full(c): Full,
    req: Result<Json<LinkFinish>, axum::extract::rejection::JsonRejection>,
) -> ApiResult {
    let req = body(req)?;
    let p = provider(&app)?;
    let now = app.auth.now();
    let (code, verifier) = (Zeroizing::new(req.code), Zeroizing::new(req.verifier));
    let Ok(Outcome::Link {
        user_id,
        session_id,
        issuer,
        subject,
    }) = p.redeem(&code, &verifier, now)
    else {
        return Err(ApiError::Invalid);
    };
    if user_id != c.user_id || session_id != c.session_id {
        return Err(ApiError::Invalid);
    }
    with_store(&app, move |s| {
        if !s.session_live(user_id, session_id)? {
            return Err(StoreError::NoSuchUser);
        }
        let linked = s.link_identity(user_id, &issuer, &subject, now);
        if matches!(linked, Err(StoreError::Conflict)) {
            s.audit(Some(user_id), "oidc.link", "conflict")?;
        }
        linked
    })
    .await?
    .map_err(|e| match e {
        StoreError::NoSuchUser => ApiError::Unauthorized,
        e => e.into(),
    })?;
    Ok(Json(json!({"ok": true})).into_response())
}

/// `DELETE /api/oidc/link`: removes the caller's own linked identity.
pub async fn unlink(State(app): State<App>, Full(c): Full) -> ApiResult {
    let p = provider(&app)?;
    let issuer = p.settings().issuer.clone();
    let now = app.auth.now();
    if with_store(&app, move |s| {
        s.unlink_identity(c.user_id, &issuer, None, now)
    })
    .await??
    {
        Ok(Json(json!({"ok": true})).into_response())
    } else {
        Err(ApiError::NotFound)
    }
}

/// `POST /api/admin/users/{id}/oidc/unlink`: an admin removes a user's linked identity.
pub async fn admin_unlink(
    State(app): State<App>,
    Admin(c): Admin,
    id: Result<axum::extract::Path<i64>, axum::extract::rejection::PathRejection>,
) -> ApiResult {
    let axum::extract::Path(id) = id.map_err(|_| ApiError::NotFound)?;
    let p = provider(&app)?;
    let issuer = p.settings().issuer.clone();
    let now = app.auth.now();
    if with_store(&app, move |s| {
        s.unlink_identity(id, &issuer, Some(c.user_id), now)
    })
    .await??
    {
        Ok(Json(json!({"ok": true})).into_response())
    } else {
        Err(ApiError::NotFound)
    }
}
