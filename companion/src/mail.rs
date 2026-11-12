// This Source Code Form is subject to the terms of the GNU Affero General Public License,
// version 3 or later. See LICENSE-AGPL-3.0 at the repository root.

//! Users' mail (T26): one core (`croft_ffi::Core`, the same one the phones run) per user, over
//! that user's own SQLCipher database at `data_dir/users/<id>/mail.db`, whose key is derived from
//! the user's data key and never stored. The core asks for account passwords through
//! [`SealedCredentials`], which opens them from the Companion's database with the same data key.
//!
//! A core has a small runtime of its own; it is opened when the user first needs it (and at start
//! for users who have accounts) and kept while the server runs. It is never dropped on an async
//! thread (a runtime cannot be dropped inside another).

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};

use croft_ffi::{Core, CredentialProvider, FfiCredential, ProviderError};

use crate::crypto::Key;
use crate::store::{Store, StoreError};

/// The purpose a user's mail database key is derived for.
pub(crate) const MAIL_DB_KEY: &[u8] = b"croft-companion/mail-db/v1";

/// Why a user's mail could not be opened.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MailError {
    /// The user's data key did not open (the wrong master key, or damaged data).
    Key,
    /// The folder or the database could not be opened.
    Open,
    /// The user is disabled.
    Disabled,
}

impl From<StoreError> for MailError {
    fn from(_: StoreError) -> Self {
        Self::Key
    }
}

/// Gives the core a user's mail passwords, opened with that user's data key. Nothing else: it
/// cannot reach another user's rows (every call names this user).
pub struct SealedCredentials {
    store: Arc<Mutex<Store>>,
    master: Key,
    user_id: i64,
}

impl SealedCredentials {
    fn store(&self) -> std::sync::MutexGuard<'_, Store> {
        self.store.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl CredentialProvider for SealedCredentials {
    fn credential(&self, account_id: i64) -> Result<Option<Arc<FfiCredential>>, ProviderError> {
        let store = self.store();
        // A password the server refused is not replayed (at every start, it would be a failed
        // login each time); the account waits for the person to give a new one.
        if store
            .mail_refused(self.user_id, account_id)
            .map_err(|_| ProviderError::Failed)?
        {
            return Ok(None);
        }
        match store.mail_credential(&self.master, self.user_id, account_id) {
            Ok(Some(secret)) => Ok(Some(FfiCredential::password(secret.to_vec()))),
            Ok(None) => Ok(None),
            Err(_) => Err(ProviderError::Failed),
        }
    }

    fn sign_in_needed(&self, account_id: i64) -> Result<(), ProviderError> {
        self.store()
            .mail_needs_sign_in(self.user_id, account_id)
            .map_err(|_| ProviderError::Failed)
    }

    fn save_tokens(&self, account_id: i64, secret: Vec<u8>) -> Result<(), ProviderError> {
        let secret = zeroize::Zeroizing::new(secret);
        self.store()
            .save_mail_credential(&self.master, self.user_id, account_id, &secret)
            .map_err(|_| ProviderError::Failed)
    }
}

/// The most accounts one user may have.
pub const MAX_ACCOUNTS_PER_USER: usize = 20;

/// The address policy the Companion sets at start: nothing internal (`netpolicy::is_internal`)
/// unless it is in one of `allowed` (the admin's `mail_allowed_networks`).
pub fn address_policy(
    allowed: Vec<crate::config::Network>,
) -> croft_protocols::netpolicy::AddressPolicy {
    Arc::new(move |ip| {
        !croft_protocols::netpolicy::is_internal(ip) || allowed.iter().any(|n| n.contains(ip))
    })
}

/// One user's slot: its core, opened at most once at a time (the slot's lock is held while it
/// opens, so two first uses at once never open two engines on one database).
type Slot = Arc<Mutex<Option<Arc<Core>>>>;

/// The open cores, one per user.
pub struct MailCores {
    data_dir: PathBuf,
    slots: Mutex<HashMap<i64, Slot>>,
    max_accounts: usize,
}

impl std::fmt::Debug for MailCores {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("MailCores")
    }
}

