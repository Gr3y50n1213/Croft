#![allow(clippy::unwrap_used, clippy::expect_used)]

use super::*;
use crate::accounts::{self, NewAccount, Protocol};
use crate::Db;

fn account(db: &Db, email: &str) -> i64 {
    accounts::insert(
        db.connection(),
        &NewAccount {
            label: email.into(),
            email: email.into(),
            protocol: Protocol::Imap,
            provider: "generic".into(),
            config: serde_json::json!({}),
            color: None,
            position: 0,
        },
        0,
    )
    .unwrap()
}

fn content(subject: &str) -> DraftContent {
    DraftContent {
        to: vec![Address {
            name: Some("Bob".into()),
            address: "bob@example.org".into(),
        }],
        cc: vec![Address::bare("cc@example.org")],
        bcc: vec![Address::bare("bcc@example.org")],
        subject: subject.into(),
        body: "Hello".into(),
        in_reply_to: Some("<p@x>".into()),
        references: vec!["<r@x>".into(), "<p@x>".into()],
        ..DraftContent::default()
    }
}

#[test]
fn a_draft_round_trips_and_its_revision_goes_up_with_each_change() {
    let db = Db::open_in_memory().unwrap();
    let a = account(&db, "a@example.org");
    let c = db.connection();
    let id = insert(c, a, "<m1@x>", &content("One"), 10).unwrap();
    let d = get(c, a, id).unwrap().unwrap();
    assert_eq!(d.content, content("One"));
    assert_eq!(d.message_id, "<m1@x>");
    assert_eq!(
        (d.state, d.revision, d.synced_revision),
        (DraftState::Draft, 1, 0)
    );
    assert!(update(c, a, id, &content("Two"), 20).unwrap());
    let d = get(c, a, id).unwrap().unwrap();
    assert_eq!(d.content.subject, "Two");
    assert_eq!(d.message_id, "<m1@x>", "the Message-ID never changes");
    assert_eq!((d.revision, d.updated_at, d.created_at), (2, 20, 10));
}

#[test]
fn another_accounts_draft_cannot_be_read_changed_or_removed() {
    let db = Db::open_in_memory().unwrap();
    let (a, b) = (account(&db, "a@example.org"), account(&db, "b@example.org"));
    let c = db.connection();
    let id = insert(c, a, "<m@x>", &content("Mine"), 0).unwrap();
    assert!(get(c, b, id).unwrap().is_none());
    assert!(!update(c, b, id, &content("Stolen"), 1).unwrap());
    assert!(!set_queued(c, b, id, 5).unwrap());
    assert!(!mark_done(c, b, id, 1).unwrap());
    assert!(!delete(c, b, id).unwrap());
    assert!(add_attachment(c, b, id, "x", "text/plain", b"x", 1).is_err());
    assert!(list(c, b).unwrap().is_empty());
    let d = get(c, a, id).unwrap().unwrap();
    assert_eq!(
        (d.content.subject.as_str(), d.state),
        ("Mine", DraftState::Draft)
    );
}

#[test]
fn a_queued_draft_is_not_edited_until_it_is_made_editable_again() {
    let db = Db::open_in_memory().unwrap();
    let a = account(&db, "a@example.org");
    let c = db.connection();
    let id = insert(c, a, "<m@x>", &content("One"), 0).unwrap();
    assert!(set_queued(c, a, id, 7).unwrap());
    let d = get(c, a, id).unwrap().unwrap();
    assert_eq!((d.state, d.outbox_id), (DraftState::Queued, Some(7)));
    assert!(!update(c, a, id, &content("Nope"), 1).unwrap());
    assert!(add_attachment(c, a, id, "x", "text/plain", b"x", 1).is_err());
    assert!(set_editable(c, a, id, 5).unwrap());
    assert!(!set_editable(c, a, id, 6).unwrap(), "only a queued draft");
    let d = get(c, a, id).unwrap().unwrap();
    assert_eq!(
        (d.state, d.outbox_id, d.revision),
        (DraftState::Draft, None, 2)
    );
    assert!(update(c, a, id, &content("Yes"), 7).unwrap());
}

#[test]
fn a_finished_draft_is_hidden_from_the_list_and_loses_its_attachments() {
    let db = Db::open_in_memory().unwrap();
    let a = account(&db, "a@example.org");
    let c = db.connection();
    let id = insert(c, a, "<m@x>", &content("One"), 0).unwrap();
    add_attachment(c, a, id, "f.txt", "text/plain", b"abc", 1).unwrap();
    assert_eq!(list(c, a).unwrap().len(), 1);
    assert!(mark_done(c, a, id, 2).unwrap());
    assert!(list(c, a).unwrap().is_empty());
    assert_eq!(in_state(c, a, DraftState::Done).unwrap().len(), 1);
    assert!(attachments(c, a, id).unwrap().is_empty());
    assert!(!update(c, a, id, &content("x"), 3).unwrap());
    assert_eq!(queued_accounts(c).unwrap(), vec![a]);
    assert!(delete(c, a, id).unwrap());
    assert!(get(c, a, id).unwrap().is_none());
    assert!(queued_accounts(c).unwrap().is_empty());
}

#[test]
fn ids_are_never_reused() {
    let db = Db::open_in_memory().unwrap();
    let a = account(&db, "a@example.org");
    let c = db.connection();
    let first = insert(c, a, "<1@x>", &content("1"), 0).unwrap();
    delete(c, a, first).unwrap();
    let second = insert(c, a, "<2@x>", &content("2"), 0).unwrap();
    assert!(second > first);
}

