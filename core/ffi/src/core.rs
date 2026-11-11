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

use croft_engine::{
    resume_demo, ActionError, Engine, EngineConfig, EngineHandle, OpId, SystemClock,
};
use croft_protocols::demo::DEMO_ADDRESS;
use croft_protocols::discovery::{
    provider_for_host, verify, Discoverer, EmailAddress, ProbeUpdate, VerifyOptions,
};
use croft_protocols::{clean_host, CertFingerprint, Credential, DemoBackend};
use croft_storage::accounts::{self, Account, NewAccount, Protocol};
use croft_storage::messages::{self, Message};
use croft_storage::{folders, Db};
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
    AccountInfo, AccountStatus, AddedAccount, ChangeBatch, DiscoveryProbe, FfiDiscovered,
    FolderInfo, MessagePage, MessageScope, ProbeProgress, RowCursor, ServerSettings,
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
    /// Held while `add_account` looks for an account with the same address and inserts or updates
    /// one, while `add_demo_account` looks for the sample mailbox and adds or starts it, and while
    /// `remove_account` removes an account: two adds at once (of one address, or of the sample
    /// mailbox) add only one, and a remove never runs between an add's row insert and the start of
    /// its task (which would leave a task running for a deleted account). A tokio lock: it is held
    /// across the awaits of these calls.
    account_changes: tokio::sync::Mutex<()>,
    /// What the last re-add of each account replaced, until the app undoes it or the next change
    /// of that account.
    previous: Mutex<HashMap<i64, Previous>>,
}

/// What `add_account` has worked out for the account, ready to be stored and started.
struct Added {
    display_name: String,
    email: String,
    provider: String,
    json: serde_json::Value,
    settings: StoredSettings,
    credential: Credential,
    /// The person confirmed that the pinned certificate changed (see `Core::add_account`).
    accept_changed_certificate: bool,
}

/// What a re-add replaced, kept so that the app can undo it when it could not store the new
/// password (see `Core::rollback_account_update`).
struct Previous {
    provider: String,
    config: serde_json::Value,
    settings: StoredSettings,
}

/// Whether two stored servers are the same: the host (cleaned up like the connection does it),
/// the port, the security mode and the user name.
fn same_stored_server(a: &StoredServer, b: &StoredServer) -> bool {
    clean_host(&a.host) == clean_host(&b.host)
        && a.port == b.port
        && a.security == b.security
        && a.username == b.username
}

/// Whether two settings records name the same servers: the IMAP server and the SMTP server (both
/// or neither), each by [`same_stored_server`]. A re-add that differs in any of them is a different
/// account: it must never silently repoint where mail is read or sent.
fn same_servers(a: &StoredSettings, b: &StoredSettings) -> bool {
    same_stored_server(&a.imap, &b.imap)
        && match (&a.smtp, &b.smtp) {
            (None, None) => true,
            (Some(x), Some(y)) => same_stored_server(x, y),
            _ => false,
        }
}

