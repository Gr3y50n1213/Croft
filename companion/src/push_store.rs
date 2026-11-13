// This Source Code Form is subject to the terms of the GNU Affero General Public License,
// version 3 or later. See LICENSE-AGPL-3.0 at the repository root.

//! Push subscriptions and the VAPID key (T28). A subscription's endpoint and keys are sealed with
//! the user's data key; the VAPID key with the master key. Every query names the user.

use rusqlite::{params, OptionalExtension, TransactionBehavior};
use zeroize::Zeroizing;

use crate::crypto::{self, Key};
use crate::store::{now_ms, Store, StoreError};

/// Most subscriptions one user may have.
pub const MAX_SUBSCRIPTIONS: i64 = 10;
/// A subscription is dropped once its deliveries have failed this many times in a row...
pub const MAX_FAILURES: i64 = 10;
/// ...over at least this long (a push service's outage of a day or two drops nothing).
pub const FAILING_FOR_MS: i64 = 3 * 24 * 60 * 60 * 1000;
const CONTEXT: &str = "push-subscription";
const VAPID_AAD: &[u8] = b"croft-companion/vapid/v1";

/// A subscription, opened for delivery.
pub struct Subscription {
    /// Row id.
    pub id: i64,
    /// The push service's URL for this device.
    pub endpoint: Zeroizing<String>,
    /// The device's P-256 public key (65 bytes).
    pub p256dh: Vec<u8>,
    /// The device's authentication secret (16 bytes).
    pub auth: Zeroizing<Vec<u8>>,
}

impl std::fmt::Debug for Subscription {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Subscription")
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}

/// A subscription as its user sees it (no endpoint or key).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubscriptionInfo {
    /// Row id.
    pub id: i64,
    /// The device (session) that made it.
    pub session_id: i64,
    /// Made at.
    pub created_ms: i64,
}

impl Store {
    /// Adds (or, for the same endpoint, replaces) a subscription of the caller's device.
    /// [`StoreError::Conflict`] when the user already has [`MAX_SUBSCRIPTIONS`].
    pub fn add_push(
        &mut self,
        master: &Key,
        user_id: i64,
        session_id: i64,
        endpoint: &str,
        p256dh: &[u8],
        auth: &[u8],
    ) -> Result<i64, StoreError> {
        let data_key = self.data_key(master, user_id)?;
        let plain = Zeroizing::new(
            serde_json::to_vec(&serde_json::json!({
                "endpoint": endpoint,
                "p256dh": crate::webpush::b64_public(p256dh),
                "auth": crate::webpush::b64_public(auth),
            }))
            .map_err(|_| StoreError::Damaged)?,
        );
        let sealed = crypto::seal_value(&data_key, user_id, CONTEXT, &plain)?;
        let hash = endpoint_key(&data_key, endpoint);
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute(
            "DELETE FROM push_subscription WHERE user_id = ?1 AND endpoint_hash = ?2",
            params![user_id, &hash[..]],
        )?;
        let count: i64 = tx.query_row(
            "SELECT COUNT(*) FROM push_subscription WHERE user_id = ?1",
            params![user_id],
            |r| r.get(0),
        )?;
        if count >= MAX_SUBSCRIPTIONS {
            return Err(StoreError::Conflict);
        }
        // Only for a live session of the user (one signed out meanwhile gets none: `NoSuchUser`).
        let added = tx.execute(
            "INSERT INTO push_subscription(user_id, session_id, sealed, endpoint_hash, created_ms)
             SELECT ?1, ?2, ?3, ?4, ?5 FROM session
             WHERE id = ?2 AND user_id = ?1 AND revoked_ms IS NULL",
            params![user_id, session_id, sealed, &hash[..], now_ms()],
        )?;
        if added != 1 {
            return Err(StoreError::NoSuchUser);
        }
        let id = tx.last_insert_rowid();
        tx.commit()?;
        Ok(id)
    }

