// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The [`Core`] object: what the screens call.

use std::collections::{BTreeSet, HashMap};
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::task::{Context, Poll};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use croft_engine::{ActionError, Engine, EngineConfig, EngineHandle, OpId, SystemClock};
use croft_protocols::demo::DEMO_ADDRESS;
use croft_protocols::discovery::{verify, Discoverer, EmailAddress, ProbeUpdate, VerifyOptions};
use croft_protocols::{CertFingerprint, DemoBackend, DemoFolderState, RemoteHeader};
use croft_storage::accounts::{self, NewAccount, Protocol};
use croft_storage::messages::{self, Message};
use croft_storage::{folders, Db, StorageError};
use tokio::runtime::{Builder, Handle, Runtime};
use tokio::task::JoinHandle;
use zeroize::Zeroizing;

use crate::changes::{listen, Hub};
use crate::credentials::{
    ask, AccountBackend, Asked, CredentialProvider, FfiCredential, ProviderError, Readiness,
};
use crate::error::{CoreError, DiscoverError, VerifyFailure};
use crate::list;
use crate::settings::{
    demo_config, is_demo, StoredSecurity, StoredServer, StoredSettings, SETTINGS_VERSION,
};
use crate::types::{
    AccountInfo, AccountStatus, ChangeBatch, DiscoveryProbe, FfiDiscovered, FolderInfo,
    MessagePage, MessageScope, ProbeProgress, RowCursor, ServerSettings,
};

/// Called by `discover` while it runs, for the "Finding your server" screen (S03). Callbacks come
/// from a blocking thread of the core, one at a time and in order, never from a runtime worker:
/// a slow observer delays only the progress it reports. Hop to the main thread before touching
/// the UI. A callback that throws is reported as [`ProviderError::Failed`] and ignored.
#[uniffi::export(with_foreign)]
pub trait DiscoveryObserver: Send + Sync {
    /// One probe changed state. Every probe's final state is delivered before `discover` returns
    /// (unless the call is cancelled: nothing is delivered after that).
    fn on_probe(&self, probe: DiscoveryProbe, state: ProbeProgress) -> Result<(), ProviderError>;
}

/// The runtime or a task of it is gone (the task panicked or was stopped).
struct Gone;

impl From<Gone> for CoreError {
    fn from(_: Gone) -> Self {
        Self::Internal
    }
}
impl From<Gone> for DiscoverError {
    fn from(_: Gone) -> Self {
        Self::Internal
    }
}
impl From<Gone> for VerifyFailure {
    fn from(_: Gone) -> Self {
        Self::Internal
    }
}

/// A task on the core's runtime. Awaited by whatever executor polls the exported future (the
/// app's coroutine machinery); the work itself runs on the core's runtime. Dropping it (the app
/// cancelled the call) aborts the task when `abort_on_drop` is set, which is right for waiting
/// and for network probing, and wrong for a change to the database that should finish.
struct Task<T> {
    handle: JoinHandle<T>,
    abort_on_drop: bool,
}

impl<T> Future for Task<T> {
    type Output = Result<T, Gone>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        Pin::new(&mut self.handle).poll(cx).map_err(|_| Gone)
    }
}

impl<T> Drop for Task<T> {
    fn drop(&mut self) {
        if self.abort_on_drop {
            self.handle.abort();
        }
    }
}

/// Everything the tasks and the synchronous calls share. Holds no runtime (see [`Core`]).
pub(crate) struct Inner {
    engine: Engine,
    handle: EngineHandle,
    hub: Arc<Hub>,
    provider: Arc<dyn CredentialProvider>,
    backends: Mutex<HashMap<i64, Arc<AccountBackend>>>,
    /// The sample mailbox, when it is running (at most one), by account id. It has no entry in
    /// `backends`: no server adapter is ever built for it.
    demos: Mutex<HashMap<i64, Arc<DemoBackend>>>,
    /// Held while `add_demo_account` looks for the sample mailbox and adds it, so that two calls
    /// at once still add only one.
    demo_adding: Mutex<()>,
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|d| i64::try_from(d.as_millis()).ok())
        .unwrap_or(0)
}

