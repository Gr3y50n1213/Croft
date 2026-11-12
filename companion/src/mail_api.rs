// This Source Code Form is subject to the terms of the GNU Affero General Public License,
// version 3 or later. See LICENSE-AGPL-3.0 at the repository root.

//! The mail account routes (T26), a part of [`super`] (the route table lists them, full sessions
//! only). Every call works on the caller's own core, so an account id of another user names
//! nothing here. Passwords come in request bodies only, are wiped after use, and are never
//! returned; one is sealed (with the user's data key) only after the core has accepted the account.
//!
//! The JSON shapes mirror the core's types (`FfiDiscovered`, `VerifyFailure`, `AccountInfo`) with
//! snake_case names.

use std::sync::Arc;

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use croft_ffi::{
    AccountStatus, AuthKind, Core, DiscoverError, DiscoverySource, FfiCredential, FfiDiscovered,
    KnownProvider, SecurityMode, ServerSettings, VerifyFailure,
};
use serde::Deserialize;
use serde_json::{json, Value};
use zeroize::Zeroizing;

use super::{body, with_store, ApiError, ApiResult, Full, MaybePeer};
use crate::http::App;

/// The caller's core, opened if needed.
async fn core(app: &App, user_id: i64) -> Result<Arc<Core>, ApiError> {
    let mail = app.mail.clone().ok_or(ApiError::NotFound)?;
    let store = app.store.clone();
    let auth = app.auth.clone();
    tokio::task::spawn_blocking(move || mail.core(&store, auth.master(), user_id))
        .await
        .map_err(|_| ApiError::Server)?
        .map_err(|_| ApiError::Server)
}

fn failure(kind: &str, reason: &str, extra: Value) -> Response {
    let mut v = json!({ "error": kind, "reason": reason });
    if let (Some(o), Some(e)) = (v.as_object_mut(), extra.as_object()) {
        for (k, x) in e {
            o.insert(k.clone(), x.clone());
        }
    }
    (StatusCode::UNPROCESSABLE_ENTITY, Json(v)).into_response()
}

// ---- JSON <-> the core's types ----

fn security_str(s: SecurityMode) -> &'static str {
    match s {
        SecurityMode::Tls => "tls",
        SecurityMode::StartTls => "starttls",
    }
}

fn provider_str(p: KnownProvider) -> &'static str {
    match p {
        KnownProvider::Gmail => "gmail",
        KnownProvider::Outlook => "outlook",
        KnownProvider::ICloud => "icloud",
        KnownProvider::Fastmail => "fastmail",
    }
}

fn server_json(s: &ServerSettings) -> Value {
    json!({ "host": s.host, "port": s.port, "security": security_str(s.security), "username": s.username })
}

