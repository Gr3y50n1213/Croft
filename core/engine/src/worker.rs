// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! One account's task: the wake-up loop, running due operations, and following a message after a
//! move (design note section 4). Folder sync is in `sync.rs`.

use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use croft_mail_core::{AccountId, AwaitingUid, Op, OpId, OpKind, Rejection};
use croft_protocols::{BackendError, FlagChange, MailBackend, Uid};
use croft_storage::{folders, messages, StorageError};

use crate::engine::{AccountSlot, Change, EngineError, Queue, Shared};

/// A message moved without a known new UID is looked for by Message-ID this many times (once
/// straight after the move, then at each later sync of the destination) before its later
/// operations are given up.
pub const MAX_LOOKUPS: u32 = 3;

/// Shown when the later operations on a moved message are given up because it could not be found
/// in its new folder.
pub const REASON_LOST_AFTER_MOVE: &str = "Couldn't find the message after moving it";

/// Shown when operations on a message are given up because its folder was renumbered on the
/// server (UIDVALIDITY changed) and the message could not be found again.
pub const REASON_LOST_AFTER_RESET: &str =
    "Couldn't find the message after the folder changed on the server";

/// Shown when a destructive operation (delete forever, or a move to Trash or Junk) on a message
/// that was moved is given up because the destination folder holds more than one copy and the
/// engine cannot tell which one is the message.
pub const REASON_AMBIGUOUS_COPY: &str =
    "Couldn't tell which copy of the message to use, so nothing was deleted";

/// What a cycle did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Cycle {
    /// Operations ran (and folders synced, when asked).
    Ran,
    /// Nothing could run: the account is offline or waiting for a sign-in.
    Skipped,
}

/// The work of one account. Cheap to build; the account task keeps one, and
/// [`crate::EngineHandle::run_once`] builds one per call.
pub(crate) struct Worker {
    pub(crate) shared: Arc<Shared>,
    pub(crate) slot: Arc<AccountSlot>,
}

impl Worker {
    pub(crate) fn new(shared: Arc<Shared>, slot: Arc<AccountSlot>) -> Self {
        Self { shared, slot }
    }

    pub(crate) fn account(&self) -> AccountId {
        self.slot.account
    }

    pub(crate) fn backend(&self) -> &dyn MailBackend {
        &*self.slot.backend
    }

    /// True when the cycle should stop at the next step: shutting down, or the server refused
    /// the sign-in.
    pub(crate) fn stop_now(&self) -> bool {
        self.slot.stopping() || self.slot.needs_sign_in()
    }

    /// True when nothing may run: offline, or waiting for the person to sign in.
    fn blocked(&self) -> bool {
        if self.slot.needs_sign_in() {
            return true;
        }
        let q = self.shared.lock();
        q.queue().is_offline(self.account()) || q.queue().is_paused(self.account())
    }

    /// Turns a server error into an engine error, and pauses the account on a sign-in failure.
    pub(crate) fn backend_error(&self, e: BackendError) -> EngineError {
        if matches!(e, BackendError::Auth(_)) {
            self.sign_in_needed();
        }
        EngineError::Backend(e)
    }

    fn sign_in_needed(&self) {
        if !self.slot.needs_sign_in.swap(true, Ordering::SeqCst) {
            self.shared.emit(Change::SignInNeeded {
                account: self.account(),
            });
        }
    }

    pub(crate) fn emit_failed(&self, failed: &[OpId], reason: &str) {
        for op in failed {
            self.shared.emit(Change::OperationFailed {
                account: self.account(),
                op: *op,
                reason: reason.to_owned(),
            });
        }
    }

