// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Tests for the review of the sync engine: an interrupted UIDVALIDITY reset, an operation in
//! flight when its account is removed, long lock holds, a failing database, a stale back-off and
//! Message-ID lookups that cannot tell two copies apart.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use common::{create_account, Call, FakeBackend, Method, Rig};
use croft_engine::{
    Change, Engine, EngineConfig, EngineError, ManualClock, OpState, REASON_AMBIGUOUS_COPY,
    REASON_LOST_AFTER_RESET,
};
use croft_protocols::{BackendError, FlagChange, Flags};
use croft_storage::{folders, messages, Db};

fn row_id(rig: &Rig, folder: &str, uid: u32) -> i64 {
    rig.row(folder, uid).unwrap().id
}

fn failures(changes: &[Change]) -> Vec<(u64, String)> {
    changes
        .iter()
        .filter_map(|c| match c {
            Change::OperationFailed { op, reason, .. } => Some((*op, reason.clone())),
            _ => None,
        })
        .collect()
}

/// How many rows the folder has locally (hidden ones included).
fn row_count(rig: &Rig, folder: &str) -> usize {
    let account = rig.account;
    rig.ui.read(|db| {
        let f = folders::find_by_path(db.connection(), account, folder)
            .unwrap()
            .unwrap();
        messages::folder_flags(db.connection(), account, f.id)
            .unwrap()
            .len()
    })
}

fn deletes(rig: &Rig) -> usize {
    rig.backend
        .calls()
        .iter()
        .filter(|c| matches!(c, Call::Delete(..)))
        .count()
}

// ---- 1. an interrupted reset -------------------------------------------------------------------

#[tokio::test]
async fn an_interrupted_fetch_after_a_uidvalidity_reset_is_repaired_by_the_next_syncs() {
    let rig = Rig::new();
    rig.backend.st().condstore = true;
    for id in ["a@x", "b@x", "c@x"] {
        rig.backend.add("INBOX", id, Flags::default());
    }
    rig.sync().await;
    assert_eq!(rig.listed("INBOX").len(), 3);
    rig.backend.change_uid_validity("INBOX", 300);
    // The bulk fetch after the reset fails once (the network drops, or the app is killed).
    rig.backend.st().fail_next.push_back((
        Method::Fetch,
        BackendError::Transient("connection dropped".into()),
    ));
    assert!(rig.ui.run_once(rig.account, true).await.is_err());
    rig.advance(60_000);
    rig.sync().await;
    rig.advance(60_000);
    rig.sync().await;
    assert_eq!(
        rig.listed("INBOX").len(),
        3,
        "the folder was left empty and later syncs skipped it"
    );
}

#[tokio::test]
async fn a_failure_between_the_deletion_chunks_of_a_reset_leaves_a_folder_the_next_sync_repairs() {
    let rig = Rig::new();
    rig.backend.st().condstore = true;
    for i in 0..1_500 {
        rig.backend
            .add("INBOX", &format!("m{i}@x"), Flags::default());
    }
    rig.sync().await;
    assert_eq!(row_count(&rig, "INBOX"), 1_500);
    rig.backend.change_uid_validity("INBOX", 300);
    // A later chunk of the deletion fails (the disk, a kill): the earlier chunks are already gone.
    rig.ui.read(|db| {
        db.connection()
            .execute_batch(
                "CREATE TRIGGER refuse_one_row BEFORE DELETE ON message WHEN OLD.uid = 1200
                 BEGIN SELECT RAISE(ABORT, 'refused'); END;",
            )
            .unwrap();
    });
    assert!(matches!(
        rig.ui.run_once(rig.account, true).await,
        Err(EngineError::Storage(_))
    ));
    let stored = rig.ui.read(|db| {
        folders::find_by_path(db.connection(), rig.account, "INBOX")
            .unwrap()
            .unwrap()
    });
    assert_eq!(
        stored.uid_validity,
        Some(100),
        "the new UIDVALIDITY is only stored by the last step"
    );
    assert!(row_count(&rig, "INBOX") < 1_500, "some chunks went");

    rig.ui.read(|db| {
        db.connection()
            .execute_batch("DROP TRIGGER refuse_one_row;")
            .unwrap();
    });
    rig.advance(60_000);
    rig.sync().await;
    assert_eq!(row_count(&rig, "INBOX"), 1_500);
    // The server renumbered in reverse order: the newest message of the first numbering is UID 1.
    let first = rig.row("INBOX", 1).unwrap();
    assert_eq!(first.message_id_hdr.as_deref(), Some("<m1499@x>"));
    let stored = rig.ui.read(|db| {
        folders::find_by_path(db.connection(), rig.account, "INBOX")
            .unwrap()
            .unwrap()
    });
    assert_eq!(stored.uid_validity, Some(300));
    assert_eq!(stored.uid_next, Some(1_501), "the full status is stored");
}

