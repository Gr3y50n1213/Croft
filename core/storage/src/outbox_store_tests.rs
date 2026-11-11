// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Tests for [`SqliteOutboxStore`] and the Outbox on top of it. "Restart" means closing the
//! database file and opening it again.

// Tests may unwrap: a panic is the failure report.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};

use croft_mail_core::outbox::{REASON_MAY_NOT_HAVE_BEEN_SENT, REASON_NOT_FILED};
use croft_mail_core::{
    FilingRejection, NewMessage, OutboxBatch, OutboxEntry, OutboxError, OutboxState, OutboxStore,
    PersistentOutbox, SendRejection, UndoError,
};
use croft_protocols::BackendError;
use rusqlite::{params, Connection};
use serde_json::json;

use crate::accounts::{self, NewAccount, Protocol};
use crate::{Db, SqliteOutboxStore, StorageError, MIGRATIONS};

const WINDOW: i64 = 5_000;

type Outbox = PersistentOutbox<SqliteOutboxStore>;

struct Fixture {
    _dir: tempfile::TempDir,
    path: PathBuf,
    a1: i64,
    a2: i64,
}

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
        c.execute(
            "INSERT INTO identity (id, account_id, display_name, address, is_default) VALUES (?1, ?2, 'Me', ?3, 1)",
            params![n * 10, a, email],
        )
        .unwrap();
        ids.push(a);
    }
    Fixture {
        _dir: dir,
        path,
        a1: ids[0],
        a2: ids[1],
    }
}

fn open(path: &Path) -> Outbox {
    PersistentOutbox::open(SqliteOutboxStore::new(Db::open(path, None).unwrap())).unwrap()
}

fn msg(account: i64) -> NewMessage {
    NewMessage {
        account,
        identity: Some(account * 10),
        envelope_from: "one@example.org".into(),
        recipients: vec!["you@example.org".into(), "bcc@example.org".into()],
    }
}

fn raw(path: &Path) -> Connection {
    Connection::open(path).unwrap()
}

fn db_state(path: &Path, id: u64) -> Option<String> {
    raw(path)
        .query_row("SELECT state FROM outbox WHERE id = ?1", [id as i64], |r| {
            r.get(0)
        })
        .ok()
}

fn row_count(path: &Path) -> i64 {
    raw(path)
        .query_row("SELECT count(*) FROM outbox", [], |r| r.get(0))
        .unwrap()
}

