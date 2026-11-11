// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The Outbox in the account task: handing due messages to the server, then saving their copies
//! in Sent (`docs/tasks/T06-smtp-outbox.md`, Part B).
//!
//! The Outbox is `croft_mail_core::PersistentOutbox`; it has its own connection and lock (see
//! [`crate::Engine::open_with_outbox`]). The lock is held only to take a message or record the
//! answer, never across the network call.
//!
//! # Never twice
//!
//! [`PersistentOutbox::take_due`] writes `sending` to disk before it returns the message. If the
//! process dies from then on, the next start fails the message with "May not have been sent;
//! check Sent before retrying" and it is not sent again. If only the future is dropped (the task
//! is aborted, or a `run_once` caller gives up) while the process lives, [`InFlight`] does the same
//! in memory by calling `abandon`. The engine never cancels a send on its own: shutdown waits for
//! it (see `Engine::join`).
//!
//! # One account's task, one account's mail
//!
//! The task asks the Outbox for its own account's messages only (`take_due_for`,
//! `take_filing_for`) and, as a second guard, refuses a message whose account is not its own. Its
//! wake time is the Outbox's own (`next_wake_ms_for`), which follows the same pause, offline and
//! back-off rules as the takes, so the task never wakes at a time where nothing can be handed out.

use std::sync::atomic::Ordering;

use croft_mail_core::{
    AccountId, DueMessage, FilingRejection, OutboxEntry, OutboxId, OutboxState, SendRejection,
};
use croft_protocols::{BackendError, MailSubmitter};
use croft_storage::StorageError;

use crate::engine::{AccountSlot, Change, EngineError, Outbox, Shared};
use crate::mime;
use crate::worker::Worker;

/// Recorded when the Outbox handed an account's task a message of another account. It cannot
/// happen through the per-account takes; if it ever does, the message is failed with this fixed
/// text (no address, no content) instead of being sent from the wrong account.
pub(crate) const REASON_WRONG_ACCOUNT: &str =
    "This message was not sent: it was handed to the wrong account";

/// What a guard does if it is dropped before the answer is recorded.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Step {
    /// Handing the message to the server.
    Send,
    /// Saving the copy in Sent.
    File,
}

/// Held while a message is out of the Outbox's hands. If the future that owns it is dropped, the
/// message is not left in Sending or Filing for ever: a send that may have reached the server is
/// Failed with "may not have been sent", a copy that was being saved goes back to wait.
struct InFlight<'a> {
    shared: &'a Shared,
    slot: &'a AccountSlot,
    account: AccountId,
    id: OutboxId,
    step: Step,
    armed: bool,
}

impl<'a> InFlight<'a> {
    /// A [`Step::Send`] guard also marks the account as sending (so shutdown waits for the
    /// answer); the mark is cleared when the guard is dropped, however that happens.
    fn new(shared: &'a Shared, slot: &'a AccountSlot, id: OutboxId, step: Step) -> Self {
        if step == Step::Send {
            slot.sending.store(true, Ordering::SeqCst);
        }
        Self {
            shared,
            slot,
            account: slot.account,
            id,
            step,
            armed: true,
        }
    }

    /// The answer is recorded: nothing to undo.
    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for InFlight<'_> {
    fn drop(&mut self) {
        // First, whatever happens next: nothing of this account is being sent any more, so a
        // shutdown must not wait for it.
        if self.step == Step::Send {
            self.slot.sending.store(false, Ordering::SeqCst);
        }
        if !self.armed {
            return;
        }
        let now = self.shared.now();
        let done = {
            let Some(mut ob) = self.shared.outbox() else {
                return;
            };
            match self.step {
                Step::Send => ob.abandon(self.account, self.id).is_ok(),
                // Saving the copy again is safe: the Sent folder is searched for the Message-ID
                // first. The retry counts as an attempt.
                Step::File => ob
                    .filing_failed(
                        self.id,
                        &BackendError::Transient("interrupted".to_owned()),
                        now,
                    )
                    .is_ok(),
            }
        };
        if !done {
            log::warn!(
                "account {}: could not record an interrupted outbox step",
                self.account
            );
            return;
        }
        self.shared.emit(Change::Outbox {
            account: self.account,
        });
        if self.step == Step::Send {
            self.shared.emit(Change::SendFailed {
                account: self.account,
                id: self.id,
            });
        }
    }
}