// ---- 2. an operation in flight when the account goes away --------------------------------------

#[tokio::test]
async fn an_operation_in_flight_when_its_account_is_removed_is_sent_once_after_it_is_added_again() {
    let dir = tempfile::tempdir().unwrap();
    let db = Db::open(&dir.path().join("croft.db"), None).unwrap();
    let account = create_account(&db, "one@example.org");
    let clock = ManualClock::new(1_000_000);
    let backend = FakeBackend::new(Arc::new(clock.clone()));
    backend.add("INBOX", "a@x", Flags::default());
    let config = EngineConfig {
        shutdown_grace_ms: 50,
        ..EngineConfig::default()
    };
    let engine = Engine::open(db, Arc::new(clock.clone()), config).unwrap();
    engine.attach_account(account, backend.clone()).unwrap();
    let ui = engine.handle();
    ui.run_once(account, true).await.unwrap();
    let message = ui.read(|db| {
        let f = folders::find_by_path(db.connection(), account, "INBOX")
            .unwrap()
            .unwrap();
        messages::find_by_uid(db.connection(), account, f.id, 1)
            .unwrap()
            .unwrap()
            .id
    });

    // A task that is stuck in the server's answer to a flag change.
    engine.remove_account(account).await;
    let _gate = backend.gate(Method::SetFlag);
    engine.add_account(account, backend.clone()).unwrap();
    tokio::time::sleep(Duration::from_millis(50)).await;
    ui.mark_seen(account, message, true).unwrap();
    clock.advance(2_000);
    ui.sync_now(account);
    for _ in 0..1_000 {
        if backend
            .calls()
            .iter()
            .any(|c| matches!(c, Call::SetFlag(..)))
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    assert!(backend
        .calls()
        .iter()
        .any(|c| matches!(c, Call::SetFlag(..))));
    // The grace period is 50 ms: the task is aborted in the middle of the call.
    engine.remove_account(account).await;
    backend.ungate(Method::SetFlag);

    engine.attach_account(account, backend.clone()).unwrap();
    backend.clear_calls();
    clock.advance(100_000);
    ui.run_once(account, true).await.unwrap();
    let sent = backend
        .calls()
        .iter()
        .filter(|c| matches!(c, Call::SetFlag(..)))
        .count();
    assert_eq!(sent, 1, "the flag change was never handed out again");
    assert!(backend.flags("INBOX", 1).unwrap().seen);
    let states: Vec<_> = (1..10)
        .filter_map(|i| ui.operation(i))
        .map(|o| o.state)
        .collect();
    assert_eq!(states, vec![OpState::Confirmed]);
}

#[tokio::test]
async fn a_failed_commit_of_an_answer_puts_the_operation_back_instead_of_leaving_it_applying() {
    let rig = Rig::new();
    rig.backend.add("INBOX", "a@x", Flags::default());
    rig.sync().await;
    rig.backend.clear_calls();
    let message = row_id(&rig, "INBOX", 1);
    rig.ui.mark_seen(rig.account, message, true).unwrap();
    // The database refuses to record that the server confirmed the change.
    rig.ui.read(|db| {
        db.connection()
            .execute_batch(
                "CREATE TRIGGER refuse_confirm BEFORE UPDATE ON op_queue
                   WHEN NEW.state = 'confirmed'
                 BEGIN SELECT RAISE(ABORT, 'refused'); END;",
            )
            .unwrap();
    });
    rig.advance(2_000);
    assert!(matches!(
        rig.ui.run_once(rig.account, false).await,
        Err(EngineError::Storage(_))
    ));
    let op = (1..10).find_map(|i| rig.ui.operation(i)).unwrap();
    assert_eq!(op.state, OpState::Queued, "not stuck as applying");

    rig.ui.read(|db| {
        db.connection()
            .execute_batch("DROP TRIGGER refuse_confirm;")
            .unwrap();
    });
    rig.run().await;
    let op = (1..10).find_map(|i| rig.ui.operation(i)).unwrap();
    assert_eq!(op.state, OpState::Confirmed);
    assert!(rig.backend.flags("INBOX", 1).unwrap().seen);
}

// ---- 3. long lock holds ------------------------------------------------------------------------

#[tokio::test]
async fn a_large_sync_applies_its_changes_in_chunks_and_respects_an_action_made_between_them() {
    let rig = Rig::new();
    for i in 0..5_000 {
        rig.backend
            .add("INBOX", &format!("m{i}@x"), Flags::default());
    }
    rig.sync().await;
    assert_eq!(row_count(&rig, "INBOX"), 5_000);
    // The server removes the 3,000 oldest messages, reads the rest, and gets ten new ones.
    {
        let mut st = rig.backend.st();
        let inbox = st.folders.get_mut("INBOX").unwrap();
        inbox.messages.retain(|uid, _| *uid > 3_000);
        for m in inbox.messages.values_mut() {
            m.flags.seen = true;
        }
    }
    for i in 0..10 {
        rig.backend
            .add("INBOX", &format!("new{i}@x"), Flags::default());
    }
    // The last of the messages that are going to be removed. The person acts on it while the sync
    // is still applying its plan; the sync must leave it alone from then on.
    let doomed = row_id(&rig, "INBOX", 3_000);

    let acted = Arc::new(AtomicBool::new(false));
    let accepted = Arc::new(AtomicBool::new(false));
    let acted_at_fetch = Arc::new(AtomicBool::new(false));
    rig.backend.st().fetch_probe = Some((acted.clone(), acted_at_fetch.clone()));
    {
        let ui = rig.ui.clone();
        let account = rig.account;
        let (acted, accepted) = (acted.clone(), accepted.clone());
        // Another task, on the same thread: it runs only when the sync lets go of the thread.
        tokio::spawn(async move {
            accepted.store(
                ui.mark_seen(account, doomed, true).is_ok(),
                Ordering::SeqCst,
            );
            acted.store(true, Ordering::SeqCst);
        });
    }
    rig.sync().await;

    assert!(
        acted_at_fetch.load(Ordering::SeqCst),
        "the action was still waiting when the sync reached its fetch: no step let go of the thread"
    );
    assert!(accepted.load(Ordering::SeqCst), "the action was accepted");
    let kept = rig
        .row("INBOX", 3_000)
        .expect("a row with an action is kept");
    assert!(kept.is_seen && kept.pending_ops == 1);
    assert_eq!(row_count(&rig, "INBOX"), 2_000 + 10 + 1);
    assert!(rig.row("INBOX", 2_999).is_none(), "the others went");
    assert!(
        rig.row("INBOX", 3_001).unwrap().is_seen,
        "server flags taken"
    );
    assert!(rig.row_by_id("INBOX", "new9@x").is_some());
}

// ---- 4. a failing database ---------------------------------------------------------------------

struct CountingLogger {
    database_errors: AtomicUsize,
}

impl log::Log for CountingLogger {
    fn enabled(&self, _: &log::Metadata<'_>) -> bool {
        true
    }
    fn log(&self, record: &log::Record<'_>) {
        if record
            .args()
            .to_string()
            .contains("cycle failed: database error")
        {
            self.database_errors.fetch_add(1, Ordering::SeqCst);
        }
    }
    fn flush(&self) {}
}

static LOGGER: CountingLogger = CountingLogger {
    database_errors: AtomicUsize::new(0),
};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_database_error_makes_the_account_task_wait_before_it_tries_again() {
    log::set_logger(&LOGGER).unwrap();
    log::set_max_level(log::LevelFilter::Warn);

    let db = Db::open_in_memory().unwrap();
    let account = create_account(&db, "one@example.org");
    let clock = ManualClock::new(1_000_000);
    let backend = FakeBackend::new(Arc::new(clock.clone()));
    backend.add("INBOX", "a@x", Flags::default());
    let config = EngineConfig {
        sync_retry_ms: 200,
        ..EngineConfig::default()
    };
    let engine = Engine::open(db, Arc::new(clock.clone()), config).unwrap();
    let ui = engine.handle();
    engine.add_account(account, backend.clone()).unwrap();
    let find = || {
        ui.read(|db| {
            let f = folders::find_by_path(db.connection(), account, "INBOX").unwrap()?;
            messages::find_by_uid(db.connection(), account, f.id, 1).unwrap()
        })
    };
    for _ in 0..1_000 {
        if find().is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    let message = find().unwrap().id;
    ui.mark_seen(account, message, true).unwrap();
    // The database refuses the next change of the queue's rows.
    ui.read(|db| {
        db.connection()
            .execute_batch(
                "CREATE TRIGGER refuse_queue BEFORE UPDATE ON op_queue
                 BEGIN SELECT RAISE(ABORT, 'refused'); END;",
            )
            .unwrap();
    });
    let before = LOGGER.database_errors.load(Ordering::SeqCst);
    clock.advance(2_000);
    ui.sync_now(account);
    tokio::time::sleep(Duration::from_millis(500)).await;
    let tries = LOGGER.database_errors.load(Ordering::SeqCst) - before;
    assert!(tries >= 1, "the task did try");
    assert!(
        tries <= 4,
        "the task spun: {tries} attempts in half a second"
    );

    // The database works again: the change goes out on a later attempt.
    ui.read(|db| {
        db.connection()
            .execute_batch("DROP TRIGGER refuse_queue;")
            .unwrap();
    });
    for _ in 0..1_000 {
        if backend.writes().len() == 1 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    assert_eq!(
        backend.writes(),
        vec![Call::SetFlag(
            "INBOX".into(),
            vec![1],
            FlagChange::Seen(true)
        )]
    );
    engine.shutdown().await;
}

// ---- 5. back-off and connectivity --------------------------------------------------------------

#[tokio::test]
async fn coming_back_online_ends_the_back_off_so_the_change_goes_out_at_once() {
    let rig = Rig::new();
    rig.backend.add("INBOX", "a@x", Flags::default());
    rig.sync().await;
    rig.backend.clear_calls();
    let message = row_id(&rig, "INBOX", 1);
    rig.ui.mark_seen(rig.account, message, true).unwrap();
    rig.backend.st().fail_all = Some(BackendError::Transient("no route to host".into()));
    rig.advance(1_500);
    rig.run().await;
    for wait in [5_000, 30_000, 120_000] {
        rig.advance(wait);
        rig.run().await;
    }
    // Four failures: the next try is ten minutes away.
    assert_eq!(rig.backend.calls().len(), 4);
    rig.backend.st().fail_all = None;
    rig.run().await;
    assert!(rig.backend.writes().is_empty(), "still backing off");

    rig.ui.set_online(rig.account, false);
    rig.ui.set_online(rig.account, true);
    rig.run().await;
    assert_eq!(
        rig.backend.writes(),
        vec![Call::SetFlag(
            "INBOX".into(),
            vec![1],
            FlagChange::Seen(true)
        )]
    );
}

// ---- 6. a Message-ID lookup that cannot tell two copies apart ----------------------------------

/// INBOX and Archive both hold a@x; the server has no UIDPLUS. The person archives the INBOX
/// copy and then does `later` to it, which waits for the new UID.
async fn moved_next_to_a_copy(later: impl FnOnce(&Rig, i64)) -> Rig {
    let rig = Rig::new();
    rig.backend.add("INBOX", "a@x", Flags::default());
    rig.backend.add("Archive", "a@x", Flags::default());
    rig.sync().await;
    rig.backend.clear_calls();
    rig.backend.st().uidplus = false;
    let message = row_id(&rig, "INBOX", 1);
    rig.ui.move_to(rig.account, message, "Archive").unwrap();
    later(&rig, message);
    rig
}

#[tokio::test]
async fn a_delete_after_a_move_is_given_up_when_the_destination_already_had_a_copy() {
    let mut rig = moved_next_to_a_copy(|rig, message| {
        rig.ui.delete_forever(rig.account, message).unwrap();
    })
    .await;
    rig.drain_changes();
    rig.advance(5_000);
    rig.run().await;
    rig.advance(60_000);
    rig.sync().await;
    rig.advance(60_000);
    rig.run().await;

    let failed = failures(&rig.drain_changes());
    assert_eq!(failed.len(), 1, "{failed:?}");
    assert_eq!(failed[0].1, REASON_AMBIGUOUS_COPY);
    assert_eq!(deletes(&rig), 0, "nothing was deleted on a guess");
    // Both copies are still in Archive, with the server's values.
    assert_eq!(rig.backend.st().folders["Archive"].messages.len(), 2);
    assert!(rig.row("INBOX", 1).is_none(), "the old row is removed");
}

#[tokio::test]
async fn a_move_to_trash_after_a_move_is_given_up_when_the_destination_already_had_a_copy() {
    let mut rig = moved_next_to_a_copy(|rig, message| {
        rig.ui.move_to(rig.account, message, "Trash").unwrap();
    })
    .await;
    rig.drain_changes();
    rig.advance(5_000);
    rig.run().await;
    rig.advance(60_000);
    rig.sync().await;
    rig.advance(60_000);
    rig.run().await;

    let failed = failures(&rig.drain_changes());
    assert_eq!(failed.len(), 1, "{failed:?}");
    assert_eq!(failed[0].1, REASON_AMBIGUOUS_COPY);
    assert!(
        !rig.backend
            .calls()
            .iter()
            .any(|c| matches!(c, Call::Move(_, _, d) if d == "Trash")),
        "nothing was moved to Trash on a guess"
    );
    assert!(rig.backend.st().folders["Trash"].messages.is_empty());
}

#[tokio::test]
async fn a_star_after_a_move_still_follows_the_message_when_the_destination_already_had_a_copy() {
    // Only destructive changes give up: a star on the wrong copy is cheap and visible.
    let mut rig = moved_next_to_a_copy(|rig, message| {
        rig.ui.set_flagged(rig.account, message, true).unwrap();
    })
    .await;
    rig.drain_changes();
    rig.advance(5_000);
    rig.run().await;
    rig.advance(60_000);
    rig.run().await;
    assert!(failures(&rig.drain_changes()).is_empty());
    assert!(rig
        .backend
        .writes()
        .iter()
        .any(|c| matches!(c, Call::SetFlag(f, _, FlagChange::Flagged(true)) if f == "Archive")));
}

#[tokio::test]
async fn a_delete_after_a_move_without_a_new_uid_follows_the_moved_copy_when_it_is_the_only_one() {
    let rig = Rig::new();
    rig.backend.add("INBOX", "a@x", Flags::default());
    rig.backend.add("Archive", "z@x", Flags::default());
    rig.sync().await;
    rig.backend.clear_calls();
    rig.backend.st().uidplus = false;
    let message = row_id(&rig, "INBOX", 1);
    rig.ui.move_to(rig.account, message, "Archive").unwrap();
    rig.ui.delete_forever(rig.account, message).unwrap();
    rig.advance(5_000);
    rig.run().await;
    rig.advance(5_000);
    rig.run().await;
    assert_eq!(
        rig.backend
            .writes()
            .into_iter()
            .filter(|c| matches!(c, Call::Delete(..)))
            .collect::<Vec<_>>(),
        vec![Call::Delete("Archive".into(), vec![2])]
    );
    assert!(rig.backend.uid_of("Archive", "z@x").is_some());
    assert!(rig.backend.uid_of("Archive", "a@x").is_none());
}

#[tokio::test]
async fn a_delete_is_given_up_when_two_old_uids_would_map_to_one_new_uid_after_a_reset() {
    // INBOX holds the same message twice (UIDs 1 and 2). After the folder is renumbered the
    // server can name only one UID for that Message-ID, so a delete of "UID 1" cannot be told
    // from a delete of "UID 2".
    let mut rig = Rig::new();
    rig.backend.add("INBOX", "dup@x", Flags::default());
    rig.backend.add("INBOX", "dup@x", Flags::default());
    rig.backend.add("INBOX", "other@x", Flags::default());
    rig.sync().await;
    rig.backend.clear_calls();
    rig.drain_changes();
    let first = row_id(&rig, "INBOX", 1);
    let doomed = rig.ui.delete_forever(rig.account, first).unwrap();
    rig.backend.change_uid_validity("INBOX", 300);
    rig.advance(10_000);
    rig.sync().await;
    rig.advance(10_000);
    rig.run().await;

    let failed = failures(&rig.drain_changes());
    assert_eq!(failed, vec![(doomed, REASON_LOST_AFTER_RESET.to_owned())]);
    assert_eq!(deletes(&rig), 0, "nothing was deleted on a guess");
    assert_eq!(rig.backend.st().folders["INBOX"].messages.len(), 3);
    assert_eq!(row_count(&rig, "INBOX"), 3);
    assert_eq!(rig.listed("INBOX").len(), 3, "no row stays hidden");
    assert_eq!(rig.ui.operation(doomed).unwrap().state, OpState::Failed);
}
