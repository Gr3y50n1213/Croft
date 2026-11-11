// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Tests for the batches [`PersistentQueue`] builds, against an in-memory store. The SQLite store
//! has its own tests in `croft-storage`.

// Tests may unwrap: a panic is the failure report.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::BTreeMap;
use std::fmt;

use super::*;

const A: AccountId = 1;
const WINDOW: i64 = 5_000;

#[derive(Debug)]
struct MemError;

impl fmt::Display for MemError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("memory store refused the write")
    }
}

impl std::error::Error for MemError {}

/// A store that keeps rows in memory and records every batch and effect it was given.
#[derive(Default)]
struct MemStore {
    ops: BTreeMap<OpId, Op>,
    awaiting: BTreeMap<(AccountId, String, Uid), AwaitingUid>,
    effects: Vec<LocalEffect>,
    commits: usize,
    fail: bool,
    max_id: OpId,
}

impl OpStore for MemStore {
    type Error = MemError;

    fn load(&mut self) -> Result<StoredQueue, MemError> {
        Ok(StoredQueue {
            ops: self
                .ops
                .values()
                .filter(|o| !o.state.is_finished())
                .cloned()
                .collect(),
            awaiting: self.awaiting.values().cloned().collect(),
            max_id: self.max_id,
            unreadable: Vec::new(),
        })
    }

    fn commit(&mut self, batch: &Batch) -> Result<(), MemError> {
        if self.fail {
            return Err(MemError);
        }
        self.commits += 1;
        for op in &batch.ops {
            self.max_id = self.max_id.max(op.id);
            self.ops.insert(op.id, op.clone());
        }
        for (_, id) in &batch.purged {
            self.ops.remove(id);
        }
        for a in &batch.awaiting_put {
            self.awaiting
                .insert((a.account, a.folder.clone(), a.uid), a.clone());
        }
        for k in &batch.awaiting_removed {
            self.awaiting.remove(k);
        }
        self.effects.extend(batch.effects.iter().cloned());
        Ok(())
    }
}

fn open() -> PersistentQueue<MemStore> {
    PersistentQueue::open(MemStore::default()).unwrap()
}

fn seen(uid: Uid, value: bool) -> OpKind {
    OpKind::SetSeen {
        folder: "INBOX".into(),
        uid,
        value,
    }
}

fn starred(uid: Uid, value: bool) -> OpKind {
    OpKind::SetFlagged {
        folder: "INBOX".into(),
        uid,
        value,
    }
}

fn mv(uid: Uid, dest: &str) -> OpKind {
    OpKind::MoveTo {
        folder: "INBOX".into(),
        uid,
        dest: dest.into(),
    }
}

fn add(q: &mut PersistentQueue<MemStore>, kind: OpKind, now: i64) -> OpId {
    match q.enqueue(A, kind, now, WINDOW).unwrap() {
        Enqueued::Queued(id) => id,
        other => panic!("expected a new operation, got {other:?}"),
    }
}

fn take_effects(q: &mut PersistentQueue<MemStore>) -> Vec<LocalEffect> {
    std::mem::take(&mut q.store.effects)
}

fn delta(folder: &str, uid: Uid, delta: i32) -> LocalEffect {
    LocalEffect::PendingDelta {
        account: A,
        folder: folder.into(),
        uid,
        delta,
    }
}

fn set_seen(uid: Uid, value: bool) -> LocalEffect {
    LocalEffect::SetSeen {
        account: A,
        folder: "INBOX".into(),
        uid,
        value,
    }
}

#[test]
fn enqueue_writes_the_row_and_the_local_change_in_one_batch() {
    let mut q = open();
    let id = add(&mut q, seen(1, true), 0);
    assert_eq!(q.store.commits, 1);
    assert_eq!(q.store.ops.get(&id).map(|o| o.state), Some(OpState::Queued));
    assert_eq!(
        take_effects(&mut q),
        vec![set_seen(1, true), delta("INBOX", 1, 1)]
    );

    add(&mut q, mv(2, "Archive"), 0);
    assert_eq!(
        take_effects(&mut q),
        vec![
            LocalEffect::Hide {
                account: A,
                folder: "INBOX".into(),
                uid: 2
            },
            delta("INBOX", 2, 1)
        ]
    );
}

#[test]
fn a_duplicate_writes_nothing() {
    let mut q = open();
    add(&mut q, seen(1, true), 0);
    let commits = q.store.commits;
    assert!(matches!(
        q.enqueue(A, seen(1, true), 10, WINDOW).unwrap(),
        Enqueued::Duplicate(_)
    ));
    assert_eq!(q.store.commits, commits);
}

