// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Keeping the Outbox across a kill.
//!
//! [`Outbox`] holds the rules; this module adds persistence and stays free of SQLite and of any
//! clock, in the style of [`crate::persist`]:
//!
//! * [`OutboxStore`] is the storage contract: [`OutboxStore::load`] returns the unfinished
//!   messages, [`OutboxStore::commit`] writes one [`OutboxBatch`] atomically, and
//!   [`OutboxStore::mime`] reads one message's bytes. `croft-storage` implements it over the
//!   `outbox` table.
//! * [`PersistentOutbox`] wraps an `Outbox` and a store. Every method that changes the Outbox
//!   works out the batch and commits it **before returning**. If the commit fails, the in-memory
//!   state is put back as it was, so memory never runs ahead of the disk, and the error is
//!   returned. In particular, [`PersistentOutbox::take_due`] has written `sending` to disk before
//!   it hands the message to the caller: that is what lets a restart know the message may have
//!   gone out.
//!
//! The message bytes are written once, when the message is queued, and read when it is handed out
//! or taken back. The offline flag, pauses and back-off are not stored; the engine sets them again
//! on start.

use std::fmt;

use croft_protocols::BackendError;

use crate::outbox::{
    AccountId, ActionError, FilingRejection, NewMessage, Outbox, OutboxEntry, OutboxId,
    SendRejection, UndoError,
};

/// What changes in one step, written atomically by [`OutboxStore::commit`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OutboxBatch {
    /// New messages with their bytes.
    pub inserted: Vec<(OutboxEntry, Vec<u8>)>,
    /// Messages whose state, attempts, time or reason changed. The bytes are not rewritten.
    pub updated: Vec<OutboxEntry>,
    /// Messages that left the Outbox (undone, edited, discarded, sent): account and id. The store
    /// deletes the row and the bytes with it.
    pub removed: Vec<(AccountId, OutboxId)>,
}

impl OutboxBatch {
    /// True when there is nothing to write.
    pub fn is_empty(&self) -> bool {
        self.inserted.is_empty() && self.updated.is_empty() && self.removed.is_empty()
    }
}

/// What [`OutboxStore::load`] returns.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StoredOutbox {
    /// Unfinished messages, in id order.
    pub entries: Vec<OutboxEntry>,
    /// The largest id the store has ever held, including rows that are gone (a store must keep a
    /// high-water mark, because the largest id still present can shrink). New ids start above.
    pub max_id: OutboxId,
}

/// Where the Outbox is kept. Implemented over SQLite by `croft-storage`; tests use memory.
pub trait OutboxStore {
    /// The store's error. It must not contain message content or full addresses.
    type Error: std::error::Error;

    /// Returns the unfinished messages (not the finished ones, which are history).
    fn load(&mut self) -> Result<StoredOutbox, Self::Error>;

    /// Writes the batch, all or nothing.
    fn commit(&mut self, batch: &OutboxBatch) -> Result<(), Self::Error>;

    /// The bytes of one message, or `None` when there is no such message for that account.
    fn mime(&mut self, account: AccountId, id: OutboxId) -> Result<Option<Vec<u8>>, Self::Error>;
}

/// Why a change to the Outbox did not happen.
#[derive(Debug)]
pub enum OutboxError<E> {
    /// Undo refused.
    Undo(UndoError),
    /// Retry, Edit or Discard refused.
    Action(ActionError),
    /// The store could not write or read; nothing changed.
    Store(E),
}

impl<E: fmt::Display> fmt::Display for OutboxError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Undo(UndoError::NotFound) | Self::Action(ActionError::NotFound) => {
                f.write_str("no such message")
            }
            Self::Undo(UndoError::TooLate) => f.write_str("too late to undo"),
            Self::Action(ActionError::WrongState) => {
                f.write_str("the message is already being sent")
            }
            Self::Store(e) => e.fmt(f),
        }
    }
}

impl<E: std::error::Error + 'static> std::error::Error for OutboxError<E> {}

/// A message handed out for sending or filing, with its bytes.
#[derive(Clone, PartialEq, Eq)]
pub struct DueMessage {
    /// The entry, already in its new state (Sending or Filing).
    pub entry: OutboxEntry,
    /// The whole RFC 5322 message, as stored.
    pub mime: Vec<u8>,
}