impl Inner {
    fn backends(&self) -> MutexGuard<'_, HashMap<i64, Arc<AccountBackend>>> {
        self.backends.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Builds the account's backend from `credential` and starts the account's task in the
    /// engine. Needs a runtime context (the engine spawns the task). `credential` is what the app
    /// gave for a new account (`add_account`) or what the provider answered for a stored one (see
    /// [`Inner::start_stored_account`]); `None` starts the account in the needs-sign-in state.
    fn start_account(
        &self,
        id: i64,
        settings: &StoredSettings,
        credential: Asked,
    ) -> Result<(), CoreError> {
        let config = settings.imap_config().map_err(|_| CoreError::BadSettings)?;
        config.validate().map_err(|_| CoreError::BadSettings)?;
        let backend = Arc::new(AccountBackend::new(config, self.provider.clone(), id));
        match backend.set_asked(credential) {
            Readiness::Ready => {}
            Readiness::NeedsSignIn => self.hub.sign_in_needed(id),
            // The app's secure storage is busy: not a sign-in problem. The backend answers
            // Transient and asks again later.
            Readiness::NotReady => {}
            Readiness::Broken => return Err(CoreError::BadSettings),
        }
        self.engine.add_account(id, backend.clone())?;
        self.backends().insert(id, backend);
        if self.hub.is_offline() {
            self.handle.set_online(id, false);
        }
        Ok(())
    }

    fn demos(&self) -> MutexGuard<'_, HashMap<i64, Arc<DemoBackend>>> {
        self.demos.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Starts the sample mailbox's task on `backend`, with a submitter that sends nothing (it files
    /// the copy in the sample Sent folder). No credential is asked for and no server adapter is
    /// built. The account is never set offline: it needs no network, so its actions are confirmed
    /// at once even in airplane mode. Needs a runtime context (the engine spawns the task).
    fn start_demo_account(&self, id: i64, backend: DemoBackend) -> Result<(), CoreError> {
        let backend = Arc::new(backend);
        self.engine.add_account_with_submitter(
            id,
            backend.clone(),
            Arc::new(backend.submitter()),
        )?;
        self.demos().insert(id, backend);
        Ok(())
    }

    /// True when the account is the running sample mailbox.
    fn is_demo_running(&self, id: i64) -> bool {
        self.demos().contains_key(&id)
    }

    /// The account list rows, in list order.
    fn account_infos(&self) -> Result<Vec<AccountInfo>, CoreError> {
        let stored = self.handle.read(|db| {
            let conn = db.connection();
            accounts::list(conn)?
                .into_iter()
                .map(|a| {
                    let last_synced_ms = folders::list(conn, a.id)?
                        .iter()
                        .filter_map(|f| f.last_synced_at)
                        .max();
                    Ok::<_, CoreError>((a, last_synced_ms))
                })
                .collect::<Result<Vec<_>, _>>()
        })?;
        Ok(stored
            .into_iter()
            .map(|(a, last_synced_ms)| {
                let is_demo = is_demo(&a.config);
                let status = match self.hub.status(a.id) {
                    // The sample mailbox needs no connection: the device being offline does not
                    // concern it.
                    AccountStatus::Offline if is_demo => AccountStatus::Ok,
                    other => other,
                };
                AccountInfo {
                    status,
                    id: a.id,
                    label: a.label,
                    email: a.email,
                    last_synced_ms,
                    is_demo,
                }
            })
            .collect())
    }

    /// Starts an account the app stored earlier: asks the provider for its credential. Blocks
    /// while the provider answers: call it from `open` or a blocking thread. The provider is
    /// never asked about an account the app has not stored a secret for (a new account's
    /// credential arrives with `add_account`).
    fn start_stored_account(&self, id: i64, settings: &StoredSettings) -> Result<(), CoreError> {
        self.start_account(id, settings, ask(&*self.provider, id))
    }

    /// Every account id.
    fn account_ids(&self) -> Result<Vec<i64>, CoreError> {
        self.handle.read(|db| {
            Ok(accounts::list(db.connection())?
                .into_iter()
                .map(|a| a.id)
                .collect())
        })
    }

    /// Finds the account of each message id (the ids are local row ids, unique across accounts).
    /// Fails with `NoSuchMessage` if any id matches no message, before anything is changed.
    fn resolve(&self, ids: &[i64]) -> Result<Vec<Message>, CoreError> {
        self.handle.read(|db| {
            let conn = db.connection();
            let accounts: Vec<i64> = accounts::list(conn)?.into_iter().map(|a| a.id).collect();
            let mut seen = BTreeSet::new();
            let mut out = Vec::new();
            for &id in ids {
                if !seen.insert(id) {
                    continue;
                }
                let mut found = None;
                for &account in &accounts {
                    if let Some(m) = messages::get(conn, account, id)? {
                        found = Some(m);
                        break;
                    }
                }
                out.push(found.ok_or(CoreError::NoSuchMessage)?);
            }
            Ok(out)
        })
    }

