// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Rust port of every check in `tools/check_schema.py`, run against an in-memory database built by
//! [`Db::open_in_memory`] (so the migration path itself is under test too). The Python script stays
//! as an independent check of the same SQL.

// Tests may unwrap: a panic is the failure report.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use rusqlite::{params, Connection, ErrorCode};

use crate::{Db, MIGRATIONS};

fn one<T: rusqlite::types::FromSql>(c: &Connection, sql: &str) -> T {
    c.query_row(sql, [], |r| r.get(0)).unwrap()
}

fn pair(c: &Connection, sql: &str) -> (i64, i64) {
    c.query_row(sql, [], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
}

fn uids_newest_first(c: &Connection) -> Vec<i64> {
    let mut stmt = c
        .prepare("SELECT uid FROM message WHERE folder_id=1 ORDER BY date_ts DESC")
        .unwrap();
    let rows = stmt.query_map([], |r| r.get(0)).unwrap();
    rows.map(Result::unwrap).collect()
}

/// One account, one inbox, three messages (uid 1 unread newest, uid 2 read, uid 3 unread oldest).
fn fixture() -> Db {
    let db = Db::open_in_memory().unwrap();
    let c = db.connection();
    c.execute(
        "INSERT INTO account (id,label,email,protocol,config_json,created_at) VALUES (1,'Sam','sam@example.org','imap','{}',0)",
        [],
    )
    .unwrap();
    c.execute(
        "INSERT INTO folder (id,account_id,path,special_use) VALUES (1,1,'INBOX','inbox')",
        [],
    )
    .unwrap();
    for (uid, subj, seen, ts) in [
        (1, "Lunch on Friday?", 0, 300),
        (2, "Server migration notes", 1, 200),
        (3, "Bin collection changes", 0, 100),
    ] {
        c.execute(
            "INSERT INTO message (account_id,folder_id,uid,thread_key,subject,from_name,from_addr,date_ts,is_seen) VALUES (1,1,?,?,?,?,?,?,?)",
            params![uid, format!("t{uid}"), subj, "Sam Rivera", "sam@example.org", ts, seen],
        )
        .unwrap();
    }
    db
}

/// The fixture after the script's counter checks: uid 1 flipped read and back, uid 3 deleted.
fn after_counter_steps() -> Db {
    let db = fixture();
    let c = db.connection();
    c.execute("UPDATE message SET is_seen = 1 WHERE uid = 1", [])
        .unwrap();
    c.execute("UPDATE message SET is_seen = 0 WHERE uid = 1", [])
        .unwrap();
    c.execute("DELETE FROM message WHERE uid = 3", []).unwrap();
    db
}

fn counters(c: &Connection) -> (i64, i64) {
    pair(c, "SELECT total_count, unread_count FROM folder WHERE id=1")
}

#[test]
fn folder_counters_after_insert_total_3_unread_2() {
    assert_eq!(counters(fixture().connection()), (3, 2));
}

#[test]
fn marking_read_lowers_unread_to_1() {
    let db = fixture();
    let c = db.connection();
    c.execute("UPDATE message SET is_seen = 1 WHERE uid = 1", [])
        .unwrap();
    assert_eq!(
        one::<i64>(c, "SELECT unread_count FROM folder WHERE id=1"),
        1
    );
}

#[test]
fn marking_unread_raises_unread_to_2() {
    let db = fixture();
    let c = db.connection();
    c.execute("UPDATE message SET is_seen = 1 WHERE uid = 1", [])
        .unwrap();
    c.execute("UPDATE message SET is_seen = 0 WHERE uid = 1", [])
        .unwrap();
    assert_eq!(
        one::<i64>(c, "SELECT unread_count FROM folder WHERE id=1"),
        2
    );
}

#[test]
fn deleting_an_unread_message_updates_both_counters() {
    let db = after_counter_steps();
    assert_eq!(counters(db.connection()), (2, 1));
}

#[test]
fn list_query_is_newest_first() {
    let db = after_counter_steps();
    assert_eq!(uids_newest_first(db.connection()), vec![1, 2]);
}

fn plan(c: &Connection, sql: &str, params: impl rusqlite::Params) -> String {
    let mut stmt = c.prepare(&format!("EXPLAIN QUERY PLAN {sql}")).unwrap();
    let rows = stmt.query_map(params, |r| r.get::<_, String>(3)).unwrap();
    rows.map(Result::unwrap).collect::<Vec<_>>().join(" ")
}

#[test]
fn list_query_uses_message_folder_date_without_a_sort_step() {
    let db = after_counter_steps();
    let p = plan(
        db.connection(),
        "SELECT uid FROM message WHERE folder_id=1 ORDER BY date_ts DESC",
        [],
    );
    assert!(
        p.contains("message_folder_date") && !p.contains("TEMP B-TREE"),
        "{p}"
    );
}

#[test]
fn search_finds_subject_words() {
    let db = after_counter_steps();
    let n: i64 = one(
        db.connection(),
        "SELECT count(*) FROM message_fts WHERE message_fts MATCH 'lunch'",
    );
    assert_eq!(n, 1);
}

#[test]
fn search_finds_sender() {
    let db = after_counter_steps();
    let n: i64 = one(
        db.connection(),
        "SELECT count(*) FROM message_fts WHERE message_fts MATCH 'rivera'",
    );
    assert_eq!(n, 2);
}

fn add_dns_body(c: &Connection) {
    c.execute(
        "INSERT INTO message_body (message_id, text_plain) VALUES (2, 'Thanks, I will test the new DNS records tomorrow')",
        [],
    )
    .unwrap();
}

#[test]
fn search_finds_body_text_added_later() {
    let db = after_counter_steps();
    add_dns_body(db.connection());
    let rowid: i64 = one(
        db.connection(),
        "SELECT rowid FROM message_fts WHERE message_fts MATCH 'dns'",
    );
    assert_eq!(rowid, 2);
}

#[test]
fn search_follows_subject_edits() {
    let db = after_counter_steps();
    let c = db.connection();
    c.execute(
        "UPDATE message SET subject = 'Lunch moved to Thursday' WHERE uid = 1",
        [],
    )
    .unwrap();
    let thursday: i64 = one(
        c,
        "SELECT count(*) FROM message_fts WHERE message_fts MATCH 'thursday'",
    );
    let friday: i64 = one(
        c,
        "SELECT count(*) FROM message_fts WHERE message_fts MATCH 'friday'",
    );
    assert_eq!((thursday, friday), (1, 0));
}

#[test]
fn search_drops_deleted_messages() {
    let db = after_counter_steps();
    let c = db.connection();
    add_dns_body(c);
    c.execute("DELETE FROM message WHERE uid = 2", []).unwrap();
    let n: i64 = one(
        c,
        "SELECT count(*) FROM message_fts WHERE message_fts MATCH 'dns'",
    );
    assert_eq!(n, 0);
}

fn queue_fixture() -> Db {
    let db = fixture();
    let c = db.connection();
    c.execute(
        "INSERT INTO op_queue (account_id,kind,payload_json,target_key,created_at,not_before) VALUES (1,'set_seen','{}','m1',1000,6000)",
        [],
    )
    .unwrap();
    c.execute(
        "INSERT INTO op_queue (account_id,kind,payload_json,target_key,created_at,not_before,state) VALUES (1,'move','{}','m2',1000,500,'confirmed')",
        [],
    )
    .unwrap();
    db
}

fn due(c: &Connection, now: i64) -> i64 {
    c.query_row(
        "SELECT count(*) FROM op_queue WHERE state='queued' AND not_before <= ?",
        [now],
        |r| r.get(0),
    )
    .unwrap()
}

#[test]
fn op_inside_undo_window_is_not_due() {
    assert_eq!(due(queue_fixture().connection(), 5999), 0);
}

#[test]
fn op_becomes_due_at_not_before() {
    assert_eq!(due(queue_fixture().connection(), 6000), 1);
}

#[test]
fn cancelled_op_is_never_due() {
    let db = queue_fixture();
    let c = db.connection();
    c.execute("UPDATE op_queue SET state='cancelled' WHERE id=1", [])
        .unwrap();
    assert_eq!(due(c, 99999), 0);
}

fn is_constraint(e: &rusqlite::Error) -> bool {
    matches!(e, rusqlite::Error::SqliteFailure(f, _) if f.code == ErrorCode::ConstraintViolation)
}

#[test]
fn category_check_rejects_unknown_values() {
    let db = fixture();
    let err = db
        .connection()
        .execute(
            "INSERT INTO message (account_id,folder_id,uid,thread_key,date_ts,category) VALUES (1,1,9,'t9',0,'spam')",
            [],
        )
        .unwrap_err();
    assert!(is_constraint(&err), "{err}");
}

#[test]
fn foreign_key_rejects_a_missing_account() {
    let db = fixture();
    let err = db
        .connection()
        .execute("INSERT INTO folder (account_id,path) VALUES (99,'X')", [])
        .unwrap_err();
    assert!(is_constraint(&err), "{err}");
}

#[test]
fn removing_an_account_removes_its_folders_messages_and_queue() {
    let db = queue_fixture();
    let c = db.connection();
    c.execute("DELETE FROM account WHERE id = 1", []).unwrap();
    let left: i64 = one(
        c,
        "SELECT (SELECT count(*) FROM folder) + (SELECT count(*) FROM message) + (SELECT count(*) FROM op_queue)",
    );
    assert_eq!(left, 0);
}

#[test]
fn opening_applies_every_migration_and_sets_user_version() {
    let db = Db::open_in_memory().unwrap();
    assert_eq!(db.migrations_run_on_open(), MIGRATIONS.len());
    assert_eq!(db.schema_version().unwrap(), crate::latest_version());
}