impl fmt::Debug for DueMessage {
    /// Never prints the message.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DueMessage")
            .field("id", &self.entry.id)
            .field("state", &self.entry.state)
            .field("mime_bytes", &self.mime.len())
            .finish()
    }
}

/// A message taken out of the Outbox by Undo, Edit or Discard.
#[derive(Clone, PartialEq, Eq)]
pub struct RemovedMessage {
    /// The entry as it was.
    pub entry: OutboxEntry,
    /// The whole message, for reopening a draft. Empty if the stored bytes were already gone.
    pub mime: Vec<u8>,
}

impl fmt::Debug for RemovedMessage {
    /// Never prints the message.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RemovedMessage")
            .field("id", &self.entry.id)
            .field("mime_bytes", &self.mime.len())
            .finish()
    }
}

/// Message bytes read for some ids, and the ids whose bytes could not be found.
type BytesRead = (Vec<(OutboxId, Vec<u8>)>, Vec<OutboxId>);

/// An [`Outbox`] kept in an [`OutboxStore`].
#[derive(Debug)]
pub struct PersistentOutbox<S: OutboxStore> {
    outbox: Outbox,
    store: S,
    recovered: Vec<OutboxId>,
}

impl<S: OutboxStore> PersistentOutbox<S> {
    /// Loads the unfinished messages from `store` and applies the restart rule
    /// ([`Outbox::recover_after_restart`]): a message that was Sending becomes Failed with "May not
    /// have been sent; check Sent before retrying" and is **not** sent again. The result is
    /// written back before this returns.
    pub fn open(mut store: S) -> Result<Self, S::Error> {
        let stored = store.load()?;
        let mut outbox = Outbox::from_parts(stored.entries, stored.max_id);
        let before = outbox.clone();
        let recovered = outbox.recover_after_restart();
        let batch = diff(&before, &outbox, Vec::new());
        if !batch.is_empty() {
            store.commit(&batch)?;
        }
        Ok(Self {
            outbox,
            store,
            recovered,
        })
    }

    /// Ids of the messages that [`PersistentOutbox::open`] moved from Sending to Failed. The
    /// engine tells the person once.
    pub fn recovered_after_restart(&self) -> &[OutboxId] {
        &self.recovered
    }

    /// Read access to the state: `get`, `iter`, `visible`, `next_wake_ms`, `is_paused`, ...
    pub fn outbox(&self) -> &Outbox {
        &self.outbox
    }

    /// The store.
    pub fn store(&self) -> &S {
        &self.store
    }

    /// Gives the store back (for example to reopen it in a test).
    pub fn into_store(self) -> S {
        self.store
    }

    /// Records whether the account has a connection (not stored). See [`Outbox::set_offline`].
    pub fn set_offline(&mut self, account: AccountId, offline: bool) {
        self.outbox.set_offline(account, offline);
    }

    /// Lets an account's outgoing mail run again after a sign-in (not stored). See
    /// [`Outbox::resume`].
    pub fn resume(&mut self, account: AccountId) {
        self.outbox.resume(account);
    }

    /// Runs `change` on the Outbox, then commits what it changed plus the bytes of new messages.
    /// Puts the Outbox back if the commit fails.
    fn run<T>(
        &mut self,
        change: impl FnOnce(&mut Outbox) -> T,
        new_bytes: impl FnOnce(&T) -> Vec<(OutboxId, Vec<u8>)>,
    ) -> Result<T, S::Error> {
        let before = self.outbox.clone();
        let out = change(&mut self.outbox);
        self.outbox.purge_finished();
        let batch = diff(&before, &self.outbox, new_bytes(&out));
        if batch.is_empty() {
            return Ok(out);
        }
        match self.store.commit(&batch) {
            Ok(()) => Ok(out),
            Err(e) => {
                self.outbox = before;
                Err(e)
            }
        }
    }

    /// Queues a message: due when the Undo window ends. The message and its bytes are on disk when
    /// this returns. Keep the id for Undo.
    pub fn queue(
        &mut self,
        message: NewMessage,
        mime: Vec<u8>,
        now_ms: i64,
        undo_window_ms: i64,
    ) -> Result<OutboxId, S::Error> {
        let mut mime = Some(mime);
        self.run(
            |o| o.queue(message, now_ms, undo_window_ms),
            |id| mime.take().map(|m| (*id, m)).into_iter().collect(),
        )
    }

