// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Keeping the operation queue across a kill.
//!
//! [`OpQueue`](crate::ops::OpQueue) holds the rules; this module only adds persistence, and stays
//! free of SQLite and of any clock:
//!
//! * [`OpStore`] is the storage contract: [`OpStore::load`] returns the unfinished work, and
//!   [`OpStore::commit`] writes one [`Batch`] atomically. `croft-storage` implements it over the
//!   `op_queue` and `op_awaiting_uid` tables.
//! * A [`Batch`] holds the queue rows that changed **and** the [`LocalEffect`]s (the optimistic
//!   change to the person's view: `is_seen`, hidden rows, `pending_ops`). The store applies both in
//!   one transaction, so the screen and the queue can never disagree after a kill.
//! * [`PersistentQueue`] wraps an `OpQueue` and a store. Every method that changes the queue
//!   works out the batch and commits it. If the commit fails, the in-memory queue is put back as it
//!   was, so memory never runs ahead of the disk, and the error is returned.
//!
//! The offline flag, pauses and the account back-off are not stored; the engine sets them again on
//! start.

use std::collections::BTreeMap;
use std::fmt;

use croft_protocols::{BackendError, RemoteHeader, Uid};

use crate::ops::{
    AccountId, AwaitingUid, CancelError, Enqueued, Location, Op, OpId, OpKind, OpQueue, OpState,
    Rejection,
};

/// The optimistic change to the person's view that goes with a queue change. The store applies the
/// effects of a [`Batch`], in order, in the same transaction as the queue rows.
///
/// Three effects carry server state rather than a local change ([`LocalEffect::ClearFolder`],
/// [`LocalEffect::PutHeader`], [`LocalEffect::SetFolderValidity`]): they are only used by
/// [`PersistentQueue::reset_folder`], where the folder's rows and status must change in the same
/// transaction as the operations that point into it.
///
/// A message is addressed by account, folder path and UID. An effect on a message that has no
/// local row (not synced yet, or already gone) does nothing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LocalEffect {
    /// Set `is_seen` on the message row.
    SetSeen {
        /// Owning account.
        account: AccountId,
        /// Folder path.
        folder: String,
        /// UID in the folder.
        uid: Uid,
        /// New value: true means read.
        value: bool,
    },
    /// Set `is_flagged` on the message row.
    SetFlagged {
        /// Owning account.
        account: AccountId,
        /// Folder path.
        folder: String,
        /// UID in the folder.
        uid: Uid,
        /// New value: true means starred.
        value: bool,
    },
    /// Hide the row from lists (a move or delete is waiting). The row is kept.
    Hide {
        /// Owning account.
        account: AccountId,
        /// Folder path.
        folder: String,
        /// UID in the folder.
        uid: Uid,
    },
    /// Show the row again (the move or delete was cancelled or failed).
    Unhide {
        /// Owning account.
        account: AccountId,
        /// Folder path.
        folder: String,
        /// UID in the folder.
        uid: Uid,
    },
    /// Add `delta` to the row's `pending_ops`: +1 when an operation is queued, -1 when it
    /// finishes. The count never goes below 0.
    PendingDelta {
        /// Owning account.
        account: AccountId,
        /// Folder path.
        folder: String,
        /// UID in the folder.
        uid: Uid,
        /// Change to apply.
        delta: i32,
    },
    /// Delete the row. Only after the server confirmed that the message is gone from there.
    RemoveMessage {
        /// Owning account.
        account: AccountId,
        /// Folder path.
        folder: String,
        /// UID in the folder.
        uid: Uid,
    },
    /// Delete every message row of a folder (its UIDVALIDITY changed). The folder itself stays.
    ClearFolder {
        /// Owning account.
        account: AccountId,
        /// Folder path.
        folder: String,
    },
    /// Insert or update the row for a header from the server (as a folder sync does: local
    /// `is_seen` and `is_flagged` are kept while the row has pending operations).
    PutHeader {
        /// Owning account.
        account: AccountId,
        /// Folder path.
        folder: String,
        /// The header (boxed: it is much larger than the other effects).
        header: Box<RemoteHeader>,
    },
    /// Store only the folder's new UIDVALIDITY, and set its UIDNEXT and HIGHESTMODSEQ to NULL
    /// (and leave the sync time alone). A UIDVALIDITY reset does not make the folder synced: its
    /// messages are fetched afterwards, and until the sync that does that has finished, the folder
    /// must never look "unchanged" to a later sync. The full status is stored by the sync itself,
    /// at its end.
    SetFolderValidity {
        /// Owning account.
        account: AccountId,
        /// Folder path.
        folder: String,
        /// The UIDVALIDITY the server reported.
        uid_validity: u32,
    },
    /// After a confirmed move with a known new UID: copy the row to its new address (visible,
    /// keeping its pending count, flags, cached body and attachments list). Does nothing when
    /// there is no row at the old address.
    CopyMessage {
        /// Owning account.
        account: AccountId,
        /// Old folder path and UID.
        from: (String, Uid),
        /// New folder path and UID.
        to: (String, Uid),
    },
}

