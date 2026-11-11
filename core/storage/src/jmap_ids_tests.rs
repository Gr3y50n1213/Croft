// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Tests of the JMAP id map: the store on its own, the rules the schema keeps, and the store under
//! the JMAP adapter (in-process JMAP server) across a restart.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;

use croft_protocols::jmap::{IdStoreError, JmapBackend, JmapConfig, JmapIdStore};
use croft_protocols::{Credential, MailBackend};
use croft_test_support::{JmapServer, JMAP_PASSWORD, JMAP_USER};
use rusqlite::{params, Connection, ErrorCode};

use crate::{Db, SqliteJmapIdStore};

fn ids(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| s.to_string()).collect()
}

fn with_account(db: &Db, id: i64) {
    db.connection()
        .execute(
            "INSERT INTO account (id,label,email,protocol,config_json,created_at) VALUES (?1,'S','s@example.org','jmap','{}',0)",
            params![id],
        )
        .unwrap();
}

fn memory_store() -> SqliteJmapIdStore {
    let db = Db::open_in_memory().unwrap();
    with_account(&db, 1);
    SqliteJmapIdStore::new(db, 1)
}

fn constraint(r: rusqlite::Result<usize>) -> bool {
    matches!(r, Err(rusqlite::Error::SqliteFailure(e, _)) if e.code == ErrorCode::ConstraintViolation)
}

#[test]
fn uids_increase_from_one_and_are_never_reused() {
    let s = memory_store();
    assert_eq!(s.next_uid("m").unwrap(), 1);
    assert_eq!(
        s.assign("m", &ids(&["a", "b", "c"])).unwrap(),
        vec![1, 2, 3]
    );
    s.remove("m", &ids(&["c", "b", "unknown"])).unwrap();
    assert_eq!(s.next_uid("m").unwrap(), 4);
    assert_eq!(
        s.assign("m", &ids(&["d", "a", "d"])).unwrap(),
        vec![4, 1, 4]
    );
    // An id that was removed and comes back is a new message as far as UIDs go.
    assert_eq!(s.assign("m", &ids(&["c"])).unwrap(), vec![5]);
    let entries = s.entries("m").unwrap();
    assert_eq!(
        entries.into_iter().collect::<Vec<_>>(),
        vec![(1, "a".to_string()), (4, "d".into()), (5, "c".into())]
    );
    let found = s.email_ids("m", &[1, 2, 5, 99]).unwrap();
    assert_eq!(found.len(), 2);
    assert_eq!(found[&5], "c");
}

#[test]
fn mailboxes_number_independently_with_a_stable_random_validity() {
    let s = memory_store();
    assert_eq!(s.assign("m1", &ids(&["a"])).unwrap(), vec![1]);
    assert_eq!(s.assign("m2", &ids(&["a"])).unwrap(), vec![1]);
    let v1 = s.uid_validity("m1").unwrap();
    assert!(v1 != 0);
    assert_eq!(s.uid_validity("m1").unwrap(), v1);
    // Validity is created on first sight, by any call.
    assert!(s.uid_validity("brand-new").unwrap() != 0);
}

#[test]
fn reset_changes_the_validity_and_restarts_the_numbering() {
    let s = memory_store();
    s.assign("m", &ids(&["a", "b"])).unwrap();
    let before = s.uid_validity("m").unwrap();
    let after = s.reset_mailbox("m").unwrap();
    assert_ne!(before, after);
    assert_eq!(s.uid_validity("m").unwrap(), after);
    assert!(s.entries("m").unwrap().is_empty());
    assert_eq!(s.assign("m", &ids(&["b"])).unwrap(), vec![1]);
    s.forget_mailbox("m").unwrap();
    assert!(s.entries("m").unwrap().is_empty());
    assert_eq!(s.next_uid("m").unwrap(), 1);
}

#[test]
fn the_email_state_round_trips() {
    let s = memory_store();
    assert_eq!(s.email_state().unwrap(), None);
    s.set_email_state(Some("s1")).unwrap();
    s.set_email_state(Some("s2")).unwrap();
    assert_eq!(s.email_state().unwrap().as_deref(), Some("s2"));
    s.set_email_state(None).unwrap();
    assert_eq!(s.email_state().unwrap(), None);
}

