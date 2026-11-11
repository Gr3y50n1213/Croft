// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Tests for [`SqliteOpStore`] and the queue on top of it. "Drop the queue and reload it" means
//! closing the database file and opening it again.

// Tests may unwrap: a panic is the failure report.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};

use croft_mail_core::{
    AwaitingUid, Batch, Enqueued, LocalEffect, Op, OpKind, OpState, OpStore, PersistentQueue,
    QueueError, Rejection,
};
use croft_protocols::{BackendError, Flags, RemoteFolder, RemoteHeader, SpecialUse};
use serde_json::json;

use crate::accounts::{self, NewAccount, Protocol};
use crate::messages::{self, Message};
use crate::{folders, Db, SqliteOpStore, StorageError};

const WINDOW: i64 = 5_000;

type Queue = PersistentQueue<SqliteOpStore>;

struct Fixture {
    _dir: tempfile::TempDir,
    path: PathBuf,
    a1: i64,
    a2: i64,
}

fn folder(path: &str) -> RemoteFolder {
    RemoteFolder {
        path: path.into(),
        delimiter: Some('/'),
        special_use: SpecialUse::Other,
        selectable: true,
    }
}

fn header(uid: u32) -> RemoteHeader {
    RemoteHeader {
        uid,
        message_id: Some(format!("<m{uid}@example.org>")),
        references: vec![],
        subject: format!("Subject {uid}"),
        from_name: Some("Sam Rivera".into()),
        from_addr: Some("sam@example.org".into()),
        date_ms: 1_000 + i64::from(uid),
        size: 100,
        flags: Flags::default(),
    }
}

/// Two accounts, each with INBOX and Archive; messages with UIDs 1 to 3 in each INBOX (unread).
fn fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("croft.db");
    let db = Db::open(&path, None).unwrap();
    let c = db.connection();
    let mut ids = Vec::new();
    for (n, email) in [(1, "one@example.org"), (2, "two@example.org")] {
        let a = accounts::insert(
            c,
            &NewAccount {
                label: format!("Account {n}"),
                email: email.into(),
                protocol: Protocol::Imap,
                provider: "generic".into(),
                config: json!({}),
                color: None,
                position: n,
            },
            1,
        )
        .unwrap();
        let inbox = folders::upsert(c, a, &folder("INBOX")).unwrap();
        folders::upsert(c, a, &folder("Archive")).unwrap();
        for uid in 1..=3 {
            messages::upsert(c, a, inbox, &header(uid)).unwrap();
        }
        ids.push(a);
    }
    Fixture {
        _dir: dir,
        path,
        a1: ids[0],
        a2: ids[1],
    }
}

fn open(path: &Path) -> Queue {
    PersistentQueue::open(SqliteOpStore::new(Db::open(path, None).unwrap())).unwrap()
}

fn seen(uid: u32, value: bool) -> OpKind {
    OpKind::SetSeen {
        folder: "INBOX".into(),
        uid,
        value,
    }
}

fn starred(uid: u32, value: bool) -> OpKind {
    OpKind::SetFlagged {
        folder: "INBOX".into(),
        uid,
        value,
    }
}

fn mv(uid: u32, dest: &str) -> OpKind {
    OpKind::MoveTo {
        folder: "INBOX".into(),
        uid,
        dest: dest.into(),
    }
}

fn add(q: &mut Queue, account: i64, kind: OpKind, now: i64) -> u64 {
    match q.enqueue(account, kind, now, WINDOW).unwrap() {
        Enqueued::Queued(id) => id,
        other => panic!("expected a new operation, got {other:?}"),
    }
}

fn all_ops(q: &Queue) -> Vec<Op> {
    q.queue().iter().cloned().collect()
}

fn db(q: &Queue) -> &Db {
    q.store().db()
}