/// Everything that changes in one step, written atomically by [`OpStore::commit`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Batch {
    /// Operations that were added or changed (insert or update by id), in id order.
    pub ops: Vec<Op>,
    /// Finished operations that were dropped from the queue: account and id.
    pub purged: Vec<(AccountId, OpId)>,
    /// Awaiting-UID entries that were added or changed.
    pub awaiting_put: Vec<AwaitingUid>,
    /// Awaiting-UID entries that were resolved: account, old folder, old UID.
    pub awaiting_removed: Vec<(AccountId, String, Uid)>,
    /// Changes to the person's view, to apply in this order after the queue rows.
    pub effects: Vec<LocalEffect>,
}

impl Batch {
    /// True when there is nothing to write.
    pub fn is_empty(&self) -> bool {
        self.ops.is_empty()
            && self.purged.is_empty()
            && self.awaiting_put.is_empty()
            && self.awaiting_removed.is_empty()
            && self.effects.is_empty()
    }
}

/// A message whose move was confirmed without a new UID (an awaiting-UID entry) and whose source
/// folder is the one being reset. Part of a [`FolderReset`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MovedOut {
    /// The old UID in the reset folder (the awaiting entry's address).
    pub old_uid: Uid,
    /// Where the engine found the message by Message-ID, with its header, or `None` if it did not.
    pub found: Option<(Location, RemoteHeader)>,
}

/// What a UIDVALIDITY change of one folder does, as worked out by the sync engine (which did the
/// Message-ID lookups and fetched the headers). Applied by [`PersistentQueue::reset_folder`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FolderReset {
    /// The folder whose UIDVALIDITY changed.
    pub folder: String,
    /// Its new UIDVALIDITY, stored in the same transaction **alone**: UIDNEXT and HIGHESTMODSEQ
    /// become NULL until a completed sync stores the full status (see
    /// [`LocalEffect::SetFolderValidity`]).
    pub uid_validity: u32,
    /// Fresh headers for the folder: at least the rows the re-pointed operations need.
    pub headers: Vec<RemoteHeader>,
    /// Old UID in the folder to new UID, for messages with unfinished operations that were found
    /// again by Message-ID.
    pub matched: Vec<(Uid, Uid)>,
    /// Old UIDs in the folder whose messages were not found: their operations fail with `reason`.
    pub unmatched: Vec<Uid>,
    /// Awaiting-UID entries whose **source** is this folder. Their old row went with the folder,
    /// and the old address may belong to another message now, so they are resolved here and
    /// never left waiting: found means re-pointed to the destination, not found means failed.
    pub moved_out: Vec<MovedOut>,
    /// Awaiting-UID entries whose **destination** is this folder and that were found here: old
    /// location (in another folder) to new UID in this folder. Their old hidden row is removed.
    pub moved_in: Vec<(Location, Uid)>,
    /// Shown for operations that fail.
    pub reason: String,
}

/// A stored operation that could not be read back (a newer app wrote it, or the row is damaged).
/// The store marks it failed rather than guessing what it meant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnreadableOp {
    /// The row's id.
    pub id: OpId,
    /// The row's account.
    pub account: AccountId,
    /// A plain reason, safe to show.
    pub reason: String,
}

/// What [`OpStore::load`] returns.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StoredQueue {
    /// Unfinished operations (queued or applying), in id order.
    pub ops: Vec<Op>,
    /// Messages that are waiting for their new UID.
    pub awaiting: Vec<AwaitingUid>,
    /// The largest id the store has ever held, including rows that are gone (a store must keep a
    /// high-water mark, because the largest id still present can shrink). New ids start above.
    pub max_id: OpId,
    /// Rows that could not be read. The store has marked them failed and repaired the person's
    /// view once, and keeps the rows, so every load lists them again until
    /// [`PersistentQueue::acknowledge_unreadable`] deletes them.
    pub unreadable: Vec<UnreadableOp>,
}