/// Whether two addresses are the same account's: compared with full Unicode lower-casing, after
/// trimming. (The core stores the address as it was typed.)
fn same_address(a: &str, b: &str) -> bool {
    a.trim().to_lowercase() == b.trim().to_lowercase()
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

    /// The account with this address that `add_account` would update: not the sample mailbox, and
    /// the same address without regard to case ([`same_address`]: the domain never has case, and no
    /// provider tells `Alice@` from `alice@`). Reads the database: call it from a blocking thread.
    fn account_with_address(&self, email: &str) -> Result<Option<Account>, CoreError> {
        Ok(self.handle.read(|db| {
            accounts::list(db.connection()).map(|all| {
                all.into_iter()
                    .find(|a| !is_demo(&a.config, &a.email) && same_address(&a.email, email))
            })
        })?)
    }

    /// The certificate pin of the account that `settings` would re-add (same address, same
    /// servers), if it has one that applies: the pin `verify` checks against when the caller has
    /// none, so that another certificate is reported as changed. Reads the database: call it from
    /// a blocking thread.
    fn stored_pin_for(&self, email: &str, settings: &StoredSettings) -> Option<CertFingerprint> {
        let account = self.account_with_address(email).ok()??;
        if account.protocol != Protocol::Imap {
            return None;
        }
        let old = StoredSettings::from_json(&account.config).ok()?;
        if !same_servers(&old, settings) {
            return None;
        }
        // `imap_config` leaves the pin out for a provider's host.
        old.imap_config().ok()?.pinned_cert
    }

    fn previous(&self) -> MutexGuard<'_, HashMap<i64, Previous>> {
        self.previous.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Inserts a new account and starts it with the credential, or removes the row again when it
    /// cannot start. Blocks: call it from a blocking thread, with `account_changes` held.
    fn insert_account(&self, new: Added) -> Result<i64, CoreError> {
        let Added {
            display_name,
            email,
            provider,
            json,
            settings,
            credential,
            accept_changed_certificate: _,
        } = new;
        let id = self.handle.read(|db| {
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
        if let Err(e) = self.start_account(id, &settings, Asked::Credential(credential)) {
            let _ = self.handle.read(|db| accounts::delete(db.connection(), id));
            return Err(e);
        }
        self.hub.accounts_changed([id]);
        Ok(id)
    }

    /// `add_account` for an address that is already added: the same servers mean the person signs
    /// in again (see `Core::add_account`), other servers are [`CoreError::AccountExists`], and a
    /// pinned certificate is never replaced or dropped unless the person confirmed the change
    /// ([`CoreError::CertificateChanged`]). With `account_changes` held.
    async fn sign_in_again(
        self: &Arc<Self>,
        existing: Account,
        new: Added,
    ) -> Result<i64, CoreError> {
        let id = existing.id;
        // Settings this version cannot read (a newer app wrote them) are never overwritten.
        let old =
            StoredSettings::from_json(&existing.config).map_err(|_| CoreError::AccountExists)?;
        if existing.protocol != Protocol::Imap || !same_servers(&old, &new.settings) {
            return Err(CoreError::AccountExists);
        }
        // A trusted certificate is not replaced or dropped without the person's say. The pin that
        // applies is the one the connection would use (none for a provider's host, and none when
        // the stored pin is unusable, so a broken record can be fixed by signing in again).
        let old_pin = old.imap_config().ok().and_then(|c| c.pinned_cert);
        let new_pin = new.settings.imap_config().ok().and_then(|c| c.pinned_cert);
        if old_pin.is_some() && old_pin != new_pin && !new.accept_changed_certificate {
            return Err(CoreError::CertificateChanged);
        }
        // Whatever an earlier re-add replaced is not this one's to undo.
        self.previous().remove(&id);
        let previous = Previous {
            provider: existing.provider,
            config: existing.config,
            settings: old,
        };
        // The new settings first, so a restart in the middle starts the account with them.
        self.handle
            .read(|db| accounts::set_settings(db.connection(), id, &new.provider, &new.json))?;
        // Stop the account's task (its queue and outbox stay stored) and start it again with the
        // credential the person just verified.
        self.engine.remove_account(id).await;
        self.backends().remove(&id);
        let inner = self.clone();
        let Added {
            settings,
            credential,
            ..
        } = new;
        let started = match tokio::task::spawn_blocking(move || {
            inner.start_account(id, &settings, Asked::Credential(credential))
        })
        .await
        {
            Ok(result) => result,
            Err(_) => Err(CoreError::Internal),
        };
        match started {
            Ok(()) => {
                // Engine first, then the hub, as in `signed_in`: the account leaves the
                // needs-sign-in state and syncs and sends what waited.
                self.hub.set_broken(id, false);
                self.handle.signed_in(id);
                self.hub.clear_sign_in(id);
                self.hub.accounts_changed([id]);
                self.previous().insert(id, previous);
                Ok(id)
            }
            Err(e) => {
                // A failed re-add costs nothing: the old settings are back and the account runs
                // as it did.
                let _ = self.restore(id, previous).await;
                Err(e)
            }
        }
    }

    /// Puts an account's earlier settings back and starts it as it was (asking the app for its
    /// stored credential). If that fails the account shows as an error. With `account_changes`
    /// held.
    async fn restore(self: &Arc<Self>, id: i64, previous: Previous) -> Result<(), CoreError> {
        self.engine.remove_account(id).await;
        self.backends().remove(&id);
        let back = self.clone();
        let restored = tokio::task::spawn_blocking(move || {
            back.handle.read(|db| {
                accounts::set_settings(db.connection(), id, &previous.provider, &previous.config)
            })?;
            back.start_stored_account(id, &previous.settings)
        })
        .await;
        match restored {
            Ok(Ok(())) => {
                self.hub.accounts_changed([id]);
                Ok(())
            }
            Ok(Err(e)) => {
                self.hub.set_broken(id, true);
                Err(e)
            }
            Err(_) => {
                self.hub.set_broken(id, true);
                Err(CoreError::Internal)
            }
        }
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
        let submitter = self.handle.demo_submitter(id, backend.clone());
        self.engine
            .add_account_with_submitter(id, backend.clone(), submitter)?;
        self.demos().insert(id, backend);
        Ok(())
    }

    /// Starts the sample mailbox stored earlier, rebuilt from what the device stored (so a sync
    /// changes nothing the person did). Needs a runtime context.
    fn start_stored_demo(&self, id: i64) -> Result<(), CoreError> {
        let backend = self.handle.read(|db| resume_demo(db, id, now_ms()))?;
        self.start_demo_account(id, backend)
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
                let is_demo = is_demo(&a.config, &a.email);
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

    /// The stored row of an account, for tests that check what was written.
    #[cfg(test)]
    pub(crate) fn stored_account(&self, id: i64) -> Option<Account> {
        self.inner
            .handle
            .read(|db| accounts::get(db.connection(), id))
            .ok()
            .flatten()
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
            account_changes: tokio::sync::Mutex::new(()),
            previous: Mutex::new(HashMap::new()),
        });

        let stored = inner.handle.read(|db| accounts::list(db.connection()))?;
        for account in stored {
            if is_demo(&account.config, &account.email) {
                // Rebuilt from what the device stored, so a sync changes nothing the person did.
                let started = inner.start_stored_demo(account.id);
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
    /// (S05 "Test connection"). On failure the typed [`VerifyFailure`] says what to show: a wrong
    /// password, an app password needed, an untrusted certificate with its fingerprint (pass it
    /// back as `pinned_fingerprint` after the person trusted it), and so on. Use
    /// `verify_failure_message_key` for the text.
    ///
    /// **A certificate the person trusted before stays the one that counts.** When no
    /// `pinned_fingerprint` is passed and an account with the same address (not the sample mailbox)
    /// and the same servers is stored with a pin, that pin is the expected certificate: any other
    /// certificate, one that chains to a trusted authority included, is
    /// `VerifyFailure::CertificateChanged` with the fingerprint seen, never the first-use
    /// `UntrustedCertificate`. The app then shows the stronger warning, and passes the new
    /// fingerprint back after the person confirmed it.
    ///
    /// Returns the fingerprint the connection was checked against, if any: the pin passed in, or
    /// the stored one. **Pass it to `add_account` as its `pinned_fingerprint`.** It is `None`
    /// for a provider's server (a pin is never used there) and when there is no pin.
    pub async fn verify(
        &self,
        discovered: FfiDiscovered,
        credential: Arc<FfiCredential>,
        pinned_fingerprint: Option<String>,
    ) -> Result<Option<String>, VerifyFailure> {
        let pin = pinned_fingerprint
            .map(|p| CertFingerprint::from_hex(&p))
            .transpose()
            .map_err(|_| VerifyFailure::BadSettings)?;
        let settings = discovered.to_settings();
        // Not valid UTF-8 (the bytes are wiped): no server would accept it as a password.
        let credential = credential
            .to_credential()
            .ok_or(VerifyFailure::WrongPassword)?;
        // What the stored account of this address would be compared with (servers only).
        let probe = discovered.imap.as_ref().map(|imap| StoredSettings {
            version: SETTINGS_VERSION,
            imap: stored_server(imap),
            smtp: discovered.smtp.as_ref().map(stored_server),
            pinned_fingerprint: None,
        });
        let email = discovered.email;
        let inner = self.inner.clone();
        let task = self.spawn(
            async move {
                let pin = match (pin, probe) {
                    (None, Some(probe)) => {
                        // A database read: not on a runtime worker.
                        tokio::task::spawn_blocking(move || inner.stored_pin_for(&email, &probe))
                            .await
                            .map_err(|_| VerifyFailure::Internal)?
                    }
                    (pin, _) => pin,
                };
                let options = VerifyOptions {
                    pinned_cert: pin,
                    ..VerifyOptions::default()
                };
                verify(&settings, credential, options)
                    .await
                    .map(|verified| verified.imap.pinned_cert.map(|pin| pin.to_string()))
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
    /// so that the provider can answer for it from then on (`open`, `signed_in`). A new account's
    /// id is never reused, so it has no stored secrets of an earlier account.
    ///
    /// **An address that is already added** (the same address, compared without regard to case;
    /// the sample mailbox never counts) is not added twice:
    /// - With the same servers as the stored account (IMAP host, port, security and user name, and
    ///   the SMTP server the same way, both present or both absent), this is the person signing in
    ///   again (a new password, a new certificate pin). The account keeps its id, its label and
    ///   everything synced so far; its stored settings are replaced (the pin included, under the
    ///   rules below), its connection is rebuilt from `credential`, and it leaves the
    ///   needs-sign-in state, so syncing and the queued changes resume with the new credential.
    ///   The result has `updated` set and the existing id, and **the app stores the credential
    ///   under it, replacing the old one**. If the app cannot store it, it calls
    ///   [`Core::rollback_account_update`] so that the account stays as it was.
    /// - With any other server, port, security mode or user name, or another SMTP server (or when
    ///   the stored settings cannot be read) it fails with [`CoreError::AccountExists`] and changes
    ///   nothing. The person removes the old account first if they mean to replace it, so that a
    ///   sign-in can never silently repoint where mail is read or sent.
    /// - **A pinned certificate is never replaced or dropped silently.** When the stored account
    ///   has a pin (a self-signed server the person trusted) and `pinned_fingerprint` is another
    ///   fingerprint or `None`, the call fails with [`CoreError::CertificateChanged`] and changes
    ///   nothing, unless `accept_changed_certificate` is true. Pass true **only** after the person
    ///   confirmed the changed-certificate warning (`VerifyFailure::CertificateChanged`, see
    ///   `verify`) in this very sign-in; never from a first-use trust question, and never by
    ///   default. It has no effect when there is no stored pin, and for a new account.
    ///
    /// A credential that is not valid UTF-8 fails with `NoCredential` before anything is written.
    ///
    /// `pinned_fingerprint` is the certificate the person trusted (see `verify`, which returns the
    /// one to pass); it is ignored for a well-known provider. A pin for a well-known provider's
    /// own server (`imap.gmail.com`) whose settings carry no provider (the person changed the port
    /// or typed the host by hand) is refused with `BadSettings`: a provider's real certificate is
    /// always valid, so a pin there could only pin an interceptor's.
    pub async fn add_account(
        &self,
        discovered: FfiDiscovered,
        display_name: String,
        pinned_fingerprint: Option<String>,
        accept_changed_certificate: bool,
        credential: Arc<FfiCredential>,
    ) -> Result<AddedAccount, CoreError> {
        let imap = discovered
            .imap
            .clone()
            .ok_or(CoreError::UnsupportedServer)?;
        let pin = match (
            &pinned_fingerprint,
            discovered.provider,
            provider_for_host(&imap.host),
        ) {
            (Some(text), None, None) => Some(Self::pin_from_text(text)?.to_string()),
            (Some(_), None, Some(_)) => return Err(CoreError::BadSettings),
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
                // One add, re-add or remove at a time: the check for the address and what follows
                // it cannot be interleaved with another call's.
                let _changing = inner.account_changes.lock().await;
                let reader = inner.clone();
                let same_address = {
                    let email = email.clone();
                    tokio::task::spawn_blocking(move || reader.account_with_address(&email))
                        .await
                        .map_err(|_| CoreError::Internal)??
                };
                let new = Added {
                    display_name,
                    email,
                    provider,
                    json,
                    settings,
                    credential,
                    accept_changed_certificate,
                };
                match same_address {
                    Some(existing) => {
                        let id = inner.sign_in_again(existing, new).await?;
                        Ok(AddedAccount { id, updated: true })
                    }
                    None => {
                        // Starting an account does blocking work: not on a runtime worker.
                        let inner = inner.clone();
                        let id = tokio::task::spawn_blocking(move || inner.insert_account(new))
                            .await
                            .map_err(|_| CoreError::Internal)??;
                        Ok(AddedAccount { id, updated: false })
                    }
                }
            },
            false,
        );
        task.await?
    }

    /// Undoes the last `add_account` that **updated** the account (`updated` was true), when the
    /// app could not store the new credential: the earlier settings and pin are written back and
    /// the account starts again with the credential the app still has stored (asking the credential
    /// provider, as at `open`). Does nothing when there is nothing to undo, for a new account, and
    /// once the account has changed since. Call it right away, from the code that stored (or failed
    /// to store) the credential.
    pub async fn rollback_account_update(&self, id: i64) -> Result<(), CoreError> {
        let inner = self.inner.clone();
        let task = self.spawn(
            async move {
                let _changing = inner.account_changes.lock().await;
                let previous = inner.previous().remove(&id);
                match previous {
                    Some(previous) => inner.restore(id, previous).await,
                    None => Ok(()),
                }
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
                // Held until the account is started (see `Inner::account_changes`).
                let _adding = inner.account_changes.lock().await;
                let blocking = inner.clone();
                // Starting an account does blocking work: not on a runtime worker.
                tokio::task::spawn_blocking(move || add_demo(&blocking))
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
                // Never between an `add_demo_account`'s insert and its start.
                let _adding = inner.account_changes.lock().await;
                inner.engine.remove_account(id).await;
                inner.backends().remove(&id);
                inner.demos().remove(&id);
                inner.previous().remove(&id);
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

/// The body of `add_demo_account`, on a blocking thread with `Inner::account_changes` held: returns
/// the sample mailbox, adding it if there is none and starting it if it is not running (its start
/// failed at `open`, say).
fn add_demo(inner: &Inner) -> Result<AccountInfo, CoreError> {
    let existing = inner.handle.read(|db| {
        Ok::<_, CoreError>(
            accounts::list(db.connection())?
                .into_iter()
                .find(|a| is_demo(&a.config, &a.email))
                .map(|a| a.id),
        )
    })?;
    let id = match existing {
        Some(id) if inner.is_demo_running(id) => id,
        Some(id) => {
            inner.start_stored_demo(id)?;
            inner.hub.set_broken(id, false);
            inner.hub.accounts_changed([id]);
            id
        }
        None => {
            let id = inner.handle.read(|db| {
                let conn = db.connection();
                let position = i64::try_from(accounts::list(conn)?.len()).unwrap_or(0);
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
            if let Err(e) = inner.start_demo_account(id, DemoBackend::new(now_ms())) {
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