    /// Undo: removes the message if it is still Queued and its time has not come, so nothing
    /// reaches the server, and returns it (with its bytes) so the draft can be reopened.
    pub fn undo(
        &mut self,
        id: OutboxId,
        now_ms: i64,
    ) -> Result<RemovedMessage, OutboxError<S::Error>> {
        let before = self.outbox.clone();
        let entry = self.outbox.undo(id, now_ms).map_err(OutboxError::Undo)?;
        self.finish_removal(before, entry)
    }

    /// Edit: takes a Queued or Failed message out of the Outbox and returns it with its bytes so
    /// it can become a draft again.
    pub fn edit_back_to_draft(
        &mut self,
        id: OutboxId,
    ) -> Result<RemovedMessage, OutboxError<S::Error>> {
        let before = self.outbox.clone();
        let entry = self
            .outbox
            .edit_back_to_draft(id)
            .map_err(OutboxError::Action)?;
        self.finish_removal(before, entry)
    }

    /// Discard: drops a Queued or Failed message and its bytes.
    pub fn discard(&mut self, id: OutboxId) -> Result<OutboxEntry, OutboxError<S::Error>> {
        let before = self.outbox.clone();
        let entry = self.outbox.discard(id).map_err(OutboxError::Action)?;
        let batch = OutboxBatch {
            removed: vec![(entry.account, entry.id)],
            ..OutboxBatch::default()
        };
        match self.store.commit(&batch) {
            Ok(()) => Ok(entry),
            Err(e) => {
                self.outbox = before;
                Err(OutboxError::Store(e))
            }
        }
    }

    /// Reads the bytes of a removed entry, then deletes the row. If either step fails the Outbox
    /// is put back.
    fn finish_removal(
        &mut self,
        before: Outbox,
        entry: OutboxEntry,
    ) -> Result<RemovedMessage, OutboxError<S::Error>> {
        let result = (|| {
            let mime = self
                .store
                .mime(entry.account, entry.id)?
                .unwrap_or_default();
            self.store.commit(&OutboxBatch {
                removed: vec![(entry.account, entry.id)],
                ..OutboxBatch::default()
            })?;
            Ok(mime)
        })();
        match result {
            Ok(mime) => Ok(RemovedMessage { entry, mime }),
            Err(e) => {
                self.outbox = before;
                Err(OutboxError::Store(e))
            }
        }
    }

    /// Hands out up to `limit` messages that are due, with their bytes, oldest first. **Each is
    /// marked Sending on disk before this returns**, so a crash while the caller is submitting it
    /// is recognised at the next start (and the message is not sent again). A message whose bytes
    /// cannot be found is Failed instead of being handed out.
    ///
    /// For each message the caller must report exactly one outcome: [`PersistentOutbox::submitted`]
    /// or [`PersistentOutbox::failed`].
    pub fn take_due(&mut self, now_ms: i64, limit: usize) -> Result<Vec<DueMessage>, S::Error> {
        let before = self.outbox.clone();
        let ids = self.outbox.due(now_ms, limit);
        let (readable, missing) = self.read_all(&ids)?;
        for id in &missing {
            self.outbox.give_up_unreadable(*id);
        }
        let entries = self
            .outbox
            .take(&readable.iter().map(|(id, _)| *id).collect::<Vec<_>>());
        self.commit_or_restore(before, &entries, readable)
    }

    /// Like [`PersistentOutbox::take_due`], for copies waiting to be saved to Sent: hands out up
    /// to `limit` Submitted messages (marked Filing on disk first), with their bytes. Report each
    /// with [`PersistentOutbox::filed`] or [`PersistentOutbox::filing_failed`].
    pub fn take_filing(&mut self, now_ms: i64, limit: usize) -> Result<Vec<DueMessage>, S::Error> {
        let before = self.outbox.clone();
        let ids = self.outbox.filing_due(now_ms, limit);
        let (readable, missing) = self.read_all(&ids)?;
        for id in &missing {
            self.outbox.give_up_unreadable(*id);
        }
        self.outbox.purge_finished();
        let entries = self
            .outbox
            .take_for_filing(&readable.iter().map(|(id, _)| *id).collect::<Vec<_>>());
        self.commit_or_restore(before, &entries, readable)
    }