/// Where the queue is kept. Implemented over SQLite by `croft-storage`; tests use memory.
pub trait OpStore {
    /// The store's error. It must not contain message content or full addresses.
    type Error: std::error::Error;

    /// Returns the unfinished work. Finished rows are history and are discarded, except rows that
    /// could not be read and have not been acknowledged yet.
    fn load(&mut self) -> Result<StoredQueue, Self::Error>;

    /// Writes the batch: queue rows first, then the effects, all or nothing.
    fn commit(&mut self, batch: &Batch) -> Result<(), Self::Error>;
}

/// Why [`PersistentQueue::cancel`] did not cancel.
#[derive(Debug)]
pub enum QueueError<E> {
    /// The queue refused (no such operation, or too late).
    Cancel(CancelError),
    /// The store could not write; nothing changed.
    Store(E),
}

impl<E: fmt::Display> fmt::Display for QueueError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Cancel(CancelError::NotFound) => f.write_str("no such operation"),
            Self::Cancel(CancelError::TooLate) => f.write_str("too late to undo"),
            Self::Store(e) => e.fmt(f),
        }
    }
}

impl<E: std::error::Error + 'static> std::error::Error for QueueError<E> {}

/// What happens to the person's view when an operation finishes badly or is dropped.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Finish {
    /// Count it down and put the old value back (or unhide the row).
    Restore,
    /// Count it down only; the row is removed by the caller.
    CountOnly,
    /// Do nothing: the row's address may belong to another message now.
    Nothing,
}

/// An [`OpQueue`] that is kept in an [`OpStore`].
///
/// Every method that changes the queue updates memory and commits one [`Batch`] (queue rows and
/// the local change together). If the store fails, memory is put back and the error is returned, so
/// the caller can tell the person and the queue still matches the disk.
#[derive(Debug)]
pub struct PersistentQueue<S: OpStore> {
    queue: OpQueue,
    store: S,
    unreadable: Vec<UnreadableOp>,
}

impl<S: OpStore> PersistentQueue<S> {
    /// Loads the unfinished work from `store` and puts anything that was mid-flight back in the
    /// queue ([`OpQueue::recover_after_restart`]), writing that back too.
    pub fn open(mut store: S) -> Result<Self, S::Error> {
        let stored = store.load()?;
        let mut queue = OpQueue::from_parts(stored.ops, stored.awaiting, stored.max_id);
        let before = queue.clone();
        queue.recover_after_restart();
        let batch = diff(&before, &queue, Finish::Restore);
        if !batch.is_empty() {
            store.commit(&batch)?;
        }
        Ok(Self {
            queue,
            store,
            unreadable: stored.unreadable,
        })
    }

    /// Read access to the queue: `get`, `iter`, `pending_uids`, `next_wake_ms`, `awaiting`, ...
    pub fn queue(&self) -> &OpQueue {
        &self.queue
    }

    /// The store.
    pub fn store(&self) -> &S {
        &self.store
    }

    /// The store, mutably. For writes that are not queue rows and must not go through the queue,
    /// such as the sync engine applying a folder sync to the same database. Never use it to commit
    /// a [`Batch`] by hand: the queue in memory would no longer match the disk.
    pub fn store_mut(&mut self) -> &mut S {
        &mut self.store
    }

    /// Gives the store back (for example to reopen it in a test).
    pub fn into_store(self) -> S {
        self.store
    }

    /// Stored operations that could not be read and were marked failed. They are listed at every
    /// start until the engine has told the person and called
    /// [`PersistentQueue::acknowledge_unreadable`].
    pub fn unreadable(&self) -> &[UnreadableOp] {
        &self.unreadable
    }

    /// The engine has reported the unreadable rows with these ids to the person: delete them from
    /// the store in one batch. Until then every [`PersistentQueue::open`] lists them again. Ids
    /// that are not in [`PersistentQueue::unreadable`] are ignored. If the store fails, nothing
    /// changes and the rows are listed again.
    pub fn acknowledge_unreadable(&mut self, ids: &[OpId]) -> Result<(), S::Error> {
        let purged: Vec<(AccountId, OpId)> = self
            .unreadable
            .iter()
            .filter(|u| ids.contains(&u.id))
            .map(|u| (u.account, u.id))
            .collect();
        if purged.is_empty() {
            return Ok(());
        }
        let batch = Batch {
            purged,
            ..Batch::default()
        };
        self.store.commit(&batch)?;
        self.unreadable
            .retain(|u| !batch.purged.contains(&(u.account, u.id)));
        Ok(())
    }