/// The message row at (account, folder path, uid), whether or not it is hidden.
fn row(q: &Queue, account: i64, folder: &str, uid: u32) -> Option<Message> {
    let c = db(q).connection();
    let id: Option<i64> = c
        .query_row(
            "SELECT m.id FROM message m JOIN folder f ON f.id = m.folder_id
              WHERE m.account_id = ?1 AND f.account_id = ?1 AND f.path = ?2 AND m.uid = ?3",
            rusqlite::params![account, folder, uid],
            |r| r.get(0),
        )
        .ok();
    id.and_then(|id| messages::get(c, account, id).unwrap())
}

fn count(q: &Queue, sql: &str) -> i64 {
    db(q).connection().query_row(sql, [], |r| r.get(0)).unwrap()
}

fn listed(q: &Queue, account: i64, folder: &str) -> Vec<i64> {
    let c = db(q).connection();
    let fid = folders::upsert(c, account, &self::folder(folder)).unwrap();
    messages::list_by_folder(c, account, fid, 0, 50, 0)
        .unwrap()
        .into_iter()
        .map(|m| m.uid)
        .collect()
}

// ---- Done when: survive a reload --------------------------------------------------------------

#[test]
fn three_enqueued_ops_come_back_the_same_and_in_order() {
    let f = fixture();
    let mut q = open(&f.path);
    add(&mut q, f.a1, seen(1, true), 1_000);
    add(&mut q, f.a1, mv(2, "Archive"), 1_100);
    add(&mut q, f.a2, starred(3, true), 1_200);
    let before = all_ops(&q);
    assert_eq!(before.len(), 3);
    assert_eq!(
        before.iter().map(|o| o.id).collect::<Vec<_>>(),
        vec![1, 2, 3]
    );
    drop(q);

    let q = open(&f.path);
    assert_eq!(all_ops(&q), before);
}

#[test]
fn an_op_killed_while_applying_is_queued_again_after_reload() {
    let f = fixture();
    let mut q = open(&f.path);
    let id = add(&mut q, f.a1, seen(1, true), 0);
    assert_eq!(q.take_due(2_000, 10).unwrap().len(), 1);
    // Simulated kill: the row says applying on disk, then the process goes away.
    assert_eq!(
        count(&q, "SELECT count(*) FROM op_queue WHERE state = 'applying'"),
        1
    );
    drop(q);

    let q = open(&f.path);
    let op = q.queue().get(id).unwrap();
    assert_eq!(op.state, OpState::Queued);
    assert_eq!(
        op.attempts, 1,
        "the attempt that may have reached the server still counts"
    );
    assert_eq!(
        count(&q, "SELECT count(*) FROM op_queue WHERE state = 'queued'"),
        1,
        "the recovery is written back"
    );
    assert_eq!(
        count(&q, "SELECT count(*) FROM op_queue WHERE state = 'applying'"),
        0
    );
}

#[test]
fn undo_inside_the_window_leaves_no_queued_row_and_shows_the_message() {
    let f = fixture();
    let mut q = open(&f.path);
    let id = add(&mut q, f.a1, mv(1, "Archive"), 0);
    assert!(
        !listed(&q, f.a1, "INBOX").contains(&1),
        "hidden while waiting"
    );
    assert_eq!(row(&q, f.a1, "INBOX", 1).unwrap().pending_ops, 1);

    q.cancel(id, 1_000).unwrap();
    assert_eq!(
        count(&q, "SELECT count(*) FROM op_queue WHERE state = 'queued'"),
        0
    );
    assert_eq!(
        count(
            &q,
            "SELECT count(*) FROM op_queue WHERE state = 'cancelled'"
        ),
        1
    );
    assert!(listed(&q, f.a1, "INBOX").contains(&1));
    assert_eq!(row(&q, f.a1, "INBOX", 1).unwrap().pending_ops, 0);
    drop(q);
    assert!(open(&f.path).queue().iter().next().is_none());
}

#[test]
fn undo_after_the_window_is_refused_and_changes_nothing() {
    let f = fixture();
    let mut q = open(&f.path);
    let id = add(&mut q, f.a1, mv(1, "Archive"), 0);
    assert!(matches!(
        q.cancel(id, WINDOW),
        Err(QueueError::Cancel(
            croft_mail_core::ops::CancelError::TooLate
        ))
    ));
    assert_eq!(
        count(&q, "SELECT count(*) FROM op_queue WHERE state = 'queued'"),
        1
    );
}