    /// The user's subscriptions.
    pub fn push_list(&self, user_id: i64) -> Result<Vec<SubscriptionInfo>, StoreError> {
        let mut stmt = self.conn.prepare(
            "SELECT id, session_id, created_ms FROM push_subscription WHERE user_id = ?1 ORDER BY id",
        )?;
        let rows = stmt.query_map(params![user_id], |r| {
            Ok(SubscriptionInfo {
                id: r.get(0)?,
                session_id: r.get(1)?,
                created_ms: r.get(2)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Removes one of the user's subscriptions; `false` if it is not theirs.
    pub fn delete_push(&mut self, user_id: i64, id: i64) -> Result<bool, StoreError> {
        Ok(self.conn.execute(
            "DELETE FROM push_subscription WHERE id = ?1 AND user_id = ?2",
            params![id, user_id],
        )? == 1)
    }

    /// The user's subscriptions to deliver to now: of live sessions only (not signed out, and still
    /// holding a refresh token that has not expired: a browser closed without signing out stops
    /// getting pings once its sign-in has lapsed, T32b review), and none for a disabled user. A row
    /// that does not open is skipped.
    pub fn push_targets(
        &self,
        master: &Key,
        user_id: i64,
        now: i64,
    ) -> Result<Vec<Subscription>, StoreError> {
        let rows: Vec<(i64, Vec<u8>)> = {
            let mut stmt = self.conn.prepare(
                "SELECT p.id, p.sealed FROM push_subscription p
                 JOIN session s ON s.id = p.session_id AND s.user_id = p.user_id
                 JOIN user u ON u.id = p.user_id
                 WHERE p.user_id = ?1 AND s.revoked_ms IS NULL AND u.disabled = 0
                   AND EXISTS (SELECT 1 FROM refresh_token r
                               WHERE r.session_id = s.id AND r.user_id = p.user_id
                                 AND r.rotated_ms IS NULL AND r.expires_ms > ?2)",
            )?;
            let r = stmt.query_map(params![user_id, now], |r| Ok((r.get(0)?, r.get(1)?)))?;
            r.collect::<rusqlite::Result<Vec<_>>>()?
        };
        if rows.is_empty() {
            return Ok(Vec::new());
        }
        let data_key = self.data_key(master, user_id)?;
        let mut out = Vec::new();
        for (id, sealed) in rows {
            let Ok(plain) = crypto::open_value(&data_key, user_id, CONTEXT, &sealed) else {
                continue;
            };
            let Ok(v) = serde_json::from_slice::<serde_json::Value>(&plain) else {
                continue;
            };
            let text = |k: &str| {
                v.get(k)
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_owned)
            };
            let (Some(endpoint), Some(p256dh), Some(auth)) = (
                text("endpoint"),
                text("p256dh").and_then(|k| crate::webpush::unb64(&k)),
                text("auth").and_then(|k| crate::webpush::unb64(&k)),
            ) else {
                continue;
            };
            out.push(Subscription {
                id,
                endpoint: Zeroizing::new(endpoint),
                p256dh,
                auth: Zeroizing::new(auth),
            });
        }
        Ok(out)
    }

    /// Records a delivery's outcome at `now`: a success clears the failures; a failure counts, and
    /// the subscription goes once it has failed [`MAX_FAILURES`] times in a row over at least
    /// [`FAILING_FOR_MS`] (or at once when `gone`).
    pub fn push_result(
        &mut self,
        user_id: i64,
        id: i64,
        ok: bool,
        gone: bool,
        now: i64,
    ) -> Result<(), StoreError> {
        if gone {
            self.delete_push(user_id, id)?;
        } else if ok {
            self.conn.execute(
                "UPDATE push_subscription SET failures = 0, failing_since_ms = NULL
                 WHERE id = ?1 AND user_id = ?2",
                params![id, user_id],
            )?;
        } else {
            self.conn.execute(
                "UPDATE push_subscription
                 SET failures = failures + 1, failing_since_ms = COALESCE(failing_since_ms, ?3)
                 WHERE id = ?1 AND user_id = ?2",
                params![id, user_id, now],
            )?;
            self.conn.execute(
                "DELETE FROM push_subscription
                 WHERE id = ?1 AND user_id = ?2 AND failures >= ?3 AND failing_since_ms <= ?4",
                params![id, user_id, MAX_FAILURES, now - FAILING_FOR_MS],
            )?;
        }
        Ok(())
    }

    /// The Companion's VAPID key (PKCS#8), made and kept (sealed with the master key) at first use.
    pub fn vapid_key(&mut self, master: &Key) -> Result<Zeroizing<Vec<u8>>, StoreError> {
        let stored: Option<Vec<u8>> = self
            .conn
            .query_row("SELECT value FROM meta WHERE key = 'vapid_key'", [], |r| {
                r.get(0)
            })
            .optional()?;
        if let Some(sealed) = stored {
            return Ok(master.open(VAPID_AAD, &sealed)?);
        }
        let pkcs8 = crate::webpush::Vapid::generate_pkcs8()
            .map_err(|_| StoreError::Crypto(crypto::CryptoError::Random))?;
        let sealed = master.seal(VAPID_AAD, &pkcs8)?;
        self.conn.execute(
            "INSERT OR IGNORE INTO meta(key, value) VALUES ('vapid_key', ?1)",
            params![sealed],
        )?;
        // Another call may have stored one first: use whichever is stored.
        let stored: Vec<u8> =
            self.conn
                .query_row("SELECT value FROM meta WHERE key = 'vapid_key'", [], |r| {
                    r.get(0)
                })?;
        Ok(master.open(VAPID_AAD, &stored)?)
    }
}

/// The key that finds a user's subscription by endpoint: an HMAC under the user's data key, so the
/// database alone does not tell whether two users share a push service endpoint, or which it is.
fn endpoint_key(data_key: &Key, endpoint: &str) -> Vec<u8> {
    let key = data_key.derive(b"croft-companion/push-endpoint/v1");
    let key = ring::hmac::Key::new(ring::hmac::HMAC_SHA256, &key[..]);
    ring::hmac::sign(&key, endpoint.as_bytes())
        .as_ref()
        .to_vec()
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::store::Role;

    #[test]
    fn subscriptions_are_sealed_per_user_capped_and_follow_their_device() {
        let master = Key::generate().unwrap();
        let mut s = Store::in_memory().unwrap();
        s.check_master_key(&master).unwrap();
        let ann = s.add_user(&master, "ann", Role::User).unwrap().id;
        let bob = s.add_user(&master, "bob", Role::User).unwrap().id;
        let t = s.start_session(ann, "phone", false, now_ms()).unwrap();
        let ann_session = s.sessions(ann).unwrap()[0].id;
        let _ = t;
        let key = [4u8; 65];
        let id = s
            .add_push(
                &master,
                ann,
                ann_session,
                "https://push.example/made-up-1",
                &key,
                &[1; 16],
            )
            .unwrap();
        // The endpoint is not in the clear.
        let raw: Vec<u8> = s
            .conn
            .query_row(
                "SELECT sealed FROM push_subscription WHERE id = ?1",
                params![id],
                |r| r.get(0),
            )
            .unwrap();
        assert!(!raw.windows(9).any(|w| w == b"made-up-1"));
        let targets = s.push_targets(&master, ann, now_ms()).unwrap();
        assert_eq!(targets.len(), 1);
        assert_eq!(
            targets[0].endpoint.as_str(),
            "https://push.example/made-up-1"
        );
        assert_eq!(targets[0].auth.as_slice(), &[1; 16]);
        assert!(s.push_targets(&master, bob, now_ms()).unwrap().is_empty());
        assert!(!s.delete_push(bob, id).unwrap(), "not Bob's");
        // The same endpoint replaces; the cap holds.
        s.add_push(
            &master,
            ann,
            ann_session,
            "https://push.example/made-up-1",
            &key,
            &[2; 16],
        )
        .unwrap();
        assert_eq!(s.push_list(ann).unwrap().len(), 1);
        for i in 2..=MAX_SUBSCRIPTIONS {
            s.add_push(
                &master,
                ann,
                ann_session,
                &format!("https://push.example/{i}"),
                &key,
                &[1; 16],
            )
            .unwrap();
        }
        assert!(matches!(
            s.add_push(
                &master,
                ann,
                ann_session,
                "https://push.example/over",
                &key,
                &[1; 16]
            ),
            Err(StoreError::Conflict)
        ));
        // A device whose sign-in lapsed (closed without signing out, its refresh token expired)
        // gets no ping, though its subscription is still listed until it is signed out.
        s.conn
            .execute(
                "UPDATE refresh_token SET expires_ms = ?1 WHERE session_id = ?2",
                params![now_ms() - 1, ann_session],
            )
            .unwrap();
        assert!(s.push_targets(&master, ann, now_ms()).unwrap().is_empty());
        assert!(!s.push_list(ann).unwrap().is_empty());
        s.conn
            .execute(
                "UPDATE refresh_token SET expires_ms = ?1 WHERE session_id = ?2",
                params![now_ms() + 60_000, ann_session],
            )
            .unwrap();
        assert!(!s.push_targets(&master, ann, now_ms()).unwrap().is_empty());
        // Signing the device out removes its subscriptions (they no longer fill the cap), and a
        // signed-out device cannot add one.
        s.revoke_session(ann, ann_session, now_ms()).unwrap();
        assert!(s.push_targets(&master, ann, now_ms()).unwrap().is_empty());
        assert!(s.push_list(ann).unwrap().is_empty());
        assert!(matches!(
            s.add_push(
                &master,
                ann,
                ann_session,
                "https://push.example/late",
                &key,
                &[1; 16]
            ),
            Err(StoreError::NoSuchUser)
        ));
        s.start_session(ann, "laptop", false, now_ms()).unwrap();
        let laptop = s.sessions(ann).unwrap()[0].id;
        for i in 1..=MAX_SUBSCRIPTIONS {
            s.add_push(
                &master,
                ann,
                laptop,
                &format!("https://push.example/{i}"),
                &key,
                &[1; 16],
            )
            .unwrap();
        }
        // Signing out everywhere (or a refresh token's reuse) removes them too.
        s.conn
            .execute(
                "UPDATE session SET revoked_ms = ?2 WHERE user_id = ?1 AND revoked_ms IS NULL",
                params![ann, now_ms()],
            )
            .unwrap();
        assert!(s.push_list(ann).unwrap().is_empty());
    }

    #[test]
    fn the_endpoint_key_is_per_user_and_not_a_plain_hash() {
        let master = Key::generate().unwrap();
        let mut s = Store::in_memory().unwrap();
        s.check_master_key(&master).unwrap();
        let ann = s.add_user(&master, "ann", Role::User).unwrap().id;
        let bob = s.add_user(&master, "bob", Role::User).unwrap().id;
        let a = endpoint_key(&s.data_key(&master, ann).unwrap(), "https://push.example/x");
        let b = endpoint_key(&s.data_key(&master, bob).unwrap(), "https://push.example/x");
        assert_ne!(a, b);
        assert_ne!(
            a,
            crate::auth::token_hash("https://push.example/x").to_vec()
        );
    }

    #[test]
    fn failures_drop_a_subscription_and_the_vapid_key_is_kept() {
        let master = Key::generate().unwrap();
        let mut s = Store::in_memory().unwrap();
        s.check_master_key(&master).unwrap();
        let ann = s.add_user(&master, "ann", Role::User).unwrap().id;
        s.start_session(ann, "phone", false, now_ms()).unwrap();
        let session = s.sessions(ann).unwrap()[0].id;
        let id = s
            .add_push(
                &master,
                ann,
                session,
                "https://push.example/a",
                &[4; 65],
                &[1; 16],
            )
            .unwrap();
        let t0 = now_ms();
        for _ in 0..MAX_FAILURES - 1 {
            s.push_result(ann, id, false, false, t0).unwrap();
        }
        s.push_result(ann, id, true, false, t0).unwrap();
        for _ in 0..MAX_FAILURES * 3 {
            s.push_result(ann, id, false, false, t0).unwrap();
        }
        // Many failures in a short outage drop nothing.
        assert_eq!(s.push_list(ann).unwrap().len(), 1);
        s.push_result(ann, id, false, false, t0 + FAILING_FOR_MS - 1)
            .unwrap();
        assert_eq!(s.push_list(ann).unwrap().len(), 1);
        // A success resets the clock.
        s.push_result(ann, id, true, false, t0 + FAILING_FOR_MS)
            .unwrap();
        s.push_result(ann, id, false, false, t0 + FAILING_FOR_MS + 1)
            .unwrap();
        s.push_result(ann, id, false, false, t0 + 2 * FAILING_FOR_MS)
            .unwrap();
        assert_eq!(s.push_list(ann).unwrap().len(), 1, "too few failures");
        for _ in 0..MAX_FAILURES {
            s.push_result(ann, id, false, false, t0 + 2 * FAILING_FOR_MS + 1)
                .unwrap();
        }
        assert!(s.push_list(ann).unwrap().is_empty());
        let a = s.vapid_key(&master).unwrap();
        assert_eq!(*a, *s.vapid_key(&master).unwrap());
        assert!(s.vapid_key(&Key::generate().unwrap()).is_err());
    }
}