#[test]
fn attachments_are_stored_counted_and_removed_with_a_new_revision() {
    let db = Db::open_in_memory().unwrap();
    let a = account(&db, "a@example.org");
    let c = db.connection();
    let id = insert(c, a, "<m@x>", &content("One"), 0).unwrap();
    let f1 = add_attachment(c, a, id, "a.bin", "application/octet-stream", &[1, 2, 3], 1).unwrap();
    let f2 = add_attachment(c, a, id, "b.bin", "text/plain", &[4, 5], 2).unwrap();
    assert_eq!(get(c, a, id).unwrap().unwrap().revision, 3);
    assert_eq!(attachments_total(c, a, id).unwrap(), 5);
    let all = attachments(c, a, id).unwrap();
    assert_eq!(all.iter().map(|x| x.size).collect::<Vec<_>>(), [3, 2]);
    assert_eq!(attachment_data(c, a, id).unwrap()[0].2, vec![1, 2, 3]);
    assert!(remove_attachment(c, a, id, f1, 3).unwrap());
    assert!(!remove_attachment(c, a, id, f1, 3).unwrap());
    assert_eq!(get(c, a, id).unwrap().unwrap().revision, 4);
    assert_eq!(attachments(c, a, id).unwrap()[0].id, f2);
    // Another account's draft id with this attachment id removes nothing.
    let b = account(&db, "b@example.org");
    assert!(!remove_attachment(c, b, id, f2, 4).unwrap());
    assert_eq!(attachments(c, a, id).unwrap().len(), 1);
}

#[test]
fn the_server_copy_record_only_moves_forward() {
    let db = Db::open_in_memory().unwrap();
    let a = account(&db, "a@example.org");
    let c = db.connection();
    let id = insert(c, a, "<m@x>", &content("One"), 0).unwrap();
    assert!(record_server_copy(c, a, id, "Drafts", Some(9), 5).unwrap());
    assert!(record_server_copy(c, a, id, "Drafts", Some(10), 3).unwrap());
    let d = get(c, a, id).unwrap().unwrap();
    assert_eq!(d.synced_revision, 5);
    assert_eq!(
        (d.server_folder.as_deref(), d.server_uid),
        (Some("Drafts"), Some(10))
    );
    assert!(clear_server_copy(c, a, id).unwrap());
    let d = get(c, a, id).unwrap().unwrap();
    assert_eq!((d.server_folder, d.server_uid), (None, None));
}

#[test]
fn removing_the_account_removes_its_drafts_and_attachments() {
    let db = Db::open_in_memory().unwrap();
    let a = account(&db, "a@example.org");
    let c = db.connection();
    let id = insert(c, a, "<m@x>", &content("One"), 0).unwrap();
    add_attachment(c, a, id, "a", "text/plain", b"x", 1).unwrap();
    accounts::delete(c, a).unwrap();
    let n: i64 = c
        .query_row(
            "SELECT (SELECT count(*) FROM draft) + (SELECT count(*) FROM draft_attachment)",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(n, 0);
}

#[test]
fn debug_output_shows_no_text() {
    let d = format!("{:?}", content("SECRET-SUBJECT"));
    assert!(!d.contains("SECRET") && !d.contains("bob@"));
}

#[test]
fn taking_a_tried_message_back_gives_the_draft_a_new_message_id_and_remembers_the_old_one() {
    let db = Db::open_in_memory().unwrap();
    let a = account(&db, "a@example.org");
    let c = db.connection();
    let id = insert(c, a, "<old@x>", &content("One"), 0).unwrap();
    record_server_copy(c, a, id, "Drafts", Some(3), 1).unwrap();
    // Not queued: nothing changes.
    assert!(!unlink_outbox_with_new_id(c, a, id, "<new@x>").unwrap());
    set_queued(c, a, id, 5).unwrap();
    assert!(unlink_outbox_with_new_id(c, a, id, "<new@x>").unwrap());
    let d = get(c, a, id).unwrap().unwrap();
    assert_eq!(d.message_id, "<new@x>");
    assert_eq!(d.stale_message_ids, ["<old@x>"]);
    assert_eq!((d.outbox_id, d.synced_revision), (None, 0));
    assert_eq!(d.server_folder.as_deref(), Some("Drafts"));
    // A second round keeps both, and the other account cannot clear them.
    assert!(set_editable(c, a, id, 9).unwrap());
    set_queued(c, a, id, 6).unwrap();
    assert!(unlink_outbox_with_new_id(c, a, id, "<newer@x>").unwrap());
    assert_eq!(
        get(c, a, id).unwrap().unwrap().stale_message_ids,
        ["<old@x>", "<new@x>"]
    );
    let b = account(&db, "b@example.org");
    clear_stale(c, b, id, &["<old@x>".to_owned()]).unwrap();
    assert_eq!(get(c, a, id).unwrap().unwrap().stale_message_ids.len(), 2);
    clear_stale(c, a, id, &["<old@x>".to_owned()]).unwrap();
    assert_eq!(
        get(c, a, id).unwrap().unwrap().stale_message_ids,
        ["<new@x>"]
    );
}

#[test]
fn a_planned_server_copy_names_the_folder_once_and_changes_no_revision() {
    let db = Db::open_in_memory().unwrap();
    let a = account(&db, "a@example.org");
    let c = db.connection();
    let id = insert(c, a, "<m@x>", &content("One"), 0).unwrap();
    assert!(plan_server_copy(c, a, id, "Drafts").unwrap());
    assert!(!plan_server_copy(c, a, id, "Other").unwrap());
    let d = get(c, a, id).unwrap().unwrap();
    assert_eq!(
        (d.server_folder.as_deref(), d.synced_revision, d.server_uid),
        (Some("Drafts"), 0, None)
    );
    let b = account(&db, "b@example.org");
    let other = insert(c, b, "<n@x>", &content("Two"), 0).unwrap();
    assert!(!plan_server_copy(c, a, other, "Drafts").unwrap());
}