#[test]
fn read_then_unread_inside_the_window_sends_nothing_and_leaves_the_row_as_it_was() {
    let f = fixture();
    let mut q = open(&f.path);
    add(&mut q, f.a1, seen(1, true), 0);
    let m = row(&q, f.a1, "INBOX", 1).unwrap();
    assert!(m.is_seen, "applied locally at once");
    assert_eq!(m.pending_ops, 1);

    assert_eq!(
        q.enqueue(f.a1, seen(1, false), 500, WINDOW).unwrap(),
        Enqueued::CancelledOut
    );
    let m = row(&q, f.a1, "INBOX", 1).unwrap();
    assert!(!m.is_seen, "back to the original value");
    assert_eq!(m.pending_ops, 0);
    assert_eq!(
        count(&q, "SELECT count(*) FROM op_queue WHERE state = 'queued'"),
        0
    );
    assert!(q.take_due(60_000, 10).unwrap().is_empty());
    drop(q);
    assert!(open(&f.path).take_due(60_000, 10).unwrap().is_empty());
}

#[test]
fn a_move_confirmed_without_a_new_uid_still_holds_the_next_operation_after_a_restart() {
    let f = fixture();
    let mut q = open(&f.path);
    let moved = add(&mut q, f.a1, mv(1, "Archive"), 0);
    let later = add(&mut q, f.a1, starred(1, true), 100);
    let due = q.take_due(WINDOW, 10).unwrap();
    assert_eq!(due.iter().map(|o| o.id).collect::<Vec<_>>(), vec![moved]);
    q.confirm_moved_with(moved, None, Some("<m1@example.org>".into()), 6_000)
        .unwrap();
    assert_eq!(
        count(&q, "SELECT count(*) FROM op_awaiting_uid"),
        1,
        "stored in the confirming transaction"
    );
    drop(q);

    let mut q = open(&f.path);
    assert!(
        q.take_due(600_000, 10).unwrap().is_empty(),
        "held after the restart"
    );
    assert_eq!(q.queue().next_wake_ms(), None);
    let waiting: Vec<&AwaitingUid> = q.queue().awaiting().collect();
    assert_eq!(waiting.len(), 1);
    assert_eq!(
        waiting[0],
        &AwaitingUid {
            account: f.a1,
            folder: "INBOX".into(),
            uid: 1,
            dest_folder: "Archive".into(),
            message_id: Some("<m1@example.org>".into()),
            lookups: 0,
            created_ms: 6_000,
        }
    );
    assert_eq!(q.queue().get(later).map(|o| o.state), Some(OpState::Queued));
    assert!(
        row(&q, f.a1, "INBOX", 1).is_some(),
        "the local row is kept until the message is found"
    );

    // Found: the later operation follows the message, and that also survives a restart.
    assert_eq!(
        q.repoint(f.a1, ("INBOX", 1), ("Archive".into(), 9))
            .unwrap(),
        1
    );
    drop(q);
    let mut q = open(&f.path);
    assert_eq!(count(&q, "SELECT count(*) FROM op_awaiting_uid"), 0);
    let due = q.take_due(600_000, 10).unwrap();
    assert_eq!(due.len(), 1);
    assert_eq!(due[0].kind.target(), ("Archive", 9));
}

// ---- the local change and the queue row are one transaction -----------------------------------

#[test]
fn enqueue_changes_the_message_and_the_queue_together() {
    let f = fixture();
    let mut q = open(&f.path);
    add(&mut q, f.a1, starred(2, true), 0);
    let m = row(&q, f.a1, "INBOX", 2).unwrap();
    assert!(m.is_flagged);
    assert_eq!(m.pending_ops, 1);
    assert_eq!(count(&q, "SELECT count(*) FROM op_queue"), 1);
}

