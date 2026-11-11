// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Tests for [`crate::reading`]: stored bodies, attachments, the raw copy, remote-content choices
//! and conversations, each also checked against another account's ids.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use croft_protocols::{Flags, RemoteFolder, RemoteHeader, SpecialUse};
use serde_json::json;

use crate::accounts::{self, NewAccount, Protocol};
use crate::reading::{self, NewAttachment, NewBody};
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

fn message(
    c: &rusqlite::Connection,
    account: i64,
    folder: i64,
    uid: u32,
    id: &str,
    refs: &[&str],
    date_ms: i64,
) -> i64 {
    messages::upsert(
        c,
        account,
        folder,
        &RemoteHeader {
            uid,
            message_id: Some(format!("<{id}>")),
            references: refs.iter().map(|r| format!("<{r}>")).collect(),
            subject: "Plans".into(),
            from_name: None,
            from_addr: Some("ann@example.org".into()),
            date_ms,
            size: 100,
            flags: Flags::default(),
        },
    )
    .unwrap()
}

fn body<'a>(attachments: &'a [NewAttachment], text: &'a str) -> NewBody<'a> {
    NewBody {
        raw: b"From: ann@example.org\r\n\r\nhello",
        fetched_at: 7,
        headers_raw: "From: ann@example.org",
        text: Some(text),
        html_clean: Some("<p>hello</p>"),
        remote_blocked: 2,
        trackers_removed: 1,
        truncated: false,
        to_json: r#"[{"name":"Ben","address":"ben@example.com"}]"#,
        cc_json: "[]",
        snippet: "hello",
        attachments,
    }
}

fn pdf() -> NewAttachment {
    NewAttachment {
        part_id: "3".into(),
        file_name: "report.pdf".into(),
        mime_type: "application/pdf".into(),
        size: 42,
        content_id: None,
        shown_inline: false,
        executable: false,
    }
}