#[test]
fn everything_survives_a_reopen_and_numbering_continues() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("croft.db");
    let (validity, state_after) = {
        let db = Db::open(&path, None).unwrap();
        with_account(&db, 1);
        let s = SqliteJmapIdStore::new(db, 1);
        assert_eq!(
            s.assign("inbox", &ids(&["a", "b", "c"])).unwrap(),
            vec![1, 2, 3]
        );
        s.remove("inbox", &ids(&["c"])).unwrap();
        s.set_email_state(Some("state-9")).unwrap();
        (s.uid_validity("inbox").unwrap(), "state-9")
    };
    let db = Db::open(&path, None).unwrap();
    let s = SqliteJmapIdStore::new(db, 1);
    assert_eq!(s.uid_validity("inbox").unwrap(), validity);
    assert_eq!(s.email_state().unwrap().as_deref(), Some(state_after));
    assert_eq!(
        s.assign("inbox", &ids(&["b", "d"])).unwrap(),
        vec![2, 4],
        "c's UID 3 is not reused"
    );
    assert_eq!(
        s.entries("inbox")
            .unwrap()
            .keys()
            .copied()
            .collect::<Vec<_>>(),
        vec![1, 2, 4]
    );
}

#[test]
fn accounts_do_not_see_each_other_and_deleting_an_account_removes_its_map() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("croft.db");
    let db1 = Db::open(&path, None).unwrap();
    with_account(&db1, 1);
    with_account(&db1, 2);
    let db2 = Db::open(&path, None).unwrap();
    let one = SqliteJmapIdStore::new(db1, 1);
    let two = SqliteJmapIdStore::new(db2, 2);
    one.assign("inbox", &ids(&["a", "b"])).unwrap();
    two.assign("inbox", &ids(&["x"])).unwrap();
    one.set_email_state(Some("one")).unwrap();
    assert_eq!(two.entries("inbox").unwrap().len(), 1);
    assert_eq!(two.email_state().unwrap(), None);
    assert_eq!(two.next_uid("inbox").unwrap(), 2);
    assert_eq!(one.next_uid("inbox").unwrap(), 3);
    one.reset_mailbox("inbox").unwrap();
    assert_eq!(
        two.entries("inbox").unwrap().len(),
        1,
        "a reset touches only its own account"
    );

    let db = one.into_inner();
    let c = db.connection();
    c.execute("DELETE FROM account WHERE id = 1", []).unwrap();
    let count = |table: &str| -> i64 {
        c.query_row(
            &format!("SELECT count(*) FROM {table} WHERE account_id = 1"),
            [],
            |r| r.get(0),
        )
        .unwrap()
    };
    assert_eq!(
        (
            count("jmap_mailbox"),
            count("jmap_email_uid"),
            count("jmap_account_state")
        ),
        (0, 0, 0)
    );
    let left: i64 = c
        .query_row(
            "SELECT count(*) FROM jmap_email_uid WHERE account_id = 2",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(left, 1);
}

#[test]
fn the_schema_itself_refuses_to_hand_out_a_uid_twice() {
    let s = memory_store();
    s.assign("m", &ids(&["a", "b"])).unwrap();
    let db = s.into_inner();
    let c = db.connection();
    // A UID at or above the counter.
    assert!(constraint(
        c.execute("INSERT INTO jmap_email_uid VALUES (1,'m','x',3)", [])
    ));
    // The same UID twice, the same email twice.
    assert!(constraint(
        c.execute("INSERT INTO jmap_email_uid VALUES (1,'m','x',2)", [])
    ));
    assert!(constraint(
        c.execute("INSERT INTO jmap_email_uid VALUES (1,'m','a',2)", [])
    ));
    // A UID is never reassigned, and the counter never goes down.
    assert!(constraint(c.execute(
        "UPDATE jmap_email_uid SET uid = 9 WHERE email_id = 'a'",
        []
    )));
    assert!(constraint(
        c.execute("UPDATE jmap_mailbox SET next_uid = 1", [])
    ));
    // Outside the range of a u32, validity 0, an entry for a mailbox that does not exist.
    assert!(constraint(
        c.execute("UPDATE jmap_mailbox SET next_uid = 4294967297", [])
    ));
    assert!(constraint(
        c.execute("UPDATE jmap_mailbox SET uid_validity = 0", [])
    ));
    assert!(constraint(c.execute(
        "INSERT INTO jmap_email_uid VALUES (1,'nope','x',1)",
        []
    )));
    assert!(constraint(
        c.execute("INSERT INTO jmap_mailbox VALUES (99,'m',5,1)", [])
    ));
}