    /// The account task. Each turn runs a cycle, then sleeps until the earliest of: the next due
    /// operation (`next_wake_ms_for`), the next periodic sync, or a poke (`wake`: every UI action,
    /// `sync_now`, `set_online`, shutdown). While offline or waiting for a sign-in it only waits
    /// for a poke.
    ///
    /// Extension point for IDLE: `MailBackend` has no push call yet. When it gains one (IMAP IDLE
    /// on the Inbox, JMAP push), add its future as a third branch of the `select!` below and set
    /// `full_sync_wanted` (or sync just that folder) when it fires.
    pub(crate) async fn run_loop(self) {
        let interval = self.shared.config.sync_interval_ms;
        let retry = self.shared.config.sync_retry_ms;
        let mut next_full = i64::MIN;
        // After a database error nothing is tried before this time (see `database_delay_ms`).
        let mut hold_until = i64::MIN;
        let mut database_errors: u32 = 0;
        loop {
            if self.slot.stopping() {
                break;
            }
            let now = self.shared.now();
            let wanted = self.slot.full_sync_wanted.swap(false, Ordering::SeqCst);
            let full = wanted || now >= next_full;
            match self.cycle(full).await {
                Ok(cycle) => {
                    database_errors = 0;
                    hold_until = i64::MIN;
                    if cycle == Cycle::Ran && full {
                        next_full = now.saturating_add(interval);
                    }
                }
                Err(e) => {
                    log::warn!(
                        "account {} cycle failed: {}",
                        self.account(),
                        error_class(&e)
                    );
                    if full {
                        next_full = now.saturating_add(retry);
                    }
                    // A due operation would make the account wake at once, try, fail and wake
                    // again: a loop that does nothing but burn the CPU and the log. A database
                    // that refuses needs time, so wait, and wait longer each time it still does.
                    if matches!(e, EngineError::Storage(_)) {
                        database_errors = database_errors.saturating_add(1);
                        hold_until = self
                            .shared
                            .now()
                            .saturating_add(database_delay_ms(retry, database_errors));
                    }
                }
            }
            if self.slot.stopping() {
                break;
            }
            match self.wait_ms(next_full, hold_until) {
                Some(ms) => {
                    tokio::select! {
                        () = self.slot.wake.notified() => {}
                        () = tokio::time::sleep(Duration::from_millis(ms)) => {}
                    }
                }
                None => self.slot.wake.notified().await,
            }
        }
    }

    /// How long to sleep before the next turn, or `None` to wait for a poke only. Never ends
    /// before `hold_until` (the wait after a database error).
    fn wait_ms(&self, next_full: i64, hold_until: i64) -> Option<u64> {
        if self.blocked() {
            return None;
        }
        let next_op = self.shared.lock().queue().next_wake_ms_for(self.account());
        let target = next_op
            .map_or(next_full, |t| t.min(next_full))
            .max(hold_until);
        Some(u64::try_from(target.saturating_sub(self.shared.now())).unwrap_or(0))
    }

    /// One cycle: due operations, then (with `full`) every folder and the operations that sync
    /// made due. Holds the account's turn for the whole cycle.
    pub(crate) async fn cycle(&self, full: bool) -> Result<Cycle, EngineError> {
        let _turn = self.slot.turn.lock().await;
        if self.blocked() {
            return Ok(Cycle::Skipped);
        }
        // Holding the turn means nothing of this account is being sent: an operation still marked
        // applying was left by a step that failed (or by a task that was aborted) and must go back.
        self.shared.lock().recover_applying(self.account())?;
        if full {
            self.shared.lock().purge_finished()?;
        }
        self.run_due_ops().await?;
        if full && !self.stop_now() {
            self.sync_all().await?;
            self.run_due_ops().await?;
        }
        Ok(Cycle::Ran)
    }

    /// Runs this account's due operations one at a time, oldest first, until none is due.
    async fn run_due_ops(&self) -> Result<(), EngineError> {
        loop {
            if self.stop_now() {
                return Ok(());
            }
            let now = self.shared.now();
            let due = self.shared.lock().take_due_for(self.account(), now, 1)?;
            let Some(op) = due.into_iter().next() else {
                return Ok(());
            };
            self.apply(op).await?;
        }
    }

    /// Checks the operation's folders, then sends the operation and records the answer. If a step
    /// fails (the database refused to record an answer, say) the operation goes back in the queue:
    /// it was handed out as applying, and nothing else would ever hand it out again.
    async fn apply(&self, op: Op) -> Result<(), EngineError> {
        let result = match self.check_folders(op).await {
            Ok(Some(op)) => self.send(op).await,
            Ok(None) => Ok(()),
            Err(e) => Err(e),
        };
        if result.is_err() {
            self.shared.recover_in_flight(self.account());
        }
        result
    }