    /// Moves each message to the destination `dest_of` picks for it. All destinations are found
    /// before the first change is made. A message already in its destination is left alone.
    fn move_each(
        &self,
        ids: &[i64],
        dest_of: impl Fn(&Db, &Message) -> Result<String, CoreError>,
    ) -> Result<Vec<OpId>, CoreError> {
        let resolved = self.resolve(ids)?;
        let planned: Vec<(i64, i64, String)> = self.handle.read(|db| {
            resolved
                .iter()
                .map(|m| Ok((m.account_id, m.id, dest_of(db, m)?)))
                .collect::<Result<_, CoreError>>()
        })?;
        let mut ops = Vec::new();
        for (account, id, dest) in planned {
            match self.handle.move_to(account, id, &dest) {
                Ok(op) => ops.push(op),
                Err(ActionError::SameFolder) => {}
                Err(e) => return Err(e.into()),
            }
        }
        Ok(ops)
    }

    /// The path of the account's folder with this special use.
    fn special_folder(
        db: &Db,
        account: i64,
        special_use: &str,
        missing: CoreError,
    ) -> Result<String, CoreError> {
        folders::list(db.connection(), account)?
            .into_iter()
            .find(|f| f.special_use == special_use)
            .map(|f| f.path)
            .ok_or(missing)
    }
}

/// The Croft core as one shared object.
///
/// # Async and the runtime
///
/// `Core` owns one tokio multi-thread runtime (2 worker threads), the sync [`Engine`] and the
/// change log. The exported `async` methods are plain futures: the app's executor (Kotlin
/// coroutines) polls them, and each one spawns its work on this runtime and awaits the task, so
/// nothing runs on the app's threads and the app never needs a tokio context. The UniFFI
/// `async_runtime = "tokio"` option is not used (it would run the futures on a second, hidden
/// runtime of its own). The synchronous methods only touch local SQLite (and, for `signed_in`
/// style calls that ask the app for a secret, nothing blocking on the UI thread).
#[derive(uniffi::Object)]
pub struct Core {
    inner: Arc<Inner>,
    rt: Handle,
    runtime: Mutex<Option<Runtime>>,
}

impl Core {
    fn spawn<T: Send + 'static>(
        &self,
        fut: impl Future<Output = T> + Send + 'static,
        abort_on_drop: bool,
    ) -> Task<T> {
        Task {
            handle: self.rt.spawn(fut),
            abort_on_drop,
        }
    }

    /// `discover` with a given discoverer (tests use one with a fake network), still run on the
    /// core's runtime like the real call.
    /// The hub, for tests that inject an engine change.
    #[cfg(test)]
    pub(crate) fn hub(&self) -> &Arc<Hub> {
        &self.inner.hub
    }

    #[cfg(test)]
    pub(crate) async fn discover_using(
        &self,
        discoverer: Discoverer,
        email: String,
        observer: Option<Arc<dyn DiscoveryObserver>>,
    ) -> Result<FfiDiscovered, DiscoverError> {
        let task = self.spawn(discover_with(discoverer, email, observer), true);
        task.await?
    }

    fn pin_from_text(text: &str) -> Result<CertFingerprint, CoreError> {
        CertFingerprint::from_hex(text).map_err(|_| CoreError::BadFingerprint)
    }
}

impl Drop for Core {
    fn drop(&mut self) {
        let runtime = self
            .runtime
            .get_mut()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        if let Some(rt) = runtime {
            // Never block inside an async context (dropping a runtime there panics); elsewhere
            // wait a moment so the account tasks release the database before a reopen.
            if Handle::try_current().is_ok() {
                rt.shutdown_background();
            } else {
                rt.shutdown_timeout(Duration::from_secs(2));
            }
        }
    }
}

