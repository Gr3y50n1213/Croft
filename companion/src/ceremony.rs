// This Source Code Form is subject to the terms of the GNU Affero General Public License,
// version 3 or later. See LICENSE-AGPL-3.0 at the repository root.

//! WebAuthn ceremonies in progress (T24): the server-side half of each challenge, kept in memory
//! under a random id for five minutes and handed out once. Taking it removes it, so a replayed
//! answer finds nothing. A restart drops them all (the person just tries again).
//!
//! Memory is bounded and nobody can lock others out by filling it: each owner (a client address,
//! or a signed-in user) keeps at most [`PER_OWNER`] ceremonies, its oldest giving way to its
//! newest; when the whole table is full, the oldest ceremony anywhere gives way. Nothing is ever
//! refused for lack of room.

use std::collections::{HashMap, VecDeque};

use webauthn_rs::prelude::{DiscoverableAuthentication, PasskeyRegistration};
use zeroize::Zeroizing;

use crate::auth::{random_token, token_hash};
use crate::store::StoreError;

/// How long a ceremony waits for its answer.
pub const TTL_MS: i64 = 5 * 60 * 1000;
/// Ceremonies kept at once.
pub const MAX: usize = 50_000;
/// Ceremonies kept at once for one owner.
pub const PER_OWNER: usize = 8;

/// Whose ceremony it is, for [`PER_OWNER`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Owner {
    /// A client address (as the rate limits count it).
    Address(u128),
    /// A signed-in user.
    User(i64),
}

/// The server's half of a ceremony.
pub enum Pending {
    /// Registering the first passkey from an enrolment link.
    Enrol {
        /// The user the link is for.
        user_id: i64,
        /// The link's hash: the finish must bring the same link.
        link_hash: [u8; 32],
        /// WebAuthn state.
        state: PasskeyRegistration,
    },
    /// A signed-in user registering another passkey.
    AddPasskey {
        /// The user.
        user_id: i64,
        /// Their session: the finish must come from the same one.
        session_id: i64,
        /// WebAuthn state.
        state: PasskeyRegistration,
    },
    /// Signing in: a named user (`Some`), or whoever a discoverable passkey names (`None`). The
    /// answer is checked against the passkey as stored when it arrives, so its counter is current.
    SignIn {
        /// The user named at the start, if any.
        user_id: Option<i64>,
        /// WebAuthn state (the challenge).
        state: DiscoverableAuthentication,
    },
}

/// The ceremonies in progress (WebAuthn ones by default; T25 keeps its OIDC flows and hand-over
/// codes in the same kind of table).
pub struct Ceremonies<T = Pending> {
    map: HashMap<[u8; 32], (i64, Owner, T)>,
    /// Ids in the order they were made (ids already taken are skipped when reached).
    order: VecDeque<[u8; 32]>,
    /// Each owner's live ids, oldest first.
    by_owner: HashMap<Owner, VecDeque<[u8; 32]>>,
    /// How long one lasts.
    ttl_ms: i64,
}

impl<T> Default for Ceremonies<T> {
    fn default() -> Self {
        Self::with_ttl(TTL_MS)
    }
}

impl<T> Ceremonies<T> {
    /// An empty table whose entries last `ttl_ms`.
    pub fn with_ttl(ttl_ms: i64) -> Self {
        Self {
            map: HashMap::new(),
            order: VecDeque::new(),
            by_owner: HashMap::new(),
            ttl_ms,
        }
    }

    fn remove(&mut self, key: &[u8; 32]) -> Option<(i64, T)> {
        let (expires, owner, pending) = self.map.remove(key)?;
        if let Some(q) = self.by_owner.get_mut(&owner) {
            q.retain(|k| k != key);
            if q.is_empty() {
                self.by_owner.remove(&owner);
            }
        }
        Some((expires, pending))
    }

    fn make_room(&mut self, owner: Owner, now: i64) {
        // Expired ones leave from the front (the oldest).
        while let Some(front) = self.order.front().copied() {
            match self.map.get(&front) {
                Some((expires, _, _)) if *expires > now && self.map.len() < MAX => break,
                Some(_) => {
                    self.remove(&front);
                    self.order.pop_front();
                }
                None => {
                    self.order.pop_front();
                }
            }
        }
        let oldest_of_owner = self
            .by_owner
            .get(&owner)
            .filter(|q| q.len() >= PER_OWNER)
            .and_then(|q| q.front().copied());
        if let Some(key) = oldest_of_owner {
            self.remove(&key);
        }
    }

    /// Keeps `pending` for `owner` and returns its id.
    pub fn put(
        &mut self,
        owner: Owner,
        pending: T,
        now: i64,
    ) -> Result<Zeroizing<String>, StoreError> {
        self.make_room(owner, now);
        let id = random_token()?;
        let key = token_hash(&id);
        self.map.insert(key, (now + self.ttl_ms, owner, pending));
        self.order.push_back(key);
        self.by_owner.entry(owner).or_default().push_back(key);
        Ok(id)
    }

    /// Keeps `pending` for `owner` under an id the caller chose (a random value it made, such as
    /// an OIDC `state`).
    pub fn put_with_id(&mut self, owner: Owner, id: &str, pending: T, now: i64) {
        self.make_room(owner, now);
        let key = token_hash(id);
        self.map.insert(key, (now + self.ttl_ms, owner, pending));
        self.order.push_back(key);
        self.by_owner.entry(owner).or_default().push_back(key);
    }

    /// Hands out the ceremony with this id, once, if it has not expired.
    pub fn take(&mut self, id: &str, now: i64) -> Option<T> {
        let (expires, pending) = self.remove(&token_hash(id))?;
        (expires > now).then_some(pending)
    }

    /// How many are waiting.
    pub fn len(&self) -> usize {
        self.map.len()
    }

    /// Whether none is waiting.
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn pending() -> Pending {
        let w = webauthn_rs::prelude::WebauthnBuilder::new(
            "companion.test",
            &url::Url::parse("https://companion.test").unwrap(),
        )
        .unwrap()
        .build()
        .unwrap();
        let (_, state) = w.start_discoverable_authentication().unwrap();
        Pending::SignIn {
            user_id: None,
            state,
        }
    }

    #[test]
    fn one_owner_keeps_only_its_newest_and_never_crowds_out_another() {
        let mut c = Ceremonies::default();
        let other = c.put(Owner::User(2), pending(), 0).unwrap();
        let mut ids = Vec::new();
        for _ in 0..1000 {
            ids.push(c.put(Owner::Address(1), pending(), 0).unwrap());
        }
        assert_eq!(c.len(), PER_OWNER + 1);
        assert!(c.take(&ids[0], 1).is_none(), "the oldest gave way");
        assert!(c.take(&ids[999], 1).is_some());
        assert!(c.take(&other, 1).is_some(), "another owner is untouched");
        assert!(c.take(&other, 1).is_none(), "once only");
    }

    #[test]
    fn expired_ceremonies_are_dropped_and_never_handed_out() {
        let mut c = Ceremonies::default();
        let id = c.put(Owner::User(1), pending(), 0).unwrap();
        assert!(c.take(&id, TTL_MS).is_none());
        let _ = c.put(Owner::User(1), pending(), 0).unwrap();
        let _ = c.put(Owner::User(2), pending(), TTL_MS + 1).unwrap();
        assert_eq!(c.len(), 1);
        assert!(!c.by_owner.contains_key(&Owner::User(1)));
    }
}
