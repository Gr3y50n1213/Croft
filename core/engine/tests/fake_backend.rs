// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Engine tests against the in-memory fake server, driven step by step with a manual clock
//! (`run_once`), so nothing here sleeps or depends on timing.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::time::Duration;

use common::{seen, Call, Method, Rig};
use croft_engine::{ActionError, Change, OpState, REASON_LOST_AFTER_MOVE, REASON_LOST_AFTER_RESET};
use croft_protocols::{BackendError, FlagChange, Flags, RemoteFolder, SpecialUse};
use croft_storage::folders;

/// A rig whose INBOX holds the given messages (unread, UIDs from 1) and that has synced once.
async fn synced(ids: &[&str]) -> Rig {
    let mut rig = Rig::new();
    for id in ids {
        rig.backend.add("INBOX", id, Flags::default());
    }
    rig.sync().await;
    rig.backend.clear_calls();
    rig.drain_changes();
    rig
}

fn row_id(rig: &Rig, folder: &str, uid: u32) -> i64 {
    rig.row(folder, uid).unwrap().id
}

fn flag(folder: &str, uid: u32, change: FlagChange) -> Call {
    Call::SetFlag(folder.into(), vec![uid], change)
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

/// Waits (briefly, in real time) until the fake server has received a call of `method`.
async fn wait_for_call(rig: &Rig, method: Method) {
    for _ in 0..500 {
        let seen = rig.backend.calls().iter().any(|c| {
            matches!(
                (c, method),
                (Call::Move(..), Method::Move) | (Call::SetFlag(..), Method::SetFlag)
            )
        });
        if seen {
            return;
        }
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
    panic!("the server never got a {method:?} call");
}

// ---- sync -------------------------------------------------------------------------------------

#[tokio::test]
async fn the_first_sync_brings_the_folders_and_messages_newest_first() {
    let mut rig = Rig::new();
    rig.backend.add("INBOX", "a@x", Flags::default());
    rig.backend.add("INBOX", "b@x", seen());
    rig.sync().await;
    let listed: Vec<_> = rig
        .listed("INBOX")
        .iter()
        .map(|m| (m.uid, m.is_seen))
        .collect();
    assert_eq!(listed, vec![(2, true), (1, false)]);
    let account = rig.account;
    let inbox = rig.ui.read(|db| {
        folders::find_by_path(db.connection(), account, "INBOX")
            .unwrap()
            .unwrap()
    });
    assert_eq!(
        (inbox.uid_validity, inbox.unread_count, inbox.total_count),
        (Some(100), 1, 2)
    );
    let changes = rig.drain_changes();
    assert!(changes.contains(&Change::Folders { account }));
    assert!(changes.contains(&Change::Messages {
        account,
        folders: vec![inbox.id]
    }));
    // Headers are fetched newest first.
    assert!(rig
        .backend
        .calls()
        .contains(&Call::Fetch("INBOX".into(), vec![2, 1])));
}

#[tokio::test]
async fn a_sync_removes_gone_messages_and_takes_server_flags() {
    let rig = synced(&["a@x", "b@x", "c@x"]).await;
    rig.backend
        .st()
        .folders
        .get_mut("INBOX")
        .unwrap()
        .messages
        .remove(&3);
    rig.backend
        .set_server_flags("INBOX", 1, |f| f.flagged = true);
    rig.sync().await;
    assert!(rig.row("INBOX", 3).is_none());
    assert!(rig.row("INBOX", 1).unwrap().is_flagged);
}

#[tokio::test]
async fn a_sync_clears_hidden_rows_and_pending_counts_that_no_operation_explains() {
    // Design note section 6: nothing else would ever clear these.
    let rig = synced(&["a@x", "b@x"]).await;
    let stuck = row_id(&rig, "INBOX", 1);
    rig.ui.read(|db| {
        db.connection()
            .execute(
                "UPDATE message SET local_hidden = 1, pending_ops = 3 WHERE id = ?1",
                [stuck],
            )
            .unwrap()
    });
    assert_eq!(rig.listed("INBOX").len(), 1);
    rig.sync().await;
    let row = rig.row("INBOX", 1).unwrap();
    assert_eq!((row.local_hidden, row.pending_ops), (false, 0));
    assert_eq!(rig.listed("INBOX").len(), 2);
}

// ---- Done when: mark read, kill, restart, exactly once ------------------------------------------

#[tokio::test]
async fn mark_read_then_kill_and_restart_reaches_the_server_exactly_once() {
    let mut rig = synced(&["a@x"]).await;
    let id = row_id(&rig, "INBOX", 1);
    rig.ui.mark_seen(rig.account, id, true).unwrap();
    assert!(rig.row("INBOX", 1).unwrap().is_seen, "local at once");

    // Killed before the 1.5 s delay ran out.
    rig.kill_and_restart();
    let row = rig.row("INBOX", 1).unwrap();
    assert!(row.is_seen && row.pending_ops == 1, "the change survived");
    rig.run().await;
    assert!(rig.backend.writes().is_empty(), "not due yet");

    rig.advance(1_500);
    rig.run().await;
    assert_eq!(
        rig.backend.writes(),
        vec![flag("INBOX", 1, FlagChange::Seen(true))]
    );
    assert!(rig.backend.flags("INBOX", 1).unwrap().seen);
    assert_eq!(rig.row("INBOX", 1).unwrap().pending_ops, 0);

    // Killed again after the confirmation: nothing is sent twice.
    rig.kill_and_restart();
    rig.advance(60_000);
    rig.sync().await;
    rig.run().await;
    assert_eq!(rig.backend.writes().len(), 1, "exactly once");
    assert!(rig.row("INBOX", 1).unwrap().is_seen);
}

#[tokio::test]
async fn a_kill_while_the_change_is_on_its_way_sends_it_again_which_is_safe() {
    // The server may or may not have applied a change that was in flight at the kill. It is sent
    // again on restart (every operation is idempotent): a flag set twice is harmless.
    let mut rig = synced(&["a@x"]).await;
    let id = row_id(&rig, "INBOX", 1);
    rig.ui.mark_seen(rig.account, id, true).unwrap();
    rig.advance(1_500);
    let gate = rig.backend.gate(Method::SetFlag);
    let ui = rig.ui.clone();
    let account = rig.account;
    let task = tokio::spawn(async move { ui.run_once(account, false).await });
    wait_for_call(&rig, Method::SetFlag).await;
    task.abort();
    let _ = task.await;
    drop(gate);
    rig.backend.ungate(Method::SetFlag);
    rig.kill_and_restart();
    rig.run().await;
    let writes = rig.backend.writes();
    assert_eq!(
        writes,
        vec![
            flag("INBOX", 1, FlagChange::Seen(true)),
            flag("INBOX", 1, FlagChange::Seen(true))
        ]
    );
    assert!(rig.backend.flags("INBOX", 1).unwrap().seen);
    assert_eq!(rig.row("INBOX", 1).unwrap().pending_ops, 0);
}

// ---- Done when: server flags never overwrite an in-flight change --------------------------------

#[tokio::test]
async fn server_flags_never_overwrite_a_local_change_on_its_way() {
    let rig = synced(&["a@x", "b@x"]).await;
    let a = row_id(&rig, "INBOX", 1);
    rig.ui.mark_seen(rig.account, a, true).unwrap();
    // Another client changes both messages; the server still says "unread" for a.
    rig.backend
        .set_server_flags("INBOX", 1, |f| f.flagged = true);
    rig.backend.set_server_flags("INBOX", 2, |f| f.seen = true);

    rig.sync().await; // before the change is due
    let row = rig.row("INBOX", 1).unwrap();
    assert!(row.is_seen, "the local change stays");
    assert!(rig.row("INBOX", 2).unwrap().is_seen, "other rows update");

    // Still in flight after a failed attempt (backing off): still not overwritten.
    rig.backend
        .st()
        .fail_next
        .push_back((Method::SetFlag, BackendError::Transient("busy".into())));
    rig.advance(1_500);
    rig.run().await;
    rig.sync().await;
    assert!(rig.row("INBOX", 1).unwrap().is_seen);

    // Once confirmed, the server's values come in again (including the other client's star).
    rig.advance(5_000);
    rig.run().await;
    rig.sync().await;
    let row = rig.row("INBOX", 1).unwrap();
    assert!(row.is_seen && row.is_flagged && row.pending_ops == 0);
}

// ---- Done when: airplane mode (Transient) -------------------------------------------------------

#[tokio::test]
async fn with_the_server_unreachable_every_action_works_locally_and_retries_back_off() {
    let mut rig = synced(&["a@x", "b@x", "c@x", "d@x"]).await;
    let (a, b, c, d) = (
        row_id(&rig, "INBOX", 1),
        row_id(&rig, "INBOX", 2),
        row_id(&rig, "INBOX", 3),
        row_id(&rig, "INBOX", 4),
    );
    rig.backend.st().fail_all = Some(BackendError::Transient("no route to host".into()));
    let t0 = rig.clock_now();

    rig.ui.mark_seen(rig.account, a, true).unwrap();
    rig.ui.set_flagged(rig.account, b, true).unwrap();
    let moved = rig.ui.move_to(rig.account, c, "Archive").unwrap();
    let undone = rig.ui.move_to(rig.account, d, "Trash").unwrap();
    rig.ui.undo(undone).unwrap();
    assert!(rig.row("INBOX", 1).unwrap().is_seen);
    assert!(rig.row("INBOX", 2).unwrap().is_flagged);
    assert!(rig.row("INBOX", 3).unwrap().local_hidden);
    assert!(!rig.row("INBOX", 4).unwrap().local_hidden);

    // The first change is tried at 1.5 s and then backs the whole account off: 5 s, 30 s,
    // 2 min, 10 min, 1 h, then hourly, and gives up after 8 attempts.
    rig.advance(1_500);
    rig.run().await;
    let mut expected = vec![t0 + 1_500];
    for wait in [
        5_000, 30_000, 120_000, 600_000, 3_600_000, 3_600_000, 3_600_000,
    ] {
        rig.advance(wait - 1);
        rig.run().await;
        rig.advance(1);
        rig.run().await;
        expected.push(rig.clock_now());
    }
    // Each attempt is one call (the folder check before sending fails first). After the eighth,
    // the next change in line makes its first attempt at the same moment.
    let times: Vec<i64> = rig
        .backend
        .timed_calls()
        .into_iter()
        .map(|(t, _)| t)
        .collect();
    let mut distinct = times.clone();
    distinct.dedup();
    assert_eq!(distinct, expected);
    assert_eq!(times.len(), 9);
    let changes = rig.drain_changes();
    let failed = failures(&changes);
    assert_eq!(failed.len(), 1, "{failed:?}");
    assert!(failed[0].1.contains("temporary problem"));
    assert!(
        !rig.row("INBOX", 1).unwrap().is_seen,
        "the row is put back after the last attempt"
    );
    // Nothing else was attempted while the account was backing off, except the next change in
    // line, which started its own attempts once the first one gave up.
    assert_eq!(rig.ui.operation(moved).unwrap().attempts, 0);

    // The server comes back: what is left goes out.
    rig.backend.st().fail_all = None;
    rig.advance(5_000);
    rig.run().await;
    assert!(rig.backend.flags("INBOX", 2).unwrap().flagged);
    assert!(rig.backend.uid_of("Archive", "c@x").is_some());
    assert!(rig.backend.uid_of("INBOX", "d@x").is_some(), "undone");
    assert_eq!(rig.ui.operation(moved).unwrap().state, OpState::Confirmed);
}

// ---- Done when: offline ------------------------------------------------------------------------

#[tokio::test]
async fn offline_actions_work_locally_use_no_attempts_and_go_out_in_order_on_reconnect() {
    let rig = synced(&["a@x", "b@x", "c@x"]).await;
    let (a, b, c) = (
        row_id(&rig, "INBOX", 1),
        row_id(&rig, "INBOX", 2),
        row_id(&rig, "INBOX", 3),
    );
    rig.ui.set_online(rig.account, false);
    let moved = rig.ui.move_to(rig.account, c, "Archive").unwrap();
    rig.ui.mark_seen(rig.account, a, true).unwrap();
    rig.ui.set_flagged(rig.account, b, true).unwrap();
    assert!(rig.row("INBOX", 3).unwrap().local_hidden);
    assert!(rig.row("INBOX", 1).unwrap().is_seen);

    rig.advance(3_600_000);
    rig.run().await;
    rig.sync().await;
    assert!(rig.backend.calls().is_empty(), "nothing is tried offline");
    assert_eq!(rig.ui.operation(moved).unwrap().attempts, 0);

    rig.ui.set_online(rig.account, true);
    rig.sync().await;
    assert_eq!(
        rig.backend.writes(),
        vec![
            Call::Move("INBOX".into(), vec![3], "Archive".into()),
            flag("INBOX", 1, FlagChange::Seen(true)),
            flag("INBOX", 2, FlagChange::Flagged(true)),
        ],
        "in the order they were made"
    );
    assert_eq!(rig.ui.operation(moved).unwrap().attempts, 1);
}

// ---- Done when: move tests ---------------------------------------------------------------------

/// Read, archive and star `a@x` (INBOX 1) at the same moment. Archive already holds `z@x` (UID 1),
/// so the moved copy gets UID 2 there.
async fn read_archive_star_setup() -> (Rig, i64) {
    let rig = Rig::new();
    rig.backend.add("INBOX", "a@x", Flags::default());
    rig.backend.add("Archive", "z@x", Flags::default());
    rig.sync().await;
    rig.backend.clear_calls();
    let id = row_id(&rig, "INBOX", 1);
    (rig, id)
}

fn assert_star_landed_on_the_moved_copy(rig: &Rig) {
    let uid = rig.backend.uid_of("Archive", "a@x").unwrap();
    let server = rig.backend.flags("Archive", uid).unwrap();
    assert!(server.seen && server.flagged, "{server:?}");
    let z = rig.backend.uid_of("Archive", "z@x").unwrap();
    assert!(
        !rig.backend.flags("Archive", z).unwrap().flagged,
        "not on z"
    );
    assert!(rig.backend.uid_of("INBOX", "a@x").is_none());
    let row = rig.row_by_id("Archive", "a@x").unwrap();
    assert_eq!(row.uid, i64::from(uid));
    assert!(row.is_seen && row.is_flagged && !row.local_hidden);
    assert_eq!(row.pending_ops, 0);
    assert!(rig.row_by_id("INBOX", "a@x").is_none());
    assert!(rig
        .backend
        .writes()
        .ends_with(&[flag("Archive", uid, FlagChange::Flagged(true))]));
}

#[tokio::test]
async fn read_archive_star_with_the_new_uid_arriving_late_lands_on_the_moved_copy() {
    let (rig, id) = read_archive_star_setup().await;
    rig.ui.mark_seen(rig.account, id, true).unwrap();
    let moved = rig.ui.move_to(rig.account, id, "Archive").unwrap();
    rig.advance(1_500);
    rig.run().await;
    assert_eq!(
        rig.backend.writes(),
        vec![flag("INBOX", 1, FlagChange::Seen(true))]
    );

    // The move goes out and its answer is slow; the star is made while it is on its way.
    rig.advance(3_500);
    let gate = rig.backend.gate(Method::Move);
    let ui = rig.ui.clone();
    let account = rig.account;
    let task = tokio::spawn(async move { ui.run_once(account, false).await });
    wait_for_call(&rig, Method::Move).await;
    rig.ui.set_flagged(rig.account, id, true).unwrap();
    gate.open.notify_one();
    task.await.unwrap().unwrap();
    assert_eq!(rig.ui.operation(moved).unwrap().state, OpState::Confirmed);
    // The copy is in Archive at once, with the local star, and the old row is gone.
    let copy = rig.row("Archive", 2).unwrap();
    assert!(copy.is_flagged && copy.pending_ops == 1 && !copy.local_hidden);
    assert!(rig.row("INBOX", 1).is_none());

    rig.advance(1_500);
    rig.run().await;
    assert_star_landed_on_the_moved_copy(&rig);
}

#[tokio::test]
async fn read_archive_star_without_uidplus_finds_the_copy_by_message_id() {
    let (rig, id) = read_archive_star_setup().await;
    rig.backend.st().uidplus = false;
    // The server's search does not see the copy straight after the move.
    rig.backend.st().search_misses = 1;
    rig.ui.mark_seen(rig.account, id, true).unwrap();
    rig.ui.move_to(rig.account, id, "Archive").unwrap();
    rig.ui.set_flagged(rig.account, id, true).unwrap();
    rig.advance(5_000);
    rig.run().await;
    // The star waits: the old row stays (hidden) until the message is found.
    let old = rig.row("INBOX", 1).unwrap();
    assert!(old.local_hidden && old.is_flagged);
    rig.advance(60_000);
    rig.run().await;
    assert!(
        !rig.backend
            .writes()
            .iter()
            .any(|c| matches!(c, Call::SetFlag(_, _, FlagChange::Flagged(_)))),
        "held"
    );

    // The next sync of Archive looks again, finds it, and the star follows.
    rig.sync().await;
    assert_star_landed_on_the_moved_copy(&rig);
    assert_eq!(
        rig.backend
            .calls()
            .iter()
            .filter(|c| matches!(c, Call::Find(..)))
            .count(),
        2
    );
}

#[tokio::test]
async fn read_archive_star_without_uidplus_survives_a_uidvalidity_change_in_between() {
    let (rig, id) = read_archive_star_setup().await;
    rig.backend.st().uidplus = false;
    rig.backend.st().search_misses = 1;
    rig.ui.mark_seen(rig.account, id, true).unwrap();
    rig.ui.move_to(rig.account, id, "Archive").unwrap();
    rig.ui.set_flagged(rig.account, id, true).unwrap();
    rig.advance(5_000);
    rig.run().await;
    // Archive is renumbered on the server: a@x 2 -> 1, z@x 1 -> 2.
    rig.backend.change_uid_validity("Archive", 200);
    assert_eq!(rig.backend.uid_of("Archive", "a@x"), Some(1));
    rig.sync().await;
    assert_star_landed_on_the_moved_copy(&rig);
    let z = rig.row_by_id("Archive", "z@x").unwrap();
    assert!(!z.is_flagged && z.pending_ops == 0 && z.uid == 2);
}

#[tokio::test]
async fn read_archive_star_with_copyuid_survives_a_uidvalidity_change_before_the_star_is_sent() {
    let (rig, id) = read_archive_star_setup().await;
    rig.ui.mark_seen(rig.account, id, true).unwrap();
    rig.ui.move_to(rig.account, id, "Archive").unwrap();
    rig.ui.set_flagged(rig.account, id, true).unwrap();
    rig.advance(5_000);
    // The connection drops right after the move's answer: the star (now pointing at Archive 2)
    // waits.
    let gate = rig.backend.gate(Method::Move);
    let ui = rig.ui.clone();
    let account = rig.account;
    let task = tokio::spawn(async move { ui.run_once(account, false).await });
    wait_for_call(&rig, Method::Move).await;
    rig.ui.set_online(rig.account, false);
    gate.open.notify_one();
    task.await.unwrap().unwrap();
    rig.backend.ungate(Method::Move);
    assert!(rig.row("Archive", 2).unwrap().is_flagged);

    // Meanwhile Archive is renumbered: UID 2 is now z@x. Sending the star to Archive 2 would star
    // the wrong message; the folder check before sending catches it.
    rig.backend.change_uid_validity("Archive", 200);
    rig.ui.set_online(rig.account, true);
    rig.run().await;
    assert_star_landed_on_the_moved_copy(&rig);
}

#[tokio::test]
async fn archive_and_star_after_the_inbox_was_renumbered_act_on_the_right_message() {
    let rig = synced(&["a@x", "b@x"]).await;
    let a = row_id(&rig, "INBOX", 1);
    rig.ui.move_to(rig.account, a, "Archive").unwrap();
    rig.ui.set_flagged(rig.account, a, true).unwrap();
    // While the Undo window runs, the server renumbers INBOX: b@x is UID 1 now, a@x is 2.
    rig.backend.change_uid_validity("INBOX", 300);
    rig.advance(5_000);
    rig.run().await;
    assert!(rig.backend.uid_of("INBOX", "b@x").is_some(), "b stays");
    let uid = rig.backend.uid_of("Archive", "a@x").unwrap();
    assert!(rig.backend.flags("Archive", uid).unwrap().flagged);
    assert!(rig
        .backend
        .writes()
        .contains(&Call::Move("INBOX".into(), vec![2], "Archive".into())));
    let b = rig.row_by_id("INBOX", "b@x").unwrap();
    assert!(!b.local_hidden && !b.is_flagged && b.pending_ops == 0 && b.uid == 1);
}

#[tokio::test]
async fn a_message_moved_out_of_a_folder_that_is_then_renumbered_is_still_followed() {
    // An awaiting entry whose source folder is reset: the old address now belongs to b@x, which
    // must not get the star.
    let rig = synced(&["a@x", "b@x"]).await;
    rig.backend.st().uidplus = false;
    rig.backend.st().search_misses = 1;
    let a = row_id(&rig, "INBOX", 1);
    rig.ui.move_to(rig.account, a, "Archive").unwrap();
    rig.ui.set_flagged(rig.account, a, true).unwrap();
    rig.advance(5_000);
    rig.run().await;
    // INBOX is renumbered: b@x takes UID 1, the old address of a@x.
    rig.backend.change_uid_validity("INBOX", 300);
    assert_eq!(rig.backend.uid_of("INBOX", "b@x"), Some(1));
    rig.sync().await;
    let uid = rig.backend.uid_of("Archive", "a@x").unwrap();
    assert!(rig.backend.flags("Archive", uid).unwrap().flagged);
    assert!(!rig.backend.flags("INBOX", 1).unwrap().flagged);
    let b = rig.row("INBOX", 1).unwrap();
    assert_eq!(b.message_id_hdr.as_deref(), Some("<b@x>"));
    assert!(!b.is_flagged && !b.local_hidden && b.pending_ops == 0);
    assert!(rig.row_by_id("Archive", "a@x").unwrap().is_flagged);
}

#[tokio::test]
async fn a_moved_message_that_cannot_be_found_fails_its_later_changes_after_three_lookups() {
    let mut rig = synced(&["a@x"]).await;
    rig.backend.st().uidplus = false;
    rig.backend.st().search_misses = 100;
    let a = row_id(&rig, "INBOX", 1);
    rig.ui.move_to(rig.account, a, "Archive").unwrap();
    rig.ui.set_flagged(rig.account, a, true).unwrap();
    rig.advance(5_000);
    rig.run().await; // lookup 1
    rig.sync().await; // lookup 2
    assert!(failures(&rig.drain_changes()).is_empty());
    rig.sync().await; // lookup 3: give up
    let failed = failures(&rig.drain_changes());
    assert_eq!(failed.len(), 1);
    assert_eq!(failed[0].1, REASON_LOST_AFTER_MOVE);
    assert!(rig.row("INBOX", 1).is_none(), "the old row is removed");
    let copy = rig.row_by_id("Archive", "a@x").unwrap();
    assert!(!copy.is_flagged && copy.pending_ops == 0, "server values");
    assert!(!rig
        .backend
        .writes()
        .iter()
        .any(|c| matches!(c, Call::SetFlag(_, _, FlagChange::Flagged(_)))));
}

#[tokio::test]
async fn changes_to_a_message_gone_after_a_uidvalidity_change_fail_with_a_reason() {
    let mut rig = synced(&["a@x", "b@x"]).await;
    let a = row_id(&rig, "INBOX", 1);
    rig.ui.set_flagged(rig.account, a, true).unwrap();
    // INBOX is recreated without a@x.
    rig.backend
        .st()
        .folders
        .get_mut("INBOX")
        .unwrap()
        .messages
        .remove(&1);
    rig.backend.change_uid_validity("INBOX", 300);
    rig.sync().await;
    let failed = failures(&rig.drain_changes());
    assert_eq!(failed.len(), 1);
    assert_eq!(failed[0].1, REASON_LOST_AFTER_RESET);
    let b = rig.row("INBOX", 1).unwrap();
    assert_eq!(b.message_id_hdr.as_deref(), Some("<b@x>"));
    assert!(!b.is_flagged, "the star did not move to another message");
    rig.advance(10_000);
    rig.run().await;
    assert!(rig.backend.writes().is_empty());
}

// ---- other rules -------------------------------------------------------------------------------

#[tokio::test]
async fn undo_inside_the_window_sends_nothing_and_shows_the_row_again() {
    let rig = synced(&["a@x"]).await;
    let a = row_id(&rig, "INBOX", 1);
    let op = rig.ui.move_to(rig.account, a, "Trash").unwrap();
    assert!(rig.listed("INBOX").is_empty());
    rig.advance(4_999);
    rig.ui.undo(op).unwrap();
    assert_eq!(rig.listed("INBOX").len(), 1);
    rig.advance(10_000);
    rig.run().await;
    assert!(rig.backend.writes().is_empty());
    assert!(matches!(rig.ui.undo(op), Err(ActionError::TooLate)));

    let op = rig.ui.move_to(rig.account, a, "Trash").unwrap();
    rig.advance(5_000);
    assert!(matches!(rig.ui.undo(op), Err(ActionError::TooLate)));
    assert!(matches!(
        rig.ui.undo(9_999),
        Err(ActionError::NoSuchOperation)
    ));
}

#[tokio::test]
async fn read_then_unread_inside_the_delay_sends_nothing() {
    let rig = synced(&["a@x"]).await;
    let a = row_id(&rig, "INBOX", 1);
    rig.ui.mark_seen(rig.account, a, true).unwrap();
    rig.ui.mark_seen(rig.account, a, false).unwrap();
    // Marking unread something already unread is not an error and queues nothing.
    rig.ui.mark_seen(rig.account, a, false).unwrap();
    let row = rig.row("INBOX", 1).unwrap();
    assert!(!row.is_seen && row.pending_ops == 0);
    rig.advance(5_000);
    rig.run().await;
    assert!(rig.backend.writes().is_empty());
}

#[tokio::test]
async fn a_permanent_refusal_fails_the_move_and_puts_the_row_back() {
    let mut rig = synced(&["a@x"]).await;
    let account = rig.account;
    // A folder known locally but gone from the server.
    rig.ui.read(|db| {
        folders::upsert(
            db.connection(),
            account,
            &RemoteFolder {
                path: "Old".into(),
                delimiter: Some('/'),
                special_use: SpecialUse::Other,
                selectable: true,
            },
        )
        .unwrap()
    });
    let a = row_id(&rig, "INBOX", 1);
    let op = rig.ui.move_to(account, a, "Old").unwrap();
    rig.advance(5_000);
    rig.run().await;
    let failed = failures(&rig.drain_changes());
    assert_eq!(failed.len(), 1);
    assert_eq!(failed[0].0, op);
    assert!(failed[0].1.starts_with("server refused"));
    assert_eq!(rig.listed("INBOX").len(), 1, "row restored");
    assert_eq!(rig.ui.operation(op).unwrap().attempts, 1);
}

#[tokio::test]
async fn a_sign_in_failure_pauses_the_account_until_signed_in_without_using_an_attempt() {
    let mut rig = synced(&["a@x"]).await;
    let a = row_id(&rig, "INBOX", 1);
    rig.backend
        .st()
        .fail_next
        .push_back((Method::SetFlag, BackendError::Auth("expired".into())));
    rig.ui.mark_seen(rig.account, a, true).unwrap();
    rig.advance(1_500);
    rig.run().await;
    assert!(rig.drain_changes().contains(&Change::SignInNeeded {
        account: rig.account
    }));
    rig.backend.clear_calls();
    rig.advance(3_600_000);
    rig.sync().await;
    assert!(rig.backend.calls().is_empty(), "paused: nothing at all");
    assert!(rig.row("INBOX", 1).unwrap().is_seen, "still local");

    rig.ui.signed_in(rig.account);
    rig.run().await;
    assert_eq!(
        rig.backend.writes(),
        vec![flag("INBOX", 1, FlagChange::Seen(true))]
    );
}

#[tokio::test]
async fn delete_forever_hides_the_row_and_removes_it_when_the_server_confirms() {
    let rig = synced(&["a@x", "b@x"]).await;
    let a = row_id(&rig, "INBOX", 1);
    rig.ui.delete_forever(rig.account, a).unwrap();
    assert!(rig.row("INBOX", 1).unwrap().local_hidden);
    rig.advance(5_000);
    rig.run().await;
    assert_eq!(
        rig.backend.writes(),
        vec![Call::Delete("INBOX".into(), vec![1])]
    );
    assert!(rig.row("INBOX", 1).is_none());
    assert!(rig.backend.uid_of("INBOX", "a@x").is_none());
}

#[tokio::test]
async fn actions_refuse_unknown_messages_other_accounts_and_bad_folders() {
    let rig = synced(&["a@x"]).await;
    let a = row_id(&rig, "INBOX", 1);
    assert!(matches!(
        rig.ui.mark_seen(rig.account, 9_999, true),
        Err(ActionError::NoSuchMessage)
    ));
    let other = rig
        .ui
        .read(|db| common::create_account(db, "two@example.org"));
    assert!(matches!(
        rig.ui.mark_seen(other, a, true),
        Err(ActionError::NoSuchMessage)
    ));
    assert!(matches!(
        rig.ui.move_to(rig.account, a, "Nowhere"),
        Err(ActionError::NoSuchFolder)
    ));
    assert!(matches!(
        rig.ui.move_to(rig.account, a, "INBOX"),
        Err(ActionError::SameFolder)
    ));
    assert!(!rig.row("INBOX", 1).unwrap().is_seen, "nothing changed");
}

#[tokio::test]
async fn every_action_announces_the_changed_folder() {
    let mut rig = synced(&["a@x"]).await;
    let a = row_id(&rig, "INBOX", 1);
    let inbox = rig.row("INBOX", 1).unwrap().folder_id;
    rig.ui.set_flagged(rig.account, a, true).unwrap();
    assert_eq!(
        rig.drain_changes(),
        vec![Change::Messages {
            account: rig.account,
            folders: vec![inbox]
        }]
    );
}

#[tokio::test]
async fn a_longer_undo_window_delays_moves_but_not_flags() {
    let rig = synced(&["a@x", "b@x"]).await;
    rig.ui.set_undo_window_ms(10_000);
    let a = row_id(&rig, "INBOX", 1);
    let b = row_id(&rig, "INBOX", 2);
    rig.ui.move_to(rig.account, a, "Archive").unwrap();
    rig.ui.mark_seen(rig.account, b, true).unwrap();
    rig.advance(9_999);
    rig.run().await;
    assert_eq!(
        rig.backend.writes(),
        vec![flag("INBOX", 2, FlagChange::Seen(true))]
    );
    rig.advance(1);
    rig.run().await;
    assert_eq!(rig.backend.writes().len(), 2);
}
