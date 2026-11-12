// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Tests for [`crate::search`] on a populated in-memory database: ranking, account scoping, the
//! index triggers, bodies, every filter, and hostile input.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use croft_mail_core::search::parse;
use croft_protocols::{Flags, RemoteFolder, RemoteHeader, SpecialUse};
use rusqlite::params;
use serde_json::json;

use crate::accounts::{self, NewAccount, Protocol};
use crate::reading::{self, NewBody};
use crate::search::{search, MARK_END, MARK_START};
use crate::{folders, messages, Db};

fn account(c: &rusqlite::Connection, email: &str) -> i64 {
    accounts::insert(
        c,
        &NewAccount {
            label: email.into(),
            email: email.into(),
            protocol: Protocol::Imap,
            provider: "generic".into(),
            config: json!({}),
            color: None,
            position: 0,
        },
        1,
    )
    .unwrap()
}

fn folder(c: &rusqlite::Connection, account: i64, path: &str, special_use: SpecialUse) -> i64 {
    folders::upsert(
        c,
        account,
        &RemoteFolder {
            path: path.into(),
            delimiter: Some('/'),
            special_use,
            selectable: true,
        },
    )
    .unwrap()
}

#[allow(clippy::too_many_arguments)]
fn msg(
    c: &rusqlite::Connection,
    account: i64,
    folder: i64,
    uid: u32,
    subject: &str,
    from: (&str, &str),
    date_ms: i64,
) -> i64 {
    messages::upsert(
        c,
        account,
        folder,
        &RemoteHeader {
            uid,
            message_id: Some(format!("<m{uid}.{account}@example.org>")),
            references: vec![],
            subject: subject.into(),
            from_name: Some(from.0.into()),
            from_addr: Some(from.1.into()),
            date_ms,
            size: 100,
            flags: Flags::default(),
        },
    )
    .unwrap()
}

fn store_body(c: &rusqlite::Connection, account: i64, id: i64, text: &str, to: &str) {
    reading::store_body(
        c,
        account,
        id,
        &NewBody {
            raw: b"x",
            raw_sha256: &[1; 32],
            fetched_at: 1,
            headers_raw: "",
            text: Some(text),
            html_clean: None,
            remote_blocked: 0,
            trackers_removed: 0,
            truncated: false,
            too_complex: false,
            html_pending: false,
            to_json: to,
            cc_json: "[]",
            snippet: "",
            attachments: &[],
        },
    )
    .unwrap();
}

fn ids(c: &rusqlite::Connection, accounts: &[i64], q: &str) -> Vec<i64> {
    search(c, accounts, &parse(q).unwrap(), 50)
        .unwrap()
        .into_iter()
        .map(|h| h.message_id)
        .collect()
}

struct Fixture {
    db: Db,
    a1: i64,
    a2: i64,
    inbox: i64,
    trash: i64,
    archive: i64,
}

fn fixture() -> Fixture {
    let db = Db::open_in_memory().unwrap();
    let c = db.connection();
    let a1 = account(c, "me@example.org");
    let a2 = account(c, "other@example.org");
    let inbox = folder(c, a1, "INBOX", SpecialUse::Inbox);
    let trash = folder(c, a1, "Trash", SpecialUse::Trash);
    let archive = folder(c, a1, "Projects/Archive 2026", SpecialUse::Archive);
    Fixture {
        db,
        a1,
        a2,
        inbox,
        trash,
        archive,
    }
}