#[test]
fn queue_stores_the_message_as_given_with_its_envelope() {
    let f = fixture();
    let mut o = open(&f.path);
    // Binary on purpose: NUL, 8-bit bytes, bare line feeds, and a megabyte of it.
    let mut mime: Vec<u8> = b"Subject: x\n\n\x00\xff\xfe\r\n.\r\n".to_vec();
    mime.extend(std::iter::repeat_n(0xA5u8, 1_000_000));
    let id = o.queue(msg(f.a1), mime.clone(), 1_000, WINDOW).unwrap();
    let c = raw(&f.path);
    let (blob, state, send_at, from, rcpts): (Vec<u8>, String, i64, String, String) = c
        .query_row(
            "SELECT mime_blob, state, send_at, envelope_from, recipients FROM outbox WHERE id = ?1",
            [id as i64],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        )
        .unwrap();
    assert!(blob == mime, "the bytes are stored exactly as given");
    assert_eq!((state.as_str(), send_at), ("queued", 1_000 + WINDOW));
    assert_eq!(from, "one@example.org");
    assert_eq!(rcpts, r#"["you@example.org","bcc@example.org"]"#);
    // Reading it back gives the same bytes.
    let due = o.take_due(1_000 + WINDOW, 10).unwrap();
    assert!(due[0].mime == mime);
    assert_eq!(
        due[0].entry.recipients,
        ["you@example.org", "bcc@example.org"]
    );
}

#[test]
fn undo_inside_the_window_removes_the_row_and_take_due_never_returns_it() {
    let f = fixture();
    let mut o = open(&f.path);
    let id = o.queue(msg(f.a1), b"MIME".to_vec(), 0, WINDOW).unwrap();
    assert_eq!(row_count(&f.path), 1);
    let back = o.undo(id, WINDOW - 1).unwrap();
    assert_eq!(back.mime, b"MIME");
    assert_eq!(row_count(&f.path), 0, "nothing is left to send");
    for t in [WINDOW, WINDOW * 100, i64::MAX / 2] {
        assert!(o.take_due(t, 10).unwrap().is_empty());
    }
    // Also after a restart.
    drop(o);
    let mut o = open(&f.path);
    assert!(o.take_due(i64::MAX / 2, 10).unwrap().is_empty());
}

#[test]
fn undo_at_or_after_the_end_of_the_window_is_too_late() {
    let f = fixture();
    let mut o = open(&f.path);
    let id = o.queue(msg(f.a1), b"MIME".to_vec(), 0, WINDOW).unwrap();
    assert!(matches!(
        o.undo(id, WINDOW),
        Err(OutboxError::Undo(UndoError::TooLate))
    ));
    assert_eq!(db_state(&f.path, id).as_deref(), Some("queued"));
}

#[test]
fn offline_hands_out_nothing_and_spends_no_attempt_then_online_hands_it_out_once() {
    let f = fixture();
    let mut o = open(&f.path);
    let id = o.queue(msg(f.a1), b"MIME".to_vec(), 0, WINDOW).unwrap();
    o.set_offline(f.a1, true);
    for t in [WINDOW, 10 * WINDOW, 1_000_000] {
        assert!(o.take_due(t, 10).unwrap().is_empty());
    }
    let attempts: i64 = raw(&f.path)
        .query_row(
            "SELECT attempts FROM outbox WHERE id = ?1",
            [id as i64],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(attempts, 0);
    assert_eq!(db_state(&f.path, id).as_deref(), Some("queued"));
    o.set_offline(f.a1, false);
    assert_eq!(o.take_due(1_000_000, 10).unwrap().len(), 1);
    assert!(o.take_due(1_000_000, 10).unwrap().is_empty());
    assert_eq!(db_state(&f.path, id).as_deref(), Some("sending"));
}

#[test]
fn sending_is_on_disk_before_take_due_returns() {
    let f = fixture();
    let mut o = open(&f.path);
    let id = o.queue(msg(f.a1), b"MIME".to_vec(), 0, 0).unwrap();
    let due = o.take_due(0, 10).unwrap();
    assert_eq!(due.len(), 1);
    // Another connection to the file already sees `sending`.
    assert_eq!(db_state(&f.path, id).as_deref(), Some("sending"));
}

#[test]
fn a_restart_with_a_row_in_sending_fails_it_and_never_resends_it() {
    let f = fixture();
    let mut o = open(&f.path);
    let id = o.queue(msg(f.a1), b"MIME".to_vec(), 0, 0).unwrap();
    let other = o.queue(msg(f.a2), b"OTHER".to_vec(), 0, 0).unwrap();
    // Only the first is handed out (the other account is offline); then the app is killed.
    o.set_offline(f.a2, true);
    assert_eq!(o.take_due(0, 10).unwrap().len(), 1);
    drop(o);

    let mut o = open(&f.path);
    assert_eq!(o.recovered_after_restart(), &[id]);
    let e = o.outbox().get(id).unwrap();
    assert_eq!(e.state, OutboxState::Failed);
    assert_eq!(e.last_error.as_deref(), Some(REASON_MAY_NOT_HAVE_BEEN_SENT));
    assert_eq!(db_state(&f.path, id).as_deref(), Some("failed"));
    // Not handed out again, whatever the time. The untouched message of the other account still is.
    let due: Vec<_> = o
        .take_due(i64::MAX / 2, 10)
        .unwrap()
        .iter()
        .map(|d| d.entry.id)
        .collect();
    assert_eq!(due, vec![other]);
    // The second message was handed out above and the app is killed again: it is treated the same
    // way, and the first one, already failed, is not "recovered" a second time.
    drop(o);
    let mut o = open(&f.path);
    assert_eq!(o.recovered_after_restart(), &[other]);
    assert!(o.take_due(i64::MAX / 2, 10).unwrap().is_empty());
    // The person checked Sent and chose Retry.
    o.retry(id, 7).unwrap();
    assert_eq!(db_state(&f.path, id).as_deref(), Some("queued"));
    assert_eq!(o.take_due(7, 10).unwrap().len(), 1);
}

#[test]
fn submitted_and_filing_survive_a_restart_and_finish_by_deleting_the_row() {
    let f = fixture();
    let mut o = open(&f.path);
    let id = o.queue(msg(f.a1), b"MIME".to_vec(), 0, 0).unwrap();
    o.take_due(0, 1).unwrap();
    assert!(o.submitted(id, 10).unwrap());
    assert_eq!(db_state(&f.path, id).as_deref(), Some("submitted"));
    drop(o);

    // Crash between submit and append: not resent, copy not forgotten.
    let mut o = open(&f.path);
    assert!(o.take_due(i64::MAX / 2, 10).unwrap().is_empty());
    let filing = o.take_filing(10, 10).unwrap();
    assert_eq!(filing[0].mime, b"MIME");
    assert_eq!(db_state(&f.path, id).as_deref(), Some("filing"));
    drop(o);

    // Crash in the middle of the append: back to submitted.
    let mut o = open(&f.path);
    assert_eq!(db_state(&f.path, id).as_deref(), Some("submitted"));
    assert_eq!(o.take_filing(10, 10).unwrap().len(), 1);
    assert!(o.filed(id).unwrap());
    assert_eq!(row_count(&f.path), 0, "the message and its bytes are gone");
}

#[test]
fn giving_up_on_the_copy_deletes_the_row() {
    let f = fixture();
    let mut o = open(&f.path);
    let id = o.queue(msg(f.a1), b"MIME".to_vec(), 0, 0).unwrap();
    o.take_due(0, 1).unwrap();
    o.submitted(id, 0).unwrap();
    o.take_filing(0, 1).unwrap();
    let r = o
        .filing_failed(id, &BackendError::Permanent("no folder".into()), 1)
        .unwrap();
    assert_eq!(r, Some(FilingRejection::GaveUp));
    assert_eq!(row_count(&f.path), 0);
    let _ = REASON_NOT_FILED;
}

#[test]
fn transient_failures_are_stored_with_their_retry_time_and_reason() {
    let f = fixture();
    let mut o = open(&f.path);
    let id = o.queue(msg(f.a1), b"MIME".to_vec(), 0, 0).unwrap();
    o.take_due(0, 1).unwrap();
    let r = o
        .failed(
            id,
            &BackendError::Transient("The server asked to try again later".into()),
            100,
        )
        .unwrap();
    assert_eq!(
        r,
        Some(SendRejection::RetryAt(
            100 + croft_mail_core::ops::backoff_ms(1)
        ))
    );
    let (state, send_at, attempts, err): (String, i64, i64, Option<String>) = raw(&f.path)
        .query_row(
            "SELECT state, send_at, attempts, last_error FROM outbox WHERE id = ?1",
            [id as i64],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .unwrap();
    assert_eq!(state, "queued");
    assert_eq!(send_at, 100 + croft_mail_core::ops::backoff_ms(1));
    assert_eq!(attempts, 1);
    assert_eq!(err.as_deref(), Some("The server asked to try again later"));
}

#[test]
fn edit_and_discard_return_or_drop_the_bytes() {
    let f = fixture();
    let mut o = open(&f.path);
    let id = o.queue(msg(f.a1), b"MIME".to_vec(), 0, 0).unwrap();
    o.take_due(0, 1).unwrap();
    o.failed(
        id,
        &BackendError::Permanent("The server refused the message".into()),
        1,
    )
    .unwrap();
    let back = o.edit_back_to_draft(id).unwrap();
    assert_eq!(back.mime, b"MIME");
    assert_eq!(row_count(&f.path), 0);
    let id2 = o.queue(msg(f.a1), b"TWO".to_vec(), 0, 0).unwrap();
    o.discard(id2).unwrap();
    assert_eq!(row_count(&f.path), 0);
}

#[test]
fn every_statement_is_scoped_by_account() {
    let f = fixture();
    let mut store = SqliteOutboxStore::new(Db::open(&f.path, None).unwrap());
    let mut o = open(&f.path);
    let id = o.queue(msg(f.a1), b"MIME-ONE".to_vec(), 0, 0).unwrap();
    drop(o);

    // Reading with the wrong account finds nothing.
    assert_eq!(store.mime(f.a2, id).unwrap(), None);
    assert_eq!(
        store.mime(f.a1, id).unwrap().as_deref(),
        Some(&b"MIME-ONE"[..])
    );

    // Removing or updating with the wrong account touches nothing (and the update says so).
    store
        .commit(&OutboxBatch {
            removed: vec![(f.a2, id)],
            ..OutboxBatch::default()
        })
        .unwrap();
    assert_eq!(row_count(&f.path), 1);
    let mut entry = store.load().unwrap().entries.remove(0);
    entry.account = f.a2;
    entry.state = OutboxState::Failed;
    let err = store
        .commit(&OutboxBatch {
            updated: vec![entry],
            ..OutboxBatch::default()
        })
        .unwrap_err();
    assert!(matches!(err, StorageError::NotFound(_)));
    assert_eq!(db_state(&f.path, id).as_deref(), Some("queued"));

    // Each account's load returns its own messages with its own account id.
    let mut o = open(&f.path);
    let id2 = o.queue(msg(f.a2), b"MIME-TWO".to_vec(), 0, 0).unwrap();
    drop(o);
    let loaded = store.load().unwrap();
    let by_account: Vec<(u64, i64)> = loaded.entries.iter().map(|e| (e.id, e.account)).collect();
    assert_eq!(by_account, vec![(id, f.a1), (id2, f.a2)]);
}

#[test]
fn removing_an_account_removes_its_outbox() {
    let f = fixture();
    let mut o = open(&f.path);
    o.queue(msg(f.a1), b"ONE".to_vec(), 0, 0).unwrap();
    o.queue(msg(f.a2), b"TWO".to_vec(), 0, 0).unwrap();
    drop(o);
    let c = raw(&f.path);
    c.execute_batch("PRAGMA foreign_keys = ON").unwrap();
    c.execute("DELETE FROM account WHERE id = ?1", [f.a1])
        .unwrap();
    assert_eq!(row_count(&f.path), 1);
}

#[test]
fn ids_are_not_reused_after_the_newest_row_is_deleted_or_across_restarts() {
    let f = fixture();
    let mut o = open(&f.path);
    let first = o.queue(msg(f.a1), b"A".to_vec(), 0, WINDOW).unwrap();
    o.undo(first, 1).unwrap();
    let second = o.queue(msg(f.a1), b"B".to_vec(), 0, WINDOW).unwrap();
    assert!(second > first);
    o.undo(second, 1).unwrap();
    drop(o);
    // The table is empty; the high-water mark is not.
    let mut o = open(&f.path);
    let third = o.queue(msg(f.a1), b"C".to_vec(), 0, WINDOW).unwrap();
    assert!(third > second);
    // A stale Undo for an old id cannot touch the new message.
    assert!(matches!(
        o.undo(first, 1),
        Err(OutboxError::Undo(UndoError::NotFound))
    ));
    assert_eq!(row_count(&f.path), 1);
}

#[test]
fn a_row_that_cannot_be_read_becomes_failed_not_guessed() {
    let f = fixture();
    let mut o = open(&f.path);
    let id = o.queue(msg(f.a1), b"MIME".to_vec(), 0, 0).unwrap();
    drop(o);
    raw(&f.path)
        .execute(
            "UPDATE outbox SET recipients = 'not json' WHERE id = ?1",
            [id as i64],
        )
        .unwrap();
    let mut o = open(&f.path);
    let e = o.outbox().get(id).unwrap();
    assert_eq!(e.state, OutboxState::Failed);
    assert_eq!(
        e.last_error.as_deref(),
        Some(croft_mail_core::outbox::REASON_UNREADABLE)
    );
    assert!(o.take_due(i64::MAX / 2, 10).unwrap().is_empty());
    // The person can still take the message back.
    assert_eq!(o.edit_back_to_draft(id).unwrap().mime, b"MIME");
}

#[test]
fn a_leftover_sent_row_is_deleted_on_load() {
    let f = fixture();
    let mut o = open(&f.path);
    let id = o.queue(msg(f.a1), b"MIME".to_vec(), 0, 0).unwrap();
    drop(o);
    raw(&f.path)
        .execute(
            "UPDATE outbox SET state = 'sent' WHERE id = ?1",
            [id as i64],
        )
        .unwrap();
    let o = open(&f.path);
    assert_eq!(o.outbox().iter().count(), 0);
    assert_eq!(row_count(&f.path), 0);
}

#[test]
fn the_state_column_only_accepts_known_states() {
    let f = fixture();
    let mut o = open(&f.path);
    let id = o.queue(msg(f.a1), b"MIME".to_vec(), 0, 0).unwrap();
    drop(o);
    let c = raw(&f.path);
    for ok in ["queued", "sending", "submitted", "filing", "sent", "failed"] {
        c.execute(
            "UPDATE outbox SET state = ?2 WHERE id = ?1",
            params![id as i64, ok],
        )
        .unwrap();
    }
    assert!(c
        .execute(
            "UPDATE outbox SET state = 'bogus' WHERE id = ?1",
            [id as i64]
        )
        .is_err());
}

#[test]
fn migration_4_keeps_existing_rows_and_the_id_counter() {
    // A version-3 database with a row written under the version-1 table.
    let c = Connection::open_in_memory().unwrap();
    c.execute_batch("PRAGMA foreign_keys = ON").unwrap();
    for m in &MIGRATIONS[..3] {
        c.execute_batch(m.sql).unwrap();
    }
    c.execute_batch(
        "INSERT INTO account (id,label,email,protocol,config_json,created_at) VALUES (1,'Sam','sam@example.org','imap','{}',0);
         INSERT INTO identity (id,account_id,display_name,address,is_default) VALUES (5,1,'Sam','sam@example.org',1);
         INSERT INTO outbox (id,account_id,identity_id,mime_blob,state,send_at,attempts,last_error)
              VALUES (41,1,5,x'414243','failed',99,3,'old');",
    )
    .unwrap();
    c.execute_batch(MIGRATIONS[3].sql).unwrap();
    let row: (i64, Vec<u8>, String, i64, i64, Option<String>, String, String) = c
        .query_row(
            "SELECT id, mime_blob, state, send_at, attempts, last_error, envelope_from, recipients FROM outbox",
            [],
            |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                    r.get(6)?,
                    r.get(7)?,
                ))
            },
        )
        .unwrap();
    assert_eq!(
        row,
        (
            41,
            b"ABC".to_vec(),
            "failed".into(),
            99,
            3,
            Some("old".into()),
            String::new(),
            "[]".into()
        )
    );
    // New rows continue after the old ones, and AUTOINCREMENT keeps the mark after a delete.
    c.execute(
        "INSERT INTO outbox (account_id, mime_blob, send_at) VALUES (1, x'00', 0)",
        [],
    )
    .unwrap();
    assert_eq!(
        c.query_row("SELECT max(id) FROM outbox", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        42
    );
    c.execute("DELETE FROM outbox", []).unwrap();
    c.execute(
        "INSERT INTO outbox (account_id, mime_blob, send_at) VALUES (1, x'00', 0)",
        [],
    )
    .unwrap();
    assert_eq!(
        c.query_row("SELECT max(id) FROM outbox", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        43
    );
    // Both indexes exist again after the rebuild (dropping the table dropped the old one).
    let indexes: i64 = c
        .query_row(
            "SELECT count(*) FROM sqlite_master WHERE type = 'index' AND name IN ('outbox_due', 'outbox_account')",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(indexes, 2);
    c.execute("DELETE FROM account WHERE id = 1", []).unwrap();
    assert_eq!(
        c.query_row("SELECT count(*) FROM outbox", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        0
    );
}

#[test]
fn entries_loaded_from_sqlite_equal_the_ones_written() {
    let f = fixture();
    let mut o = open(&f.path);
    let id = o.queue(msg(f.a2), b"MIME".to_vec(), 123, WINDOW).unwrap();
    drop(o);
    let mut store = SqliteOutboxStore::new(Db::open(&f.path, None).unwrap());
    let loaded = store.load().unwrap();
    assert_eq!(loaded.max_id, id);
    assert_eq!(
        loaded.entries,
        vec![OutboxEntry {
            id,
            account: f.a2,
            identity: Some(f.a2 * 10),
            envelope_from: "one@example.org".into(),
            recipients: vec!["you@example.org".into(), "bcc@example.org".into()],
            state: OutboxState::Queued,
            send_at_ms: 123 + WINDOW,
            attempts: 0,
            last_error: None,
        }]
    );
}