    fn read_all(&mut self, ids: &[OutboxId]) -> Result<BytesRead, S::Error> {
        let mut readable = Vec::new();
        let mut missing = Vec::new();
        for id in ids {
            let Some(account) = self.outbox.get(*id).map(|e| e.account) else {
                continue;
            };
            match self.store.mime(account, *id)? {
                Some(bytes) => readable.push((*id, bytes)),
                None => missing.push(*id),
            }
        }
        Ok((readable, missing))
    }

    fn commit_or_restore(
        &mut self,
        before: Outbox,
        entries: &[OutboxEntry],
        mut readable: Vec<(OutboxId, Vec<u8>)>,
    ) -> Result<Vec<DueMessage>, S::Error> {
        let batch = diff(&before, &self.outbox, Vec::new());
        if !batch.is_empty() {
            if let Err(e) = self.store.commit(&batch) {
                self.outbox = before;
                return Err(e);
            }
        }
        Ok(entries
            .iter()
            .filter_map(|entry| {
                let at = readable.iter().position(|(id, _)| *id == entry.id)?;
                Some(DueMessage {
                    entry: entry.clone(),
                    mime: readable.swap_remove(at).1,
                })
            })
            .collect())
    }

    /// The server accepted the message: Sending becomes Submitted, on disk. Returns false when the
    /// message is not Sending.
    pub fn submitted(&mut self, id: OutboxId, now_ms: i64) -> Result<bool, S::Error> {
        self.run(|o| o.submitted(id, now_ms), |_| Vec::new())
    }

    /// Sending failed: retry later, give up, or pause the account. See [`Outbox::failed`].
    pub fn failed(
        &mut self,
        id: OutboxId,
        error: &BackendError,
        now_ms: i64,
    ) -> Result<Option<SendRejection>, S::Error> {
        self.run(|o| o.failed(id, error, now_ms), |_| Vec::new())
    }

    /// The copy is in Sent (or the server files it itself): the message is finished and its row
    /// and bytes are removed. Returns false when the message is not Filing or Submitted.
    pub fn filed(&mut self, id: OutboxId) -> Result<bool, S::Error> {
        self.run(|o| o.filed(id), |_| Vec::new())
    }

    /// Saving the copy failed. See [`Outbox::filing_failed`]; when it gives up, the row and bytes
    /// are removed and the caller should tell the person once.
    pub fn filing_failed(
        &mut self,
        id: OutboxId,
        error: &BackendError,
        now_ms: i64,
    ) -> Result<Option<FilingRejection>, S::Error> {
        self.run(|o| o.filing_failed(id, error, now_ms), |_| Vec::new())
    }

    /// Retry on a Failed message. See [`Outbox::retry`].
    pub fn retry(&mut self, id: OutboxId, now_ms: i64) -> Result<(), OutboxError<S::Error>> {
        self.run(|o| o.retry(id, now_ms), |_| Vec::new())
            .map_err(OutboxError::Store)?
            .map_err(OutboxError::Action)
    }
}

