// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The UI's side of the engine. Every action method is synchronous: it applies the local change
//! and stores the queue row in one SQLite transaction, wakes the account's task, and returns. None
//! of them waits for the network.

use std::fmt;
use std::sync::Arc;

use croft_mail_core::{
    AccountId, CancelError, Enqueued, NewMessage, Op, OpId, OpKind, OutboxEntry, OutboxError,
    OutboxId, QueueError, RemovedMessage, UnreadableOp,
};
use croft_protocols::{MailSubmitter, Uid};
use croft_storage::messages::Message;
use croft_storage::{folders, messages, Db, StorageError};
use tokio::sync::broadcast;

use crate::engine::{Change, EngineError, Shared};
use crate::mime;
use crate::worker::Worker;

/// Why a UI action was refused. Nothing was changed.
#[derive(Debug)]
pub enum ActionError {
    /// No message with this row id in this account.
    NoSuchMessage,
    /// No folder with this path in this account.
    NoSuchFolder,
    /// The message is already in that folder.
    SameFolder,
    /// No such operation (or it finished and was cleaned up).
    NoSuchOperation,
    /// Too late to undo: the operation has started or its window has closed.
    TooLate,
    /// The database failed.
    Storage(StorageError),
}

impl fmt::Display for ActionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoSuchMessage => f.write_str("no such message"),
            Self::NoSuchFolder => f.write_str("no such folder"),
            Self::SameFolder => f.write_str("the message is already in that folder"),
            Self::NoSuchOperation => f.write_str("no such operation"),
            Self::TooLate => f.write_str("too late to undo"),
            Self::Storage(e) => write!(f, "database error: {e}"),
        }
    }
}

impl std::error::Error for ActionError {}

impl From<StorageError> for ActionError {
    fn from(e: StorageError) -> Self {
        Self::Storage(e)
    }
}

/// Why [`EngineHandle::send`] refused. Nothing was stored.
#[derive(Debug)]
pub enum SendError {
    /// The engine was opened without an Outbox ([`crate::Engine::open`]).
    NoOutbox,
    /// No account with this id was added.
    UnknownAccount(AccountId),
    /// The account was added without a submitter, so it cannot send.
    NoSubmitter(AccountId),
    /// The message has a `Bcc:` or `Resent-Bcc:` header. A Bcc recipient belongs in `NewMessage::recipients` only:
    /// the bytes are sent as stored, so a header would show the recipient to everyone else.
    BccHeader,
    /// The message has no `Message-ID:` header. The copy in Sent is recognised by it: without one
    /// the copy cannot be looked up first, and a retry would file a second copy.
    NoMessageId,
    /// The `Message-ID:` has a character that is not ASCII (RFC 5322 allows none), which the
    /// look-up in Sent may not find again.
    MessageIdNotAscii,
    /// The envelope has no recipient.
    NoRecipients,
    /// The envelope sender is empty.
    NoSender,
    /// The database failed.
    Storage(StorageError),
}

impl fmt::Display for SendError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoOutbox => f.write_str("the engine has no outbox"),
            Self::UnknownAccount(a) => write!(f, "no backend for account {a}"),
            Self::NoSubmitter(a) => write!(f, "account {a} cannot send mail"),
            Self::BccHeader => f.write_str("the message has a Bcc header"),
            Self::NoMessageId => f.write_str("the message has no Message-ID"),
            Self::MessageIdNotAscii => f.write_str("the Message-ID is not ASCII"),
            Self::NoRecipients => f.write_str("the message has no recipients"),
            Self::NoSender => f.write_str("the message has no sender"),
            Self::Storage(e) => write!(f, "database error: {e}"),
        }
    }
}

impl std::error::Error for SendError {}

impl From<StorageError> for SendError {
    fn from(e: StorageError) -> Self {
        Self::Storage(e)
    }
}

/// Why an Outbox action (Undo, Retry, Edit, Discard) was refused. Nothing was changed.
#[derive(Debug)]
pub enum OutboxActionError {
    /// The engine was opened without an Outbox ([`crate::Engine::open`]).
    NoOutbox,
    /// No such message in this account (or it is finished and gone).
    NoSuchMessage,
    /// Too late to undo: the Undo window has closed or the message already started to go out.
    TooLate,
    /// The message is in a state that does not allow this: for example it is being sent, or it is
    /// not Failed (Retry).
    WrongState,
    /// The database failed.
    Storage(StorageError),
}