    /// A UID only means something together with its folder's UIDVALIDITY. Before an operation is
    /// sent, the current UIDVALIDITY of its folder (and, for a move, of the destination) is read;
    /// if it is not the one stored at the last sync, that folder is synced first and the
    /// operation continues as the queue now has it.
    ///
    /// * Source folder: the reset matches the operation again by Message-ID, or fails it.
    ///   Without this, an operation queued before the server renumbered the folder would be sent
    ///   to whatever message has its old UID now.
    /// * Destination of a move: the new UID in the server's answer is in the new numbering. With
    ///   the destination's rows still in the old one, the moved copy would be written over the
    ///   stale row that has the same UID, and later operations would follow the wrong message.
    ///
    /// Costs one status request per folder per operation. Returns `None` when the operation is
    /// no longer to be sent (it failed, or it was handed back for a retry).
    async fn check_folders(&self, op: Op) -> Result<Option<Op>, EngineError> {
        let account = self.account();
        let mut to_check = vec![op.kind.target().0.to_owned()];
        if let OpKind::MoveTo { dest, .. } = &op.kind {
            to_check.push(dest.clone());
        }
        let mut synced = false;
        for folder in &to_check {
            let Some(stored) = self.stored_uid_validity(folder)? else {
                continue;
            };
            let status = match self.backend().folder_status(folder).await {
                Ok(status) => status,
                Err(e) => {
                    self.rejected(&op, e)?;
                    return Ok(None);
                }
            };
            if i64::from(status.uid_validity) == stored {
                continue;
            }
            match self.sync_folder(folder).await {
                Ok(()) => synced = true,
                Err(EngineError::Backend(e)) => {
                    self.rejected(&op, e)?;
                    return Ok(None);
                }
                Err(e) => return Err(e),
            }
            if self.stored_uid_validity(folder)? != Some(i64::from(status.uid_validity)) {
                // The reset did not happen (an action changed things meanwhile): try again shortly.
                let again = BackendError::Transient("the folder changed on the server".into());
                self.rejected(&op, again)?;
                return Ok(None);
            }
        }
        if !synced {
            return Ok(Some(op));
        }
        let now = self.shared.lock().queue().get(op.id).cloned();
        Ok(now.filter(|o| o.account == account && o.state == croft_mail_core::OpState::Applying))
    }

    fn stored_uid_validity(&self, folder: &str) -> Result<Option<i64>, StorageError> {
        let q = self.shared.lock();
        Ok(
            folders::find_by_path(q.store().db().connection(), self.account(), folder)?
                .and_then(|f| f.uid_validity),
        )
    }

    /// The server (or the folder check) refused the operation: retry, fail or pause, as the
    /// queue decides.
    fn rejected(&self, op: &Op, e: BackendError) -> Result<(), EngineError> {
        let now = self.shared.now();
        let decision = self.shared.lock().reject(op.id, &e, now)?;
        match decision {
            Some(Rejection::Failed) => {
                let reason = self
                    .shared
                    .lock()
                    .queue()
                    .get(op.id)
                    .and_then(|o| o.last_error.clone())
                    .unwrap_or_else(|| e.to_string());
                self.emit_failed(&[op.id], &reason);
            }
            Some(Rejection::AccountNeedsAttention) => self.sign_in_needed(),
            Some(Rejection::RetryAt(_)) | None => {}
        }
        self.shared
            .emit_folders(self.account(), &[op.kind.target().0]);
        Ok(())
    }