/// What changed between two states of the Outbox. New messages need their bytes, which come from
/// `new_bytes`; a new entry without bytes is not written (it cannot happen through the methods
/// above).
fn diff(before: &Outbox, after: &Outbox, mut new_bytes: Vec<(OutboxId, Vec<u8>)>) -> OutboxBatch {
    let mut batch = OutboxBatch::default();
    for entry in after.iter() {
        match before.get(entry.id) {
            None => {
                if let Some(at) = new_bytes.iter().position(|(id, _)| *id == entry.id) {
                    batch
                        .inserted
                        .push((entry.clone(), new_bytes.swap_remove(at).1));
                }
            }
            Some(old) if old != entry => batch.updated.push(entry.clone()),
            Some(_) => {}
        }
    }
    for old in before.iter() {
        if after.get(old.id).is_none() {
            batch.removed.push((old.account, old.id));
        }
    }
    batch
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::outbox::{OutboxState, REASON_MAY_NOT_HAVE_BEEN_SENT};
    use std::collections::BTreeMap;

    #[derive(Debug)]
    struct StoreFailed;

    impl fmt::Display for StoreFailed {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("store failed")
        }
    }

    impl std::error::Error for StoreFailed {}

    #[derive(Debug, Default, Clone)]
    struct MemStore {
        rows: BTreeMap<(AccountId, OutboxId), (OutboxEntry, Vec<u8>)>,
        max_id: OutboxId,
        fail_commits: bool,
    }

    impl OutboxStore for MemStore {
        type Error = StoreFailed;

        fn load(&mut self) -> Result<StoredOutbox, StoreFailed> {
            Ok(StoredOutbox {
                entries: self.rows.values().map(|(e, _)| e.clone()).collect(),
                max_id: self.max_id,
            })
        }

        fn commit(&mut self, batch: &OutboxBatch) -> Result<(), StoreFailed> {
            if self.fail_commits {
                return Err(StoreFailed);
            }
            for (e, m) in &batch.inserted {
                self.max_id = self.max_id.max(e.id);
                self.rows.insert((e.account, e.id), (e.clone(), m.clone()));
            }
            for e in &batch.updated {
                if let Some(row) = self.rows.get_mut(&(e.account, e.id)) {
                    row.0 = e.clone();
                }
            }
            for key in &batch.removed {
                self.rows.remove(key);
            }
            Ok(())
        }

        fn mime(
            &mut self,
            account: AccountId,
            id: OutboxId,
        ) -> Result<Option<Vec<u8>>, StoreFailed> {
            Ok(self.rows.get(&(account, id)).map(|(_, m)| m.clone()))
        }
    }

    const A: AccountId = 1;
    const WINDOW: i64 = 5_000;

    fn new_msg() -> NewMessage {
        NewMessage {
            account: A,
            identity: None,
            envelope_from: "me@example.org".into(),
            recipients: vec!["you@example.org".into()],
        }
    }

    fn open() -> PersistentOutbox<MemStore> {
        PersistentOutbox::open(MemStore::default()).unwrap()
    }

    fn row_state(p: &PersistentOutbox<MemStore>, id: OutboxId) -> Option<OutboxState> {
        p.store().rows.get(&(A, id)).map(|(e, _)| e.state)
    }

    #[test]
    fn queue_writes_the_row_and_the_bytes() {
        let mut p = open();
        let id = p.queue(new_msg(), b"MIME".to_vec(), 0, WINDOW).unwrap();
        let (entry, bytes) = p.store().rows.get(&(A, id)).unwrap();
        assert_eq!(bytes, b"MIME");
        assert_eq!(
            (entry.state, entry.send_at_ms),
            (OutboxState::Queued, WINDOW)
        );
    }

    #[test]
    fn undo_inside_the_window_removes_the_row_and_nothing_is_ever_handed_out() {
        let mut p = open();
        let id = p.queue(new_msg(), b"MIME".to_vec(), 0, WINDOW).unwrap();
        let back = p.undo(id, 1_000).unwrap();
        assert_eq!(back.mime, b"MIME");
        assert!(p.store().rows.is_empty());
        assert!(p.take_due(i64::MAX / 2, 10).unwrap().is_empty());
        assert!(matches!(
            p.undo(id, 1_000),
            Err(OutboxError::Undo(UndoError::NotFound))
        ));
    }

    #[test]
    fn undo_after_the_window_is_refused_and_changes_nothing() {
        let mut p = open();
        let id = p.queue(new_msg(), b"MIME".to_vec(), 0, WINDOW).unwrap();
        assert!(matches!(
            p.undo(id, WINDOW),
            Err(OutboxError::Undo(UndoError::TooLate))
        ));
        assert_eq!(row_state(&p, id), Some(OutboxState::Queued));
    }

    #[test]
    fn sending_is_on_disk_before_the_message_is_handed_out() {
        let mut p = open();
        let id = p.queue(new_msg(), b"MIME".to_vec(), 0, 0).unwrap();
        let due = p.take_due(0, 10).unwrap();
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].mime, b"MIME");
        assert_eq!(due[0].entry.state, OutboxState::Sending);
        // What the caller holds is already what the disk says.
        assert_eq!(row_state(&p, id), Some(OutboxState::Sending));
        assert!(p.take_due(0, 10).unwrap().is_empty());
    }

    #[test]
    fn a_failing_write_hands_nothing_out_and_leaves_the_message_queued() {
        let mut p = open();
        let id = p.queue(new_msg(), b"MIME".to_vec(), 0, 0).unwrap();
        p.store.fail_commits = true;
        assert!(p.take_due(0, 10).is_err());
        assert_eq!(p.outbox().get(id).unwrap().state, OutboxState::Queued);
        assert_eq!(p.outbox().get(id).unwrap().attempts, 0);
        p.store.fail_commits = false;
        assert_eq!(p.take_due(0, 10).unwrap().len(), 1);
    }

    #[test]
    fn a_failing_write_when_queueing_leaves_no_message() {
        let mut p = open();
        p.store.fail_commits = true;
        assert!(p.queue(new_msg(), b"MIME".to_vec(), 0, 0).is_err());
        assert_eq!(p.outbox().iter().count(), 0);
        p.store.fail_commits = false;
        assert_eq!(p.queue(new_msg(), b"MIME".to_vec(), 0, 0).unwrap(), 1);
    }

    #[test]
    fn a_restart_with_a_message_in_sending_fails_it_and_never_resends() {
        let mut p = open();
        let id = p.queue(new_msg(), b"MIME".to_vec(), 0, 0).unwrap();
        p.take_due(0, 10).unwrap();
        // The app is killed here. Reopen from what the store holds.
        let store = p.into_store();
        let mut p = PersistentOutbox::open(store).unwrap();
        assert_eq!(p.recovered_after_restart(), &[id]);
        let e = p.outbox().get(id).unwrap();
        assert_eq!(e.state, OutboxState::Failed);
        assert_eq!(e.last_error.as_deref(), Some(REASON_MAY_NOT_HAVE_BEEN_SENT));
        assert_eq!(row_state(&p, id), Some(OutboxState::Failed));
        assert!(p.take_due(i64::MAX / 2, 10).unwrap().is_empty());
        // A second restart keeps it failed (it is not recovered twice).
        let mut p = PersistentOutbox::open(p.into_store()).unwrap();
        assert!(p.recovered_after_restart().is_empty());
        assert!(p.take_due(i64::MAX / 2, 10).unwrap().is_empty());
        // The person checked Sent and chose Retry.
        p.retry(id, 100).unwrap();
        assert_eq!(p.take_due(100, 10).unwrap().len(), 1);
    }

    #[test]
    fn a_queued_message_survives_a_restart_and_is_sent_once() {
        let mut p = open();
        let id = p.queue(new_msg(), b"MIME".to_vec(), 0, WINDOW).unwrap();
        let mut p = PersistentOutbox::open(p.into_store()).unwrap();
        assert!(p.recovered_after_restart().is_empty());
        assert!(p.take_due(WINDOW - 1, 10).unwrap().is_empty());
        let due = p.take_due(WINDOW, 10).unwrap();
        assert_eq!(due[0].entry.id, id);
        assert!(p.take_due(WINDOW, 10).unwrap().is_empty());
    }

    #[test]
    fn offline_then_online_hands_it_out_once_with_no_attempt_spent() {
        let mut p = open();
        let id = p.queue(new_msg(), b"MIME".to_vec(), 0, WINDOW).unwrap();
        p.set_offline(A, true);
        assert!(p.take_due(10_000_000, 10).unwrap().is_empty());
        assert_eq!(row_state(&p, id), Some(OutboxState::Queued));
        assert_eq!(p.store().rows[&(A, id)].0.attempts, 0);
        p.set_offline(A, false);
        assert_eq!(p.take_due(10_000_000, 10).unwrap().len(), 1);
        assert!(p.take_due(10_000_000, 10).unwrap().is_empty());
    }

    #[test]
    fn the_full_happy_path_leaves_nothing_behind() {
        let mut p = open();
        let id = p.queue(new_msg(), b"MIME".to_vec(), 0, 0).unwrap();
        p.take_due(0, 1).unwrap();
        assert!(p.submitted(id, 5).unwrap());
        assert_eq!(row_state(&p, id), Some(OutboxState::Submitted));
        let filing = p.take_filing(5, 1).unwrap();
        assert_eq!(filing[0].mime, b"MIME");
        assert_eq!(row_state(&p, id), Some(OutboxState::Filing));
        assert!(p.filed(id).unwrap());
        assert!(
            p.store().rows.is_empty(),
            "sent mail is not kept in the Outbox"
        );
        assert_eq!(p.outbox().iter().count(), 0);
    }

    #[test]
    fn a_crash_between_submit_and_append_neither_resends_nor_forgets_the_copy() {
        let mut p = open();
        let id = p.queue(new_msg(), b"MIME".to_vec(), 0, 0).unwrap();
        p.take_due(0, 1).unwrap();
        p.submitted(id, 5).unwrap();
        // Crash here, then again in the middle of the append.
        let mut p = PersistentOutbox::open(p.into_store()).unwrap();
        assert!(p.take_due(i64::MAX / 2, 10).unwrap().is_empty());
        assert_eq!(p.take_filing(10, 10).unwrap().len(), 1);
        let mut p = PersistentOutbox::open(p.into_store()).unwrap();
        assert_eq!(row_state(&p, id), Some(OutboxState::Submitted));
        assert!(p.take_due(i64::MAX / 2, 10).unwrap().is_empty());
        let again = p.take_filing(10, 10).unwrap();
        assert_eq!(again.len(), 1, "the copy is not forgotten");
        assert!(p.filed(id).unwrap());
    }

    #[test]
    fn giving_up_on_the_copy_removes_the_row() {
        let mut p = open();
        let id = p.queue(new_msg(), b"MIME".to_vec(), 0, 0).unwrap();
        p.take_due(0, 1).unwrap();
        p.submitted(id, 0).unwrap();
        p.take_filing(0, 1).unwrap();
        let r = p
            .filing_failed(id, &BackendError::Permanent("no folder".into()), 1)
            .unwrap();
        assert_eq!(r, Some(FilingRejection::GaveUp));
        assert!(p.store().rows.is_empty());
    }

    #[test]
    fn failure_edit_and_discard_through_the_store() {
        let mut p = open();
        let id = p.queue(new_msg(), b"MIME".to_vec(), 0, 0).unwrap();
        p.take_due(0, 1).unwrap();
        let r = p
            .failed(
                id,
                &BackendError::Permanent("The server refused the message".into()),
                1,
            )
            .unwrap();
        assert_eq!(r, Some(SendRejection::Failed));
        assert_eq!(
            p.store().rows[&(A, id)].0.last_error.as_deref(),
            Some("The server refused the message")
        );
        let back = p.edit_back_to_draft(id).unwrap();
        assert_eq!(back.mime, b"MIME");
        assert!(p.store().rows.is_empty());

        let id2 = p.queue(new_msg(), b"TWO".to_vec(), 0, 0).unwrap();
        assert!(matches!(
            p.discard(id2).map(|e| e.id),
            Ok(i) if i == id2
        ));
        assert!(p.store().rows.is_empty());
        // Ids are not reused, even though the table is empty again.
        let p2 = PersistentOutbox::open(p.into_store()).unwrap();
        let mut p2 = p2;
        assert!(p2.queue(new_msg(), b"X".to_vec(), 0, 0).unwrap() > id2);
    }

    #[test]
    fn a_message_whose_bytes_are_gone_fails_instead_of_being_sent_empty() {
        let mut p = open();
        let id = p.queue(new_msg(), b"MIME".to_vec(), 0, 0).unwrap();
        // The store no longer has the message (a damaged database), but memory still lists it.
        p.store.rows.remove(&(A, id));
        assert!(p.take_due(0, 10).unwrap().is_empty());
        let e = p.outbox().get(id).unwrap();
        assert_eq!(e.state, OutboxState::Failed);
        assert_eq!(
            e.last_error.as_deref(),
            Some(crate::outbox::REASON_UNREADABLE)
        );
    }

    #[test]
    fn debug_output_never_shows_the_message() {
        let d = DueMessage {
            entry: OutboxEntry {
                id: 1,
                account: A,
                identity: None,
                envelope_from: "me@example.org".into(),
                recipients: vec![],
                state: OutboxState::Sending,
                send_at_ms: 0,
                attempts: 1,
                last_error: None,
            },
            mime: b"Subject: SECRET-SUBJECT".to_vec(),
        };
        assert!(!format!("{d:?}").contains("SECRET"));
        let r = RemovedMessage {
            entry: d.entry.clone(),
            mime: d.mime.clone(),
        };
        assert!(!format!("{r:?}").contains("SECRET"));
    }
}
