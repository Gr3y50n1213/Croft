// This Source Code Form is subject to the terms of the GNU Affero General Public License,
// version 3 or later. See LICENSE-AGPL-3.0 at the repository root.

//! The push relay (T28): each user's open core is watched for changes; a change sends one
//! content-free ping (`{"v":1,"type":"changed"}`, Web Push encrypted, with VAPID) to every live
//! subscription of that user, at most one per window (30 seconds) per user. The app then syncs.
//!
//! Delivery goes through the core's HTTPS client to public addresses only (an endpoint is a
//! client's choice; `mail_allowed_networks` does not apply). `404`/`410` drop a subscription; other
//! refusals (and an endpoint that now leads to an internal address) count
//! (`push_store::MAX_FAILURES` over `FAILING_FOR_MS`); an unreachable service, a
//! `5xx` or a `429` counts nothing (an outage of the push service is not the subscription's fault).

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

    /// Whether an endpoint may be used: `https` as the delivering client reads it (plain `http` on
    /// this machine only in tests), no user name, at most 2048 characters. Where it may connect is
    /// checked again at delivery.
    pub fn endpoint_ok(&self, endpoint: &str) -> bool {
        use croft_protocols::oauth::fetch;
        // Both parsers must agree: the standard one for the user name, the delivering one for what
        // is sent.
        let Ok(url) = url::Url::parse(endpoint) else {
            return false;
        };
        let reachable = fetch::is_push_url(endpoint)
            || (self.allow_loopback_http && fetch::is_loopback_http(endpoint));
        endpoint.len() <= 2048
            && reachable
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
            self.forget(user_id, epoch);
            return;
        };
        // Changes from here on are watched (read now, not when the task first runs).
        let after = core.change_seq();
        handle.spawn(watch_loop(
            app.clone(),
            user_id,
            epoch,
            after,
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

async fn watch_loop(app: App, user_id: i64, epoch: u64, mut after: u64, weak: Weak<Core>) {
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
        let mut closing = match app.mail.as_ref() {
            Some(m) => m.closing(),
            None => break,
        };
        closing.mark_unchanged();
        let changed = tokio::select! {
            r = tokio::time::timeout(CHECK_EVERY, core.next_change(after)) => Some(r),
            // A core is being closed: let this one go at once (it may be the one).
            _ = closing.changed() => None,
        };
        let Some(changed) = changed else {
            drop(core);
            continue;
        };
        match changed {
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

/// What a delivery's answer means for the subscription.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Outcome {
    Delivered,
    /// The push service says the subscription is no more (`404`, `410`).
    Gone,
    /// The push service refused this message (another `4xx`).
    Refused,
    /// Nothing is learnt about the subscription: unreachable, `5xx`, `429`, `408`.
    Unknown,
}

fn outcome(
    answer: &Result<
        croft_protocols::oauth::fetch::Answer,
        croft_protocols::oauth::fetch::FetchError,
    >,
) -> Outcome {
    match answer {
        Ok(a) if (200..300).contains(&a.status) => Outcome::Delivered,
        Ok(a) if a.status == 404 || a.status == 410 => Outcome::Gone,
        Ok(a) if a.status == 408 || a.status == 429 => Outcome::Unknown,
        Ok(a) if (400..500).contains(&a.status) => Outcome::Refused,
        // Never a push URL, or one that now leads somewhere it may not go.
        Err(croft_protocols::oauth::fetch::FetchError::BadUrl) => Outcome::Gone,
        Err(croft_protocols::oauth::fetch::FetchError::Refused) => Outcome::Refused,
        _ => Outcome::Unknown,
    }
}

/// Sends the ping to every live subscription of the user, to all of them at once.
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
    let vapid = Arc::new(vapid);
    let now_s = crate::store::now_ms() / 1000;
    let subject: Arc<str> = Arc::from(app.auth.public_url());
    let mut sends = tokio::task::JoinSet::new();
    for t in targets {
        let (app, vapid, subject) = (app.clone(), vapid.clone(), subject.clone());
        sends.spawn(async move {
            let result = deliver(&app, &vapid, &subject, now_s, &t).await;
            (t.id, result)
        });
    }
    let mut results = Vec::new();
    while let Some(r) = sends.join_next().await {
        if let Ok(r) = r {
            results.push(r);
        }
    }
    let store = app.store.clone();
    let now = crate::store::now_ms();
    let _ = tokio::task::spawn_blocking(move || {
        let mut s = store.lock().unwrap_or_else(PoisonError::into_inner);
        for (id, result) in results {
            let _ = match result {
                Outcome::Delivered => s.push_result(user_id, id, true, false, now),
                Outcome::Gone => s.push_result(user_id, id, false, true, now),
                Outcome::Refused => s.push_result(user_id, id, false, false, now),
                Outcome::Unknown => Ok(()),
            };
        }
    })
    .await;
}

async fn deliver(
    app: &App,
    vapid: &Vapid,
    subject: &str,
    now_s: i64,
    t: &crate::push_store::Subscription,
) -> Outcome {
    use croft_protocols::oauth::fetch;
    if !app.relay.endpoint_ok(&t.endpoint) {
        return Outcome::Gone;
    }
    let (Ok(body), Ok(authorization)) = (
        webpush::encrypt(&t.p256dh, &t.auth, PAYLOAD),
        vapid.authorization(&t.endpoint, subject, now_s),
    ) else {
        return Outcome::Gone;
    };
    let headers = [
        ("TTL", "3600"),
        ("Urgency", "normal"),
        ("Content-Encoding", "aes128gcm"),
    ];
    let answer = if app.relay.allow_loopback_http && fetch::is_loopback_http(&t.endpoint) {
        fetch::post_push_to_loopback_for_tests(&t.endpoint, &authorization, &headers, &body).await
    } else {
        fetch::post_push(&t.endpoint, &authorization, &headers, &body).await
    };
    outcome(&answer)
}

#[cfg(test)]
mod tests {
    use super::*;
    use croft_protocols::oauth::fetch::{Answer, FetchError};

    #[test]
    fn only_a_refusal_counts_against_a_subscription() {
        let status = |status| {
            Ok::<_, FetchError>(Answer {
                status,
                body: Vec::new().into(),
            })
        };
        assert_eq!(outcome(&status(201)), Outcome::Delivered);
        assert_eq!(outcome(&status(410)), Outcome::Gone);
        assert_eq!(outcome(&status(404)), Outcome::Gone);
        assert_eq!(outcome(&status(403)), Outcome::Refused);
        assert_eq!(outcome(&status(413)), Outcome::Refused);
        assert_eq!(outcome(&status(429)), Outcome::Unknown);
        assert_eq!(outcome(&status(503)), Outcome::Unknown);
        assert_eq!(outcome(&Err(FetchError::Unreachable)), Outcome::Unknown);
        assert_eq!(outcome(&Err(FetchError::Refused)), Outcome::Refused);
        assert_eq!(outcome(&Err(FetchError::BadUrl)), Outcome::Gone);
    }
}