impl Worker {
    /// True when this account can send and the engine has an Outbox.
    fn outbox_ready(&self) -> bool {
        self.slot.submitter().is_some() && self.shared.has_outbox()
    }

    /// When this account's task should next wake to send or save a copy, or `None` when nothing is
    /// waiting. It is the Outbox's own answer ([`croft_mail_core::Outbox::next_wake_ms_for`]), so it
    /// is `None` while the Outbox has the account paused (waiting for a sign-in) or offline, and
    /// never earlier than what `take_due_for` would hand out: a task that woke at a time where
    /// nothing is handed out would spin.
    pub(crate) fn outbox_wake_ms(&self) -> Option<i64> {
        if !self.outbox_ready() {
            return None;
        }
        self.shared
            .outbox()?
            .outbox()
            .next_wake_ms_for(self.account())
    }

    /// Sends every message of this account that is due, one at a time, then saves the copies in
    /// Sent. Stops early when the engine is stopping or the account needs the person to sign in.
    pub(crate) async fn run_outbox(&self) -> Result<(), EngineError> {
        let Some(submitter) = self.slot.submitter() else {
            return Ok(());
        };
        if !self.shared.has_outbox() {
            return Ok(());
        }
        self.finish_self_filed(&*submitter)?;
        loop {
            if self.stop_now() {
                return Ok(());
            }
            let Some(due) = self.take(|ob, account, now| ob.take_due_for(account, now, 1))? else {
                break;
            };
            self.deliver(&*submitter, due).await?;
        }
        loop {
            if self.stop_now() {
                return Ok(());
            }
            let Some(due) = self.take(|ob, account, now| ob.take_filing_for(account, now, 1))?
            else {
                break;
            };
            self.file(&*submitter, due).await?;
        }
        Ok(())
    }

    /// Takes the next message of this account that `pick` hands out (already marked Sending or
    /// Filing on disk). Messages of this account whose stored bytes could not be read back turn
    /// into Failed inside the take; nobody else reports them, so they are announced here.
    fn take(
        &self,
        pick: impl FnOnce(&mut Outbox, AccountId, i64) -> Result<Vec<DueMessage>, StorageError>,
    ) -> Result<Option<DueMessage>, EngineError> {
        let account = self.account();
        let now = self.shared.now();
        let failed_ids = |ob: &Outbox| -> Vec<OutboxId> {
            ob.outbox()
                .iter()
                .filter(|e| e.account == account && e.state == OutboxState::Failed)
                .map(|e| e.id)
                .collect()
        };
        let (taken, unreadable) = {
            let Some(mut ob) = self.shared.outbox() else {
                return Ok(None);
            };
            let before = failed_ids(&ob);
            let taken = pick(&mut ob, account, now);
            let new: Vec<OutboxId> = failed_ids(&ob)
                .into_iter()
                .filter(|id| !before.contains(id))
                .collect();
            (taken, new)
        };
        if !unreadable.is_empty() {
            self.shared.emit(Change::Outbox { account });
            for id in unreadable {
                self.shared.emit(Change::SendFailed { account, id });
            }
        }
        Ok(taken?.into_iter().next())
    }

