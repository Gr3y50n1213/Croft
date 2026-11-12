// This Source Code Form is subject to the terms of the GNU Affero General Public License,
// version 3 or later. See LICENSE-AGPL-3.0 at the repository root.

//! Sign-in sessions for the web app (T30), a part of [`super`]. A browser never holds a long-lived
//! secret a script can read: its refresh token lives in an `HttpOnly`, `SameSite=Strict` cookie
//! (`__Host-croft_refresh` over https), and the app keeps only the 15-minute access token, in
//! memory. Every request that reads or sets the cookie must carry `X-Croft-Web: 1` (which a page on
//! another origin cannot send without a preflight the Companion never grants) and, when the browser
//! names them, this origin and a same-origin fetch.

use axum::extract::State;
use axum::http::{header, HeaderMap, HeaderValue};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;
use zeroize::Zeroizing;

use super::{failed, limit, ApiError, ApiResult, MaybePeer};
use crate::auth::{RefreshError, Tokens, REFRESH_TTL_MS};
use crate::http::App;

/// The header a web request must carry.
pub const WEB_HEADER: &str = "x-croft-web";

/// The cookie's name: `__Host-` (secure, this host only, path `/`) over https; plain on
/// `http://localhost` (development and tests), where a browser refuses `Secure` cookies.
fn cookie_name(app: &App) -> &'static str {
    if secure(app) {
        "__Host-croft_refresh"
    } else {
        "croft_refresh"
    }
}

fn secure(app: &App) -> bool {
    app.auth.public_url().starts_with("https://")
}

/// The public URL's origin (`scheme://host[:port]`).
fn origin(app: &App) -> &str {
    let url = app.auth.public_url();
    let rest = url.split_once("://").map_or(url, |(_, r)| r);
    let end = url.len() - rest.len() + rest.find('/').unwrap_or(rest.len());
    &url[..end]
}

/// Whether a request is the web app's own: the header, and the origin and fetch site when the
/// browser sends them.
pub(crate) fn from_web_app(app: &App, headers: &HeaderMap) -> bool {
    let text = |name: &str| headers.get(name).and_then(|v| v.to_str().ok());
    text(WEB_HEADER) == Some("1")
        && text("origin").is_none_or(|o| o.eq_ignore_ascii_case(origin(app)))
        && text("sec-fetch-site").is_none_or(|s| s == "same-origin")
}

fn set_cookie(app: &App, value: &str, max_age_s: i64) -> Option<HeaderValue> {
    let mut c = Zeroizing::new(format!(
        "{}={value}; Path=/; Max-Age={max_age_s}; HttpOnly; SameSite=Strict",
        cookie_name(app)
    ));
    if secure(app) {
        c.push_str("; Secure");
    }
    HeaderValue::from_str(&c).ok()
}

/// The refresh token in the request's cookie; `None` when there is none, or more than one (a
/// cookie set by someone else under the same name must not be picked by chance).
fn cookie(app: &App, headers: &HeaderMap) -> Option<Zeroizing<String>> {
    let name = cookie_name(app);
    let mut found = None;
    for value in headers.get_all(header::COOKIE) {
        let Ok(value) = value.to_str() else {
            continue;
        };
        for pair in value.split(';') {
            if let Some((k, v)) = pair.trim().split_once('=') {
                if k == name {
                    if found.is_some() {
                        return None;
                    }
                    found = Some(Zeroizing::new(v.to_owned()));
                }
            }
        }
    }
    found.filter(|v| !v.is_empty() && v.len() <= 128)
}

/// The answer to a sign-in: for an app, both tokens in the body (as before T30); for the web app,
/// the refresh token as the cookie and only the access token in the body.
pub(crate) fn tokens_response(app: &App, tokens: &Tokens, recovery: bool, web: bool) -> Response {
    if !web {
        return Json(json!({
            "access_token": tokens.access.as_str(),
            "refresh_token": tokens.refresh.as_str(),
            "access_expires_ms": tokens.access_expires_ms,
            "recovery": recovery,
        }))
        .into_response();
    }
    let mut res = Json(json!({
        "access_token": tokens.access.as_str(),
        "access_expires_ms": tokens.access_expires_ms,
        "recovery": recovery,
    }))
    .into_response();
    match set_cookie(app, &tokens.refresh, REFRESH_TTL_MS / 1000) {
        Some(c) => {
            res.headers_mut().insert(header::SET_COOKIE, c);
            res
        }
        None => ApiError::Server.into_response(),
    }
}

/// Refuses a sign-in that asks for a web session but is not the web app's own request.
pub(crate) fn check_web(app: &App, headers: &HeaderMap, web: bool) -> Result<(), ApiError> {
    if web && !from_web_app(app, headers) {
        return Err(ApiError::Forbidden);
    }
    Ok(())
}

fn cleared(app: &App, mut res: Response) -> Response {
    if let Some(c) = set_cookie(app, "", 0) {
        res.headers_mut().insert(header::SET_COOKIE, c);
    }
    res
}

/// `POST /api/web/refresh`: a new access token, and the cookie rotated. A refresh token used twice
/// ends the session (as for apps), and the cookie is cleared.
pub async fn refresh(
    State(app): State<App>,
    MaybePeer(peer): MaybePeer,
    headers: HeaderMap,
) -> Response {
    let ip = match limit(&app, peer, &headers) {
        Ok(ip) => ip,
        Err(e) => return e.into_response(),
    };
    if !from_web_app(&app, &headers) {
        return ApiError::Forbidden.into_response();
    }
    let Some(token) = cookie(&app, &headers) else {
        failed(&app, ip);
        return cleared(&app, ApiError::Invalid.into_response());
    };
    let now = app.auth.now();
    match super::with_store(&app, move |s| s.refresh(&token, now)).await {
        Ok(Ok(tokens)) => tokens_response(&app, &tokens, false, true),
        Ok(Err(RefreshError::Invalid | RefreshError::Reused)) => {
            failed(&app, ip);
            cleared(&app, ApiError::Invalid.into_response())
        }
        Ok(Err(RefreshError::Store(_))) | Err(_) => ApiError::Server.into_response(),
    }
}

/// `POST /api/web/sign-out`: ends the browser's session (named by its cookie) and clears the
/// cookie. Answers `ok` also when there was no session.
pub async fn sign_out(State(app): State<App>, headers: HeaderMap) -> ApiResult {
    if !from_web_app(&app, &headers) {
        return Err(ApiError::Forbidden);
    }
    if let Some(token) = cookie(&app, &headers) {
        let now = app.auth.now();
        super::with_store(&app, move |s| {
            if let Some((user, session)) = s.session_of_refresh(&token, now)? {
                s.revoke_session(user, session, now)?;
            }
            Ok::<_, crate::store::StoreError>(())
        })
        .await??;
    }
    Ok(cleared(&app, Json(json!({"ok": true})).into_response()))
}