    /// Runs `change` on the queue, then commits the rows and effects it caused plus whatever
    /// `extras` adds. Puts the queue back if the commit fails.
    fn run<T>(
        &mut self,
        finish: Finish,
        change: impl FnOnce(&mut OpQueue) -> T,
        extras: impl FnOnce(&OpQueue, &OpQueue, &T) -> Vec<LocalEffect>,
    ) -> Result<T, S::Error> {
        let before = self.queue.clone();
        let out = change(&mut self.queue);
        let mut batch = diff(&before, &self.queue, finish);
        batch.effects.extend(extras(&before, &self.queue, &out));
        if batch.is_empty() {
            return Ok(out);
        }
        match self.store.commit(&batch) {
            Ok(()) => Ok(out),
            Err(e) => {
                self.queue = before;
                Err(e)
            }
        }
    }

    /// Records a change and applies its optimistic effect (the new flag value, or hiding the row)
    /// in the same transaction. The caller must only enqueue real changes: the value a flag had
    /// before is taken to be the opposite of `value`.
    pub fn enqueue(
        &mut self,
        account: AccountId,
        kind: OpKind,
        now_ms: i64,
        undo_window_ms: i64,
    ) -> Result<Enqueued, S::Error> {
        self.run(
            Finish::Restore,
            |q| q.enqueue(account, kind, now_ms, undo_window_ms),
            |_, _, _| Vec::new(),
        )
    }

    /// Undo. A cancelled move or delete shows its row again; a cancelled flag change puts the old
    /// value back.
    pub fn cancel(&mut self, id: OpId, now_ms: i64) -> Result<(), QueueError<S::Error>> {
        self.run(
            Finish::Restore,
            |q| q.cancel(id, now_ms),
            |_, _, _| Vec::new(),
        )
        .map_err(QueueError::Store)?
        .map_err(QueueError::Cancel)
    }

    /// Hands out due operations and stores them as applying, so a kill is recovered on restart.
    pub fn take_due(&mut self, now_ms: i64, limit: usize) -> Result<Vec<Op>, S::Error> {
        self.run(
            Finish::Restore,
            |q| q.take_due(now_ms, limit),
            |_, _, _| Vec::new(),
        )
    }

    /// Like [`PersistentQueue::take_due`], for one account only (see [`OpQueue::take_due_for`]).
    pub fn take_due_for(
        &mut self,
        account: AccountId,
        now_ms: i64,
        limit: usize,
    ) -> Result<Vec<Op>, S::Error> {
        self.run(
            Finish::Restore,
            |q| q.take_due_for(account, now_ms, limit),
            |_, _, _| Vec::new(),
        )
    }

    /// The server accepted the operation. A confirmed delete removes the row; a move should be
    /// confirmed with [`PersistentQueue::confirm_moved`] so later operations follow the message,
    /// but a move confirmed here removes the old row and nothing follows it.
    pub fn confirm(&mut self, id: OpId) -> Result<(), S::Error> {
        self.run(
            Finish::Restore,
            |q| q.confirm(id),
            |before, after, ()| {
                let Some(old) = before.get(id).filter(|o| o.state == OpState::Applying) else {
                    return Vec::new();
                };
                let confirmed = after.get(id).is_some_and(|o| o.state == OpState::Confirmed);
                match &old.kind {
                    OpKind::MoveTo { folder, uid, .. } if confirmed => {
                        vec![LocalEffect::RemoveMessage {
                            account: old.account,
                            folder: folder.clone(),
                            uid: *uid,
                        }]
                    }
                    _ => Vec::new(),
                }
            },
        )
    }

    /// The server accepted a move. With the new location, the row is copied there and the old row
    /// removed, and later operations are re-pointed. Without it, the awaiting-UID mark is stored in
    /// the same transaction (with no Message-ID, so it can never be found again; prefer
    /// [`PersistentQueue::confirm_moved_with`]).
    pub fn confirm_moved(
        &mut self,
        id: OpId,
        new_location: Option<(String, Uid)>,
    ) -> Result<(), S::Error> {
        self.confirm_moved_with(id, new_location, None, 0)
    }