#[test]
fn read_then_unread_puts_the_original_value_back_and_the_count_to_zero() {
    let mut q = open();
    let id = add(&mut q, seen(1, true), 0);
    take_effects(&mut q);
    assert_eq!(
        q.enqueue(A, seen(1, false), 500, WINDOW).unwrap(),
        Enqueued::CancelledOut
    );
    assert_eq!(
        take_effects(&mut q),
        vec![delta("INBOX", 1, -1), set_seen(1, false)]
    );
    assert_eq!(
        q.store.ops.get(&id).map(|o| o.state),
        Some(OpState::Cancelled)
    );
    assert_eq!(q.queue().unfinished(), 0);
}

#[test]
fn undo_cancels_the_row_and_shows_the_message_again() {
    let mut q = open();
    let id = add(&mut q, mv(1, "Trash"), 0);
    take_effects(&mut q);
    q.cancel(id, 1_000).unwrap();
    assert!(q.store.ops.values().all(|o| o.state != OpState::Queued));
    assert_eq!(
        take_effects(&mut q),
        vec![
            delta("INBOX", 1, -1),
            LocalEffect::Unhide {
                account: A,
                folder: "INBOX".into(),
                uid: 1
            }
        ]
    );
}

#[test]
fn undo_that_is_too_late_changes_nothing() {
    let mut q = open();
    let id = add(&mut q, mv(1, "Trash"), 0);
    let commits = q.store.commits;
    assert!(matches!(
        q.cancel(id, WINDOW),
        Err(QueueError::Cancel(CancelError::TooLate))
    ));
    assert!(matches!(
        q.cancel(99, 0),
        Err(QueueError::Cancel(CancelError::NotFound))
    ));
    assert_eq!(q.store.commits, commits);
}

#[test]
fn a_failed_flag_change_goes_back_to_the_value_still_waiting_or_the_opposite() {
    let mut q = open();
    // First change is in flight when the second one is made.
    let first = add(&mut q, seen(1, true), 0);
    q.take_due(2_000, 10).unwrap();
    let second = add(&mut q, seen(1, false), 2_100);
    take_effects(&mut q);

    // The first fails while the second still waits: the person's latest value stays.
    q.reject(first, &BackendError::Permanent("no".into()), 2_200)
        .unwrap();
    assert_eq!(
        take_effects(&mut q),
        vec![delta("INBOX", 1, -1), set_seen(1, false)]
    );

    // The second then fails: nothing else is waiting, so the opposite of what it set.
    q.take_due(60_000, 10).unwrap();
    q.reject(second, &BackendError::Permanent("no".into()), 60_000)
        .unwrap();
    assert_eq!(
        take_effects(&mut q),
        vec![delta("INBOX", 1, -1), set_seen(1, true)]
    );
}

#[test]
fn cancelling_a_later_change_keeps_the_value_of_the_earlier_one_in_flight() {
    let mut q = open();
    let first = add(&mut q, starred(1, true), 0);
    q.take_due(2_000, 10).unwrap();
    let second = add(&mut q, starred(1, false), 2_100);
    take_effects(&mut q);
    q.cancel(second, 2_200).unwrap();
    assert_eq!(
        take_effects(&mut q),
        vec![
            delta("INBOX", 1, -1),
            LocalEffect::SetFlagged {
                account: A,
                folder: "INBOX".into(),
                uid: 1,
                value: true
            }
        ]
    );
    assert_eq!(
        q.queue().get(first).map(|o| o.state),
        Some(OpState::Applying)
    );
}

#[test]
fn confirming_counts_down_without_restoring() {
    let mut q = open();
    let id = add(&mut q, seen(1, true), 0);
    q.take_due(2_000, 10).unwrap();
    take_effects(&mut q);
    q.confirm(id).unwrap();
    assert_eq!(take_effects(&mut q), vec![delta("INBOX", 1, -1)]);
}

#[test]
fn a_confirmed_delete_removes_the_row() {
    let mut q = open();
    let id = add(
        &mut q,
        OpKind::DeleteForever {
            folder: "Trash".into(),
            uid: 4,
        },
        0,
    );
    q.take_due(WINDOW, 10).unwrap();
    take_effects(&mut q);
    q.confirm(id).unwrap();
    assert_eq!(
        take_effects(&mut q),
        vec![
            delta("Trash", 4, -1),
            LocalEffect::RemoveMessage {
                account: A,
                folder: "Trash".into(),
                uid: 4
            }
        ]
    );
}