impl MailCores {
    /// The users' mail under `data_dir`.
    pub fn new(data_dir: PathBuf) -> Self {
        Self {
            data_dir,
            slots: Mutex::new(HashMap::new()),
            max_accounts: MAX_ACCOUNTS_PER_USER,
        }
    }

    /// The same, with another limit on accounts per user (tests).
    pub fn with_max_accounts(mut self, max: usize) -> Self {
        self.max_accounts = max;
        self
    }

    /// The most accounts one user may have.
    pub fn max_accounts(&self) -> usize {
        self.max_accounts
    }

    fn slot(&self, user_id: i64) -> Slot {
        self.slots
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .entry(user_id)
            .or_default()
            .clone()
    }

    /// The user's mail database file.
    pub fn database(&self, user_id: i64) -> PathBuf {
        self.data_dir
            .join("users")
            .join(user_id.to_string())
            .join("mail.db")
    }

    /// The user's core, opened if needed (never for a disabled user). Blocking: call it off the
    /// async threads.
    pub fn core(
        &self,
        store: &Arc<Mutex<Store>>,
        master: &Key,
        user_id: i64,
    ) -> Result<Arc<Core>, MailError> {
        let slot = self.slot(user_id);
        let mut held = slot.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(core) = held.as_ref() {
            return Ok(core.clone());
        }
        let data_key = {
            let s = store.lock().unwrap_or_else(PoisonError::into_inner);
            // Checked under the slot's lock, so a user disabled meanwhile is not reopened.
            if s.user_by_id(user_id)?.is_none_or(|u| u.disabled) {
                return Err(MailError::Disabled);
            }
            s.data_key(master, user_id)?
        };
        let db_key = data_key.derive(MAIL_DB_KEY);
        let path = self.database(user_id);
        if let Some(dir) = path.parent() {
            crate::config::create_private_dir(dir).map_err(|_| MailError::Open)?;
        }
        let provider = Arc::new(SealedCredentials {
            store: store.clone(),
            master: master.clone(),
            user_id,
        });
        let core = Core::open(
            path.to_string_lossy().into_owned(),
            Some(db_key.to_vec()),
            provider,
        )
        .map_err(|_| MailError::Open)?;
        *held = Some(core.clone());
        Ok(core)
    }

    /// The user's core if it is open (no opening).
    pub fn open_core(&self, user_id: i64) -> Option<Arc<Core>> {
        self.slot(user_id)
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Stops the user's core (a disabled user). Blocking: call it off the async threads.
    pub fn close(&self, user_id: i64) {
        let slot = self.slot(user_id);
        let core = slot.lock().unwrap_or_else(PoisonError::into_inner).take();
        drop(core);
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::config::Network;

    #[test]
    fn the_companions_policy_refuses_its_network_except_the_allowed_ones() {
        let p = address_policy(vec![Network::parse("192.168.1.0/24").unwrap()]);
        let ip = |s: &str| s.parse::<std::net::IpAddr>().unwrap();
        assert!(!p(ip("127.0.0.1")));
        assert!(!p(ip("10.0.0.1")));
        assert!(!p(ip("169.254.169.254")));
        assert!(!p(ip("64:ff9b::a00:1")));
        assert!(p(ip("192.168.1.25")));
        assert!(p(ip("::ffff:192.168.1.25")));
        assert!(!p(ip("192.168.2.25")));
        assert!(p(ip("93.184.216.34")));
    }

    #[test]
    fn a_refused_password_is_not_offered_again() {
        let master = Key::generate().unwrap();
        let mut s = Store::in_memory().unwrap();
        s.check_master_key(&master).unwrap();
        let ann = s
            .add_user(&master, "ann", crate::store::Role::User)
            .unwrap()
            .id;
        s.save_mail_credential(&master, ann, 1, b"made-up").unwrap();
        let p = SealedCredentials {
            store: Arc::new(Mutex::new(s)),
            master: master.clone(),
            user_id: ann,
        };
        assert!(p.credential(1).unwrap().is_some());
        p.sign_in_needed(1).unwrap();
        assert!(
            p.credential(1).unwrap().is_none(),
            "replayed after a refusal"
        );
        // A new password clears it.
        p.store()
            .save_mail_credential(&master, ann, 1, b"made-up-2")
            .unwrap();
        assert!(p.credential(1).unwrap().is_some());
    }
}