#[uniffi::export]
impl Core {
    /// Opens (creating it if needed) the database at `db_path`, migrates it, and starts every
    /// stored account: for each one the `credentials` provider is asked for its credential (see
    /// [`CredentialProvider`]). An account without one starts in the needs-sign-in state.
    ///
    /// `db_key` is the 32-byte database key from the platform keystore, or `None` for an
    /// unencrypted database (tests; a build without SQLCipher refuses a key).
    ///
    /// This reads and migrates the database and calls the provider, so call it off the main
    /// thread (the one synchronous call that is not instant).
    #[uniffi::constructor]
    pub fn open(
        db_path: String,
        db_key: Option<Vec<u8>>,
        credentials: Arc<dyn CredentialProvider>,
    ) -> Result<Arc<Self>, CoreError> {
        let key = db_key.map(Zeroizing::new);
        let db = Db::open(
            std::path::Path::new(&db_path),
            key.as_deref().map(Vec::as_slice),
        )?;
        drop(key);

        let runtime = Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("croft-core")
            .enable_all()
            .build()
            .map_err(|_| CoreError::Internal)?;
        let rt = runtime.handle().clone();
        let _enter = rt.enter();

        let engine = Engine::open(db, Arc::new(SystemClock), EngineConfig::default())?;
        let handle = engine.handle();
        let hub = Hub::new(credentials.clone());
        hub.attach_engine(handle.clone());
        // Subscribe before any account starts, so no change is missed.
        rt.spawn(listen(hub.clone(), handle.subscribe()));
        let inner = Arc::new(Inner {
            engine,
            handle,
            hub,
            provider: credentials,
            backends: Mutex::new(HashMap::new()),
            demos: Mutex::new(HashMap::new()),
            demo_adding: Mutex::new(()),
        });

        let stored = inner.handle.read(|db| accounts::list(db.connection()))?;
        for account in stored {
            if is_demo(&account.config) {
                // Rebuilt from what the device stored, so a sync changes nothing the person did.
                let started = inner
                    .handle
                    .read(|db| demo_state(db, account.id))
                    .map_err(CoreError::from)
                    .and_then(|saved| {
                        inner.start_demo_account(account.id, DemoBackend::resume(now_ms(), saved))
                    });
                if started.is_err() {
                    inner.hub.set_broken(account.id, true);
                }
                continue;
            }
            let usable = (account.protocol == Protocol::Imap)
                .then(|| StoredSettings::from_json(&account.config).ok())
                .flatten();
            let started = match usable {
                Some(settings) => inner.start_stored_account(account.id, &settings),
                None => Err(CoreError::BadSettings),
            };
            if started.is_err() {
                inner.hub.set_broken(account.id, true);
            }
        }

        Ok(Arc::new(Self {
            inner,
            rt,
            runtime: Mutex::new(Some(runtime)),
        }))
    }

    /// Looks up the server settings for an address (S02 to S04). Nothing secret is sent. The
    /// optional `observer` is told how each probe is doing. Cancelling the call stops the
    /// probes.
    pub async fn discover(
        &self,
        email: String,
        observer: Option<Arc<dyn DiscoveryObserver>>,
    ) -> Result<FfiDiscovered, DiscoverError> {
        // A malformed address is refused before anything else is set up.
        EmailAddress::parse(&email).map_err(|e| DiscoverError::from(&e))?;
        let task = self.spawn(
            async move {
                let discoverer =
                    Discoverer::with_system_network().map_err(|e| DiscoverError::from(&e))?;
                discover_with(discoverer, email, observer).await
            },
            true,
        );
        task.await?
    }

    /// Signs in with the discovered settings and the credential the person typed, to check them
    /// (S05 "Test connection"). Returns nothing on success. On failure the typed
    /// [`VerifyFailure`] says what to show: a wrong password, an app password needed, an
    /// untrusted certificate with its fingerprint (pass it back as `pinned_fingerprint` after
    /// the person trusted it), and so on. Use `verify_failure_message_key` for the text.
    pub async fn verify(
        &self,
        discovered: FfiDiscovered,
        credential: Arc<FfiCredential>,
        pinned_fingerprint: Option<String>,
    ) -> Result<(), VerifyFailure> {
        let pin = pinned_fingerprint
            .map(|p| CertFingerprint::from_hex(&p))
            .transpose()
            .map_err(|_| VerifyFailure::BadSettings)?;
        let settings = discovered.to_settings();
        // Not valid UTF-8 (the bytes are wiped): no server would accept it as a password.
        let credential = credential
            .to_credential()
            .ok_or(VerifyFailure::WrongPassword)?;
        let task = self.spawn(
            async move {
                let options = VerifyOptions {
                    pinned_cert: pin,
                    ..VerifyOptions::default()
                };
                verify(&settings, credential, options)
                    .await
                    .map(|_| ())
                    .map_err(|e| VerifyFailure::from_verify_error(&e))
            },
            true,
        );
        task.await?
    }