    /// For a server that saves the copy in Sent by itself: messages it accepted (also one left
    /// over from a stop before the copy was recorded) are finished without a copy.
    fn finish_self_filed(&self, submitter: &dyn MailSubmitter) -> Result<(), EngineError> {
        if !submitter.files_sent_itself() {
            return Ok(());
        }
        let account = self.account();
        let Some(mut ob) = self.shared.outbox() else {
            return Ok(());
        };
        let waiting: Vec<OutboxId> = ob
            .outbox()
            .iter()
            .filter(|e| e.account == account && e.state == OutboxState::Submitted)
            .map(|e| e.id)
            .collect();
        for id in waiting {
            ob.filed(id)?;
        }
        Ok(())
    }

    /// Hands one message to the server and records the answer. The send is awaited here and never
    /// cancelled by the engine; see the module notes.
    async fn deliver(
        &self,
        submitter: &dyn MailSubmitter,
        due: DueMessage,
    ) -> Result<(), EngineError> {
        let DueMessage { entry, mime } = due;
        if entry.account != self.account() {
            return self.refuse_foreign(&entry, Step::Send);
        }
        let (account, id) = (entry.account, entry.id);
        self.shared.emit(Change::Outbox { account });
        let mut guard = InFlight::new(&self.shared, &self.slot, id, Step::Send);
        let result = submitter
            .send(&entry.envelope_from, &entry.recipients, &mime)
            .await;
        let now = self.shared.now();
        // If recording the answer fails, the guard stays armed and fails the message as "may not
        // have been sent" when this function returns: the message may well have gone out.
        let rejection = match result {
            Ok(()) => {
                let mut ob = self.shared.outbox().ok_or(EngineError::NoOutbox)?;
                if ob.submitted(id, now)? && submitter.files_sent_itself() {
                    ob.filed(id)?;
                }
                None
            }
            Err(e) => {
                let mut ob = self.shared.outbox().ok_or(EngineError::NoOutbox)?;
                ob.failed(id, &e, now)?
            }
        };
        guard.disarm();
        match rejection {
            Some(SendRejection::RetryAt(_)) | None => {}
            Some(SendRejection::Failed) => self.shared.emit(Change::SendFailed { account, id }),
            Some(SendRejection::AccountNeedsAttention) => self.sign_in_needed(),
        }
        self.shared.emit(Change::Outbox { account });
        Ok(())
    }

    /// Saves the copy of a sent message in Sent and records the answer. A copy that cannot be
    /// saved never fails the message: it is retried, and given up on with a notice.
    async fn file(
        &self,
        submitter: &dyn MailSubmitter,
        due: DueMessage,
    ) -> Result<(), EngineError> {
        let DueMessage { entry, mime } = due;
        if entry.account != self.account() {
            return self.refuse_foreign(&entry, Step::File);
        }
        let (account, id) = (entry.account, entry.id);
        let mut guard = InFlight::new(&self.shared, &self.slot, id, Step::File);
        let message_id = mime::message_id(&mime);
        let result = submitter.file_in_sent(message_id.as_deref(), &mime).await;
        let now = self.shared.now();
        let rejection = match result {
            Ok(()) => {
                let mut ob = self.shared.outbox().ok_or(EngineError::NoOutbox)?;
                ob.filed(id)?;
                None
            }
            Err(e) => {
                let mut ob = self.shared.outbox().ok_or(EngineError::NoOutbox)?;
                ob.filing_failed(id, &e, now)?
            }
        };
        guard.disarm();
        match rejection {
            Some(FilingRejection::RetryAt(_)) | None => {}
            Some(FilingRejection::GaveUp) => self.shared.emit(Change::NotFiled { account, id }),
            Some(FilingRejection::AccountNeedsAttention) => self.sign_in_needed(),
        }
        Ok(())
    }