#[test]
fn running_out_of_uids_is_reported_and_changes_nothing() {
    let s = memory_store();
    s.assign("m", &ids(&["a"])).unwrap();
    let db = s.into_inner();
    db.connection()
        .execute("UPDATE jmap_mailbox SET next_uid = 4294967295", [])
        .unwrap();
    let s = SqliteJmapIdStore::new(db, 1);
    assert_eq!(
        s.assign("m", &ids(&["x", "y"])),
        Err(IdStoreError::Exhausted)
    );
    assert_eq!(
        s.next_uid("m").unwrap(),
        4_294_967_295,
        "nothing was assigned"
    );
    assert_eq!(s.assign("m", &ids(&["x"])).unwrap(), vec![4_294_967_295]);
    assert_eq!(s.next_uid("m"), Err(IdStoreError::Exhausted));
    assert_eq!(
        s.assign("m", &ids(&["x"])).unwrap(),
        vec![4_294_967_295],
        "known ids still resolve"
    );
    assert_eq!(s.assign("m", &ids(&["z"])), Err(IdStoreError::Exhausted));
    // A reset is the way out.
    s.reset_mailbox("m").unwrap();
    assert_eq!(s.assign("m", &ids(&["z"])).unwrap(), vec![1]);
}

#[test]
fn a_map_that_contradicts_itself_is_reported_as_corrupt() {
    let s = memory_store();
    s.assign("m", &ids(&["a"])).unwrap();
    let db = s.into_inner();
    // Someone removed the guard and wrote an entry above the counter.
    db.connection()
        .execute_batch("DROP TRIGGER jmap_email_uid_below_next")
        .unwrap();
    db.connection()
        .execute("INSERT INTO jmap_email_uid VALUES (1,'m','bad',50)", [])
        .unwrap();
    let s = SqliteJmapIdStore::new(db, 1);
    assert_eq!(s.entries("m"), Err(IdStoreError::Corrupt));
    // The adapter answers that by resetting the mailbox.
    s.reset_mailbox("m").unwrap();
    assert!(s.entries("m").unwrap().is_empty());
}

#[test]
fn a_missing_account_is_a_failure_not_a_silent_success() {
    let db = Db::open_in_memory().unwrap();
    let s = SqliteJmapIdStore::new(db, 42);
    assert!(s.uid_validity("m").is_err());
    assert!(s.assign("m", &ids(&["a"])).is_err());
}

fn config(server: &JmapServer) -> JmapConfig {
    JmapConfig::new(server.session_url(), JMAP_USER).with_timeout(Duration::from_secs(5))
}

fn adapter(server: &JmapServer, path: &std::path::Path) -> JmapBackend {
    let db = Db::open(path, None).unwrap();
    let store: Arc<dyn JmapIdStore + Send + Sync> = Arc::new(SqliteJmapIdStore::new(db, 1));
    JmapBackend::new(config(server), Credential::password(JMAP_PASSWORD), store).unwrap()
}

fn raw(id: &str) -> String {
    format!(
        "From: Ann <ann@example.org>\r\nSubject: S\r\nDate: Mon, 02 Mar 2026 09:30:00 +0000\r\nMessage-ID: <{id}>\r\n\r\nBody\r\n"
    )
}

#[tokio::test]
async fn the_adapter_keeps_its_uids_across_a_restart_and_gets_a_new_validity_when_the_map_is_lost()
{
    let server = JmapServer::start().await;
    let first_id = server.add_message("inbox", &raw("a@x"), &[]);
    server.add_message("inbox", &raw("b@x"), &[]);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("croft.db");
    {
        let db = Db::open(&path, None).unwrap();
        with_account(&db, 1);
    }

    let (validity, first) = {
        let b = adapter(&server, &path);
        let snap = b.snapshot("Inbox").await.unwrap();
        assert_eq!(
            snap.messages.keys().copied().collect::<Vec<_>>(),
            vec![1, 2]
        );
        // Moving gives the message a new UID in the destination and frees nothing in the source.
        let moved = b.move_to("Inbox", &[1], "Archive").await.unwrap();
        assert_eq!(moved.dest_uids[&1], 1);
        (snap.status.uid_validity, snap)
    };
    let _ = (first, first_id);

    // A new adapter on the same database file: same validity, same UIDs, numbering continues.
    server.add_message("inbox", &raw("c@x"), &[]);
    let b = adapter(&server, &path);
    let snap = b.snapshot("Inbox").await.unwrap();
    assert_eq!(snap.status.uid_validity, validity);
    assert_eq!(
        snap.messages.keys().copied().collect::<Vec<_>>(),
        vec![2, 3],
        "UID 1 left with the move and is not reused"
    );
    let archive = b.snapshot("Archive").await.unwrap();
    assert_eq!(
        archive.messages.keys().copied().collect::<BTreeSet<_>>(),
        BTreeSet::from([1])
    );

    // The map is lost (the table is emptied): the folder starts over with a new validity.
    {
        let db = Db::open(&path, None).unwrap();
        let c: &Connection = db.connection();
        c.execute("DELETE FROM jmap_mailbox", []).unwrap();
    }
    let b = adapter(&server, &path);
    let snap = b.snapshot("Inbox").await.unwrap();
    assert_ne!(snap.status.uid_validity, validity);
    assert_eq!(snap.messages.len(), 2);
}