    /// Like [`PersistentQueue::confirm_moved`], and when the new UID is not known, stores the
    /// message's `Message-ID` and the time with the awaiting-UID mark.
    pub fn confirm_moved_with(
        &mut self,
        id: OpId,
        new_location: Option<(String, Uid)>,
        message_id: Option<String>,
        now_ms: i64,
    ) -> Result<(), S::Error> {
        let to = new_location.clone();
        self.run(
            Finish::Restore,
            |q| q.confirm_moved_with(id, new_location, message_id, now_ms),
            |before, after, ()| {
                let confirmed = after.get(id).is_some_and(|o| o.state == OpState::Confirmed);
                let old = before.get(id).filter(|o| o.state == OpState::Applying);
                match (old, to, confirmed) {
                    (Some(old), Some(to), true) => {
                        let (folder, uid) = old.kind.target();
                        move_away(after, old.account, (folder, uid), to)
                    }
                    _ => Vec::new(),
                }
            },
        )
    }

    /// The engine found the message that used to be at `from` at `to`. Later operations are
    /// re-pointed, the row is copied to its new address and the old row removed.
    pub fn repoint(
        &mut self,
        account: AccountId,
        from: (&str, Uid),
        to: (String, Uid),
    ) -> Result<usize, S::Error> {
        if from.0 == to.0 && from.1 == to.1 {
            // Nothing moved. Copying a row onto itself and then removing the "old" row would
            // delete the message.
            return Ok(0);
        }
        let target = to.clone();
        self.run(
            Finish::Restore,
            |q| q.repoint(account, from, to),
            |before, after, changed| {
                if *changed > 0 || before.awaiting_for(account, from).is_some() {
                    move_away(after, account, from, target)
                } else {
                    Vec::new()
                }
            },
        )
    }

    /// Like [`PersistentQueue::repoint`], for a folder whose UIDVALIDITY changed: the old rows
    /// are gone and the folder was synced again, so there is nothing to copy. The row at `to` gets
    /// the pending count and the person's local flag values (or hidden state) back instead.
    ///
    /// When `from` has an awaiting-UID entry (a confirmed move whose new UID was never learned),
    /// the old address is in the move's source folder, which was **not** reset: its hidden row is
    /// still there, so it is removed here (its pending count goes with it, and the new row gets
    /// its own count). The entry itself is resolved.
    pub fn repoint_after_reset(
        &mut self,
        account: AccountId,
        from: (&str, Uid),
        to: (String, Uid),
    ) -> Result<usize, S::Error> {
        let target = to.clone();
        self.run(
            Finish::Nothing,
            |q| q.repoint(account, from, to),
            |before, after, _| {
                let mut effects = Vec::new();
                let same = from.0 == target.0 && from.1 == target.1;
                let leftover = before.awaiting_for(account, from).is_some() && !same;
                for op in after.iter() {
                    let was_here = before
                        .get(op.id)
                        .is_some_and(|old| !old.state.is_finished() && old.kind.target() == from);
                    if was_here && !op.state.is_finished() {
                        effects.extend(created_effects(
                            op.account,
                            &op.kind,
                            (&target.0, target.1),
                        ));
                    }
                }
                if leftover {
                    effects.push(remove_row(account, from));
                }
                effects
            },
        )
    }

