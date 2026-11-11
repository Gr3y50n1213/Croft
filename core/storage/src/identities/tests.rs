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

#[test]
fn an_account_gets_its_own_address_once_and_other_identities_are_kept() {
    let db = Db::open_in_memory().unwrap();
    let (a, b) = (account(&db, "a@example.org"), account(&db, "b@example.org"));
    let c = db.connection();
    let first = list_or_create(c, a, "Ann", "a@example.org").unwrap();
    assert_eq!(first.len(), 1);
    assert!(first[0].is_default);
    assert_eq!(first[0].address, "a@example.org");
    assert_eq!(
        list_or_create(c, a, "Other", "other@example.org").unwrap(),
        first
    );
    c.execute(
        "INSERT INTO identity (account_id, display_name, address, is_default) VALUES (?1, 'Work', 'w@example.org', 0)",
        [a],
    )
    .unwrap();
    let all = list_or_create(c, a, "Ann", "a@example.org").unwrap();
    assert_eq!(all.len(), 2);
    assert!(all[0].is_default);
    // The other account has its own, and cannot read this one's by id.
    let theirs = list_or_create(c, b, "Bo", "b@example.org").unwrap();
    assert_eq!(theirs.len(), 1);
    assert!(get(c, b, all[1].id).unwrap().is_none());
    assert!(get(c, a, all[1].id).unwrap().is_some());
}
