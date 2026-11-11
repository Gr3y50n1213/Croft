// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The engine itself: the shared state, the per-account tasks, and starting and stopping them.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use croft_mail_core::{AccountId, OpId, PersistentQueue};
use croft_protocols::{BackendError, MailBackend};
use croft_storage::{folders, Db, SqliteOpStore, StorageError};
use tokio::sync::{broadcast, Notify};
use tokio::task::JoinHandle;

use crate::clock::Clock;
use crate::handle::EngineHandle;
use crate::worker::Worker;

/// The queue and the database, behind the one lock (see the crate documentation).
pub(crate) type Queue = PersistentQueue<SqliteOpStore<Db>>;

/// Settings for [`Engine::open`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EngineConfig {
    /// The person's Undo window for moves and deletes, in milliseconds (3, 5 or 10 s). Can be
    /// changed later with [`EngineHandle::set_undo_window_ms`].
    pub undo_window_ms: i64,
    /// How often an account syncs every folder when nothing else wakes it.
    pub sync_interval_ms: i64,
    /// How soon a full sync that failed (no connection, server busy) is tried again.
    pub sync_retry_ms: i64,
    /// How many headers are fetched in one request, newest first.
    pub fetch_chunk: usize,
    /// How many [`Change`]s a slow subscriber may fall behind before it is told it lagged (and
    /// should re-run all its queries).
    pub change_capacity: usize,
    /// How long [`Engine::shutdown`] waits for an account task that is in the middle of a
    /// network call before it stops it anyway.
    pub shutdown_grace_ms: u64,
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            undo_window_ms: 5_000,
            sync_interval_ms: 15 * 60 * 1_000,
            sync_retry_ms: 60_000,
            fetch_chunk: 100,
            change_capacity: 256,
            shutdown_grace_ms: 5_000,
        }
    }
}

/// Something the screens should react to. Sent on the channel from [`EngineHandle::subscribe`].
///
/// A subscriber that lags (the channel reports `Lagged`) should re-run all of its queries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Change {
    /// Message rows or folder counters changed in these folders (ids from the `folder` table):
    /// re-run the queries that read them.
    Messages {
        /// The account.
        account: AccountId,
        /// Folder ids, without duplicates.
        folders: Vec<i64>,
    },
    /// The account's folder list changed.
    Folders {
        /// The account.
        account: AccountId,
    },
    /// An operation failed and its row was put back (or, for a message that could not be found
    /// after a move, it reappears with the server's values at the next sync). The screen shows a
    /// snackbar such as "Couldn't archive".
    OperationFailed {
        /// The account.
        account: AccountId,
        /// The operation.
        op: OpId,
        /// A plain reason, safe to show: no message content, no addresses.
        reason: String,
    },
    /// The server refused the sign-in. The account does nothing until
    /// [`EngineHandle::signed_in`].
    SignInNeeded {
        /// The account.
        account: AccountId,
    },
}

/// Why an engine call failed. Never contains message content, passwords or addresses.
#[derive(Debug)]
pub enum EngineError {
    /// The database failed.
    Storage(StorageError),
    /// The server failed (only from [`EngineHandle::run_once`]; the account task logs these and
    /// tries again later).
    Backend(BackendError),
    /// No backend was added for this account.
    UnknownAccount(AccountId),
    /// A backend was already added for this account.
    AccountExists(AccountId),
    /// [`Engine::add_account`] was called outside a tokio runtime.
    NoRuntime,
}

impl fmt::Display for EngineError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Storage(e) => write!(f, "database error: {e}"),
            Self::Backend(e) => e.fmt(f),
            Self::UnknownAccount(a) => write!(f, "no backend for account {a}"),
            Self::AccountExists(a) => write!(f, "account {a} already has a backend"),
            Self::NoRuntime => f.write_str("no tokio runtime to run the account task on"),
        }
    }
}

impl std::error::Error for EngineError {}

impl From<StorageError> for EngineError {
    fn from(e: StorageError) -> Self {
        Self::Storage(e)
    }
}

/// Per-account state shared by the account task, [`EngineHandle::run_once`] and the UI calls.
pub(crate) struct AccountSlot {
    pub(crate) account: AccountId,
    pub(crate) backend: Arc<dyn MailBackend>,
    /// Poked by every UI action, `sync_now`, `set_online` and shutdown.
    pub(crate) wake: Notify,
    /// Held for a whole cycle, so an account never runs two operations at once even when
    /// `run_once` is called while the task is running.
    pub(crate) turn: tokio::sync::Mutex<()>,
    pub(crate) full_sync_wanted: AtomicBool,
    pub(crate) needs_sign_in: AtomicBool,
    pub(crate) stopping: AtomicBool,
}