#[test]
fn a_batch_that_fails_halfway_changes_nothing() {
    let f = fixture();
    let mut q = open(&f.path);
    let good = Op {
        id: 1,
        account: f.a1,
        kind: seen(1, true),
        state: OpState::Queued,
        created_ms: 0,
        not_before_ms: 1_500,
        attempts: 0,
        last_error: None,
    };
    let bad = Op {
        id: 2,
        account: 999, // no such account: the foreign key refuses it
        ..good.clone()
    };
    let batch = Batch {
        ops: vec![good, bad],
        effects: vec![LocalEffect::SetSeen {
            account: f.a1,
            folder: "INBOX".into(),
            uid: 1,
            value: true,
        }],
        ..Batch::default()
    };
    let mut store = SqliteOpStore::new(Db::open(&f.path, None).unwrap());
    let err = store.commit(&batch).unwrap_err();
    assert!(matches!(err, StorageError::Sqlite(_)), "{err}");
    assert_eq!(count(&q, "SELECT count(*) FROM op_queue"), 0);
    assert!(!row(&q, f.a1, "INBOX", 1).unwrap().is_seen);

    // The same failure through the queue: memory does not run ahead of the disk.
    assert!(q.enqueue(999, seen(1, true), 0, WINDOW).is_err());
    assert_eq!(q.queue().iter().count(), 0);
}

#[test]
fn an_op_id_that_belongs_to_another_account_is_never_overwritten() {
    let f = fixture();
    let mut q = open(&f.path);
    let id = add(&mut q, f.a1, seen(1, true), 0);
    let mut other = q.queue().get(id).unwrap().clone();
    other.account = f.a2;
    other.state = OpState::Failed;
    let mut store = SqliteOpStore::new(Db::open(&f.path, None).unwrap());
    let batch = Batch {
        ops: vec![other],
        ..Batch::default()
    };
    assert!(matches!(
        store.commit(&batch),
        Err(StorageError::NotFound(_))
    ));
    assert_eq!(
        count(&q, "SELECT count(*) FROM op_queue WHERE state = 'queued'"),
        1
    );
}

#[test]
fn effects_on_one_account_never_touch_the_same_path_and_uid_in_another() {
    let f = fixture();
    let mut q = open(&f.path);
    add(&mut q, f.a1, seen(1, true), 0);
    add(&mut q, f.a1, mv(2, "Archive"), 0);
    let other = row(&q, f.a2, "INBOX", 1).unwrap();
    assert!(!other.is_seen);
    assert_eq!(other.pending_ops, 0);
    assert!(listed(&q, f.a2, "INBOX").contains(&2), "not hidden");
}

#[test]
fn the_pending_count_never_goes_below_zero() {
    let f = fixture();
    let mut store = SqliteOpStore::new(Db::open(&f.path, None).unwrap());
    store
        .commit(&Batch {
            effects: vec![LocalEffect::PendingDelta {
                account: f.a1,
                folder: "INBOX".into(),
                uid: 1,
                delta: -3,
            }],
            ..Batch::default()
        })
        .unwrap();
    let q = open(&f.path);
    assert_eq!(row(&q, f.a1, "INBOX", 1).unwrap().pending_ops, 0);
}

#[test]
fn effects_on_a_message_that_is_not_there_do_nothing() {
    let f = fixture();
    let mut q = open(&f.path);
    add(&mut q, f.a1, seen(77, true), 0);
    assert_eq!(count(&q, "SELECT count(*) FROM op_queue"), 1);
    assert!(row(&q, f.a1, "INBOX", 77).is_none());
}

// ---- finishing --------------------------------------------------------------------------------