impl fmt::Display for OutboxActionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoOutbox => f.write_str("the engine has no outbox"),
            Self::NoSuchMessage => f.write_str("no such message"),
            Self::TooLate => f.write_str("too late to undo"),
            Self::WrongState => f.write_str("the message is already being sent"),
            Self::Storage(e) => write!(f, "database error: {e}"),
        }
    }
}

impl std::error::Error for OutboxActionError {}

impl From<OutboxError<StorageError>> for OutboxActionError {
    fn from(e: OutboxError<StorageError>) -> Self {
        use croft_mail_core::{ActionError as Action, UndoError};
        match e {
            OutboxError::Undo(UndoError::NotFound) | OutboxError::Action(Action::NotFound) => {
                Self::NoSuchMessage
            }
            OutboxError::Undo(UndoError::TooLate) => Self::TooLate,
            OutboxError::Action(Action::WrongState) => Self::WrongState,
            OutboxError::Store(e) => Self::Storage(e),
        }
    }
}

/// The UI's handle on the engine. Cheap to clone and `Send + Sync`.
///
/// Messages are named by their row id (`message.id`) and always looked up within the given
/// account, so an id from another account is [`ActionError::NoSuchMessage`].
#[derive(Clone)]
pub struct EngineHandle {
    shared: Arc<Shared>,
}

impl fmt::Debug for EngineHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("EngineHandle { .. }")
    }
}

impl EngineHandle {
    pub(crate) fn new(shared: Arc<Shared>) -> Self {
        Self { shared }
    }

    /// Marks a message read or unread. No snackbar: the change is sent after 1.5 s, and a second
    /// swipe inside that time cancels both. Doing nothing is not an error (already in that state).
    pub fn mark_seen(
        &self,
        account: AccountId,
        message: i64,
        seen: bool,
    ) -> Result<(), ActionError> {
        self.act(account, message, |_, row, folder, uid| {
            Ok((row.is_seen != seen).then(|| OpKind::SetSeen {
                folder: folder.to_owned(),
                uid,
                value: seen,
            }))
        })
        .map(|_| ())
    }

    /// Stars or unstars a message. Same timing as [`EngineHandle::mark_seen`].
    pub fn set_flagged(
        &self,
        account: AccountId,
        message: i64,
        flagged: bool,
    ) -> Result<(), ActionError> {
        self.act(account, message, |_, row, folder, uid| {
            Ok((row.is_flagged != flagged).then(|| OpKind::SetFlagged {
                folder: folder.to_owned(),
                uid,
                value: flagged,
            }))
        })
        .map(|_| ())
    }

    /// Moves a message to the folder at `dest_folder_path` (archive, trash, spam and Move to all
    /// use this). The row is hidden at once and kept until the server confirms. Returns the
    /// operation id for the Undo snackbar; the move waits the Undo window before it is sent.
    pub fn move_to(
        &self,
        account: AccountId,
        message: i64,
        dest_folder_path: &str,
    ) -> Result<OpId, ActionError> {
        self.act(account, message, |db, row, folder, uid| {
            let dest = folders::find_by_path(db.connection(), account, dest_folder_path)?
                .ok_or(ActionError::NoSuchFolder)?;
            if dest.id == row.folder_id {
                return Err(ActionError::SameFolder);
            }
            Ok(Some(OpKind::MoveTo {
                folder: folder.to_owned(),
                uid,
                dest: dest.path,
            }))
        })?
        .ok_or(ActionError::NoSuchOperation)
    }

    /// Deletes a message permanently (the screen has already asked). The row is hidden at once
    /// and removed when the server confirms. Returns the operation id for the Undo snackbar.
    pub fn delete_forever(&self, account: AccountId, message: i64) -> Result<OpId, ActionError> {
        self.act(account, message, |_, _, folder, uid| {
            Ok(Some(OpKind::DeleteForever {
                folder: folder.to_owned(),
                uid,
            }))
        })?
        .ok_or(ActionError::NoSuchOperation)
    }