    /// Applies a UIDVALIDITY change of one folder in **one** batch: the folder's rows are
    /// replaced, its new UIDVALIDITY is stored (and nothing else of its status), and every operation
    /// that pointed into it is re-pointed or failed, all in the same transaction.
    ///
    /// The engine may delete most of the folder's rows in chunks **before** this call, to keep each
    /// lock hold short: that is safe because the UIDVALIDITY is stored only here, so a sync that is
    /// cut short between the chunks sees the old UIDVALIDITY and starts the reset again. The rows
    /// of messages that have unfinished operations must be kept until this call (their Message-ID
    /// is read from them), and [`LocalEffect::ClearFolder`] then removes what is left.
    ///
    /// It has to be one transaction. Operations are addressed by UID, and the folder's stored
    /// UIDVALIDITY says which numbering a UID belongs to. If a kill could land between re-pointing
    /// the operations and storing the new UIDVALIDITY, the next start would read operations in the
    /// new numbering as if they were in the old one (or the reverse) and act on the wrong message.
    ///
    /// The effects, in order: [`LocalEffect::ClearFolder`]; the headers of messages found in other
    /// folders ([`FolderReset::moved_out`]); the folder's fresh headers; the pending count and local
    /// value (or hidden state) of every re-pointed operation on its new row; the removal of the
    /// old hidden row of each [`FolderReset::moved_in`] entry; [`LocalEffect::SetFolderValidity`].
    /// Failed operations change no row (the folder's rows are gone and the server's values come
    /// back with the fresh headers). Returns the ids of the failed operations.
    pub fn reset_folder(
        &mut self,
        account: AccountId,
        reset: &FolderReset,
    ) -> Result<Vec<OpId>, S::Error> {
        let folder = reset.folder.as_str();
        let mut moves: Vec<(Location, Location)> = Vec::new();
        for moved in &reset.moved_out {
            if let Some((to, _)) = &moved.found {
                moves.push(((folder.to_owned(), moved.old_uid), to.clone()));
            }
        }
        for (old, new) in &reset.matched {
            moves.push(((folder.to_owned(), *old), (folder.to_owned(), *new)));
        }
        for (from, new) in &reset.moved_in {
            moves.push((from.clone(), (folder.to_owned(), *new)));
        }
        let moves_ref = &moves;
        self.run(
            Finish::Nothing,
            |q| {
                let mut failed = Vec::new();
                for moved in reset.moved_out.iter().filter(|m| m.found.is_none()) {
                    failed.extend(q.abandon(account, (folder, moved.old_uid), &reset.reason));
                }
                for old in &reset.unmatched {
                    failed.extend(q.abandon(account, (folder, *old), &reset.reason));
                }
                q.repoint_many(account, moves_ref);
                failed
            },
            |before, after, _| {
                let mut effects = vec![LocalEffect::ClearFolder {
                    account,
                    folder: folder.to_owned(),
                }];
                for moved in &reset.moved_out {
                    if let Some((to, header)) = &moved.found {
                        effects.push(LocalEffect::PutHeader {
                            account,
                            folder: to.0.clone(),
                            header: Box::new(header.clone()),
                        });
                    }
                }
                for header in &reset.headers {
                    effects.push(LocalEffect::PutHeader {
                        account,
                        folder: folder.to_owned(),
                        header: Box::new(header.clone()),
                    });
                }
                let map: BTreeMap<(&str, Uid), &Location> = moves_ref
                    .iter()
                    .map(|(from, to)| ((from.0.as_str(), from.1), to))
                    .collect();
                for op in after.iter().filter(|op| !op.state.is_finished()) {
                    let Some(old) = before.get(op.id).filter(|old| !old.state.is_finished()) else {
                        continue;
                    };
                    if let Some(to) = map.get(&old.kind.target()) {
                        effects.extend(created_effects(op.account, &op.kind, (&to.0, to.1)));
                    }
                }
                for (from, _) in &reset.moved_in {
                    if from.0 != folder && before.awaiting_for(account, (&from.0, from.1)).is_some()
                    {
                        effects.push(remove_row(account, (&from.0, from.1)));
                    }
                }
                effects.push(LocalEffect::SetFolderValidity {
                    account,
                    folder: folder.to_owned(),
                    uid_validity: reset.uid_validity,
                });
                effects
            },
        )
    }

    /// Gives up on the unfinished operations on a message that was moved and could not be found
    /// again. They fail with `reason`, and the old (hidden) row is removed. Returns their ids.
    pub fn abandon(
        &mut self,
        account: AccountId,
        from: (&str, Uid),
        reason: &str,
    ) -> Result<Vec<OpId>, S::Error> {
        self.run(
            Finish::CountOnly,
            |q| q.abandon(account, from, reason),
            |before, _, failed| {
                if !failed.is_empty() || before.awaiting_for(account, from).is_some() {
                    vec![LocalEffect::RemoveMessage {
                        account,
                        folder: from.0.to_owned(),
                        uid: from.1,
                    }]
                } else {
                    Vec::new()
                }
            },
        )
    }

    /// Like [`PersistentQueue::abandon`], for a folder whose UIDVALIDITY changed. The old address
    /// may now belong to a different message, so the local rows are left alone.
    ///
    /// The one exception is an address with an awaiting-UID entry: that is the source folder of a
    /// confirmed move, which was not reset, so its hidden row is removed (with its pending count)
    /// as [`PersistentQueue::abandon`] does.
    pub fn abandon_after_reset(
        &mut self,
        account: AccountId,
        from: (&str, Uid),
        reason: &str,
    ) -> Result<Vec<OpId>, S::Error> {
        self.run(
            Finish::Nothing,
            |q| q.abandon(account, from, reason),
            |before, _, _| {
                if before.awaiting_for(account, from).is_some() {
                    vec![remove_row(account, from)]
                } else {
                    Vec::new()
                }
            },
        )
    }

