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
const MAIL_DB_KEY: &[u8] = b"croft-companion/mail-db/v1";

/// Why a user's mail could not be opened.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MailError {
    /// The user's data key did not open (the wrong master key, or damaged data).
    Key,
    /// The folder or the database could not be opened.
    Open,
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
        match self
            .store()
            .mail_credential(&self.master, self.user_id, account_id)
        {
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

/// The open cores, one per user.
pub struct MailCores {
    data_dir: PathBuf,
    cores: Mutex<HashMap<i64, Arc<Core>>>,
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
            cores: Mutex::new(HashMap::new()),
        }
    }

    fn cores(&self) -> std::sync::MutexGuard<'_, HashMap<i64, Arc<Core>>> {
        self.cores.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The user's mail database file.
    pub fn database(&self, user_id: i64) -> PathBuf {
        self.data_dir
            .join("users")
            .join(user_id.to_string())
            .join("mail.db")
    }

    /// The user's core, opened if needed. Blocking: call it off the async threads.
    pub fn core(
        &self,
        store: &Arc<Mutex<Store>>,
        master: &Key,
        user_id: i64,
    ) -> Result<Arc<Core>, MailError> {
        if let Some(core) = self.cores().get(&user_id) {
            return Ok(core.clone());
        }
        let data_key = store
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .data_key(master, user_id)?;
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
        // Another request may have opened it meanwhile: keep the first, drop this one here (off
        // the async threads, as the caller is).
        let mut cores = self.cores();
        Ok(cores.entry(user_id).or_insert(core).clone())
    }

    /// The user's core if it is open (no opening).
    pub fn open_core(&self, user_id: i64) -> Option<Arc<Core>> {
        self.cores().get(&user_id).cloned()
    }

    /// Stops the user's core (a disabled user). Blocking: call it off the async threads.
    pub fn close(&self, user_id: i64) {
        let core = self.cores().remove(&user_id);
        drop(core);
    }
}
