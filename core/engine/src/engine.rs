// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The engine itself: the shared state, the per-account tasks, and starting and stopping them.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use croft_mail_core::{AccountId, OpId, OutboxId, PersistentOutbox, PersistentQueue};
use croft_protocols::{BackendError, MailBackend, MailSubmitter};
use croft_storage::{folders, Db, SqliteOpStore, SqliteOutboxStore, StorageError};
use tokio::sync::{broadcast, Notify};
use tokio::task::JoinHandle;

use crate::clock::Clock;
use crate::handle::EngineHandle;
use crate::worker::Worker;

/// The queue and the database, behind the one lock (see the crate documentation).
pub(crate) type Queue = PersistentQueue<SqliteOpStore<Db>>;

/// The Outbox and its own connection to the database file (see the crate documentation).
pub(crate) type Outbox = PersistentOutbox<SqliteOutboxStore<Db>>;

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
    /// The account's Outbox changed (a message was queued, taken back, started sending, was
    /// sent, failed or went back to wait for a retry): re-run the query behind
    /// [`EngineHandle::outbox`].
    Outbox {
        /// The account.
        account: AccountId,
    },
    /// A message could not be sent and is in the Outbox as Failed, or it may not have been sent
    /// (the app stopped while it was going out, or the connection dropped after the message body).
    /// The screen tells the person once; the reason is `last_error` of the entry in
    /// [`EngineHandle::outbox`], and for "may not have been sent" the person should look in Sent
    /// before choosing Retry.
    SendFailed {
        /// The account.
        account: AccountId,
        /// The Outbox message.
        id: OutboxId,
    },
    /// A message was sent but its copy could not be saved in the Sent folder, and the engine gave
    /// up on the copy. The message itself is not in the Outbox any more. The screen says "Sent,
    /// but couldn't save a copy" once.
    NotFiled {
        /// The account.
        account: AccountId,
        /// The Outbox message that was sent.
        id: OutboxId,
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
    /// The engine was opened with [`Engine::open`] and has no Outbox.
    NoOutbox,
}

impl fmt::Display for EngineError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Storage(e) => write!(f, "database error: {e}"),
            Self::Backend(e) => e.fmt(f),
            Self::UnknownAccount(a) => write!(f, "no backend for account {a}"),
            Self::AccountExists(a) => write!(f, "account {a} already has a backend"),
            Self::NoRuntime => f.write_str("no tokio runtime to run the account task on"),
            Self::NoOutbox => f.write_str("the engine was opened without an outbox"),
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
    /// Sends the account's mail and files the copy in Sent. `None`: the account cannot send.
    /// Replaced by [`EngineHandle::replace_submitter`]; read with [`AccountSlot::submitter`].
    submitter: Mutex<Option<Arc<dyn MailSubmitter>>>,
    /// True while a message is being handed to the server. [`Engine::shutdown`] and
    /// [`Engine::remove_account`] then keep waiting instead of cancelling the send. Set and
    /// cleared only by the guard in `outbox.rs`, so it is cleared however the send ends,
    /// including when its future is dropped.
    pub(crate) sending: AtomicBool,
    /// How many turns the account's task has taken (diagnostics: a task that wakes without
    /// anything to do shows up as a number that keeps growing).
    pub(crate) turns_taken: AtomicU64,
    /// Poked by every UI action, `sync_now`, `set_online` and shutdown.
    pub(crate) wake: Notify,
    /// Held for a whole cycle, so an account never runs two operations at once even when
    /// `run_once` is called while the task is running. One per account id in [`Shared`], shared
    /// by every slot the account has over time, so a task started after a remove and re-add waits
    /// for a `run_once` of the old slot that is still in the middle of a send.
    pub(crate) turn: Arc<tokio::sync::Mutex<()>>,
    pub(crate) full_sync_wanted: AtomicBool,
    pub(crate) needs_sign_in: AtomicBool,
    pub(crate) stopping: AtomicBool,
}