    /// Looks the message up, builds the operation with `make` (or `None` for nothing to do),
    /// and queues it with its local change in one transaction.
    fn act(
        &self,
        account: AccountId,
        message: i64,
        make: impl FnOnce(&Db, &Message, &str, Uid) -> Result<Option<OpKind>, ActionError>,
    ) -> Result<Option<OpId>, ActionError> {
        let now = self.shared.now();
        let undo = self.shared.undo_window_ms();
        let (out, folder_id) = {
            let mut q = self.shared.lock();
            let db = q.store().db();
            let row = messages::get(db.connection(), account, message)?
                .ok_or(ActionError::NoSuchMessage)?;
            let folder = folders::get(db.connection(), account, row.folder_id)?
                .ok_or(ActionError::NoSuchMessage)?;
            let uid = Uid::try_from(row.uid).map_err(|_| ActionError::NoSuchMessage)?;
            let Some(kind) = make(db, &row, &folder.path, uid)? else {
                return Ok(None);
            };
            (q.enqueue(account, kind, now, undo)?, row.folder_id)
        };
        self.shared.emit(Change::Messages {
            account,
            folders: vec![folder_id],
        });
        self.shared.poke(account);
        Ok(match out {
            Enqueued::Queued(id) | Enqueued::Duplicate(id) => Some(id),
            Enqueued::CancelledOut => None,
        })
    }

    /// Undo: cancels an operation that has not started and whose window is still open, and puts
    /// the row back. It never reverses something the server already did.
    ///
    /// Operation ids are global (one counter for all accounts), so this takes no account. The UI
    /// only ever holds ids it received itself, from the call that queued the operation (for the
    /// Undo snackbar): it must not guess or enumerate ids.
    pub fn undo(&self, op: OpId) -> Result<(), ActionError> {
        let now = self.shared.now();
        let (account, folder) = {
            let mut q = self.shared.lock();
            let target = q
                .queue()
                .get(op)
                .map(|o| (o.account, o.kind.target().0.to_owned()))
                .ok_or(ActionError::NoSuchOperation)?;
            q.cancel(op, now).map_err(|e| match e {
                QueueError::Cancel(CancelError::NotFound) => ActionError::NoSuchOperation,
                QueueError::Cancel(CancelError::TooLate) => ActionError::TooLate,
                QueueError::Store(e) => ActionError::Storage(e),
            })?;
            target
        };
        self.shared.emit_folders(account, &[&folder]);
        Ok(())
    }

    /// Sends a message: stores it in the Outbox (message and bytes are on disk when this returns)
    /// and wakes the account's task. Nothing reaches the server before the Undo window has passed
    /// ([`EngineHandle::set_undo_window_ms`]), and nothing while the account is offline: the
    /// message waits and goes out once, when the connection is back. Returns the id for the
    /// "Sending... Undo" snackbar ([`EngineHandle::undo_send`]).
    ///
    /// `message` is the envelope: `envelope_from` is the identity's address and `recipients` are
    /// To, Cc and Bcc as bare addresses. `mime` is the whole RFC 5322 message with a Message-ID
    /// (it is how the copy in Sent is recognised, so a message with none is refused with
    /// [`SendError::NoMessageId`], and one with a non-ASCII Message-ID with
    /// [`SendError::MessageIdNotAscii`]) and **without a `Bcc:` or `Resent-Bcc:` header**: it is
    /// refused with [`SendError::BccHeader`], because the bytes are sent exactly as stored. Lines
    /// are read as the SMTP adapter writes them, so a lone CR does not hide a header. The message
    /// should be 7-bit or UTF-8 text, or the copy in Sent cannot be saved
    /// ([`croft_protocols::submit`]).
    pub fn send(&self, message: NewMessage, mime: Vec<u8>) -> Result<OutboxId, SendError> {
        if mime::has_bcc_header(&mime) {
            return Err(SendError::BccHeader);
        }
        match mime::message_id(&mime) {
            None => return Err(SendError::NoMessageId),
            Some(id) if !id.is_ascii() => return Err(SendError::MessageIdNotAscii),
            Some(_) => {}
        }
        if message.recipients.is_empty() {
            return Err(SendError::NoRecipients);
        }
        if message.envelope_from.trim().is_empty() {
            return Err(SendError::NoSender);
        }
        let account = message.account;
        let slot = self
            .shared
            .slot(account)
            .ok_or(SendError::UnknownAccount(account))?;
        if slot.submitter().is_none() {
            return Err(SendError::NoSubmitter(account));
        }
        let now = self.shared.now();
        let window = self.shared.undo_window_ms();
        let id = {
            let mut ob = self.shared.outbox().ok_or(SendError::NoOutbox)?;
            ob.queue(message, mime, now, window)?
        };
        self.shared.emit(Change::Outbox { account });
        self.shared.poke(account);
        Ok(id)
    }