    /// Adds the account and starts syncing it. Stores the settings (no secret) in the account
    /// row and builds the account's connection from `credential`, the one the person just
    /// verified. The core keeps it only in the protocol layer's wiping string and never asks the
    /// provider about a new account. The returned id is the account's: **the app stores the
    /// credential under that id right after this returns** (and stores nothing when this fails),
    /// so that the provider can answer for it from then on (`open`, `signed_in`). Ids are never
    /// reused, so the id has no stored secrets of an earlier account.
    ///
    /// A credential that is not valid UTF-8 fails with `NoCredential` before anything is written.
    ///
    /// `pinned_fingerprint` is the certificate the person trusted (see `verify`); it is ignored
    /// for a well-known provider.
    pub async fn add_account(
        &self,
        discovered: FfiDiscovered,
        display_name: String,
        pinned_fingerprint: Option<String>,
        credential: Arc<FfiCredential>,
    ) -> Result<i64, CoreError> {
        let imap = discovered
            .imap
            .clone()
            .ok_or(CoreError::UnsupportedServer)?;
        let pin = match (&pinned_fingerprint, discovered.provider) {
            (Some(text), None) => Some(Self::pin_from_text(text)?.to_string()),
            _ => None,
        };
        let settings = StoredSettings {
            version: SETTINGS_VERSION,
            imap: stored_server(&imap),
            smtp: discovered.smtp.as_ref().map(stored_server),
            pinned_fingerprint: pin,
        };
        let json = settings.to_json().map_err(|_| CoreError::BadSettings)?;
        settings
            .imap_config()
            .map_err(|_| CoreError::BadSettings)?
            .validate()
            .map_err(|_| CoreError::BadSettings)?;
        let provider = discovered
            .provider
            .map_or("generic", |p| p.db_name())
            .to_owned();
        let email = discovered.email;
        // Not valid UTF-8 (the bytes are wiped): no server would accept it as a password.
        let credential = credential.to_credential().ok_or(CoreError::NoCredential)?;
        let inner = self.inner.clone();
        let task = self.spawn(
            async move {
                // Starting an account does blocking work: not on a runtime worker.
                tokio::task::spawn_blocking(move || {
                    let id = inner.handle.read(|db| {
                        let conn = db.connection();
                        let position = i64::try_from(accounts::list(conn)?.len()).unwrap_or(0);
                        Ok::<_, CoreError>(accounts::insert(
                            conn,
                            &NewAccount {
                                label: display_name,
                                email,
                                protocol: Protocol::Imap,
                                provider,
                                config: json,
                                color: None,
                                position,
                            },
                            now_ms(),
                        )?)
                    })?;
                    if let Err(e) =
                        inner.start_account(id, &settings, Asked::Credential(credential))
                    {
                        let _ = inner
                            .handle
                            .read(|db| accounts::delete(db.connection(), id));
                        return Err(e);
                    }
                    inner.hub.accounts_changed([id]);
                    Ok(id)
                })
                .await
                .map_err(|_| CoreError::Internal)?
            },
            false,
        );
        task.await?
    }

    /// Every account, in list order.
    pub fn list_accounts(&self) -> Result<Vec<AccountInfo>, CoreError> {
        self.inner.account_infos()
    }