    /// The backend call failed. A failed operation is counted down and its change put back.
    pub fn reject(
        &mut self,
        id: OpId,
        error: &BackendError,
        now_ms: i64,
    ) -> Result<Option<Rejection>, S::Error> {
        self.run(
            Finish::Restore,
            |q| q.reject(id, error, now_ms),
            |_, _, _| Vec::new(),
        )
    }

    /// Counts one more Message-ID lookup for the message that used to be at `from`.
    pub fn record_lookup(
        &mut self,
        account: AccountId,
        from: (&str, Uid),
    ) -> Result<Option<u32>, S::Error> {
        self.run(
            Finish::Restore,
            |q| q.record_lookup(account, from),
            |_, _, _| Vec::new(),
        )
    }

    /// Drops finished operations from memory and from the store. Returns how many.
    pub fn purge_finished(&mut self) -> Result<usize, S::Error> {
        self.run(Finish::Restore, OpQueue::purge_finished, |_, _, _| {
            Vec::new()
        })
    }

    /// Puts the account's operations that are marked applying back in the queue, in one batch
    /// ([`OpQueue::recover_applying`]). Call it when nothing of the account is being sent any more:
    /// its task was stopped or aborted in the middle of a call, or a step failed after an
    /// operation was handed out. Returns how many were put back.
    pub fn recover_applying(&mut self, account: AccountId) -> Result<usize, S::Error> {
        self.run(
            Finish::Restore,
            |q| q.recover_applying(account),
            |_, _, _| Vec::new(),
        )
    }

    /// Ends the account's back-off ([`OpQueue::clear_hold`]) and stores the operations' new retry
    /// times.
    pub fn clear_hold(&mut self, account: AccountId, now_ms: i64) -> Result<(), S::Error> {
        self.run(
            Finish::Restore,
            |q| q.clear_hold(account, now_ms),
            |_, _, _| Vec::new(),
        )
    }

    /// Records whether the account has a connection. Not stored.
    pub fn set_offline(&mut self, account: AccountId, offline: bool) {
        self.queue.set_offline(account, offline);
    }

    /// Lets an account's queue run again after the person signed in. Not stored.
    pub fn resume(&mut self, account: AccountId) {
        self.queue.resume(account);
    }
}

/// Remove the row at `at`.
fn remove_row(account: AccountId, at: (&str, Uid)) -> LocalEffect {
    LocalEffect::RemoveMessage {
        account,
        folder: at.0.to_owned(),
        uid: at.1,
    }
}

/// True when an unfinished move or delete-forever (in `queue`) acts on the message at `at`, so the
/// row has to stay hidden.
fn hidden_by_pending(queue: &OpQueue, account: AccountId, at: (&str, Uid)) -> bool {
    queue.iter().any(|o| {
        o.account == account
            && !o.state.is_finished()
            && matches!(o.kind, OpKind::MoveTo { .. } | OpKind::DeleteForever { .. })
            && o.kind.target() == at
    })
}

/// Copy a confirmed move's row to its new address, then remove the old one. The copy is shown,
/// unless a later move or delete that was re-pointed at it (see `after`) is still waiting: then it
/// is hidden straight away.
fn move_away(
    after: &OpQueue,
    account: AccountId,
    from: (&str, Uid),
    to: (String, Uid),
) -> Vec<LocalEffect> {
    let hide = hidden_by_pending(after, account, (&to.0, to.1));
    let hidden_at = (to.0.clone(), to.1);
    let mut effects = vec![
        LocalEffect::CopyMessage {
            account,
            from: (from.0.to_owned(), from.1),
            to,
        },
        remove_row(account, from),
    ];
    if hide {
        effects.push(LocalEffect::Hide {
            account,
            folder: hidden_at.0,
            uid: hidden_at.1,
        });
    }
    effects
}

/// The local change that goes with a new operation at `at`: the new value (or hiding), and +1.
fn created_effects(account: AccountId, kind: &OpKind, at: (&str, Uid)) -> Vec<LocalEffect> {
    let (folder, uid) = (at.0.to_owned(), at.1);
    let apply = match kind {
        OpKind::SetSeen { value, .. } => LocalEffect::SetSeen {
            account,
            folder: folder.clone(),
            uid,
            value: *value,
        },
        OpKind::SetFlagged { value, .. } => LocalEffect::SetFlagged {
            account,
            folder: folder.clone(),
            uid,
            value: *value,
        },
        OpKind::MoveTo { .. } | OpKind::DeleteForever { .. } => LocalEffect::Hide {
            account,
            folder: folder.clone(),
            uid,
        },
    };
    vec![
        apply,
        LocalEffect::PendingDelta {
            account,
            folder,
            uid,
            delta: 1,
        },
    ]
}