#[test]
fn a_confirmed_move_with_a_new_uid_copies_then_removes_and_repoints() {
    let mut q = open();
    let moved = add(&mut q, mv(1, "Archive"), 0);
    let later = add(&mut q, starred(1, true), 0);
    q.take_due(WINDOW, 10).unwrap();
    take_effects(&mut q);
    q.confirm_moved(moved, Some(("Archive".into(), 77)))
        .unwrap();
    assert_eq!(
        take_effects(&mut q),
        vec![
            delta("INBOX", 1, -1),
            LocalEffect::CopyMessage {
                account: A,
                from: ("INBOX".into(), 1),
                to: ("Archive".into(), 77)
            },
            LocalEffect::RemoveMessage {
                account: A,
                folder: "INBOX".into(),
                uid: 1
            }
        ]
    );
    assert_eq!(
        q.store.ops.get(&later).map(|o| o.kind.target().1),
        Some(77),
        "the later change is stored against the new UID"
    );
}

#[test]
fn a_confirmed_move_without_a_uid_stores_the_awaiting_mark_and_keeps_the_row() {
    let mut q = open();
    let moved = add(&mut q, mv(1, "Archive"), 0);
    q.take_due(WINDOW, 10).unwrap();
    take_effects(&mut q);
    q.confirm_moved_with(moved, None, Some("<m1@example.org>".into()), 9_000)
        .unwrap();
    assert_eq!(take_effects(&mut q), vec![delta("INBOX", 1, -1)]);
    let stored = q
        .store
        .awaiting
        .get(&(A, "INBOX".to_owned(), 1))
        .unwrap()
        .clone();
    assert_eq!(stored.dest_folder, "Archive");
    assert_eq!(stored.message_id.as_deref(), Some("<m1@example.org>"));
    assert_eq!(stored.created_ms, 9_000);
}

#[test]
fn repoint_resolves_the_awaiting_mark_and_moves_the_row() {
    let mut q = open();
    let moved = add(&mut q, mv(1, "Archive"), 0);
    add(&mut q, seen(1, true), 0);
    q.take_due(WINDOW, 10).unwrap();
    q.confirm_moved(moved, None).unwrap();
    take_effects(&mut q);
    assert_eq!(
        q.repoint(A, ("INBOX", 1), ("Archive".into(), 9)).unwrap(),
        1
    );
    assert!(q.store.awaiting.is_empty());
    assert_eq!(
        take_effects(&mut q),
        vec![
            LocalEffect::CopyMessage {
                account: A,
                from: ("INBOX".into(), 1),
                to: ("Archive".into(), 9)
            },
            LocalEffect::RemoveMessage {
                account: A,
                folder: "INBOX".into(),
                uid: 1
            }
        ]
    );
}

#[test]
fn repoint_with_nothing_waiting_writes_nothing() {
    let mut q = open();
    let commits = q.store.commits;
    assert_eq!(
        q.repoint(A, ("INBOX", 1), ("Archive".into(), 9)).unwrap(),
        0
    );
    assert_eq!(q.store.commits, commits);
}

#[test]
fn repoint_after_a_reset_puts_the_local_state_on_the_new_row() {
    let mut q = open();
    add(&mut q, seen(5, true), 0);
    add(&mut q, mv(5, "Trash"), 0);
    take_effects(&mut q);
    assert_eq!(
        q.repoint_after_reset(A, ("INBOX", 5), ("INBOX".into(), 2))
            .unwrap(),
        2
    );
    assert_eq!(
        take_effects(&mut q),
        vec![
            set_seen(2, true),
            delta("INBOX", 2, 1),
            LocalEffect::Hide {
                account: A,
                folder: "INBOX".into(),
                uid: 2
            },
            delta("INBOX", 2, 1)
        ]
    );
}

#[test]
fn abandon_fails_the_waiting_operations_and_removes_the_old_row() {
    let mut q = open();
    let moved = add(&mut q, mv(1, "Archive"), 0);
    let later = add(&mut q, seen(1, true), 0);
    q.take_due(WINDOW, 10).unwrap();
    q.confirm_moved(moved, None).unwrap();
    take_effects(&mut q);
    let failed = q
        .abandon(A, ("INBOX", 1), "Couldn't find the message after moving it")
        .unwrap();
    assert_eq!(failed, vec![later]);
    assert!(q.store.awaiting.is_empty());
    assert_eq!(
        q.store
            .ops
            .get(&later)
            .and_then(|o| o.last_error.as_deref()),
        Some("Couldn't find the message after moving it")
    );
    assert_eq!(
        take_effects(&mut q),
        vec![
            delta("INBOX", 1, -1),
            LocalEffect::RemoveMessage {
                account: A,
                folder: "INBOX".into(),
                uid: 1
            }
        ]
    );
}