    /// Adds the sample mailbox ("Try with sample mail") and starts it, or returns the one that is
    /// already there: there is never more than one. It needs no credential, no secret and no
    /// network, and never opens a connection. Its fictional messages (people and addresses at
    /// `example.org` and `example.com` only) come from the app itself; read, star, move, archive
    /// and delete work and stay on this device (the account has `is_demo`, and its rows show "Not
    /// saved to server"). The app stores no credential for it. Remove it with `remove_account`.
    pub async fn add_demo_account(&self) -> Result<AccountInfo, CoreError> {
        let inner = self.inner.clone();
        let task = self.spawn(
            async move {
                // Starting an account does blocking work: not on a runtime worker.
                tokio::task::spawn_blocking(move || {
                    let _adding = inner
                        .demo_adding
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner);
                    let existing = inner.handle.read(|db| {
                        Ok::<_, CoreError>(
                            accounts::list(db.connection())?
                                .into_iter()
                                .find(|a| is_demo(&a.config))
                                .map(|a| a.id),
                        )
                    })?;
                    let id = match existing {
                        Some(id) => id,
                        None => {
                            let id = inner.handle.read(|db| {
                                let conn = db.connection();
                                let position =
                                    i64::try_from(accounts::list(conn)?.len()).unwrap_or(0);
                                Ok::<_, CoreError>(accounts::insert(
                                    conn,
                                    &NewAccount {
                                        label: DEMO_LABEL.to_owned(),
                                        email: DEMO_ADDRESS.to_owned(),
                                        protocol: Protocol::Imap,
                                        provider: "generic".to_owned(),
                                        config: demo_config(),
                                        color: None,
                                        position,
                                    },
                                    now_ms(),
                                )?)
                            })?;
                            if let Err(e) = inner.start_demo_account(id, DemoBackend::new(now_ms()))
                            {
                                let _ = inner
                                    .handle
                                    .read(|db| accounts::delete(db.connection(), id));
                                return Err(e);
                            }
                            inner.hub.accounts_changed([id]);
                            id
                        }
                    };
                    inner
                        .account_infos()?
                        .into_iter()
                        .find(|a| a.id == id)
                        .ok_or(CoreError::Internal)
                })
                .await
                .map_err(|_| CoreError::Internal)?
            },
            false,
        );
        task.await?
    }

    /// Removes the account: stops its task and deletes its folders, messages and queued changes
    /// from the device. The server is not touched. Removing the stored credential is the app's
    /// job.
    pub async fn remove_account(&self, id: i64) -> Result<(), CoreError> {
        let exists = self
            .inner
            .handle
            .read(|db| accounts::get(db.connection(), id))?
            .is_some();
        if !exists {
            return Err(CoreError::NoSuchAccount);
        }
        let inner = self.inner.clone();
        let task = self.spawn(
            async move {
                inner.engine.remove_account(id).await;
                inner.backends().remove(&id);
                inner.demos().remove(&id);
                inner.hub.forget(id);
                inner
                    .handle
                    .read(|db| accounts::delete(db.connection(), id))?;
                inner.hub.accounts_changed([id]);
                Ok::<(), CoreError>(())
            },
            false,
        );
        task.await?
    }

    /// Every folder of the account, ordered by path. An unknown account has none.
    pub fn list_folders(&self, account_id: i64) -> Result<Vec<FolderInfo>, CoreError> {
        let folders = self
            .inner
            .handle
            .read(|db| folders::list(db.connection(), account_id))?;
        Ok(folders.into_iter().map(FolderInfo::from).collect())
    }

    /// One page of messages, newest first (by date, then id), without snoozed messages and
    /// without messages hidden while a move or delete waits for the server. Pass the `next` of
    /// the previous page as `before` to continue; `limit` is at most 500.
    pub fn list_messages(
        &self,
        scope: MessageScope,
        before: Option<RowCursor>,
        limit: u32,
    ) -> Result<MessagePage, CoreError> {
        self.inner.handle.read(|db| {
            let conn = db.connection();
            let ids = match scope {
                MessageScope::UnifiedInbox | MessageScope::Starred { account: None } => {
                    accounts::list(conn)?.into_iter().map(|a| a.id).collect()
                }
                MessageScope::Folder { .. } | MessageScope::Starred { .. } => Vec::new(),
            };
            list::page(conn, &ids, scope, now_ms(), before, limit)
        })
    }

    /// Waits until something changed after `after` and returns everything that changed since,
    /// merged. Start with `after = 0`, then pass the returned `seq` each time. Returns at once
    /// when there is already something newer. Cancelling the call is safe. See [`ChangeBatch`].
    pub async fn next_change(&self, after: u64) -> Result<ChangeBatch, CoreError> {
        let hub = self.inner.hub.clone();
        let task = self.spawn(
            async move {
                let mut seq = hub.subscribe();
                loop {
                    if let Some(batch) = hub.batch_after(after) {
                        return Ok(batch);
                    }
                    if seq.changed().await.is_err() {
                        return Err(CoreError::Internal);
                    }
                }
            },
            true,
        );
        task.await?
    }

    /// Marks messages read or unread. The change is sent after 1.5 seconds; the opposite call on
    /// the same message inside that time cancels both (that is the undo for this action, and it
    /// is why there is no operation id to return).
    pub fn mark_seen(&self, ids: Vec<i64>, value: bool) -> Result<(), CoreError> {
        for m in self.inner.resolve(&ids)? {
            self.inner.handle.mark_seen(m.account_id, m.id, value)?;
        }
        Ok(())
    }

    /// Stars or unstars messages. Same timing and undo as `mark_seen`.
    pub fn set_flagged(&self, ids: Vec<i64>, value: bool) -> Result<(), CoreError> {
        for m in self.inner.resolve(&ids)? {
            self.inner.handle.set_flagged(m.account_id, m.id, value)?;
        }
        Ok(())
    }

    /// Moves messages to their account's Archive folder. Fails with `NoArchiveFolder` (changing
    /// nothing) when an account has none. Returns the operation ids for the Undo snackbar.
    pub fn archive(&self, ids: Vec<i64>) -> Result<Vec<u64>, CoreError> {
        self.inner.move_each(&ids, |db, m| {
            Inner::special_folder(db, m.account_id, "archive", CoreError::NoArchiveFolder)
        })
    }

    /// Moves messages to their account's Trash folder. Fails with `NoTrashFolder` (changing
    /// nothing) when an account has none. Returns the operation ids for the Undo snackbar.
    pub fn trash(&self, ids: Vec<i64>) -> Result<Vec<u64>, CoreError> {
        self.inner.move_each(&ids, |db, m| {
            Inner::special_folder(db, m.account_id, "trash", CoreError::NoTrashFolder)
        })
    }

    /// Moves messages to the folder with this row id, which must belong to each message's own
    /// account (`NoSuchFolder` otherwise, changing nothing). Returns the operation ids.
    pub fn move_to(&self, ids: Vec<i64>, folder_id: i64) -> Result<Vec<u64>, CoreError> {
        self.inner.move_each(&ids, |db, m| {
            folders::get(db.connection(), m.account_id, folder_id)?
                .map(|f| f.path)
                .ok_or(CoreError::NoSuchFolder)
        })
    }

    /// Deletes messages permanently (the screen has asked first). Returns the operation ids for
    /// the Undo snackbar.
    pub fn delete_forever(&self, ids: Vec<i64>) -> Result<Vec<u64>, CoreError> {
        let resolved = self.inner.resolve(&ids)?;
        let mut ops = Vec::new();
        for m in resolved {
            ops.push(self.inner.handle.delete_forever(m.account_id, m.id)?);
        }
        Ok(ops)
    }

    /// Undo: cancels a move or delete that has not started and whose window is open, and puts
    /// the message back. Returns `true` when it was cancelled and `false` when it was too late
    /// (or already gone); it never reverses something the server already did.
    pub fn undo(&self, op_id: u64) -> Result<bool, CoreError> {
        match self.inner.handle.undo(op_id) {
            Ok(()) => Ok(true),
            Err(ActionError::TooLate | ActionError::NoSuchOperation) => Ok(false),
            Err(e) => Err(e.into()),
        }
    }

    /// Sets the Undo window for moves and deletes made from now on, in milliseconds (the app
    /// offers 3, 5 and 10 seconds).
    pub fn set_undo_window_ms(&self, ms: i64) {
        self.inner.handle.set_undo_window_ms(ms);
    }

    /// Syncs every folder of the account now (pull to refresh). Returns at once.
    pub fn sync_now(&self, account_id: i64) -> Result<(), CoreError> {
        let known = self.inner.backends().contains_key(&account_id)
            || self.inner.is_demo_running(account_id);
        if !known {
            return Err(CoreError::NoSuchAccount);
        }
        self.inner.handle.sync_now(account_id);
        Ok(())
    }

    /// Reports whether the device has a connection (from the platform's connectivity callback).
    /// Applies to every account except the sample mailbox, which needs no connection. While
    /// offline nothing is sent and actions still work locally; going online sends what waited and
    /// syncs.
    pub fn set_network_available(&self, available: bool) -> Result<(), CoreError> {
        let changed = self.inner.hub.set_offline(!available);
        let ids: Vec<i64> = self.inner.backends().keys().copied().collect();
        for &id in &ids {
            self.inner.handle.set_online(id, available);
        }
        if changed {
            self.inner.hub.accounts_changed(self.inner.account_ids()?);
        }
        Ok(())
    }

    /// The person fixed the sign-in (a new password, a new token): asks the credential provider
    /// for the credential again and resumes the account. Fails with `NoCredential` when the
    /// provider still has none (the account stays in the needs-sign-in state). For the sample
    /// mailbox there is nothing to sign in to: it returns at once.
    pub async fn signed_in(&self, account_id: i64) -> Result<(), CoreError> {
        if self.inner.is_demo_running(account_id) {
            return Ok(());
        }
        let backend = self
            .inner
            .backends()
            .get(&account_id)
            .cloned()
            .ok_or(CoreError::NoSuchAccount)?;
        let inner = self.inner.clone();
        let task = self.spawn(
            async move {
                // The provider callback may block: not on a runtime worker.
                tokio::task::spawn_blocking(move || {
                    let credential = ask(&*inner.provider, account_id);
                    match backend.set_asked(credential) {
                        Readiness::Ready => {
                            inner.hub.set_broken(account_id, false);
                            // Engine first: a notice applied from here on sees it resumed.
                            inner.handle.signed_in(account_id);
                            inner.hub.clear_sign_in(account_id);
                            Ok(())
                        }
                        Readiness::NeedsSignIn => Err(CoreError::NoCredential),
                        // The app's storage is busy right now: the person did sign in, so the
                        // account leaves the needs-sign-in state and the backend asks again
                        // by itself (Transient meanwhile).
                        Readiness::NotReady => {
                            inner.handle.signed_in(account_id);
                            inner.hub.clear_sign_in(account_id);
                            Ok(())
                        }
                        Readiness::Broken => {
                            inner.hub.set_broken(account_id, true);
                            Err(CoreError::BadSettings)
                        }
                    }
                })
                .await
                .map_err(|_| CoreError::Internal)?
            },
            false,
        );
        task.await?
    }
}