#[test]
fn a_failed_flag_change_puts_the_old_value_back_and_a_failed_move_shows_the_row() {
    let f = fixture();
    let mut q = open(&f.path);
    let flag = add(&mut q, f.a1, seen(1, true), 0);
    let moved = add(&mut q, f.a1, mv(2, "Archive"), 0);
    q.take_due(WINDOW, 10).unwrap();
    let err = BackendError::Permanent("refused".into());
    assert_eq!(
        q.reject(flag, &err, WINDOW).unwrap(),
        Some(Rejection::Failed)
    );
    assert_eq!(
        q.reject(moved, &err, WINDOW).unwrap(),
        Some(Rejection::Failed)
    );

    let m = row(&q, f.a1, "INBOX", 1).unwrap();
    assert!(!m.is_seen);
    assert_eq!(m.pending_ops, 0);
    assert!(listed(&q, f.a1, "INBOX").contains(&2));
    assert_eq!(row(&q, f.a1, "INBOX", 2).unwrap().pending_ops, 0);
    // The failure reason is stored for the detail view until the next start.
    let reason: String = db(&q)
        .connection()
        .query_row(
            "SELECT last_error FROM op_queue WHERE id = ?1",
            [flag as i64],
            |r| r.get(0),
        )
        .unwrap();
    assert!(!reason.is_empty());
}

#[test]
fn a_confirmed_flag_change_keeps_the_value_and_counts_down() {
    let f = fixture();
    let mut q = open(&f.path);
    let id = add(&mut q, f.a1, seen(1, true), 0);
    q.take_due(WINDOW, 10).unwrap();
    q.confirm(id).unwrap();
    let m = row(&q, f.a1, "INBOX", 1).unwrap();
    assert!(m.is_seen);
    assert_eq!(m.pending_ops, 0);
}

#[test]
fn a_confirmed_delete_removes_the_row() {
    let f = fixture();
    let mut q = open(&f.path);
    let id = add(
        &mut q,
        f.a1,
        OpKind::DeleteForever {
            folder: "INBOX".into(),
            uid: 3,
        },
        0,
    );
    assert!(row(&q, f.a1, "INBOX", 3).is_some(), "kept while waiting");
    q.take_due(WINDOW, 10).unwrap();
    assert!(row(&q, f.a1, "INBOX", 3).is_some(), "kept until confirmed");
    q.confirm(id).unwrap();
    assert!(row(&q, f.a1, "INBOX", 3).is_none());
    assert!(row(&q, f.a2, "INBOX", 3).is_some());
}

#[test]
fn a_confirmed_move_with_a_new_uid_shows_the_copy_in_the_destination() {
    let f = fixture();
    let mut q = open(&f.path);
    // A cached body and an attachment must come along.
    let c = db(&q).connection();
    let old = row(&q, f.a1, "INBOX", 1).unwrap();
    c.execute(
        "INSERT INTO message_body (message_id, text_plain) VALUES (?1, 'hello')",
        [old.id],
    )
    .unwrap();
    c.execute(
        "INSERT INTO attachment (message_id, part_id, mime_type) VALUES (?1, '2', 'image/png')",
        [old.id],
    )
    .unwrap();

    let moved = add(&mut q, f.a1, mv(1, "Archive"), 0);
    add(&mut q, f.a1, starred(1, true), 100);
    q.take_due(WINDOW, 10).unwrap();
    q.confirm_moved(moved, Some(("Archive".into(), 77)))
        .unwrap();

    assert!(row(&q, f.a1, "INBOX", 1).is_none(), "old row removed");
    let copy = row(&q, f.a1, "Archive", 77).unwrap();
    assert!(!copy.local_hidden);
    assert!(copy.is_flagged, "the star made while it moved is kept");
    assert_eq!(copy.pending_ops, 1, "only the star is still waiting");
    assert_eq!(copy.subject, old.subject);
    assert_eq!(listed(&q, f.a1, "Archive"), vec![77]);
    assert_eq!(
        count(
            &q,
            "SELECT count(*) FROM message_body b JOIN message m ON m.id = b.message_id WHERE m.uid = 77 AND b.text_plain = 'hello'"
        ),
        1
    );
    assert_eq!(
        count(
            &q,
            "SELECT count(*) FROM attachment a JOIN message m ON m.id = a.message_id WHERE m.uid = 77"
        ),
        1
    );
    // The star now runs against the copy.
    let due = q.take_due(600_000, 10).unwrap();
    assert_eq!(due.len(), 1);
    assert_eq!(due[0].kind.target(), ("Archive", 77));
    q.confirm(due[0].id).unwrap();
    assert_eq!(row(&q, f.a1, "Archive", 77).unwrap().pending_ops, 0);
}