#[test]
fn subject_matches_rank_above_body_matches_and_ties_are_newest_first() {
    let f = fixture();
    let c = f.db.connection();
    let body_old = msg(
        c,
        f.a1,
        f.inbox,
        1,
        "Weekly notes",
        ("Ann", "ann@example.org"),
        1_000,
    );
    let subject = msg(
        c,
        f.a1,
        f.inbox,
        2,
        "Lunch on Friday",
        ("Ben", "ben@example.org"),
        500,
    );
    let body_new = msg(
        c,
        f.a1,
        f.inbox,
        3,
        "Other things",
        ("Cy", "cy@example.org"),
        2_000,
    );
    store_body(c, f.a1, body_old, "we could have lunch", "[]");
    store_body(c, f.a1, body_new, "lunch is at noon", "[]");
    let got = ids(c, &[f.a1], "lunch");
    assert_eq!(got[0], subject, "subject first");
    // The two body matches have the same score: newest first. The order is stable across runs.
    assert_eq!(&got[1..], &[body_new, body_old]);
    for _ in 0..5 {
        assert_eq!(ids(c, &[f.a1], "lunch"), got);
    }
    // Words match by prefix: typing "lun" already finds them.
    assert_eq!(ids(c, &[f.a1], "lun"), got);
}

#[test]
fn another_accounts_message_never_appears() {
    let f = fixture();
    let c = f.db.connection();
    let other_inbox = folder(c, f.a2, "INBOX", SpecialUse::Inbox);
    let mine = msg(
        c,
        f.a1,
        f.inbox,
        1,
        "Budget review",
        ("Ann", "ann@example.org"),
        1,
    );
    let theirs = msg(
        c,
        f.a2,
        other_inbox,
        1,
        "Budget review",
        ("Ann", "ann@example.org"),
        2,
    );
    assert_eq!(ids(c, &[f.a1], "budget"), vec![mine]);
    assert_eq!(ids(c, &[f.a2], "budget"), vec![theirs]);
    assert_eq!(ids(c, &[f.a1, f.a2], "budget"), vec![theirs, mine]);
    assert_eq!(ids(c, &[], "budget"), Vec::<i64>::new());
}

#[test]
fn updates_and_deletes_reach_the_index() {
    let f = fixture();
    let c = f.db.connection();
    let id = msg(
        c,
        f.a1,
        f.inbox,
        1,
        "Old subject",
        ("Ann", "ann@example.org"),
        1,
    );
    assert_eq!(ids(c, &[f.a1], "old"), vec![id]);
    msg(
        c,
        f.a1,
        f.inbox,
        1,
        "New subject",
        ("Ann", "ann@example.org"),
        1,
    );
    assert!(
        ids(c, &[f.a1], "old").is_empty(),
        "the old subject left the index"
    );
    assert_eq!(ids(c, &[f.a1], "new"), vec![id]);
    assert!(messages::delete(c, f.a1, id).unwrap());
    assert!(ids(c, &[f.a1], "new").is_empty(), "deleted");
}

#[test]
fn the_body_is_searched_only_once_it_is_stored() {
    let f = fixture();
    let c = f.db.connection();
    let id = msg(c, f.a1, f.inbox, 1, "Hello", ("Ann", "ann@example.org"), 1);
    assert!(ids(c, &[f.a1], "zebra").is_empty());
    store_body(c, f.a1, id, "a zebra in the garden", "[]");
    let hits = search(c, &[f.a1], &parse("zebra").unwrap(), 10).unwrap();
    assert_eq!(hits.len(), 1);
    let snippet = hits[0].snippet_marked.as_deref().unwrap();
    assert!(
        snippet.contains(&format!("{MARK_START}zebra{MARK_END}")),
        "{snippet:?}"
    );
}

#[test]
fn matches_are_marked_in_the_subject() {
    let f = fixture();
    let c = f.db.connection();
    msg(
        c,
        f.a1,
        f.inbox,
        1,
        "Quarterly budget review",
        ("Ann", "ann@example.org"),
        1,
    );
    let hits = search(c, &[f.a1], &parse("budget").unwrap(), 10).unwrap();
    assert_eq!(
        hits[0].subject_marked.as_deref(),
        Some(format!("Quarterly {MARK_START}budget{MARK_END} review").as_str())
    );
}

