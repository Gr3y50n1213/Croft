// This Source Code Form is subject to the terms of the GNU Affero General Public License,
// version 3 or later. See LICENSE-AGPL-3.0 at the repository root.

//! WebAuthn ceremonies in progress (T24): the server-side half of each challenge, kept in memory
//! under a random id for five minutes and handed out once. Taking it removes it, so a replayed
//! answer finds nothing. A restart drops them all (the person just tries again). The number kept is
//! capped, so a flood of "start" requests cannot grow memory without bound.

use std::collections::HashMap;

use webauthn_rs::prelude::{DiscoverableAuthentication, PasskeyRegistration};
use zeroize::Zeroizing;

use crate::auth::{random_token, token_hash};
use crate::store::StoreError;

/// How long a ceremony waits for its answer.
pub const TTL_MS: i64 = 5 * 60 * 1000;
/// Ceremonies kept at once.
pub const MAX: usize = 10_000;

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

/// The ceremonies in progress.
#[derive(Default)]
pub struct Ceremonies {
    map: HashMap<[u8; 32], (i64, Pending)>,
}

impl Ceremonies {
    /// Keeps `pending` and returns its id. `None` when too many are waiting.
    pub fn put(
        &mut self,
        pending: Pending,
        now: i64,
    ) -> Result<Option<Zeroizing<String>>, StoreError> {
        self.map.retain(|_, (expires, _)| *expires > now);
        if self.map.len() >= MAX {
            return Ok(None);
        }
        let id = random_token()?;
        self.map.insert(token_hash(&id), (now + TTL_MS, pending));
        Ok(Some(id))
    }

    /// Hands out the ceremony with this id, once, if it has not expired.
    pub fn take(&mut self, id: &str, now: i64) -> Option<Pending> {
        let (expires, pending) = self.map.remove(&token_hash(id))?;
        (expires > now).then_some(pending)
    }

    /// How many are waiting (tests).
    pub fn len(&self) -> usize {
        self.map.len()
    }

    /// Whether none is waiting.
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }
}