/// The name of the sample mailbox in the account list (the app may show its own translated text).
const DEMO_LABEL: &str = "Sample mail";

/// What the device has stored for each folder of the sample mailbox (rows hidden while a move or
/// delete waits included), to rebuild its in-memory copy after a restart.
fn demo_state(db: &Db, account: i64) -> Result<Vec<DemoFolderState>, StorageError> {
    let conn = db.connection();
    let mut out = Vec::new();
    for f in folders::list(conn, account)? {
        let mut headers = Vec::new();
        for (uid, flags) in messages::folder_flags(conn, account, f.id)? {
            if let Some(m) = messages::find_by_uid(conn, account, f.id, uid)? {
                headers.push(RemoteHeader {
                    uid,
                    message_id: m.message_id_hdr,
                    references: Vec::new(),
                    subject: m.subject,
                    from_name: m.from_name,
                    from_addr: m.from_addr,
                    date_ms: m.date_ts,
                    size: u32::try_from(m.size).unwrap_or(0),
                    flags,
                });
            }
        }
        out.push(DemoFolderState {
            path: f.path,
            synced: f.uid_validity.is_some(),
            uid_next: f.uid_next.and_then(|n| u32::try_from(n).ok()),
            messages: headers,
        });
    }
    Ok(out)
}