impl AccountSlot {
    /// The account's current submitter, if it has one.
    pub(crate) fn submitter(&self) -> Option<Arc<dyn MailSubmitter>> {
        self.submitter
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(crate) fn set_submitter(&self, submitter: Arc<dyn MailSubmitter>) {
        *self
            .submitter
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(submitter);
    }

    pub(crate) fn stopping(&self) -> bool {
        self.stopping.load(Ordering::SeqCst)
    }

    pub(crate) fn needs_sign_in(&self) -> bool {
        self.needs_sign_in.load(Ordering::SeqCst)
    }

    pub(crate) fn sending(&self) -> bool {
        self.sending.load(Ordering::SeqCst)
    }
}

/// Everything the account tasks and the handles share.
/// A weak handle on one message's body lock (see `Shared::body_lock`).
type BodyLock = std::sync::Weak<tokio::sync::Mutex<()>>;

pub(crate) struct Shared {
    queue: Mutex<Queue>,
    /// The Outbox, on its own connection and behind its own lock; `None` when the engine was
    /// opened with [`Engine::open`]. Never held together with `queue`, never across an `.await`.
    outbox: Option<Mutex<Outbox>>,
    /// Messages found Sending at open (now Failed, "may not have been sent") whose account has not
    /// been announced yet: (account, id).
    recovered: Mutex<Vec<(AccountId, OutboxId)>>,
    clock: Arc<dyn Clock>,
    changes: broadcast::Sender<Change>,
    accounts: Mutex<BTreeMap<AccountId, Arc<AccountSlot>>>,
    /// The per-account turns, kept for the life of the engine (one small value per account id) so
    /// that they survive `remove_account` and a later `add_account`.
    turns: Mutex<BTreeMap<AccountId, Arc<tokio::sync::Mutex<()>>>>,
    /// One lock per message whose body is being fetched or rendered (round 3 review L3: two
    /// opens of one message fetch and sanitise it once). Entries whose lock nobody holds are
    /// dropped as new ones come.
    bodies: Mutex<BTreeMap<(AccountId, i64), BodyLock>>,
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

    /// The Outbox, or `None` when this engine has none. Held only for short synchronous sections,
    /// never across an `.await`, and never at the same time as [`Shared::lock`].
    pub(crate) fn outbox(&self) -> Option<MutexGuard<'_, Outbox>> {
        self.outbox
            .as_ref()
            .map(|m| m.lock().unwrap_or_else(PoisonError::into_inner))
    }