    /// Sends one operation and records the answer. No lock is held during the network call.
    async fn send(&self, op: Op) -> Result<(), EngineError> {
        let account = self.account();
        let backend = self.backend();
        // Ok(None): done. Ok(Some(new)): a move, with the new UID if the server gave one.
        let outcome: Result<Option<Option<Uid>>, BackendError> = match &op.kind {
            OpKind::SetSeen { folder, uid, value } => backend
                .set_flag(folder, &[*uid], FlagChange::Seen(*value))
                .await
                .map(|()| None),
            OpKind::SetFlagged { folder, uid, value } => backend
                .set_flag(folder, &[*uid], FlagChange::Flagged(*value))
                .await
                .map(|()| None),
            OpKind::MoveTo { folder, uid, dest } => backend
                .move_to(folder, &[*uid], dest)
                .await
                .map(|out| Some(out.dest_uids.get(uid).copied())),
            OpKind::DeleteForever { folder, uid } => {
                backend.delete_forever(folder, &[*uid]).await.map(|()| None)
            }
        };
        let (folder, uid) = op.kind.target();
        let dest = match &op.kind {
            OpKind::MoveTo { dest, .. } => Some(dest.as_str()),
            _ => None,
        };
        let now = self.shared.now();
        match outcome {
            Ok(None) => self.shared.lock().confirm(op.id)?,
            Ok(Some(new_uid)) => {
                let dest = dest.unwrap_or_default();
                match new_uid {
                    // Design section 4, case 1: the copy is made and later operations follow it.
                    Some(new_uid) => self.shared.lock().confirm_moved_with(
                        op.id,
                        Some((dest.to_owned(), new_uid)),
                        None,
                        now,
                    )?,
                    // Case 2: keep the Message-ID with the awaiting mark and look for it now.
                    None => {
                        let mut q = self.shared.lock();
                        let message_id = message_id_at(&q, account, folder, uid)?;
                        q.confirm_moved_with(op.id, None, message_id, now)?;
                    }
                }
                self.shared.emit_folders(account, &[folder, dest]);
                let waiting = self
                    .shared
                    .lock()
                    .queue()
                    .awaiting_for(account, (folder, uid))
                    .cloned();
                if let Some(entry) = waiting {
                    match self.resolve_awaiting(entry).await {
                        // The move is confirmed; the lookup is tried again at the next sync.
                        Err(EngineError::Backend(BackendError::Transient(_))) => {}
                        other => other?,
                    }
                }
                return Ok(());
            }
            Err(e) => return self.rejected(&op, e),
        }
        self.shared.emit_folders(account, &[folder]);
        Ok(())
    }

    /// Looks for a moved message by Message-ID in its destination (design section 4, case 2).
    /// Found: later operations follow it and the row is copied there. Not found: counts the
    /// lookup, and after [`MAX_LOOKUPS`] (or at once when there is no Message-ID) gives the later
    /// operations up. A temporary server problem does not count as a lookup.
    pub(crate) async fn resolve_awaiting(&self, entry: AwaitingUid) -> Result<(), EngineError> {
        let account = self.account();
        let from = (entry.folder.as_str(), entry.uid);
        let found = match &entry.message_id {
            None => None,
            Some(id) => match self
                .backend()
                .find_by_message_id(&entry.dest_folder, id)
                .await
            {
                Ok(found) => found,
                // The folder is gone or refuses searches: as good as not found.
                Err(BackendError::Permanent(_)) => None,
                Err(e) => return Err(self.backend_error(e)),
            },
        };
        let mut failed = Vec::new();
        let mut reason = REASON_LOST_AFTER_MOVE;
        {
            let mut q = self.shared.lock();
            if q.queue().awaiting_for(account, from).is_none() {
                return Ok(());
            }
            match found {
                // A destructive operation waits for this message and the destination holds
                // another copy of its Message-ID: the UID found may be the other copy. Never
                // guess which one to delete (see the crate documentation for the limits).
                Some(new_uid) if self.is_ambiguous(&q, from, &entry, new_uid)? => {
                    reason = REASON_AMBIGUOUS_COPY;
                    failed = q.abandon(account, from, REASON_AMBIGUOUS_COPY)?;
                }
                Some(new_uid) => {
                    q.repoint(account, from, (entry.dest_folder.clone(), new_uid))?;
                }
                None => {
                    let lookups = match entry.message_id {
                        Some(_) => q.record_lookup(account, from)?.unwrap_or(MAX_LOOKUPS),
                        None => MAX_LOOKUPS,
                    };
                    if lookups >= MAX_LOOKUPS {
                        failed = q.abandon(account, from, REASON_LOST_AFTER_MOVE)?;
                    }
                }
            }
        }
        self.emit_failed(&failed, reason);
        self.shared
            .emit_folders(account, &[&entry.folder, &entry.dest_folder]);
        Ok(())
    }

    /// True when an unfinished operation on the message at `at` destroys something: delete
    /// forever, or a move into Trash or Junk.
    pub(crate) fn has_destructive_op(
        &self,
        q: &Queue,
        at: (&str, Uid),
    ) -> Result<bool, StorageError> {
        let account = self.account();
        for op in q
            .queue()
            .iter()
            .filter(|o| o.account == account && !o.state.is_finished() && o.kind.target() == at)
        {
            match &op.kind {
                OpKind::DeleteForever { .. } => return Ok(true),
                OpKind::MoveTo { dest, .. } => {
                    let found = folders::find_by_path(q.store().db().connection(), account, dest)?;
                    if found.is_some_and(|f| matches!(f.special_use.as_str(), "trash" | "junk")) {
                        return Ok(true);
                    }
                }
                _ => {}
            }
        }
        Ok(false)
    }