    /// Undo send: takes the message back if it is still waiting and its window is open, so
    /// nothing is ever sent, and returns it (with its bytes) to reopen the composer.
    /// [`OutboxActionError::TooLate`] once the window has closed.
    pub fn undo_send(
        &self,
        account: AccountId,
        id: OutboxId,
    ) -> Result<RemovedMessage, OutboxActionError> {
        let now = self.shared.now();
        let removed = {
            let mut ob = self.shared.outbox().ok_or(OutboxActionError::NoOutbox)?;
            ob.undo(account, id, now)?
        };
        self.shared.emit(Change::Outbox { account });
        Ok(removed)
    }

    /// The Outbox screen's rows for the account: Queued, Sending and Failed messages, oldest
    /// first. Empty when the engine has no Outbox. Before offering Retry on an entry for which
    /// [`OutboxEntry::may_have_been_sent`] is true, ask the person to check Sent.
    pub fn outbox(&self, account: AccountId) -> Vec<OutboxEntry> {
        self.shared
            .outbox()
            .map(|ob| ob.outbox().visible(account).into_iter().cloned().collect())
            .unwrap_or_default()
    }

    /// Retry on a Failed message: it is queued again, due now. The attempts start afresh.
    pub fn retry_send(&self, account: AccountId, id: OutboxId) -> Result<(), OutboxActionError> {
        let now = self.shared.now();
        {
            let mut ob = self.shared.outbox().ok_or(OutboxActionError::NoOutbox)?;
            ob.retry(account, id, now)?;
        }
        self.shared.emit(Change::Outbox { account });
        self.shared.poke(account);
        Ok(())
    }

    /// Edit on a Queued or Failed message: takes it out of the Outbox and returns it with its
    /// bytes so the composer can reopen it. A message that is being sent cannot be taken back
    /// ([`OutboxActionError::WrongState`]).
    pub fn edit_back_to_draft(
        &self,
        account: AccountId,
        id: OutboxId,
    ) -> Result<RemovedMessage, OutboxActionError> {
        let removed = {
            let mut ob = self.shared.outbox().ok_or(OutboxActionError::NoOutbox)?;
            ob.edit_back_to_draft(account, id)?
        };
        self.shared.emit(Change::Outbox { account });
        Ok(removed)
    }

    /// Discard on a Queued or Failed message (the screen has asked): drops it and its bytes.
    pub fn discard_send(&self, account: AccountId, id: OutboxId) -> Result<(), OutboxActionError> {
        {
            let mut ob = self.shared.outbox().ok_or(OutboxActionError::NoOutbox)?;
            ob.discard(account, id)?;
        }
        self.shared.emit(Change::Outbox { account });
        Ok(())
    }

    /// Changes notifications: changed folders, failed operations, sign-in needed. Screens re-run
    /// their queries when they get one.
    pub fn subscribe(&self) -> broadcast::Receiver<Change> {
        self.shared.subscribe()
    }

    /// Tells the engine whether the device has a connection for this account (from the
    /// platform's connectivity callbacks). While offline nothing is sent and no attempts are
    /// used up; actions still work locally. Going online sends what waited, in order, and syncs.
    ///
    /// Coming back online **from offline** also ends the account's back-off: a connection that
    /// was lost says nothing about how long the server needs, so operations that were waiting out a
    /// retry delay are tried at once (their attempts stay counted). A repeated "online" while the
    /// account was already online keeps the back-off (the server may still be failing) and only
    /// asks for a sync.
    pub fn set_online(&self, account: AccountId, online: bool) {
        let now = self.shared.now();
        {
            let mut q = self.shared.lock();
            let was_offline = q.queue().is_offline(account);
            q.set_offline(account, !online);
            // Only a return from offline says anything about the server: the platform sends the
            // "connected" callback many times while the device stays connected, and each one must
            // not wipe the back-off of a server that keeps failing.
            if online && was_offline && q.clear_hold(account, now).is_err() {
                log::warn!("account {account}: could not end the back-off");
            }
        }
        // The Outbox follows the same connection state, and a return from offline ends its
        // back-off too (it never shortens an Undo window). Its own lock, taken after the queue's
        // is let go: the two are never held together.
        if let Some(mut ob) = self.shared.outbox() {
            let was_offline = ob.outbox().is_offline(account);
            ob.set_offline(account, !online);
            if online && was_offline && ob.clear_hold(account, now).is_err() {
                log::warn!("account {account}: could not end the outbox back-off");
            }
        }
        if online {
            self.shared.request_full_sync(account);
        }
    }