#[test]
fn a_move_whose_destination_row_already_exists_keeps_that_row() {
    let f = fixture();
    let mut q = open(&f.path);
    let c = db(&q).connection();
    let archive = folders::upsert(c, f.a1, &folder("Archive")).unwrap();
    messages::upsert(c, f.a1, archive, &header(50)).unwrap();
    let moved = add(&mut q, f.a1, mv(1, "Archive"), 0);
    add(&mut q, f.a1, seen(1, true), 0);
    q.take_due(WINDOW, 10).unwrap();
    q.confirm_moved(moved, Some(("Archive".into(), 50)))
        .unwrap();
    let m = row(&q, f.a1, "Archive", 50).unwrap();
    assert!(m.is_seen);
    assert_eq!(m.pending_ops, 1);
    assert_eq!(count(&q, "SELECT count(*) FROM message WHERE uid = 50"), 1);
}

#[test]
fn abandoning_removes_the_old_row_and_fails_the_waiting_operations() {
    let f = fixture();
    let mut q = open(&f.path);
    let moved = add(&mut q, f.a1, mv(1, "Archive"), 0);
    let later = add(&mut q, f.a1, seen(1, true), 0);
    q.take_due(WINDOW, 10).unwrap();
    q.confirm_moved_with(moved, None, None, 0).unwrap();
    let failed = q
        .abandon(
            f.a1,
            ("INBOX", 1),
            "Couldn't find the message after moving it",
        )
        .unwrap();
    assert_eq!(failed, vec![later]);
    assert!(row(&q, f.a1, "INBOX", 1).is_none());
    assert_eq!(count(&q, "SELECT count(*) FROM op_awaiting_uid"), 0);
    drop(q);
    assert_eq!(open(&f.path).queue().unfinished(), 0);
}

#[test]
fn a_lookup_count_survives_a_restart() {
    let f = fixture();
    let mut q = open(&f.path);
    let moved = add(&mut q, f.a1, mv(1, "Archive"), 0);
    q.take_due(WINDOW, 10).unwrap();
    q.confirm_moved(moved, None).unwrap();
    q.record_lookup(f.a1, ("INBOX", 1)).unwrap();
    q.record_lookup(f.a1, ("INBOX", 1)).unwrap();
    drop(q);
    let q = open(&f.path);
    assert_eq!(q.queue().awaiting().next().map(|a| a.lookups), Some(2));
}

// ---- ids, purge, unreadable rows --------------------------------------------------------------

#[test]
fn new_ids_start_above_every_id_ever_stored() {
    let f = fixture();
    let mut q = open(&f.path);
    let a = add(&mut q, f.a1, seen(1, true), 0);
    let b = add(&mut q, f.a1, seen(2, true), 0);
    q.take_due(2_000, 10).unwrap();
    q.confirm(a).unwrap();
    q.confirm(b).unwrap();
    drop(q);

    let mut q = open(&f.path);
    assert_eq!(
        count(&q, "SELECT count(*) FROM op_queue"),
        0,
        "finished rows are discarded on load"
    );
    assert_eq!(add(&mut q, f.a1, seen(3, true), 5_000), b + 1);
}

#[test]
fn purging_finished_operations_deletes_their_rows() {
    let f = fixture();
    let mut q = open(&f.path);
    let a = add(&mut q, f.a1, seen(1, true), 0);
    add(&mut q, f.a1, seen(2, true), 0);
    q.take_due(2_000, 1).unwrap();
    q.confirm(a).unwrap();
    assert_eq!(q.purge_finished().unwrap(), 1);
    assert_eq!(count(&q, "SELECT count(*) FROM op_queue"), 1);
}