impl AccountSlot {
    pub(crate) fn stopping(&self) -> bool {
        self.stopping.load(Ordering::SeqCst)
    }

    pub(crate) fn needs_sign_in(&self) -> bool {
        self.needs_sign_in.load(Ordering::SeqCst)
    }
}

/// Everything the account tasks and the handles share.
pub(crate) struct Shared {
    queue: Mutex<Queue>,
    clock: Arc<dyn Clock>,
    changes: broadcast::Sender<Change>,
    accounts: Mutex<BTreeMap<AccountId, Arc<AccountSlot>>>,
    undo_window_ms: AtomicI64,
    pub(crate) config: EngineConfig,
}

impl Shared {
    /// The queue and database. Held only for short synchronous sections, never across an
    /// `.await` (the guard is not `Send`, so the compiler refuses that in the account task).
    ///
    /// A panic while the lock was held leaves it poisoned; the queue then still matches the disk
    /// (every change is one committed batch or nothing), so the lock is taken over.
    pub(crate) fn lock(&self) -> MutexGuard<'_, Queue> {
        self.queue.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub(crate) fn now(&self) -> i64 {
        self.clock.now_ms()
    }

    pub(crate) fn undo_window_ms(&self) -> i64 {
        self.undo_window_ms.load(Ordering::SeqCst)
    }

    pub(crate) fn set_undo_window_ms(&self, ms: i64) {
        self.undo_window_ms.store(ms.max(0), Ordering::SeqCst);
    }

    pub(crate) fn subscribe(&self) -> broadcast::Receiver<Change> {
        self.changes.subscribe()
    }

    pub(crate) fn emit(&self, change: Change) {
        // No subscriber is fine: nobody is looking.
        let _ = self.changes.send(change);
    }

    fn slots(&self) -> MutexGuard<'_, BTreeMap<AccountId, Arc<AccountSlot>>> {
        self.accounts.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub(crate) fn slot(&self, account: AccountId) -> Option<Arc<AccountSlot>> {
        self.slots().get(&account).cloned()
    }

    /// Wakes the account's task (it runs whatever is due and goes back to sleep).
    pub(crate) fn poke(&self, account: AccountId) {
        if let Some(slot) = self.slot(account) {
            slot.wake.notify_one();
        }
    }

    /// Asks for a sync of every folder at the next wake, and wakes the task.
    pub(crate) fn request_full_sync(&self, account: AccountId) {
        if let Some(slot) = self.slot(account) {
            slot.full_sync_wanted.store(true, Ordering::SeqCst);
            slot.wake.notify_one();
        }
    }

    pub(crate) fn clear_sign_in(&self, account: AccountId) {
        if let Some(slot) = self.slot(account) {
            slot.needs_sign_in.store(false, Ordering::SeqCst);
        }
    }

    /// Folder ids of these paths in the account, for a [`Change::Messages`]. Unknown paths are
    /// left out.
    pub(crate) fn folder_ids(
        q: &Queue,
        account: AccountId,
        paths: &[&str],
    ) -> Result<Vec<i64>, StorageError> {
        let mut ids = Vec::new();
        for path in paths {
            if let Some(f) = folders::find_by_path(q.store().db().connection(), account, path)? {
                if !ids.contains(&f.id) {
                    ids.push(f.id);
                }
            }
        }
        Ok(ids)
    }

    /// Emits a [`Change::Messages`] for these folder paths.
    pub(crate) fn emit_folders(&self, account: AccountId, paths: &[&str]) {
        let ids = Self::folder_ids(&self.lock(), account, paths);
        match ids {
            Ok(ids) if !ids.is_empty() => self.emit(Change::Messages {
                account,
                folders: ids,
            }),
            Ok(_) => {}
            Err(_) => log::warn!("could not read folder ids for a change notification"),
        }
    }
}

/// The sync engine: owns the database, the operation queue and one task per account.
///
/// Create it with [`Engine::open`], add each account's backend with [`Engine::add_account`], give
/// the UI an [`EngineHandle`] from [`Engine::handle`], and stop it with [`Engine::shutdown`].
/// Dropping it without `shutdown` asks every account task to stop at its next step.
pub struct Engine {
    shared: Arc<Shared>,
    tasks: Mutex<BTreeMap<AccountId, JoinHandle<()>>>,
}

impl fmt::Debug for Engine {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Engine")
            .field("config", &self.shared.config)
            .finish_non_exhaustive()
    }
}