/// The local change when an unfinished operation `old` became finished as `new`.
fn finished_effects(old: &Op, new: &Op, after: &OpQueue, finish: Finish) -> Vec<LocalEffect> {
    if finish == Finish::Nothing {
        return Vec::new();
    }
    let (folder, uid) = old.kind.target();
    let (folder, account) = (folder.to_owned(), old.account);
    let mut effects = vec![LocalEffect::PendingDelta {
        account,
        folder: folder.clone(),
        uid,
        delta: -1,
    }];
    match new.state {
        OpState::Failed | OpState::Cancelled if finish == Finish::Restore => {
            effects.extend(restore_effect(old, after));
        }
        OpState::Confirmed => {
            if matches!(old.kind, OpKind::DeleteForever { .. }) {
                effects.push(LocalEffect::RemoveMessage {
                    account,
                    folder,
                    uid,
                });
            }
        }
        _ => {}
    }
    effects
}

/// What to put back when `old` was cancelled or failed. A flag goes back to the value of the
/// latest operation on the same flag that is still waiting, or else to the opposite of what `old`
/// set. A move or delete shows its row again, unless another unfinished move or delete on the same
/// message is still waiting (then there is nothing to do).
fn restore_effect(old: &Op, after: &OpQueue) -> Option<LocalEffect> {
    let (folder, uid) = old.kind.target();
    let (folder, account) = (folder.to_owned(), old.account);
    let still_waiting = |want_seen: bool| {
        after
            .iter()
            .filter(|o| {
                o.id != old.id
                    && o.account == account
                    && !o.state.is_finished()
                    && o.kind.target() == (folder.as_str(), uid)
            })
            .filter_map(|o| match (&o.kind, want_seen) {
                (OpKind::SetSeen { value, .. }, true)
                | (OpKind::SetFlagged { value, .. }, false) => Some(*value),
                _ => None,
            })
            .last()
    };
    Some(match &old.kind {
        OpKind::SetSeen { value, .. } => LocalEffect::SetSeen {
            account,
            uid,
            value: still_waiting(true).unwrap_or(!*value),
            folder,
        },
        OpKind::SetFlagged { value, .. } => LocalEffect::SetFlagged {
            account,
            uid,
            value: still_waiting(false).unwrap_or(!*value),
            folder,
        },
        OpKind::MoveTo { .. } | OpKind::DeleteForever { .. } => {
            if hidden_by_pending(after, account, (&folder, uid)) {
                return None;
            }
            LocalEffect::Unhide {
                account,
                folder,
                uid,
            }
        }
    })
}

/// Works out what changed between two states of the queue.
fn diff(before: &OpQueue, after: &OpQueue, finish: Finish) -> Batch {
    let mut batch = Batch::default();
    for op in after.iter() {
        match before.get(op.id) {
            None => {
                batch.ops.push(op.clone());
                batch
                    .effects
                    .extend(created_effects(op.account, &op.kind, op.kind.target()));
            }
            Some(old) if old != op => {
                batch.ops.push(op.clone());
                if !old.state.is_finished() && op.state.is_finished() {
                    batch
                        .effects
                        .extend(finished_effects(old, op, after, finish));
                }
            }
            Some(_) => {}
        }
    }
    batch.purged = before
        .iter()
        .filter(|op| after.get(op.id).is_none())
        .map(|op| (op.account, op.id))
        .collect();

    let key = |a: &AwaitingUid| (a.account, a.folder.clone(), a.uid);
    let old_awaiting: BTreeMap<_, _> = before.awaiting().map(|a| (key(a), a)).collect();
    let new_awaiting: BTreeMap<_, _> = after.awaiting().map(|a| (key(a), a)).collect();
    for (k, a) in &new_awaiting {
        if old_awaiting.get(k) != Some(a) {
            batch.awaiting_put.push((*a).clone());
        }
    }
    for k in old_awaiting.keys() {
        if !new_awaiting.contains_key(k) {
            batch.awaiting_removed.push(k.clone());
        }
    }
    batch
}

#[cfg(test)]
mod tests;
