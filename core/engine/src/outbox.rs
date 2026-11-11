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

use croft_mail_core::{
    AccountId, DueMessage, FilingRejection, OutboxId, OutboxState, SendRejection,
};
use croft_protocols::{BackendError, MailSubmitter};
use croft_storage::StorageError;

use crate::engine::{Change, EngineError, Outbox, Shared};
use crate::mime;
use crate::worker::Worker;

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
    account: AccountId,
    id: OutboxId,
    step: Step,
    armed: bool,
}

impl<'a> InFlight<'a> {
    fn new(shared: &'a Shared, account: AccountId, id: OutboxId, step: Step) -> Self {
        Self {
            shared,
            account,
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

/// Runs `f` on the Outbox with every other account's mail switched off (offline), so that
/// `take_due` and `take_filing`, which look at all accounts, only hand out this account's
/// messages. The flags are put back before the lock is let go. Also returns the ids that `f`
/// turned into Failed (stored messages that could not be read back), which nobody else reports.
fn only_account<T>(
    ob: &mut Outbox,
    account: AccountId,
    f: impl FnOnce(&mut Outbox) -> T,
) -> (T, Vec<OutboxId>) {
    let failed_ids = |ob: &Outbox| -> Vec<OutboxId> {
        ob.outbox()
            .iter()
            .filter(|e| e.account == account && e.state == OutboxState::Failed)
            .map(|e| e.id)
            .collect()
    };
    let before = failed_ids(ob);
    let mut others: Vec<AccountId> = ob
        .outbox()
        .iter()
        .map(|e| e.account)
        .filter(|a| *a != account && !ob.outbox().is_offline(*a))
        .collect();
    others.sort_unstable();
    others.dedup();
    for other in &others {
        ob.set_offline(*other, true);
    }
    let out = f(ob);
    for other in &others {
        ob.set_offline(*other, false);
    }
    let new = failed_ids(ob)
        .into_iter()
        .filter(|id| !before.contains(id))
        .collect();
    (out, new)
}

impl Worker {
    /// True when this account can send and the engine has an Outbox.
    fn outbox_ready(&self) -> bool {
        self.slot.submitter.is_some() && self.shared.has_outbox()
    }

    /// When this account's task should next wake to send or save a copy, or `None` when nothing is
    /// waiting. Uses the entries' own times and the holds the task recorded after transient
    /// failures, so it is never earlier than what `take_due` would hand out (a task that woke at
    /// a time where nothing is handed out would spin).
    pub(crate) fn outbox_wake_ms(&self) -> Option<i64> {
        if !self.outbox_ready() {
            return None;
        }
        let account = self.account();
        let send_hold = self
            .slot
            .send_hold_ms
            .load(std::sync::atomic::Ordering::SeqCst);
        let file_hold = self
            .slot
            .file_hold_ms
            .load(std::sync::atomic::Ordering::SeqCst);
        let ob = self.shared.outbox()?;
        ob.outbox()
            .iter()
            .filter(|e| e.account == account)
            .filter_map(|e| match e.state {
                OutboxState::Queued => Some(e.send_at_ms.max(send_hold)),
                OutboxState::Submitted => Some(e.send_at_ms.max(file_hold)),
                _ => None,
            })
            .min()
    }

    /// Sends every message of this account that is due, one at a time, then saves the copies in
    /// Sent. Stops early when the engine is stopping or the account needs the person to sign in.
    pub(crate) async fn run_outbox(&self) -> Result<(), EngineError> {
        let Some(submitter) = self.slot.submitter.clone() else {
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
            let Some(due) = self.take(|ob, now| ob.take_due(now, 1))? else {
                break;
            };
            self.deliver(&*submitter, due).await?;
        }
        loop {
            if self.stop_now() {
                return Ok(());
            }
            let Some(due) = self.take(|ob, now| ob.take_filing(now, 1))? else {
                break;
            };
            self.file(&*submitter, due).await?;
        }
        Ok(())
    }

    /// Takes the next message of this account that `pick` hands out (already marked Sending or
    /// Filing on disk).
    fn take(
        &self,
        pick: impl FnOnce(&mut Outbox, i64) -> Result<Vec<DueMessage>, StorageError>,
    ) -> Result<Option<DueMessage>, EngineError> {
        let account = self.account();
        let now = self.shared.now();
        let (taken, unreadable) = {
            let Some(mut ob) = self.shared.outbox() else {
                return Ok(None);
            };
            only_account(&mut ob, account, |ob| pick(ob, now))
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
        let (account, id) = (entry.account, entry.id);
        self.shared.emit(Change::Outbox { account });
        let mut guard = InFlight::new(&self.shared, account, id, Step::Send);
        self.slot
            .sending
            .store(true, std::sync::atomic::Ordering::SeqCst);
        let result = submitter
            .send(&entry.envelope_from, &entry.recipients, &mime)
            .await;
        self.slot
            .sending
            .store(false, std::sync::atomic::Ordering::SeqCst);
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
            Some(SendRejection::RetryAt(at)) => {
                self.slot
                    .send_hold_ms
                    .fetch_max(at, std::sync::atomic::Ordering::SeqCst);
            }
            Some(SendRejection::Failed) => self.shared.emit(Change::SendFailed { account, id }),
            Some(SendRejection::AccountNeedsAttention) => self.sign_in_needed(),
            None => {}
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
        let (account, id) = (entry.account, entry.id);
        let mut guard = InFlight::new(&self.shared, account, id, Step::File);
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
            Some(FilingRejection::RetryAt(at)) => {
                self.slot
                    .file_hold_ms
                    .fetch_max(at, std::sync::atomic::Ordering::SeqCst);
            }
            Some(FilingRejection::GaveUp) => self.shared.emit(Change::NotFiled { account, id }),
            Some(FilingRejection::AccountNeedsAttention) => self.sign_in_needed(),
            None => {}
        }
        Ok(())
    }
}
