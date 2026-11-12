// This Source Code Form is subject to the terms of the GNU Affero General Public License,
// version 3 or later. See LICENSE-AGPL-3.0 at the repository root.

//! The push subscription routes (T28), a part of [`super`] (full sessions only). A subscription
//! belongs to the caller and to the device (session) that made it.

use axum::extract::State;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

use super::{body, with_store, ApiError, ApiResult, Full};
use crate::http::App;
use crate::store::StoreError;

/// `GET /api/push/vapid`: the Companion's VAPID public key (a browser's `applicationServerKey`).
pub async fn vapid(State(app): State<App>, Full(_): Full) -> ApiResult {
    let auth = app.auth.clone();
    let pkcs8 = with_store(&app, move |s| s.vapid_key(auth.master())).await??;
    let key = crate::webpush::Vapid::from_pkcs8(&pkcs8).map_err(|_| ApiError::Server)?;
    Ok(Json(json!({ "key": key.public_key() })).into_response())
}

#[derive(Deserialize)]
pub struct Keys {
    p256dh: String,
    auth: String,
}

/// A subscription as `PushSubscription.toJSON()` and UnifiedPush give it.
#[derive(Deserialize)]
pub struct Subscribe {
    endpoint: String,
    keys: Keys,
    #[serde(rename = "expirationTime")]
    _expiration: Option<Value>,
}

/// `POST /api/push/subscriptions`
pub async fn subscribe(
    State(app): State<App>,
    Full(c): Full,
    req: Result<Json<Subscribe>, axum::extract::rejection::JsonRejection>,
) -> ApiResult {
    app.auth.jmap_limit(c.user_id)?;
    let req = body(req)?;
    if !app.relay.endpoint_ok(&req.endpoint) {
        return Err(ApiError::BadRequest);
    }
    let p256dh = crate::webpush::unb64(&req.keys.p256dh)
        .filter(|k| k.len() == 65 && k[0] == 4)
        .ok_or(ApiError::BadRequest)?;
    let auth_secret = crate::webpush::unb64(&req.keys.auth)
        .filter(|a| a.len() == 16)
        .ok_or(ApiError::BadRequest)?;
    // The key must be a point on the curve: a message to it must be possible.
    if crate::webpush::encrypt(&p256dh, &auth_secret, b"").is_err() {
        return Err(ApiError::BadRequest);
    }
    let (user_id, session_id) = (c.user_id, c.session_id);
    let auth = app.auth.clone();
    let id = with_store(&app, move |s| {
        let id = s.add_push(
            auth.master(),
            user_id,
            session_id,
            &req.endpoint,
            &p256dh,
            &auth_secret,
        )?;
        s.audit(Some(user_id), "push.subscribe", "ok")?;
        Ok::<_, StoreError>(id)
    })
    .await??;
    // Pings come from the user's core: make sure it is open and watched.
    let _ = app.user_core(user_id).await;
    Ok(Json(json!({ "id": id })).into_response())
}

/// `GET /api/push/subscriptions`: the caller's subscriptions (no endpoint or key).
pub async fn list(State(app): State<App>, Full(c): Full) -> ApiResult {
    let user_id = c.user_id;
    let subs = with_store(&app, move |s| s.push_list(user_id)).await??;
    let items: Vec<Value> = subs
        .iter()
        .map(|s| json!({ "id": s.id, "created_ms": s.created_ms, "current_device": s.session_id == c.session_id }))
        .collect();
    Ok(Json(json!({ "subscriptions": items })).into_response())
}

/// `DELETE /api/push/subscriptions/{id}`
pub async fn remove(
    State(app): State<App>,
    Full(c): Full,
    id: Result<axum::extract::Path<i64>, axum::extract::rejection::PathRejection>,
) -> Result<Response, ApiError> {
    let axum::extract::Path(id) = id.map_err(|_| ApiError::NotFound)?;
    let user_id = c.user_id;
    if with_store(&app, move |s| s.delete_push(user_id, id)).await?? {
        Ok(Json(json!({ "ok": true })).into_response())
    } else {
        Err(ApiError::NotFound)
    }
}