#[test]
fn a_stored_body_reads_back_for_its_account_only_and_is_searchable() {
    let db = Db::open_in_memory().unwrap();
    let c = db.connection();
    let (a1, a2) = (account(c, "one@example.org"), account(c, "two@example.org"));
    let f1 = folder(c, a1, "INBOX", SpecialUse::Inbox);
    let m = message(c, a1, f1, 1, "m1@example.org", &[], 10);

    assert!(!reading::store_body(c, a2, m, &body(&[pdf()], "nope")).unwrap());
    assert!(
        reading::body(c, a1, m).unwrap().is_none(),
        "nothing written for the wrong account"
    );
    assert!(reading::store_body(c, a1, m, &body(&[pdf()], "zebra crossing")).unwrap());

    let b = reading::body(c, a1, m).unwrap().unwrap();
    assert_eq!(b.text.as_deref(), Some("zebra crossing"));
    assert_eq!((b.remote_blocked, b.trackers_removed), (2, 1));
    assert!(reading::body(c, a2, m).unwrap().is_none());
    assert_eq!(
        reading::raw(c, a1, m).unwrap().unwrap(),
        b"From: ann@example.org\r\n\r\nhello"
    );
    assert!(reading::raw(c, a2, m).unwrap().is_none());

    let row = messages::get(c, a1, m).unwrap().unwrap();
    assert_eq!(row.body_state, "full");
    assert!(row.has_attachments);
    assert_eq!(row.snippet, "hello");
    let hits: i64 = c
        .query_row(
            "SELECT count(*) FROM message_fts WHERE message_fts MATCH 'zebra'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(hits, 1);

    // Storing again replaces, it does not add.
    assert!(reading::store_body(c, a1, m, &body(&[pdf()], "zebra again")).unwrap());
    assert_eq!(reading::attachments(c, a1, m).unwrap().len(), 1);
}

#[test]
fn attachments_are_scoped_by_account_and_message() {
    let db = Db::open_in_memory().unwrap();
    let c = db.connection();
    let (a1, a2) = (account(c, "one@example.org"), account(c, "two@example.org"));
    let f1 = folder(c, a1, "INBOX", SpecialUse::Inbox);
    let m = message(c, a1, f1, 1, "m1@example.org", &[], 10);
    let other = message(c, a1, f1, 2, "m2@example.org", &[], 11);
    let inline = NewAttachment {
        part_id: "2".into(),
        file_name: "logo.png".into(),
        mime_type: "image/png".into(),
        size: 9,
        content_id: Some("logo@example.org".into()),
        shown_inline: true,
        executable: false,
    };
    reading::store_body(c, a1, m, &body(&[inline.clone(), pdf()], "x")).unwrap();
    let list = reading::attachments(c, a1, m).unwrap();
    assert_eq!(list.len(), 2);
    assert_eq!(list[0].content_id.as_deref(), Some("logo@example.org"));
    assert!(list[0].shown_inline);
    let id = list[1].id;
    assert_eq!(
        reading::attachment(c, a1, m, id)
            .unwrap()
            .unwrap()
            .file_name,
        "report.pdf"
    );
    assert!(reading::attachment(c, a2, m, id).unwrap().is_none());
    assert!(reading::attachment(c, a1, other, id).unwrap().is_none());
    assert!(reading::attachments(c, a2, m).unwrap().is_empty());

    // Only inline images: no paperclip on the row.
    reading::store_body(c, a1, other, &body(&[inline], "x")).unwrap();
    assert!(
        !messages::get(c, a1, other)
            .unwrap()
            .unwrap()
            .has_attachments
    );
}

#[test]
fn remote_content_choices_are_per_message_and_per_account() {
    let db = Db::open_in_memory().unwrap();
    let c = db.connection();
    let (a1, a2) = (account(c, "one@example.org"), account(c, "two@example.org"));
    let f1 = folder(c, a1, "INBOX", SpecialUse::Inbox);
    let m = message(c, a1, f1, 1, "m1@example.org", &[], 10);

    assert!(!reading::remote_allowed(c, a1, m).unwrap());
    assert!(!reading::set_remote_allowed(c, a2, m, true).unwrap());
    assert!(!reading::remote_allowed(c, a1, m).unwrap());
    assert!(reading::set_remote_allowed(c, a1, m, true).unwrap());
    assert!(reading::remote_allowed(c, a1, m).unwrap());
    assert!(!reading::remote_allowed(c, a2, m).unwrap());

    assert!(reading::trust_sender(c, a1, "ann@example.org", 5).unwrap());
    assert!(
        reading::trust_sender(c, a1, "ann@example.org", 6).unwrap(),
        "twice is harmless"
    );
    assert!(!reading::trust_sender(c, 999, "ann@example.org", 5).unwrap());
    assert!(reading::is_trusted(c, a1, "ann@example.org").unwrap());
    assert!(!reading::is_trusted(c, a2, "ann@example.org").unwrap());
    assert_eq!(
        reading::trusted_senders(c, a1).unwrap(),
        ["ann@example.org"]
    );
    assert!(reading::untrust_sender(c, a1, "ann@example.org").unwrap());
    assert!(!reading::is_trusted(c, a1, "ann@example.org").unwrap());

    // Removing the account removes its choices.
    reading::trust_sender(c, a2, "x@example.org", 1).unwrap();
    accounts::delete(c, a2).unwrap();
    let left: i64 = c
        .query_row("SELECT count(*) FROM trusted_sender", [], |r| r.get(0))
        .unwrap();
    assert_eq!(left, 0);
}

#[test]
fn a_conversation_spans_folders_oldest_first_without_trash_or_duplicates() {
    let db = Db::open_in_memory().unwrap();
    let c = db.connection();
    let (a1, a2) = (account(c, "one@example.org"), account(c, "two@example.org"));
    let inbox = folder(c, a1, "INBOX", SpecialUse::Inbox);
    let sent = folder(c, a1, "Sent", SpecialUse::Sent);
    let trash = folder(c, a1, "Trash", SpecialUse::Trash);
    let all = folder(c, a1, "All Mail", SpecialUse::All);
    let drafts = folder(c, a1, "Drafts", SpecialUse::Drafts);
    let other_inbox = folder(c, a2, "INBOX", SpecialUse::Inbox);

    let a = message(c, a1, inbox, 1, "a@x", &[], 100);
    let b = message(c, a1, sent, 1, "b@x", &["a@x"], 200);
    let cc = message(c, a1, inbox, 2, "c@x", &["a@x", "b@x"], 300);
    let c_copy = message(c, a1, all, 1, "c@x", &["a@x", "b@x"], 300);
    let d = message(c, a1, trash, 1, "d@x", &["a@x"], 400);
    // Only In-Reply-To C: its key is C's id, not the root's.
    let e = message(c, a1, inbox, 3, "e@x", &["c@x"], 500);
    let draft = message(c, a1, drafts, 1, "f@x", &["a@x"], 600);
    let _unrelated = message(c, a1, inbox, 4, "z@x", &[], 50);
    let foreign = message(c, a2, other_inbox, 1, "a2@x", &["a@x"], 250);

    let ids = |opened: i64| -> Vec<i64> {
        reading::conversation(c, a1, opened)
            .unwrap()
            .into_iter()
            .map(|r| r.message.id)
            .collect()
    };
    assert_eq!(
        ids(cc),
        [a, b, cc, e],
        "Sent in, Trash, Drafts and the All Mail copy out"
    );
    assert_eq!(ids(a), [a, b, cc, e]);
    assert_eq!(
        ids(e),
        [a, b, cc, e],
        "a reply without References still finds its thread"
    );
    assert_eq!(ids(d), [a, b, cc, d, e], "opened from Trash, Trash is in");
    assert_eq!(ids(draft), [a, b, cc, e, draft]);
    assert_eq!(
        ids(c_copy),
        [a, b, c_copy, e],
        "opened from All Mail, that copy is shown"
    );
    assert!(!ids(cc).contains(&foreign));
    assert!(
        reading::conversation(c, a2, cc).unwrap().is_empty(),
        "another account's id"
    );
    assert!(ids(foreign).is_empty(), "not an account-1 message");

    let rows = reading::conversation(c, a1, b).unwrap();
    assert_eq!(rows[1].folder_special_use, "sent");
}
