// This Source Code Form is subject to the terms of the GNU Affero General Public License,
// version 3 or later. See LICENSE-AGPL-3.0 at the repository root.

//! The push relay (T28): each user's open core is watched for changes; a change sends one
//! content-free ping (`{"v":1,"type":"changed"}`, Web Push encrypted, with VAPID) to every live
//! subscription of that user, at most one per window (30 seconds) per user. The app then syncs.
//!
//! Delivery goes through the core's HTTPS client with the address policy (an endpoint is a
//! client's choice). `404`/`410` drop a subscription; other failures are counted
//! (`push_store::MAX_FAILURES`).

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError, Weak};
use std::time::Duration;

use croft_ffi::Core;

use crate::http::App;
use crate::webpush::{self, Vapid};

/// The ping's whole content.
pub const PAYLOAD: &[u8] = br#"{"v":1,"type":"changed"}"#;
/// At most one ping per user per this many milliseconds.
pub const WINDOW_MS: i64 = 30_000;
/// How long a watcher waits for a change before checking that its core is still the user's.
const CHECK_EVERY: Duration = Duration::from_secs(30);

/// The relay's state: which user's core (by epoch) is being watched.
pub struct Relay {
    window_ms: i64,
    watching: Mutex<HashMap<i64, u64>>,
    /// Tests only: endpoints over plain http on this machine (the fake push service).
    pub(crate) allow_loopback_http: bool,
}

impl std::fmt::Debug for Relay {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Relay")
    }
}

impl Default for Relay {
    fn default() -> Self {
        Self::with_window(WINDOW_MS)
    }
}

impl Relay {
    /// A relay pinging at most once per `window_ms` per user.
    pub fn with_window(window_ms: i64) -> Self {
        Self {
            window_ms,
            watching: Mutex::new(HashMap::new()),
            allow_loopback_http: false,
        }
    }

    /// Whether an endpoint may be used: `https` (plain `http` on this machine only in tests), no
    /// user name, at most 2048 characters. The address policy is applied again at delivery.
    pub fn endpoint_ok(&self, endpoint: &str) -> bool {
        let Ok(url) = url::Url::parse(endpoint) else {
            return false;
        };
        let scheme_ok = url.scheme() == "https"
            || (self.allow_loopback_http
                && url.scheme() == "http"
                && croft_protocols::oauth::fetch::is_loopback_http(endpoint));
        endpoint.len() <= 2048
            && scheme_ok
            && url.username().is_empty()
            && url.password().is_none()
            && url.host_str().is_some()
    }

    /// Starts watching the user's core (once per opening of it).
    pub fn watch(&self, app: &App, user_id: i64, core: &Arc<Core>, epoch: u64) {
        {
            let mut w = self.watching.lock().unwrap_or_else(PoisonError::into_inner);
            if w.get(&user_id) == Some(&epoch) {
                return;
            }
            w.insert(user_id, epoch);
        }
        let Ok(handle) = tokio::runtime::Handle::try_current() else {
            return;
        };
        handle.spawn(watch_loop(
            app.clone(),
            user_id,
            epoch,
            Arc::downgrade(core),
        ));
    }

    fn forget(&self, user_id: i64, epoch: u64) {
        let mut w = self.watching.lock().unwrap_or_else(PoisonError::into_inner);
        if w.get(&user_id) == Some(&epoch) {
            w.remove(&user_id);
        }
    }
}

async fn watch_loop(app: App, user_id: i64, epoch: u64, weak: Weak<Core>) {
    let mut after = weak.upgrade().map_or(0, |c| c.change_seq());
    let mut last_sent = i64::MIN / 2;
    loop {
        let Some(core) = weak.upgrade() else {
            break;
        };
        // Still the user's core? (A closed or replaced core is not watched any more.)
        let current = app.mail.as_ref().and_then(|m| m.open_core(user_id));
        if !current.is_some_and(|c| Arc::ptr_eq(&c, &core)) {
            break;
        }
        match tokio::time::timeout(CHECK_EVERY, core.next_change(after)).await {
            Err(_) => continue,
            Ok(Err(_)) => {
                drop(core);
                tokio::time::sleep(Duration::from_secs(1)).await;
                continue;
            }
            Ok(Ok(batch)) => after = batch.seq,
        }
        drop(core);
        // At most one ping per window: changes within it fold into one at its end.
        let wait = last_sent + app.relay.window_ms - crate::store::now_ms();
        if wait > 0 {
            tokio::time::sleep(Duration::from_millis(u64::try_from(wait).unwrap_or(0))).await;
            if let Some(core) = weak.upgrade() {
                after = after.max(core.change_seq());
            }
        }
        ping(&app, user_id).await;
        last_sent = crate::store::now_ms();
    }
    app.relay.forget(user_id, epoch);
}

/// Sends the ping to every live subscription of the user.
pub async fn ping(app: &App, user_id: i64) {
    let auth = app.auth.clone();
    let store = app.store.clone();
    let prepared = tokio::task::spawn_blocking(move || {
        let mut s = store.lock().unwrap_or_else(PoisonError::into_inner);
        let targets = s.push_targets(auth.master(), user_id)?;
        if targets.is_empty() {
            return Ok::<_, crate::store::StoreError>(None);
        }
        let vapid = s.vapid_key(auth.master())?;
        Ok(Some((targets, vapid)))
    })
    .await;
    let Ok(Ok(Some((targets, vapid)))) = prepared else {
        return;
    };
    let Ok(vapid) = Vapid::from_pkcs8(&vapid) else {
        return;
    };
    let now_s = crate::store::now_ms() / 1000;
    let subject = app.auth.public_url().to_owned();
    for t in targets {
        if !app.relay.endpoint_ok(&t.endpoint) {
            record(app, user_id, t.id, false, true).await;
            continue;
        }
        let (Ok(body), Ok(authorization)) = (
            webpush::encrypt(&t.p256dh, &t.auth, PAYLOAD),
            vapid.authorization(&t.endpoint, &subject, now_s),
        ) else {
            record(app, user_id, t.id, false, true).await;
            continue;
        };
        let headers = [
            ("TTL", "3600"),
            ("Urgency", "normal"),
            ("Content-Encoding", "aes128gcm"),
        ];
        let answer =
            croft_protocols::oauth::fetch::post_push(&t.endpoint, &authorization, &headers, &body)
                .await;
        let (ok, gone) = match answer {
            Ok(a) if (200..300).contains(&a.status) => (true, false),
            Ok(a) if a.status == 404 || a.status == 410 => (false, true),
            _ => (false, false),
        };
        record(app, user_id, t.id, ok, gone).await;
    }
}

async fn record(app: &App, user_id: i64, id: i64, ok: bool, gone: bool) {
    let store = app.store.clone();
    let _ = tokio::task::spawn_blocking(move || {
        store
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push_result(user_id, id, ok, gone)
    })
    .await;
}
