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

fn a(name: Option<&str>, address: &str) -> Address {
    Address {
        name: name.map(str::to_owned),
        address: address.to_owned(),
    }
}

fn found(db: &Db, account: i64, prefix: &str) -> Vec<String> {
    suggest(db.connection(), account, prefix, 10)
        .unwrap()
        .into_iter()
        .map(|s| s.address)
        .collect()
}

#[test]
fn matches_the_start_of_the_address_the_name_or_a_word_of_the_name() {
    let db = Db::open_in_memory().unwrap();
    let acc = account(&db, "me@example.org");
    record(
        db.connection(),
        acc,
        &[
            a(Some("John Smith"), "js@example.com"),
            a(None, "smithers@example.com"),
            a(Some("Ann Lee"), "ann@example.com"),
        ],
        1,
    )
    .unwrap();
    assert_eq!(
        found(&db, acc, "smi"),
        ["js@example.com", "smithers@example.com"]
    );
    assert_eq!(found(&db, acc, "JOH"), ["js@example.com"]);
    assert_eq!(found(&db, acc, "ann"), ["ann@example.com"]);
    assert_eq!(found(&db, acc, "lee"), ["ann@example.com"]);
    // The middle of a word or of an address is not a match.
    assert!(found(&db, acc, "mith").is_empty());
    assert!(found(&db, acc, "example").is_empty());
    assert!(found(&db, acc, "").is_empty());
    assert!(found(&db, acc, "   ").is_empty());
}

#[test]
fn wildcards_in_the_text_are_plain_characters() {
    let db = Db::open_in_memory().unwrap();
    let acc = account(&db, "me@example.org");
    record(
        db.connection(),
        acc,
        &[a(None, "abc@example.com"), a(None, "a_c@example.com")],
        1,
    )
    .unwrap();
    assert_eq!(found(&db, acc, "a_"), ["a_c@example.com"]);
    assert!(found(&db, acc, "%").is_empty());
    assert!(found(&db, acc, "\\").is_empty());
}

#[test]
fn suggestions_are_scoped_to_the_account() {
    let db = Db::open_in_memory().unwrap();
    let (one, two) = (account(&db, "a@example.org"), account(&db, "b@example.org"));
    record(db.connection(), one, &[a(None, "only-one@example.com")], 1).unwrap();
    assert_eq!(found(&db, one, "only"), ["only-one@example.com"]);
    assert!(found(&db, two, "only").is_empty());
}

#[test]
fn the_most_used_comes_first_then_the_most_recent_and_the_list_is_capped() {
    let db = Db::open_in_memory().unwrap();
    let acc = account(&db, "me@example.org");
    let c = db.connection();
    record(c, acc, &[a(None, "p1@example.com")], 1).unwrap();
    record(c, acc, &[a(None, "p2@example.com")], 2).unwrap();
    record(c, acc, &[a(None, "p3@example.com")], 3).unwrap();
    record(c, acc, &[a(None, "p1@example.com")], 4).unwrap();
    assert_eq!(
        found(&db, acc, "p"),
        ["p1@example.com", "p3@example.com", "p2@example.com"]
    );
    assert_eq!(suggest(c, acc, "p", 2).unwrap().len(), 2);
    for i in 0..60 {
        record(c, acc, &[a(None, &format!("q{i}@example.com"))], 5).unwrap();
    }
    assert_eq!(suggest(c, acc, "q", 1000).unwrap().len(), 50);
}

#[test]
fn an_address_is_kept_once_in_lower_case_and_the_newest_name_wins() {
    let db = Db::open_in_memory().unwrap();
    let acc = account(&db, "me@example.org");
    let c = db.connection();
    record(c, acc, &[a(Some("Old Name"), "Bob@Example.com")], 1).unwrap();
    record(c, acc, &[a(None, "bob@example.com")], 2).unwrap();
    assert_eq!(suggest(c, acc, "bob", 10).unwrap().len(), 1);
    assert_eq!(
        suggest(c, acc, "bob", 10).unwrap()[0].name.as_deref(),
        Some("Old Name")
    );
    record(c, acc, &[a(Some("New Name"), "bob@example.com")], 3).unwrap();
    assert_eq!(
        suggest(c, acc, "bob", 10).unwrap()[0].name.as_deref(),
        Some("New Name")
    );
}