#[test]
fn rows_from_a_newer_app_or_damaged_rows_fail_with_a_reason_instead_of_being_guessed() {
    let f = fixture();
    let mut q = open(&f.path);
    let good = add(&mut q, f.a1, seen(1, true), 0);
    let hidden_move = add(&mut q, f.a1, mv(2, "Archive"), 0);
    // Rewrite the move's payload as a future version, and add a damaged row for message 3.
    let c = db(&q).connection();
    c.execute(
        "UPDATE op_queue SET payload_json = ?2 WHERE id = ?1",
        rusqlite::params![
            hidden_move as i64,
            json!({"v": 2, "folder": "INBOX", "uid": 2, "dest": "Archive", "extra": 1}).to_string()
        ],
    )
    .unwrap();
    c.execute(
        "INSERT INTO op_queue (id, account_id, kind, payload_json, target_key, created_at, not_before)
         VALUES (50, ?1, 'set_seen', 'not json', '3@INBOX', 0, 0)",
        [f.a1],
    )
    .unwrap();
    c.execute(
        "UPDATE message SET pending_ops = 1 WHERE uid = 3 AND account_id = ?1",
        [f.a1],
    )
    .unwrap();
    drop(q);

    let q = open(&f.path);
    assert_eq!(
        all_ops(&q).iter().map(|o| o.id).collect::<Vec<_>>(),
        vec![good]
    );
    let mut bad: Vec<_> = q.unreadable().iter().map(|u| (u.id, u.account)).collect();
    bad.sort_unstable();
    assert_eq!(bad, vec![(hidden_move, f.a1), (50, f.a1)]);
    assert!(q.unreadable().iter().all(|u| !u.reason.is_empty()));
    assert_eq!(
        count(&q, "SELECT count(*) FROM op_queue WHERE state = 'failed'"),
        2
    );
    // The hidden message is shown again and the stuck count is released.
    assert!(listed(&q, f.a1, "INBOX").contains(&2));
    assert_eq!(row(&q, f.a1, "INBOX", 2).unwrap().pending_ops, 0);
    assert_eq!(row(&q, f.a1, "INBOX", 3).unwrap().pending_ops, 0);
    // The readable one is untouched.
    assert_eq!(row(&q, f.a1, "INBOX", 1).unwrap().pending_ops, 1);
}

#[test]
fn payload_json_carries_a_version_and_the_fields_of_the_kind() {
    let f = fixture();
    let mut q = open(&f.path);
    add(&mut q, f.a1, mv(1, "Archive"), 0);
    let (kind, payload, target): (String, String, String) = db(&q)
        .connection()
        .query_row(
            "SELECT kind, payload_json, target_key FROM op_queue",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_eq!(kind, "move");
    assert_eq!(target, "1@INBOX");
    let v: serde_json::Value = serde_json::from_str(&payload).unwrap();
    assert_eq!(
        v,
        json!({"v": 1, "folder": "INBOX", "uid": 1, "dest": "Archive"})
    );
}

#[test]
fn offline_and_pause_are_not_stored() {
    let f = fixture();
    let mut q = open(&f.path);
    let id = add(&mut q, f.a1, seen(1, true), 0);
    q.set_offline(f.a1, true);
    drop(q);
    let mut q = open(&f.path);
    assert!(!q.queue().is_offline(f.a1));
    assert_eq!(q.take_due(2_000, 10).unwrap()[0].id, id);
}

#[test]
fn a_transient_failure_is_stored_with_its_retry_time() {
    let f = fixture();
    let mut q = open(&f.path);
    let id = add(&mut q, f.a1, seen(1, true), 0);
    q.take_due(2_000, 10).unwrap();
    q.reject(id, &BackendError::Transient("timeout".into()), 2_000)
        .unwrap();
    drop(q);
    let q = open(&f.path);
    let op = q.queue().get(id).unwrap();
    assert_eq!(op.state, OpState::Queued);
    assert_eq!(op.not_before_ms, 7_000);
    assert_eq!(op.attempts, 1);
    assert!(op.last_error.is_some());
}