#[test]
fn abandon_after_a_reset_leaves_the_rows_alone() {
    let mut q = open();
    add(&mut q, seen(1, true), 0);
    take_effects(&mut q);
    assert_eq!(
        q.abandon_after_reset(A, ("INBOX", 1), "gone")
            .unwrap()
            .len(),
        1
    );
    assert!(take_effects(&mut q).is_empty());
}

#[test]
fn take_due_is_stored_as_applying() {
    let mut q = open();
    let id = add(&mut q, seen(1, true), 0);
    q.take_due(2_000, 10).unwrap();
    let row = q.store.ops.get(&id).unwrap();
    assert_eq!((row.state, row.attempts), (OpState::Applying, 1));
}

#[test]
fn a_failed_commit_leaves_memory_as_it_was_and_returns_the_error() {
    let mut q = open();
    let id = add(&mut q, seen(1, true), 0);
    q.store.fail = true;

    assert!(q.enqueue(A, seen(2, true), 0, WINDOW).is_err());
    assert_eq!(q.queue().iter().count(), 1, "the new operation is not kept");

    assert!(q.take_due(2_000, 10).is_err());
    assert_eq!(q.queue().get(id).map(|o| o.state), Some(OpState::Queued));
    assert_eq!(q.queue().get(id).map(|o| o.attempts), Some(0));

    assert!(matches!(q.cancel(id, 100), Err(QueueError::Store(_))));
    assert_eq!(q.queue().get(id).map(|o| o.state), Some(OpState::Queued));

    q.store.fail = false;
    assert_eq!(q.take_due(2_000, 10).unwrap().len(), 1);
}

#[test]
fn a_failed_commit_does_not_burn_ids() {
    let mut q = open();
    add(&mut q, seen(1, true), 0);
    q.store.fail = true;
    assert!(q.enqueue(A, seen(2, true), 0, WINDOW).is_err());
    q.store.fail = false;
    assert_eq!(add(&mut q, seen(2, true), 0), 2);
}

#[test]
fn open_puts_applying_operations_back_and_stores_that() {
    let mut store = MemStore::default();
    let mut q = PersistentQueue::open(MemStore::default()).unwrap();
    let id = add(&mut q, seen(1, true), 0);
    q.take_due(2_000, 10).unwrap();
    std::mem::swap(&mut store, &mut q.store);

    let q = PersistentQueue::open(store).unwrap();
    assert_eq!(q.queue().get(id).map(|o| o.state), Some(OpState::Queued));
    assert_eq!(q.store.ops.get(&id).map(|o| o.state), Some(OpState::Queued));
}

#[test]
fn new_ids_start_above_every_stored_id_even_for_purged_rows() {
    let store = MemStore {
        max_id: 41,
        ..MemStore::default()
    };
    let mut q = PersistentQueue::open(store).unwrap();
    assert_eq!(add(&mut q, seen(1, true), 0), 42);
}

#[test]
fn purge_removes_finished_rows_from_the_store() {
    let mut q = open();
    let id = add(&mut q, seen(1, true), 0);
    q.take_due(2_000, 10).unwrap();
    q.confirm(id).unwrap();
    assert_eq!(q.purge_finished().unwrap(), 1);
    assert!(q.store.ops.is_empty());
    assert_eq!(q.purge_finished().unwrap(), 0);
}

#[test]
fn recording_a_lookup_is_stored() {
    let mut q = open();
    let moved = add(&mut q, mv(1, "Archive"), 0);
    q.take_due(WINDOW, 10).unwrap();
    q.confirm_moved(moved, None).unwrap();
    assert_eq!(q.record_lookup(A, ("INBOX", 1)).unwrap(), Some(1));
    assert_eq!(
        q.store
            .awaiting
            .get(&(A, "INBOX".to_owned(), 1))
            .map(|a| a.lookups),
        Some(1)
    );
    assert_eq!(q.record_lookup(A, ("INBOX", 9)).unwrap(), None);
}

#[test]
fn a_move_confirmed_without_following_removes_the_old_row() {
    let mut q = open();
    let moved = add(&mut q, mv(1, "Archive"), 0);
    q.take_due(WINDOW, 10).unwrap();
    take_effects(&mut q);
    q.confirm(moved).unwrap();
    assert_eq!(
        take_effects(&mut q),
        vec![
            delta("INBOX", 1, -1),
            LocalEffect::RemoveMessage {
                account: A,
                folder: "INBOX".into(),
                uid: 1
            }
        ]
    );
}