    /// The Outbox handed this task a message of another account (the per-account takes make that
    /// impossible; this is the second guard). Nothing is sent or filed from this account. A message
    /// that was being sent is Failed with a fixed reason (the person decides); a copy that was
    /// being saved goes back to wait for its own account. Only the class of the problem is logged.
    fn refuse_foreign(&self, entry: &OutboxEntry, step: Step) -> Result<(), EngineError> {
        log::error!(
            "account {}: the outbox handed out a message of another account; refused",
            self.account()
        );
        let now = self.shared.now();
        let owner = entry.account;
        {
            let mut ob = self.shared.outbox().ok_or(EngineError::NoOutbox)?;
            match step {
                Step::Send => {
                    ob.failed(
                        entry.id,
                        &BackendError::Permanent(REASON_WRONG_ACCOUNT.to_owned()),
                        now,
                    )?;
                }
                Step::File => {
                    ob.filing_failed(
                        entry.id,
                        &BackendError::Transient("wrong account".to_owned()),
                        now,
                    )?;
                }
            }
        }
        self.shared.emit(Change::Outbox { account: owner });
        if step == Step::Send {
            self.shared.emit(Change::SendFailed {
                account: owner,
                id: entry.id,
            });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicUsize;
    use std::sync::Arc;

    use croft_mail_core::NewMessage;
    use croft_protocols::{
        BackendFuture, Capabilities, FlagChange, FolderStatus, MailBackend, MoveOutcome,
        RemoteFolder, RemoteHeader, RemoteSnapshot, Uid,
    };
    use croft_storage::accounts::{self, NewAccount, Protocol};
    use croft_storage::Db;

    use super::*;
    use crate::{Engine, EngineConfig, ManualClock};

    /// A backend that is never called.
    struct NoBackend;

    fn never<'a, T: Send + 'a>() -> BackendFuture<'a, T> {
        Box::pin(async { Err(BackendError::Permanent("not used".into())) })
    }

    impl MailBackend for NoBackend {
        fn capabilities(&self) -> BackendFuture<'_, Capabilities> {
            never()
        }
        fn list_folders(&self) -> BackendFuture<'_, Vec<RemoteFolder>> {
            never()
        }
        fn folder_status<'a>(&'a self, _: &'a str) -> BackendFuture<'a, FolderStatus> {
            never()
        }
        fn snapshot<'a>(&'a self, _: &'a str) -> BackendFuture<'a, RemoteSnapshot> {
            never()
        }
        fn fetch_headers<'a>(
            &'a self,
            _: &'a str,
            _: &'a [Uid],
        ) -> BackendFuture<'a, Vec<RemoteHeader>> {
            never()
        }
        fn find_by_message_id<'a>(
            &'a self,
            _: &'a str,
            _: &'a str,
        ) -> BackendFuture<'a, Option<Uid>> {
            never()
        }
        fn set_flag<'a>(
            &'a self,
            _: &'a str,
            _: &'a [Uid],
            _: FlagChange,
        ) -> BackendFuture<'a, ()> {
            never()
        }
        fn move_to<'a>(
            &'a self,
            _: &'a str,
            _: &'a [Uid],
            _: &'a str,
        ) -> BackendFuture<'a, MoveOutcome> {
            never()
        }
        fn delete_forever<'a>(&'a self, _: &'a str, _: &'a [Uid]) -> BackendFuture<'a, ()> {
            never()
        }
    }

    /// A submitter that counts what it is asked to do.
    #[derive(Default)]
    struct Counting {
        sends: AtomicUsize,
        copies: AtomicUsize,
    }

    impl MailSubmitter for Counting {
        fn send<'a>(&'a self, _: &'a str, _: &'a [String], _: &'a [u8]) -> BackendFuture<'a, ()> {
            self.sends.fetch_add(1, Ordering::SeqCst);
            Box::pin(async { Ok(()) })
        }
        fn files_sent_itself(&self) -> bool {
            false
        }
        fn file_in_sent<'a>(&'a self, _: Option<&'a str>, _: &'a [u8]) -> BackendFuture<'a, ()> {
            self.copies.fetch_add(1, Ordering::SeqCst);
            Box::pin(async { Ok(()) })
        }
    }

    fn account(db: &Db, email: &str) -> AccountId {
        accounts::insert(
            db.connection(),
            &NewAccount {
                label: email.into(),
                email: email.into(),
                protocol: Protocol::Imap,
                provider: "generic".into(),
                config: serde_json::json!({}),
                color: None,
                position: 0,
            },
            0,
        )
        .unwrap()
    }

    fn message(account: AccountId) -> NewMessage {
        NewMessage {
            account,
            identity: None,
            draft: None,
            envelope_from: "a@example.org".into(),
            recipients: vec!["b@example.org".into()],
        }
    }

    /// Two accounts on one engine, a message of the first queued and handed out to its own task.
    struct Two {
        _dir: tempfile::TempDir,
        engine: Engine,
        first: AccountId,
        second_submitter: Arc<Counting>,
        second: Worker,
    }

    fn two_accounts() -> Two {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("croft.db");
        let db = Db::open(&path, None).unwrap();
        let (first, second) = (
            account(&db, "one@example.org"),
            account(&db, "two@example.org"),
        );
        drop(db);
        let engine = Engine::open_with_outbox(
            Db::open(&path, None).unwrap(),
            Db::open(&path, None).unwrap(),
            Arc::new(ManualClock::new(1_000)),
            EngineConfig::default(),
        )
        .unwrap();
        let second_submitter = Arc::new(Counting::default());
        engine
            .attach_account_with_submitter(
                first,
                Arc::new(NoBackend),
                Arc::new(Counting::default()),
            )
            .unwrap();
        engine
            .attach_account_with_submitter(second, Arc::new(NoBackend), second_submitter.clone())
            .unwrap();
        let shared = engine.shared();
        let slot = shared.slot(second).unwrap();
        Two {
            _dir: dir,
            engine,
            first,
            second_submitter,
            second: Worker::new(shared, slot),
        }
    }

    #[tokio::test]
    async fn a_message_of_another_account_is_failed_with_a_fixed_reason_and_never_sent() {
        let t = two_accounts();
        let shared = t.engine.shared();
        let (id, due) = {
            let mut ob = shared.outbox().unwrap();
            let id = ob
                .queue(
                    message(t.first),
                    b"Message-ID: <a@b>\r\n\r\nx".to_vec(),
                    0,
                    0,
                )
                .unwrap();
            // The global take, which a task never uses, hands the message out.
            let due = ob.take_due(1_000, 1).unwrap().pop().unwrap();
            (id, due)
        };
        t.second.deliver(&*t.second_submitter, due).await.unwrap();
        assert_eq!(t.second_submitter.sends.load(Ordering::SeqCst), 0);
        let ob = shared.outbox().unwrap();
        let entry = ob.outbox().get(id).unwrap();
        assert_eq!(entry.state, OutboxState::Failed);
        assert_eq!(entry.last_error.as_deref(), Some(REASON_WRONG_ACCOUNT));
    }

    #[tokio::test]
    async fn a_copy_of_another_account_goes_back_to_wait_and_is_not_filed() {
        let t = two_accounts();
        let shared = t.engine.shared();
        let (id, due) = {
            let mut ob = shared.outbox().unwrap();
            let id = ob
                .queue(
                    message(t.first),
                    b"Message-ID: <a@b>\r\n\r\nx".to_vec(),
                    0,
                    0,
                )
                .unwrap();
            ob.take_due(1_000, 1).unwrap();
            ob.submitted(id, 1_000).unwrap();
            let due = ob.take_filing(1_000, 1).unwrap().pop().unwrap();
            (id, due)
        };
        t.second.file(&*t.second_submitter, due).await.unwrap();
        assert_eq!(t.second_submitter.copies.load(Ordering::SeqCst), 0);
        let ob = shared.outbox().unwrap();
        assert_eq!(ob.outbox().get(id).unwrap().state, OutboxState::Submitted);
    }
}