/// The core's discovery result as JSON.
pub fn discovered_json(d: &FfiDiscovered) -> Value {
    json!({
        "email": d.email,
        "imap": d.imap.as_ref().map(server_json),
        "smtp": d.smtp.as_ref().map(server_json),
        "jmap": d.jmap,
        "source": match d.source {
            DiscoverySource::ProviderTable => "provider_table",
            DiscoverySource::Autoconfig => "autoconfig",
            DiscoverySource::Srv => "srv",
            DiscoverySource::JmapWellKnown => "jmap_well_known",
        },
        "provider": d.provider.map(provider_str),
        "auth_hint": match d.auth_hint {
            AuthKind::Password => "password",
            AuthKind::AppPassword => "app_password",
            AuthKind::OAuth2 => "oauth2",
            AuthKind::OAuth2OrAppPassword => "oauth2_or_app_password",
        },
    })
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ServerIn {
    host: String,
    port: u16,
    security: String,
    username: String,
}

/// Settings as the client sends them back (from discovery, or typed).
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DiscoveredIn {
    email: String,
    imap: Option<ServerIn>,
    smtp: Option<ServerIn>,
    jmap: Option<String>,
    source: Option<String>,
    provider: Option<String>,
    auth_hint: Option<String>,
}

fn server_in(s: ServerIn) -> Option<ServerSettings> {
    let security = match s.security.as_str() {
        "tls" => SecurityMode::Tls,
        "starttls" => SecurityMode::StartTls,
        _ => return None,
    };
    let ok = |t: &str| !t.is_empty() && t.len() <= 255 && !t.chars().any(char::is_control);
    if !ok(&s.host) || !ok(&s.username) || s.port == 0 {
        return None;
    }
    Some(ServerSettings {
        host: s.host,
        port: s.port,
        security,
        username: s.username,
    })
}

/// The client's settings as the core's type; `None` when something is malformed.
pub fn discovered_in(d: DiscoveredIn) -> Option<FfiDiscovered> {
    let imap = d.imap.map(server_in);
    let smtp = d.smtp.map(server_in);
    if matches!(imap, Some(None)) || matches!(smtp, Some(None)) {
        return None;
    }
    if d.email.len() > 320 || !d.email.contains('@') || d.email.chars().any(char::is_control) {
        return None;
    }
    if d.jmap.as_ref().is_some_and(|u| u.len() > 2048) {
        return None;
    }
    let provider = match d.provider.as_deref() {
        None => None,
        Some("gmail") => Some(KnownProvider::Gmail),
        Some("outlook") => Some(KnownProvider::Outlook),
        Some("icloud") => Some(KnownProvider::ICloud),
        Some("fastmail") => Some(KnownProvider::Fastmail),
        Some(_) => return None,
    };
    let auth_hint = match d.auth_hint.as_deref() {
        None | Some("password") => AuthKind::Password,
        Some("app_password") => AuthKind::AppPassword,
        Some("oauth2") => AuthKind::OAuth2,
        Some("oauth2_or_app_password") => AuthKind::OAuth2OrAppPassword,
        Some(_) => return None,
    };
    let source = match d.source.as_deref() {
        Some("provider_table") => DiscoverySource::ProviderTable,
        Some("srv") => DiscoverySource::Srv,
        Some("jmap_well_known") => DiscoverySource::JmapWellKnown,
        _ => DiscoverySource::Autoconfig,
    };
    Some(FfiDiscovered {
        email: d.email,
        imap: imap.flatten(),
        smtp: smtp.flatten(),
        jmap: d.jmap,
        source,
        provider,
        auth_hint,
    })
}

fn verify_failure(f: &VerifyFailure) -> Response {
    let (reason, extra) = match f {
        VerifyFailure::WrongPassword => ("wrong_password", json!({})),
        VerifyFailure::NeedsAppPasswordOrOAuth { provider } => (
            "needs_app_password",
            json!({ "provider": provider_str(*provider) }),
        ),
        VerifyFailure::UntrustedCertificate { fingerprint } => (
            "untrusted_certificate",
            json!({ "fingerprint": fingerprint }),
        ),
        VerifyFailure::CertificateChanged { fingerprint } => {
            ("certificate_changed", json!({ "fingerprint": fingerprint }))
        }
        VerifyFailure::ConnectionIntercepted => ("connection_intercepted", json!({})),
        VerifyFailure::CertificateInvalid => ("certificate_invalid", json!({})),
        VerifyFailure::CannotConnect => ("cannot_connect", json!({})),
        VerifyFailure::NoSecureConnection => ("no_secure_connection", json!({})),
        VerifyFailure::ServerProblem => ("server_problem", json!({})),
        VerifyFailure::JmapOnly => ("jmap_only", json!({})),
        VerifyFailure::BadSettings => ("bad_settings", json!({})),
        VerifyFailure::Internal => ("internal", json!({})),
    };
    failure("verify", reason, extra)
}

/// OAuth-only providers need a browser redirect to the Companion, which T26 leaves out.
fn oauth_only(d: &FfiDiscovered) -> bool {
    d.auth_hint == AuthKind::OAuth2
}

// ---- Routes ----

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Discover {
    email: String,
}

/// `POST /api/accounts/discover`
pub async fn discover(
    State(app): State<App>,
    MaybePeer(peer): MaybePeer,
    headers: HeaderMap,
    Full(c): Full,
    req: Result<Json<Discover>, axum::extract::rejection::JsonRejection>,
) -> ApiResult {
    let _ = (peer, headers);
    app.auth.mail_limit(c.user_id)?;
    let req = body(req)?;
    if req.email.len() > 320 {
        return Err(ApiError::BadRequest);
    }
    let core = core(&app, c.user_id).await?;
    match core.discover(req.email, None).await {
        Ok(d) => Ok(Json(discovered_json(&d)).into_response()),
        Err(e) => Ok(failure(
            "discover",
            match e {
                DiscoverError::InvalidAddress => "invalid_address",
                DiscoverError::Offline => "offline",
                DiscoverError::NothingFound => "nothing_found",
                DiscoverError::ResolverUnavailable => "resolver_unavailable",
                DiscoverError::Internal => "internal",
            },
            json!({}),
        )),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Verify {
    discovered: DiscoveredIn,
    password: String,
    /// The fingerprint the person confirmed for a self-signed server.
    pin: Option<String>,
}

/// `POST /api/accounts/verify`: checks the settings and the password without adding anything.
pub async fn verify(
    State(app): State<App>,
    MaybePeer(peer): MaybePeer,
    headers: HeaderMap,
    Full(c): Full,
    req: Result<Json<Verify>, axum::extract::rejection::JsonRejection>,
) -> ApiResult {
    let _ = (peer, headers);
    app.auth.mail_limit(c.user_id)?;
    let req = body(req)?;
    let password = Zeroizing::new(req.password);
    let d = discovered_in(req.discovered).ok_or(ApiError::BadRequest)?;
    if oauth_only(&d) {
        return Ok(failure("verify", "oauth_not_supported", json!({})));
    }
    let core = core(&app, c.user_id).await?;
    let cred = FfiCredential::password(password.as_bytes().to_vec());
    match core.verify(d, cred, req.pin).await {
        Ok(pin) => Ok(Json(json!({ "ok": true, "pin": pin })).into_response()),
        Err(f) => Ok(verify_failure(&f)),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Add {
    discovered: DiscoveredIn,
    password: String,
    display_name: Option<String>,
    /// The fingerprint `verify` returned (a server pinned on first use).
    pin: Option<String>,
    /// The new fingerprint the person confirmed after `certificate_changed`.
    confirmed_fingerprint: Option<String>,
}

/// `POST /api/accounts`: checks the password with the server, then adds the account; its
/// password is sealed once the core has it. Runs to its end even if the client goes away, so an
/// account is never left without its password (or a re-added one half updated).
pub async fn add(
    State(app): State<App>,
    MaybePeer(peer): MaybePeer,
    headers: HeaderMap,
    Full(c): Full,
    req: Result<Json<Add>, axum::extract::rejection::JsonRejection>,
) -> ApiResult {
    let _ = (peer, headers);
    app.auth.mail_limit(c.user_id)?;
    let req = body(req)?;
    let password = Zeroizing::new(req.password);
    let d = discovered_in(req.discovered).ok_or(ApiError::BadRequest)?;
    if oauth_only(&d) {
        return Ok(failure("add", "oauth_not_supported", json!({})));
    }
    let name: String = req
        .display_name
        .unwrap_or_default()
        .chars()
        .filter(|ch| !ch.is_control())
        .take(100)
        .collect();
    let core = core(&app, c.user_id).await?;
    // At most so many accounts per user (signing an existing one in again is not a new one).
    let max = app.mail.as_ref().map_or(0, |m| m.max_accounts());
    let email = d.email.clone();
    let listed = core.clone();
    let accounts = tokio::task::spawn_blocking(move || listed.list_accounts())
        .await
        .map_err(|_| ApiError::Server)?
        .map_err(|_| ApiError::Server)?;
    if accounts.len() >= max
        && !accounts
            .iter()
            .any(|a| a.email.eq_ignore_ascii_case(&email))
    {
        return Ok(failure("add", "too_many_accounts", json!({ "max": max })));
    }
    // The server must accept the password first: an account is never kept with a password
    // nobody checked (it would be replayed at every start).
    let pin = req.pin.clone().or(req.confirmed_fingerprint.clone());
    if let Err(f) = core
        .verify(
            d.clone(),
            FfiCredential::password(password.as_bytes().to_vec()),
            pin,
        )
        .await
    {
        return Ok(verify_failure(&f));
    }
    let app2 = app.clone();
    let user_id = c.user_id;
    let task = tokio::spawn(async move {
        let cred = FfiCredential::password(password.as_bytes().to_vec());
        let added = match core
            .add_account(d, name, req.pin, req.confirmed_fingerprint, cred)
            .await
        {
            Ok(a) => a,
            Err(croft_ffi::CoreError::AccountExists) => return Err(ApiError::Conflict),
            Err(croft_ffi::CoreError::CertificateChanged) => {
                return Ok(failure("add", "certificate_changed", json!({})))
            }
            Err(croft_ffi::CoreError::BadSettings | croft_ffi::CoreError::BadFingerprint) => {
                return Err(ApiError::BadRequest)
            }
            Err(_) => return Err(ApiError::Server),
        };
        let id = added.id;
        let auth = app2.auth.clone();
        let sealed = with_store(&app2, move |s| {
            s.save_mail_credential(auth.master(), user_id, id, password.as_bytes())?;
            s.audit(Some(user_id), "mail_account.add", "ok")
        })
        .await;
        let sealed = matches!(sealed, Ok(Ok(())));
        if added.updated {
            // Signing in again: the core keeps the previous settings until told the new password
            // is stored, and puts them back if it is not.
            if sealed {
                let _ = core.commit_account_update(id, added.update_token);
                let _ = core.signed_in(id).await;
            } else {
                let _ = core.rollback_account_update(id, added.update_token).await;
                return Err(ApiError::Server);
            }
        } else if !sealed {
            // A new account without its password would only ever ask for a sign-in.
            let _ = core.remove_account(id).await;
            return Err(ApiError::Server);
        }
        Ok(Json(json!({ "id": id, "updated": added.updated })).into_response())
    });
    task.await.map_err(|_| ApiError::Server)?
}

/// `GET /api/accounts`: the caller's accounts.
pub async fn list(State(app): State<App>, Full(c): Full) -> ApiResult {
    let core = core(&app, c.user_id).await?;
    let accounts = tokio::task::spawn_blocking(move || core.list_accounts())
        .await
        .map_err(|_| ApiError::Server)?
        .map_err(|_| ApiError::Server)?;
    let items: Vec<Value> = accounts
        .iter()
        .map(|a| {
            json!({
                "id": a.id,
                "label": a.label,
                "email": a.email,
                "status": match a.status {
                    AccountStatus::Ok => "ok",
                    AccountStatus::Syncing => "syncing",
                    AccountStatus::Offline => "offline",
                    AccountStatus::NeedsSignIn => "needs_sign_in",
                    AccountStatus::Error => "error",
                },
                "last_synced_ms": a.last_synced_ms,
            })
        })
        .collect();
    Ok(Json(json!({ "accounts": items })).into_response())
}

/// `DELETE /api/accounts/{id}`: removes the account, its mail and its sealed password (to the end,
/// even if the client goes away).
pub async fn remove(
    State(app): State<App>,
    Full(c): Full,
    id: Result<axum::extract::Path<i64>, axum::extract::rejection::PathRejection>,
) -> ApiResult {
    let axum::extract::Path(id) = id.map_err(|_| ApiError::NotFound)?;
    let core = core(&app, c.user_id).await?;
    let user_id = c.user_id;
    let task = tokio::spawn(async move {
        match core.remove_account(id).await {
            Ok(()) => {}
            Err(croft_ffi::CoreError::NoSuchAccount) => return Err(ApiError::NotFound),
            Err(_) => return Err(ApiError::Server),
        }
        with_store(&app, move |s| {
            s.delete_mail_credential(user_id, id)?;
            s.audit(Some(user_id), "mail_account.remove", "ok")
        })
        .await??;
        Ok(Json(json!({ "ok": true })).into_response())
    });
    task.await.map_err(|_| ApiError::Server)?
}