#[test]
fn every_filter() {
    let f = fixture();
    let c = f.db.connection();
    let read = msg(
        c,
        f.a1,
        f.inbox,
        1,
        "Report",
        ("Ann", "ann@example.org"),
        1_000,
    );
    let unread = msg(
        c,
        f.a1,
        f.inbox,
        2,
        "Report",
        ("Ben Lee", "ben@example.org"),
        2_000,
    );
    let archived = msg(
        c,
        f.a1,
        f.archive,
        3,
        "Report",
        ("Ann", "ann@example.org"),
        3_000,
    );
    let trashed = msg(
        c,
        f.a1,
        f.trash,
        4,
        "Report",
        ("Ann", "ann@example.org"),
        4_000,
    );
    for id in [read, archived, trashed] {
        c.execute("UPDATE message SET is_seen = 1 WHERE id = ?1", params![id])
            .unwrap();
    }
    c.execute(
        "UPDATE message SET is_flagged = 1, has_attachments = 1 WHERE id = ?1",
        params![archived],
    )
    .unwrap();
    store_body(
        c,
        f.a1,
        read,
        "text",
        r#"[{"name":"Dee","address":"dee@example.com"}]"#,
    );

    assert_eq!(
        ids(c, &[f.a1], "report"),
        vec![archived, unread, read],
        "Trash left out"
    );
    assert_eq!(ids(c, &[f.a1], "report is:unread"), vec![unread]);
    assert_eq!(
        ids(c, &[f.a1], "is:starred"),
        vec![archived],
        "filters alone"
    );
    assert_eq!(ids(c, &[f.a1], "has:attachment"), vec![archived]);
    assert_eq!(ids(c, &[f.a1], "from:ben"), vec![unread]);
    assert_eq!(ids(c, &[f.a1], "from:\"ben lee\""), vec![unread]);
    assert_eq!(
        ids(c, &[f.a1], "subject:report from:ann"),
        vec![archived, read]
    );
    assert_eq!(ids(c, &[f.a1], "to:dee@example"), vec![read]);
    assert_eq!(
        ids(c, &[f.a1], "to:%"),
        Vec::<i64>::new(),
        "LIKE wildcards are literal"
    );
    assert_eq!(
        ids(c, &[f.a1], "in:trash report"),
        vec![trashed],
        "named: Trash searched"
    );
    assert_eq!(ids(c, &[f.a1], "in:archive"), vec![archived], "by role");
    assert_eq!(
        ids(c, &[f.a1], "in:\"archive 2026\""),
        vec![archived],
        "by last path part"
    );
    assert_eq!(ids(c, &[f.a1], "in:nowhere report"), Vec::<i64>::new());
    // Dates: 1970-01-01 is 0 ms; the rows are at 1 to 4 seconds.
    assert_eq!(
        ids(c, &[f.a1], "report before:1970-01-01"),
        Vec::<i64>::new()
    );
    assert_eq!(ids(c, &[f.a1], "report after:1970-01-01").len(), 3);
}

#[test]
fn hostile_queries_never_fail() {
    let f = fixture();
    let c = f.db.connection();
    msg(
        c,
        f.a1,
        f.inbox,
        1,
        "Hello \"world\" (test) *",
        ("Ann", "ann@example.org"),
        1,
    );
    for q in [
        "\"",
        "*",
        "NEAR(",
        ")",
        "a OR b",
        "x AND NOT y",
        "^start",
        "-minus",
        "\"unterminated",
        "subject:\"a\"b\"c",
        "from:\"",
        "(((((",
        "{sender}: x",
        "world\"*\"",
        "\u{2}\u{3}",
        "' OR 1=1 --",
        "%' ; DROP TABLE message; --",
    ] {
        let parsed = parse(q).unwrap();
        search(c, &[f.a1], &parsed, 10).unwrap_or_else(|e| panic!("{q:?}: {e}"));
    }
    assert_eq!(ids(c, &[f.a1], "\"world\""), ids(c, &[f.a1], "world"));
    // The table is still there.
    assert_eq!(ids(c, &[f.a1], "hello").len(), 1);
}