impl Engine {
    /// Opens the engine on a migrated database. Loads the stored queue and puts anything that
    /// was in flight when the app stopped back in the queue (every operation is safe to repeat).
    ///
    /// Reads the database, so call it from a worker thread, not the UI thread.
    pub fn open(db: Db, clock: Arc<dyn Clock>, config: EngineConfig) -> Result<Self, EngineError> {
        let queue = PersistentQueue::open(SqliteOpStore::new(db))?;
        let (changes, _) = broadcast::channel(config.change_capacity.max(1));
        let shared = Arc::new(Shared {
            queue: Mutex::new(queue),
            clock,
            changes,
            accounts: Mutex::new(BTreeMap::new()),
            undo_window_ms: AtomicI64::new(config.undo_window_ms.max(0)),
            config,
        });
        Ok(Self {
            shared,
            tasks: Mutex::new(BTreeMap::new()),
        })
    }

    /// The UI's handle: cheap to clone, `Send + Sync`, and none of its action methods waits for
    /// the network.
    pub fn handle(&self) -> EngineHandle {
        EngineHandle::new(self.shared.clone())
    }

    fn insert_slot(
        &self,
        account: AccountId,
        backend: Arc<dyn MailBackend>,
    ) -> Result<Arc<AccountSlot>, EngineError> {
        let mut slots = self.shared.slots();
        if slots.contains_key(&account) {
            return Err(EngineError::AccountExists(account));
        }
        let slot = Arc::new(AccountSlot {
            account,
            backend,
            wake: Notify::new(),
            turn: tokio::sync::Mutex::new(()),
            full_sync_wanted: AtomicBool::new(true),
            needs_sign_in: AtomicBool::new(false),
            stopping: AtomicBool::new(false),
        });
        slots.insert(account, slot.clone());
        Ok(slot)
    }

    /// Adds an account's backend and starts its task on the current tokio runtime. The task
    /// syncs every folder first, then runs operations as they become due, syncs again every
    /// [`EngineConfig::sync_interval_ms`], and wakes at once on every UI action.
    ///
    /// The backend is built by the caller, so credentials never pass through the engine.
    pub fn add_account(
        &self,
        account: AccountId,
        backend: Arc<dyn MailBackend>,
    ) -> Result<(), EngineError> {
        let runtime = tokio::runtime::Handle::try_current().map_err(|_| EngineError::NoRuntime)?;
        let slot = self.insert_slot(account, backend)?;
        let worker = Worker::new(self.shared.clone(), slot);
        let task = runtime.spawn(worker.run_loop());
        self.tasks_lock().insert(account, task);
        Ok(())
    }

    /// Adds an account's backend **without** starting a task. Nothing runs until
    /// [`EngineHandle::run_once`] is called: for a one-shot background job (Android WorkManager)
    /// and for tests that drive the engine step by step.
    pub fn attach_account(
        &self,
        account: AccountId,
        backend: Arc<dyn MailBackend>,
    ) -> Result<(), EngineError> {
        self.insert_slot(account, backend).map(|_| ())
    }

    fn tasks_lock(&self) -> MutexGuard<'_, BTreeMap<AccountId, JoinHandle<()>>> {
        self.tasks.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Stops an account's task (after its current step) and forgets its backend. Its queued
    /// operations stay stored and run again once it is added back.
    pub async fn remove_account(&self, account: AccountId) {
        let slot = self.shared.slots().remove(&account);
        let task = self.tasks_lock().remove(&account);
        if let Some(slot) = slot {
            stop(&slot);
        }
        if let Some(task) = task {
            self.join(task).await;
        }
    }

    /// Stops every account task and waits for them. A task in the middle of a step finishes it
    /// first (so a confirmed operation is stored); one stuck in a network call for longer than
    /// [`EngineConfig::shutdown_grace_ms`] is stopped where it is, and whatever it was doing is
    /// put back in the queue at the next start.
    pub async fn shutdown(self) {
        let slots: Vec<Arc<AccountSlot>> = self.shared.slots().values().cloned().collect();
        for slot in &slots {
            stop(slot);
        }
        let tasks: Vec<JoinHandle<()>> = std::mem::take(&mut *self.tasks_lock())
            .into_values()
            .collect();
        for task in tasks {
            self.join(task).await;
        }
        self.shared.slots().clear();
    }

    async fn join(&self, mut task: JoinHandle<()>) {
        let grace = Duration::from_millis(self.shared.config.shutdown_grace_ms);
        if tokio::time::timeout(grace, &mut task).await.is_err() {
            log::warn!("an account task did not stop in time and was aborted");
            task.abort();
            let _ = task.await;
        }
    }
}

fn stop(slot: &AccountSlot) {
    slot.stopping.store(true, Ordering::SeqCst);
    slot.wake.notify_one();
}

impl Drop for Engine {
    fn drop(&mut self) {
        for slot in self.shared.slots().values() {
            stop(slot);
        }
    }
}