    /// True when the folder has a local row, other than the one at `except`, with this
    /// Message-ID: the lookup's one answer may be the wrong one of two copies. The server's own
    /// duplicates cannot be seen this way (`find_by_message_id` returns one UID), so this is only
    /// as good as the last sync of the folder.
    pub(crate) fn other_copies(
        &self,
        folder: &str,
        message_id: &str,
        except: Uid,
    ) -> Result<bool, StorageError> {
        let account = self.account();
        let q = self.shared.lock();
        let conn = q.store().db().connection();
        let Some(f) = folders::find_by_path(conn, account, folder)? else {
            return Ok(false);
        };
        Ok(messages::count_other_copies(conn, account, f.id, message_id, except)? > 0)
    }

    /// True when the lookup of an awaiting entry must not be trusted: a destructive operation
    /// waits on the message and the destination has another local copy of its Message-ID.
    fn is_ambiguous(
        &self,
        q: &Queue,
        from: (&str, Uid),
        entry: &AwaitingUid,
        found: Uid,
    ) -> Result<bool, StorageError> {
        let (Some(id), true) = (&entry.message_id, self.has_destructive_op(q, from)?) else {
            return Ok(false);
        };
        let account = self.account();
        let conn = q.store().db().connection();
        let Some(f) = folders::find_by_path(conn, account, &entry.dest_folder)? else {
            return Ok(false);
        };
        Ok(messages::count_other_copies(conn, account, f.id, id, found)? > 0)
    }

    /// Resolves every awaiting entry of this account whose destination is `dest` (`None`: every
    /// entry whose destination is not in `except`).
    pub(crate) async fn resolve_awaiting_into(
        &self,
        dest: Option<&str>,
        except: &[String],
    ) -> Result<(), EngineError> {
        let account = self.account();
        let entries: Vec<AwaitingUid> = self
            .shared
            .lock()
            .queue()
            .awaiting()
            .filter(|a| a.account == account)
            .filter(|a| match dest {
                Some(d) => a.dest_folder == d,
                None => !except.contains(&a.dest_folder),
            })
            .cloned()
            .collect();
        for entry in entries {
            if self.stop_now() {
                break;
            }
            match self.resolve_awaiting(entry).await {
                Ok(()) => {}
                // Not counted; tried again at the next sync.
                Err(EngineError::Backend(BackendError::Transient(_))) => {}
                Err(e) => return Err(e),
            }
        }
        Ok(())
    }
}

/// How long the account task waits after its `n`th database error in a row: the sync retry time,
/// doubled each time, at most 16 times that.
fn database_delay_ms(retry_ms: i64, n: u32) -> i64 {
    retry_ms
        .max(1)
        .saturating_mul(1 << n.saturating_sub(1).min(4))
}

/// The Message-ID of the local row at (folder path, UID), if the row exists and has one.
pub(crate) fn message_id_at(
    q: &Queue,
    account: AccountId,
    folder: &str,
    uid: Uid,
) -> Result<Option<String>, StorageError> {
    let conn = q.store().db().connection();
    let Some(f) = folders::find_by_path(conn, account, folder)? else {
        return Ok(None);
    };
    Ok(messages::find_by_uid(conn, account, f.id, uid)?.and_then(|m| m.message_id_hdr))
}

/// A log-safe word for an error (no server text, no paths).
pub(crate) fn error_class(e: &EngineError) -> &'static str {
    match e {
        EngineError::Storage(_) => "database error",
        EngineError::Backend(BackendError::Transient(_)) => "temporary server or network problem",
        EngineError::Backend(BackendError::Auth(_)) => "sign-in refused",
        EngineError::Backend(BackendError::Permanent(_)) => "server refused",
        EngineError::UnknownAccount(_) => "unknown account",
        EngineError::AccountExists(_) => "account exists",
        EngineError::NoRuntime => "no runtime",
    }
}
