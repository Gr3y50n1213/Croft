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
type Slot = Arc<Mutex<Option<(Arc<Core>, u64)>>>;

/// The open cores, one per user.
pub struct MailCores {
    data_dir: PathBuf,
    slots: Mutex<HashMap<i64, Slot>>,
    max_accounts: usize,
    /// Bumped when a core is closed: whoever holds a core while waiting on it (the push relay's
    /// watchers) lets it go, so the core is dropped where its threads are waited for.
    closing: tokio::sync::watch::Sender<u64>,
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
            closing: tokio::sync::watch::channel(0).0,
        }
    }

    /// The same, with another limit on accounts per user (tests).
    pub fn with_max_accounts(mut self, max: usize) -> Self {
        self.max_accounts = max;
        self
    }

    /// A signal that a core is being closed (see `closing`).
    pub fn closing(&self) -> tokio::sync::watch::Receiver<u64> {
        self.closing.subscribe()
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
        self.core_and_epoch(store, master, user_id).map(|(c, _)| c)
    }

    /// The user's core and a random number naming this opening of it (T27: JMAP states carry it,
    /// so a state from before a restart, when the core's change numbers start again, never looks
    /// current). Blocking: call it off the async threads.
    pub fn core_and_epoch(
        &self,
        store: &Arc<Mutex<Store>>,
        master: &Key,
        user_id: i64,
    ) -> Result<(Arc<Core>, u64), MailError> {
        let slot = self.slot(user_id);
        let mut held = slot.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some((core, epoch)) = held.as_ref() {
            return Ok((core.clone(), *epoch));
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
        let mut bytes = [0u8; 8];
        ring::rand::SecureRandom::fill(&ring::rand::SystemRandom::new(), &mut bytes)
            .map_err(|_| MailError::Open)?;
        let epoch = u64::from_le_bytes(bytes) >> 12;
        *held = Some((core.clone(), epoch));
        Ok((core, epoch))
    }

    /// The user's core if it is open (no opening).
    pub fn open_core(&self, user_id: i64) -> Option<Arc<Core>> {
        self.slot(user_id)
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
            .map(|(c, _)| c.clone())
    }

    /// Deletes a user's mail (T34e): under the user's slot lock (so nothing opens the core
    /// meanwhile), runs `delete` (the store's deletion), and when it deleted the user, stops their
    /// core and removes their folder with the database. Blocking: call it off the async threads.
    pub fn remove_user<E>(
        &self,
        user_id: i64,
        delete: impl FnOnce() -> Result<bool, E>,
    ) -> Result<bool, E> {
        let slot = self.slot(user_id);
        let mut held = slot.lock().unwrap_or_else(PoisonError::into_inner);
        if !delete()? {
            return Ok(false);
        }
        let core = held.take();
        self.release(core.into_iter().map(|(c, _)| c).collect());
        let _ = self.remove_folder(user_id);
        drop(held);
        self.slots
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&user_id);
        Ok(true)
    }

    fn remove_folder(&self, user_id: i64) -> std::io::Result<()> {
        let dir = self.data_dir.join("users").join(user_id.to_string());
        match std::fs::remove_dir_all(&dir) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        }
    }

    /// Removes the mail folders of users who no longer exist (a deletion whose folder could not be
    /// removed, or one cut short): at start, before any core opens. User ids are never reused
    /// (AUTOINCREMENT), so a folder never reaches another user. The number removed.
    pub fn sweep(&self, users: &[i64]) -> usize {
        let Ok(entries) = std::fs::read_dir(self.data_dir.join("users")) else {
            return 0;
        };
        let mut removed = 0;
        for e in entries.flatten() {
            let Some(id) = e.file_name().to_str().and_then(|n| n.parse::<i64>().ok()) else {
                continue;
            };
            if !users.contains(&id) && self.remove_folder(id).is_ok() {
                removed += 1;
            }
        }
        removed
    }

    /// Stops the user's core (a disabled user). Blocking: call it off the async threads.
    pub fn close(&self, user_id: i64) {
        let slot = self.slot(user_id);
        let core = slot.lock().unwrap_or_else(PoisonError::into_inner).take();
        self.release(core.into_iter().map(|(c, _)| c).collect());
    }

    /// Asks every holder to let the cores go, waits a moment until they have, and drops them
    /// where their threads are waited for.
    fn release(&self, cores: Vec<Arc<Core>>) {
        if cores.is_empty() {
            return;
        }
        self.closing.send_modify(|n| *n += 1);
        for _ in 0..200 {
            if cores.iter().all(|c| Arc::strong_count(c) == 1) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        drop_off_runtime(cores);
    }
}

impl Drop for MailCores {
    fn drop(&mut self) {
        let slots: Vec<Slot> = self
            .slots
            .get_mut()
            .unwrap_or_else(PoisonError::into_inner)
            .drain()
            .map(|(_, s)| s)
            .collect();
        let cores = slots
            .iter()
            .filter_map(|s| s.lock().unwrap_or_else(PoisonError::into_inner).take())
            .map(|(c, _)| c)
            .collect();
        self.release(cores);
    }
}

/// Drops cores on a thread of their own, where no async runtime is current: a core dropped there
/// waits (briefly) for its threads to finish, instead of leaving them running into the database
/// while the process goes on, or exits (T28: a test process crashed at exit in SQLCipher).
fn drop_off_runtime(cores: Vec<Arc<Core>>) {
    if cores.is_empty() {
        return;
    }
    let _ = std::thread::spawn(move || drop(cores)).join();
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