    /// Gives the account a new [`MailSubmitter`], for example one built with a new password or a
    /// refreshed token, without removing and adding the account. A send that is already in
    /// flight finishes with the submitter it started with; everything after uses the new one.
    /// Works for an account that was added without a submitter too (it can send from then on).
    ///
    /// After a refused sign-in call this first, then [`EngineHandle::signed_in`]: this call does not
    /// resume the account by itself. Replace the backend's credential the same way the caller
    /// built it (the engine has no call for that yet).
    pub fn replace_submitter(
        &self,
        account: AccountId,
        submitter: Arc<dyn MailSubmitter>,
    ) -> Result<(), EngineError> {
        let slot = self
            .shared
            .slot(account)
            .ok_or(EngineError::UnknownAccount(account))?;
        slot.set_submitter(submitter);
        self.shared.poke(account);
        Ok(())
    }

    /// How many turns the account's task has taken so far (it takes one at every wake-up). For
    /// diagnostics and tests: a number that keeps growing while nothing happens is a task that
    /// wakes for nothing. `None` for an unknown account.
    pub fn turns_taken(&self, account: AccountId) -> Option<u64> {
        self.shared
            .slot(account)
            .map(|slot| slot.turns_taken.load(std::sync::atomic::Ordering::Relaxed))
    }

    /// Syncs every folder of the account now (pull to refresh).
    pub fn sync_now(&self, account: AccountId) {
        self.shared.request_full_sync(account);
    }

    /// The person signed in again after [`Change::SignInNeeded`] (the caller has already given the
    /// backend and the submitter the new credential). The account's queue, its Outbox and its
    /// syncs run again. A refused sign-in while sending stops the account's queue too, because
    /// they share one credential.
    pub fn signed_in(&self, account: AccountId) {
        self.shared.lock().resume(account);
        if let Some(mut ob) = self.shared.outbox() {
            ob.resume(account);
        }
        self.shared.clear_sign_in(account);
        self.shared.request_full_sync(account);
    }

    /// Changes the Undo window for moves and deletes made from now on.
    pub fn set_undo_window_ms(&self, ms: i64) {
        self.shared.set_undo_window_ms(ms);
    }

    /// Runs `f` on the database, for the screens' queries (message list, folders). Holds the
    /// engine's lock while `f` runs, so keep it short.
    pub fn read<T>(&self, f: impl FnOnce(&Db) -> T) -> T {
        f(self.shared.lock().store().db())
    }

    /// One operation as the queue has it, while it is unfinished or not yet cleaned up (finished
    /// operations are dropped at the start of each full sync).
    ///
    /// Operation ids are global (one counter for all accounts), so this takes no account. The UI
    /// only ever holds ids it received itself, from the call that queued the operation (for the
    /// Undo snackbar): it must not guess or enumerate ids.
    pub fn operation(&self, op: OpId) -> Option<Op> {
        self.shared.lock().queue().get(op).cloned()
    }

    /// Stored operations that could not be read at start (written by a newer app, or damaged).
    /// They were marked failed and their rows restored. Tell the person, then call
    /// [`EngineHandle::acknowledge_unreadable`].
    pub fn unreadable(&self) -> Vec<UnreadableOp> {
        self.shared.lock().unreadable().to_vec()
    }

    /// The person has been told about these unreadable operations: forget them.
    pub fn acknowledge_unreadable(&self, ids: &[OpId]) -> Result<(), ActionError> {
        Ok(self.shared.lock().acknowledge_unreadable(ids)?)
    }

    /// Runs one cycle of the account now and waits for it: due operations, then (with
    /// `full_sync`) every folder, then operations again. **Waits for the network**: for a
    /// background job or a test, never for the UI thread. If the account's task is running a cycle,
    /// this waits for it to finish first (an account never runs two things at once).
    pub async fn run_once(&self, account: AccountId, full_sync: bool) -> Result<(), EngineError> {
        let slot = self
            .shared
            .slot(account)
            .ok_or(EngineError::UnknownAccount(account))?;
        Worker::new(self.shared.clone(), slot)
            .cycle(full_sync)
            .await
            .map(|_| ())
    }
}