fn stored_server(s: &ServerSettings) -> StoredServer {
    StoredServer {
        host: s.host.clone(),
        port: s.port,
        security: match s.security {
            crate::types::SecurityMode::Tls => StoredSecurity::Tls,
            crate::types::SecurityMode::StartTls => StoredSecurity::StartTls,
        },
        username: s.username.clone(),
    }
}

/// Runs discovery and reports each probe to the observer. Every probe's final state has been
/// delivered when this returns.
pub(crate) async fn discover_with(
    discoverer: Discoverer,
    email: String,
    observer: Option<Arc<dyn DiscoveryObserver>>,
) -> Result<FfiDiscovered, DiscoverError> {
    // Set when this call ends early (the app cancelled it): the forwarder then delivers nothing
    // more, though a callback that is already running is not interrupted.
    let cancelled = Arc::new(AtomicBool::new(false));
    let _stop = StopOnDrop(cancelled.clone());
    let (discoverer, forwarder) = match observer {
        None => (discoverer, None),
        Some(observer) => {
            let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<ProbeUpdate>();
            // The app's callback may block: not on a runtime worker. The loop ends when the
            // channel closes (the discoverer is dropped) or the call is cancelled.
            let stop = cancelled.clone();
            let task = tokio::task::spawn_blocking(move || {
                while let Some(update) = rx.blocking_recv() {
                    if stop.load(Ordering::Acquire) {
                        break;
                    }
                    // An exception on the app's side must not stop discovery.
                    let _ = observer.on_probe(update.probe.into(), update.state.into());
                }
            });
            (discoverer.with_progress(tx), Some(task))
        }
    };
    let result = discoverer.discover(&email).await;
    // Dropping the discoverer closes the channel, so the forwarder ends after the last update.
    drop(discoverer);
    if let Some(task) = forwarder {
        let _ = task.await;
    }
    match result {
        Ok(found) => Ok(FfiDiscovered::from_settings(email.trim(), &found)),
        Err(e) => Err(DiscoverError::from(&e)),
    }
}

/// Sets the flag when dropped: the forwarder's signal to stop delivering.
struct StopOnDrop(Arc<AtomicBool>);

impl Drop for StopOnDrop {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
    }
}