    /// True when the engine was opened with an Outbox.
    pub(crate) fn has_outbox(&self) -> bool {
        self.outbox.is_some()
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

    /// The account's turn: the same one for every slot the account ever has.
    fn turn(&self, account: AccountId) -> Arc<tokio::sync::Mutex<()>> {
        self.turns
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .entry(account)
            .or_default()
            .clone()
    }

    /// The lock for fetching and rendering one message's body (see `bodies`).
    pub(crate) fn body_lock(
        &self,
        account: AccountId,
        message: i64,
    ) -> Arc<tokio::sync::Mutex<()>> {
        let mut bodies = self.bodies.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(lock) = bodies
            .get(&(account, message))
            .and_then(std::sync::Weak::upgrade)
        {
            return lock;
        }
        bodies.retain(|_, weak| weak.strong_count() > 0);
        let lock = Arc::new(tokio::sync::Mutex::new(()));
        bodies.insert((account, message), Arc::downgrade(&lock));
        lock
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

    /// Puts the account's operations that are marked applying back in the queue. For when nothing
    /// of the account is being sent any more: its task was aborted in the middle of a call, or a
    /// step failed after an operation was handed out. A failure to write is logged only: the next
    /// cycle of the account does the same before it hands anything out.
    pub(crate) fn recover_in_flight(&self, account: AccountId) {
        if self.lock().recover_applying(account).is_err() {
            log::warn!(
                "account {account}: could not put an interrupted operation back in the queue"
            );
        }
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
    ///
    /// An engine opened this way has no Outbox: [`EngineHandle::send`] refuses with
    /// [`crate::SendError::NoOutbox`]. Use [`Engine::open_with_outbox`] to be able to send.
    pub fn open(db: Db, clock: Arc<dyn Clock>, config: EngineConfig) -> Result<Self, EngineError> {
        Self::build(db, None, clock, config)
    }

    /// Like [`Engine::open`], and also opens the Outbox on `outbox_db`: a **second connection to the
    /// same database file** (`Db::open` with the same path and key). Why a second connection:
    /// the queue owns the first one, and the Outbox store needs a `&mut Db` of its own for every
    /// call, so two stores cannot share one `Db` without a lock around the whole database that
    /// the screens' reads would also have to take. WAL lets two connections of one process write
    /// in turn (each write is a short transaction; a busy wait is at most a few milliseconds),
    /// and the Outbox has its own lock, so the queue's lock is never held while an Outbox
    /// message is written. `outbox_db` must be a file database: an in-memory one is a different
    /// database and has none of the account rows the Outbox refers to.
    ///
    /// Messages that were being sent when the app stopped are Failed with "May not have been sent;
    /// check Sent before retrying" at this point, never sent again. Each is announced once, as a
    /// [`Change::SendFailed`], when its account is added ([`Engine::add_account_with_submitter`]),
    /// so subscribe before adding the accounts.
    ///
    /// Reads and writes the database, so call it from a worker thread, not the UI thread.
    pub fn open_with_outbox(
        db: Db,
        outbox_db: Db,
        clock: Arc<dyn Clock>,
        config: EngineConfig,
    ) -> Result<Self, EngineError> {
        Self::build(db, Some(outbox_db), clock, config)
    }

    fn build(
        db: Db,
        outbox_db: Option<Db>,
        clock: Arc<dyn Clock>,
        config: EngineConfig,
    ) -> Result<Self, EngineError> {
        let queue = PersistentQueue::open(SqliteOpStore::new(db))?;
        let outbox = outbox_db
            .map(|db| PersistentOutbox::open(SqliteOutboxStore::new(db)))
            .transpose()?;
        let recovered = outbox
            .as_ref()
            .map(|o| {
                o.recovered_after_restart()
                    .iter()
                    .filter_map(|id| o.outbox().get(*id).map(|e| (e.account, e.id)))
                    .collect()
            })
            .unwrap_or_default();
        let (changes, _) = broadcast::channel(config.change_capacity.max(1));
        let shared = Arc::new(Shared {
            queue: Mutex::new(queue),
            outbox: outbox.map(Mutex::new),
            recovered: Mutex::new(recovered),
            clock,
            changes,
            accounts: Mutex::new(BTreeMap::new()),
            turns: Mutex::new(BTreeMap::new()),
            bodies: Mutex::new(BTreeMap::new()),
            undo_window_ms: AtomicI64::new(config.undo_window_ms.max(0)),
            config,
        });
        Ok(Self {
            shared,
            tasks: Mutex::new(BTreeMap::new()),
        })
    }

    /// The shared state, for unit tests that drive a [`Worker`] directly.
    #[cfg(test)]
    pub(crate) fn shared(&self) -> Arc<Shared> {
        self.shared.clone()
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
        submitter: Option<Arc<dyn MailSubmitter>>,
    ) -> Result<Arc<AccountSlot>, EngineError> {
        // An account that was removed and added back starts from what the Outbox knows: if a
        // sign-in failure paused its outgoing mail, the new slot also waits for the person to sign
        // in (`EngineHandle::signed_in`). Without this the slot would think everything is fine
        // while the Outbox hands out nothing. (The Outbox lock is taken and let go before the
        // slots' lock: they are never held together.)
        let outbox_paused = self
            .shared
            .outbox()
            .is_some_and(|ob| ob.outbox().is_paused(account));
        let mut slots = self.shared.slots();
        if slots.contains_key(&account) {
            return Err(EngineError::AccountExists(account));
        }
        let slot = Arc::new(AccountSlot {
            account,
            backend,
            submitter: Mutex::new(submitter),
            sending: AtomicBool::new(false),
            turns_taken: AtomicU64::new(0),
            wake: Notify::new(),
            turn: self.shared.turn(account),
            full_sync_wanted: AtomicBool::new(true),
            needs_sign_in: AtomicBool::new(outbox_paused),
            stopping: AtomicBool::new(false),
        });
        slots.insert(account, slot.clone());
        drop(slots);
        self.announce_recovered(account);
        if outbox_paused {
            self.shared.emit(Change::SignInNeeded { account });
        }
        Ok(slot)
    }

    /// Announces, once, the messages of this account that were found Sending at open.
    fn announce_recovered(&self, account: AccountId) {
        let ids: Vec<OutboxId> = {
            let mut pending = self
                .shared
                .recovered
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            let mine = pending
                .iter()
                .filter(|(a, _)| *a == account)
                .map(|(_, id)| *id)
                .collect();
            pending.retain(|(a, _)| *a != account);
            mine
        };
        if ids.is_empty() {
            return;
        }
        self.shared.emit(Change::Outbox { account });
        for id in ids {
            self.shared.emit(Change::SendFailed { account, id });
        }
    }

    /// Adds an account's backend and starts its task on the current tokio runtime. An account that
    /// was removed after a refused sign-in while sending is added back in the same state: it
    /// announces [`Change::SignInNeeded`] again and does nothing until [`EngineHandle::signed_in`]
    /// (to give it a new password without removing it, use [`EngineHandle::replace_submitter`]). The task
    /// syncs every folder first, then runs operations as they become due, syncs again every
    /// [`EngineConfig::sync_interval_ms`], and wakes at once on every UI action.
    ///
    /// The backend is built by the caller, so credentials never pass through the engine.
    pub fn add_account(
        &self,
        account: AccountId,
        backend: Arc<dyn MailBackend>,
    ) -> Result<(), EngineError> {
        self.add_account_inner(account, backend, None)
    }

    /// Like [`Engine::add_account`], for an account that can also send mail. `submitter` hands
    /// messages to the server and files the copy in Sent; the account's task runs the Outbox with
    /// it. Like the backend it is built by the caller, so credentials never pass through the
    /// engine.
    pub fn add_account_with_submitter(
        &self,
        account: AccountId,
        backend: Arc<dyn MailBackend>,
        submitter: Arc<dyn MailSubmitter>,
    ) -> Result<(), EngineError> {
        self.add_account_inner(account, backend, Some(submitter))
    }

    fn add_account_inner(
        &self,
        account: AccountId,
        backend: Arc<dyn MailBackend>,
        submitter: Option<Arc<dyn MailSubmitter>>,
    ) -> Result<(), EngineError> {
        let runtime = tokio::runtime::Handle::try_current().map_err(|_| EngineError::NoRuntime)?;
        let slot = self.insert_slot(account, backend, submitter)?;
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
        self.insert_slot(account, backend, None).map(|_| ())
    }

    /// Like [`Engine::attach_account`], for an account that can also send mail (see
    /// [`Engine::add_account_with_submitter`]).
    pub fn attach_account_with_submitter(
        &self,
        account: AccountId,
        backend: Arc<dyn MailBackend>,
        submitter: Arc<dyn MailSubmitter>,
    ) -> Result<(), EngineError> {
        self.insert_slot(account, backend, Some(submitter))
            .map(|_| ())
    }

    fn tasks_lock(&self) -> MutexGuard<'_, BTreeMap<AccountId, JoinHandle<()>>> {
        self.tasks.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Stops an account's task (after its current step) and forgets its backend. Its queued
    /// operations stay stored and run again once it is added back.
    ///
    /// A task that was stopped in the middle of a network call (see
    /// [`EngineConfig::shutdown_grace_ms`]) may have left an operation marked "applying". It is put
    /// back in the queue here, so that it is handed out again when the account is added back in
    /// this same process (after a restart this happens anyway).
    pub async fn remove_account(&self, account: AccountId) {
        let slot = self.shared.slots().remove(&account);
        let task = self.tasks_lock().remove(&account);
        if let Some(slot) = &slot {
            stop(slot);
        }
        if let Some(task) = task {
            self.join(task, slot.as_deref()).await;
        }
        if let Some(slot) = slot {
            self.recover_in_flight(&slot);
        }
    }

    /// After an account was deleted (its Outbox rows went with it): forgets its Outbox messages
    /// held in memory, so none stays listed or due until the next restart. Not for an account
    /// that is only stopped and started again ([`Engine::remove_account`]): its messages stay.
    pub fn forget_deleted_account(&self, account: AccountId) {
        if let Some(mut ob) = self.shared.outbox() {
            ob.forget_account(account);
        }
    }

    /// Puts the operations of an account that was stopped back in the queue, unless a
    /// [`EngineHandle::run_once`] still holds the account's turn (it settles its own operation).
    fn recover_in_flight(&self, slot: &AccountSlot) {
        if let Ok(_turn) = slot.turn.try_lock() {
            self.shared.recover_in_flight(slot.account);
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
        let tasks: Vec<(AccountId, JoinHandle<()>)> = std::mem::take(&mut *self.tasks_lock())
            .into_iter()
            .collect();
        for (account, task) in tasks {
            let slot = slots.iter().find(|s| s.account == account);
            self.join(task, slot.map(|s| &**s)).await;
        }
        for slot in &slots {
            self.recover_in_flight(slot);
        }
        self.shared.slots().clear();
    }

    /// Waits for an account task. After the grace period a task in the middle of a network call
    /// is stopped, **except** one that is handing a message to the server: cancelling that could
    /// leave a message the server took marked as not sent, so the wait goes on (the submitter's
    /// own deadline ends it).
    async fn join(&self, mut task: JoinHandle<()>, slot: Option<&AccountSlot>) {
        let grace = Duration::from_millis(self.shared.config.shutdown_grace_ms);
        loop {
            if tokio::time::timeout(grace, &mut task).await.is_ok() {
                return;
            }
            if !slot.is_some_and(AccountSlot::sending) {
                break;
            }
            log::warn!("waiting for a message that is being sent");
        }
        log::warn!("an account task did not stop in time and was aborted");
        task.abort();
        let _ = task.await;
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
