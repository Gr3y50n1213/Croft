// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Tests for the connection layer and the repositories.

// Tests may unwrap: a panic is the failure report.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::BTreeSet;

use croft_protocols::{FlagChange, Flags, FolderStatus, RemoteFolder, RemoteHeader, SpecialUse};
use rusqlite::Connection;
use serde_json::json;

use crate::accounts::{self, NewAccount, Protocol, Status};
use crate::messages::{self, thread_key};
use crate::{folders, latest_version, settings, Db, SchemaTooNew, StorageError};

fn new_account(label: &str, email: &str) -> NewAccount {
    NewAccount {
        label: label.into(),
        email: email.into(),
        protocol: Protocol::Imap,
        provider: "generic".into(),
        config: json!({"host": "mail.example.org", "port": 993}),
        color: None,
        position: 0,
    }
}

fn inbox() -> RemoteFolder {
    RemoteFolder {
        path: "INBOX".into(),
        delimiter: Some('/'),
        special_use: SpecialUse::Inbox,
        selectable: true,
    }
}

fn header(uid: u32, subject: &str, date_ms: i64) -> RemoteHeader {
    RemoteHeader {
        uid,
        message_id: Some(format!("<m{uid}@example.org>")),
        references: vec![],
        subject: subject.into(),
        from_name: Some("Sam Rivera".into()),
        from_addr: Some("sam@example.org".into()),
        date_ms,
        size: 1200,
        flags: Flags::default(),
    }
}

/// Two accounts, each with an inbox. Returns (db, account1, folder1, account2, folder2).
fn two_accounts() -> (Db, i64, i64, i64, i64) {
    let db = Db::open_in_memory().unwrap();
    let c = db.connection();
    let a1 = accounts::insert(c, &new_account("One", "one@example.org"), 1).unwrap();
    let a2 = accounts::insert(c, &new_account("Two", "two@example.org"), 2).unwrap();
    let f1 = folders::upsert(c, a1, &inbox()).unwrap();
    let f2 = folders::upsert(c, a2, &inbox()).unwrap();
    (db, a1, f1, a2, f2)
}

fn file_bytes(path: &std::path::Path) -> Vec<u8> {
    std::fs::read(path).unwrap()
}

// ---- connection, migrations -------------------------------------------------------------------

#[test]
fn version_0_database_is_migrated_then_reopened_without_a_second_migration() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("croft.db");
    // A brand-new database file reports user_version 0.
    {
        let raw = Connection::open(&path).unwrap();
        let v: u32 = raw
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .unwrap();
        assert_eq!(v, 0);
    }

    let first = Db::open(&path, None).unwrap();
    assert_eq!(first.migrations_run_on_open(), crate::MIGRATIONS.len());
    assert_eq!(first.schema_version().unwrap(), latest_version());
    let account = accounts::insert(
        first.connection(),
        &new_account("Keep", "keep@example.org"),
        5,
    )
    .unwrap();
    drop(first);

    let second = Db::open(&path, None).unwrap();
    assert_eq!(
        second.migrations_run_on_open(),
        0,
        "no migration may run twice"
    );
    assert_eq!(second.schema_version().unwrap(), latest_version());
    // Data written before the reopen is still there (a re-run of CREATE TABLE would have failed).
    assert_eq!(
        accounts::get(second.connection(), account)
            .unwrap()
            .unwrap()
            .label,
        "Keep"
    );
}

#[test]
fn a_database_newer_than_the_app_is_refused_and_the_file_is_unchanged() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("future.db");
    {
        let raw = Connection::open(&path).unwrap();
        raw.execute_batch("CREATE TABLE from_the_future (x INTEGER);")
            .unwrap();
        raw.pragma_update(None, "user_version", latest_version() + 1)
            .unwrap();
    }
    let before = file_bytes(&path);

    let err = Db::open(&path, None).unwrap_err();
    assert!(
        matches!(
            err,
            StorageError::SchemaTooNew(SchemaTooNew { database, supported })
                if database == latest_version() + 1 && supported == latest_version()
        ),
        "{err}"
    );

    assert_eq!(
        file_bytes(&path),
        before,
        "a too-new database must not be touched"
    );
    let names: Vec<String> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(
        names,
        vec!["future.db".to_owned()],
        "no -wal/-shm/-journal files left behind"
    );
}

#[test]
fn open_enables_foreign_keys_and_wal() {
    let dir = tempfile::tempdir().unwrap();
    let db = Db::open(&dir.path().join("croft.db"), None).unwrap();
    let c = db.connection();
    let fk: i64 = c
        .pragma_query_value(None, "foreign_keys", |r| r.get(0))
        .unwrap();
    let mode: String = c
        .pragma_query_value(None, "journal_mode", |r| r.get(0))
        .unwrap();
    assert_eq!(fk, 1);
    assert_eq!(mode, "wal");
}

#[cfg(not(feature = "sqlcipher"))]
#[test]
fn a_key_is_refused_when_the_build_has_no_sqlcipher() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("croft.db");
    let err = Db::open(&path, Some(&[7u8; 32])).unwrap_err();
    assert!(matches!(err, StorageError::EncryptionUnavailable), "{err}");
}

#[cfg(feature = "sqlcipher")]
mod encrypted {
    use super::*;

    #[test]
    fn encrypted_database_round_trips_and_is_not_plain_sqlite() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("croft.db");
        let key = [7u8; 32];
        let db = Db::open(&path, Some(&key)).unwrap();
        let id =
            accounts::insert(db.connection(), &new_account("Enc", "enc@example.org"), 1).unwrap();
        drop(db);

        assert!(
            !file_bytes(&path).starts_with(b"SQLite format 3"),
            "header must be encrypted"
        );
        let again = Db::open(&path, Some(&key)).unwrap();
        assert_eq!(again.migrations_run_on_open(), 0);
        assert_eq!(
            accounts::get(again.connection(), id)
                .unwrap()
                .unwrap()
                .label,
            "Enc"
        );
    }

    #[test]
    fn wrong_key_and_missing_key_are_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("croft.db");
        drop(Db::open(&path, Some(&[7u8; 32])).unwrap());
        let before = file_bytes(&path);
        let wrong = Db::open(&path, Some(&[8u8; 32])).unwrap_err();
        assert!(
            matches!(wrong, StorageError::WrongKeyOrNotADatabase),
            "{wrong}"
        );
        let none = Db::open(&path, None).unwrap_err();
        assert!(
            matches!(none, StorageError::WrongKeyOrNotADatabase),
            "{none}"
        );
        assert_eq!(file_bytes(&path), before);
    }

    #[test]
    fn key_must_be_32_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let err = Db::open(&dir.path().join("croft.db"), Some(&[1u8; 5])).unwrap_err();
        assert!(matches!(err, StorageError::InvalidKey), "{err}");
    }
}

#[test]
fn transaction_commits_on_ok_and_rolls_back_on_err() {
    let mut db = Db::open_in_memory().unwrap();
    let id: Result<i64, StorageError> =
        db.transaction(|tx| accounts::insert(tx, &new_account("Kept", "kept@example.org"), 1));
    let id = id.unwrap();
    assert!(accounts::get(db.connection(), id).unwrap().is_some());

    let failed: Result<(), StorageError> = db.transaction(|tx| {
        let rolled = accounts::insert(tx, &new_account("Gone", "gone@example.org"), 1)?;
        assert!(accounts::get(tx, rolled)?.is_some());
        Err(StorageError::NotFound("test"))
    });
    assert!(failed.is_err());
    let labels: Vec<String> = accounts::list(db.connection())
        .unwrap()
        .into_iter()
        .map(|a| a.label)
        .collect();
    assert_eq!(labels, vec!["Kept".to_owned()]);
}

#[test]
fn errors_do_not_leak_values() {
    let db = Db::open_in_memory().unwrap();
    // Violates the protocol CHECK; the value must not appear in the error text.
    let err = db
        .connection()
        .execute(
            "INSERT INTO account (label,email,protocol,config_json,created_at) VALUES ('x','secret.person@example.org','carrier-pigeon','{}',0)",
            [],
        )
        .map_err(StorageError::from)
        .unwrap_err();
    for text in [err.to_string(), format!("{err:?}")] {
        assert!(!text.contains("secret.person"), "{text}");
        assert!(!text.contains("carrier-pigeon"), "{text}");
    }
    let bad_json = settings::get::<serde_json::Value>(
        &{
            let c = Connection::open_in_memory().unwrap();
            c.execute_batch("CREATE TABLE setting (key TEXT, value_json TEXT, updated_at INTEGER); INSERT INTO setting VALUES ('k', '{\"password\": hunter2', 0);").unwrap();
            c
        },
        "k",
    )
    .unwrap_err();
    assert!(!bad_json.to_string().contains("hunter2"));
}

// ---- accounts, settings ----------------------------------------------------------------------

#[test]
fn accounts_round_trip_list_status_and_cascade_delete() {
    let (db, a1, f1, a2, _f2) = two_accounts();
    let c = db.connection();
    let got = accounts::get(c, a1).unwrap().unwrap();
    assert_eq!(got.email, "one@example.org");
    assert_eq!(got.config["port"], 993);
    assert_eq!(got.status, Status::Ok);
    assert_eq!(accounts::list(c).unwrap().len(), 2);
    assert!(accounts::set_status(c, a1, Status::NeedsSignIn).unwrap());
    assert_eq!(
        accounts::get(c, a1).unwrap().unwrap().status,
        Status::NeedsSignIn
    );
    assert!(!accounts::set_status(c, 999, Status::Ok).unwrap());

    messages::upsert(c, a1, f1, &header(1, "Hello", 10)).unwrap();
    assert!(accounts::delete(c, a1).unwrap());
    assert!(accounts::get(c, a1).unwrap().is_none());
    assert!(folders::list(c, a1).unwrap().is_empty());
    assert_eq!(
        folders::list(c, a2).unwrap().len(),
        1,
        "other account is untouched"
    );
}

#[test]
fn settings_round_trip_overwrite_and_remove() {
    let db = Db::open_in_memory().unwrap();
    let c = db.connection();
    assert_eq!(
        settings::get::<serde_json::Value>(c, "theme").unwrap(),
        None
    );
    settings::set(c, "theme", &json!({"mode": "dark"}), 1).unwrap();
    settings::set(c, "theme", &json!({"mode": "light"}), 2).unwrap();
    let v: serde_json::Value = settings::get(c, "theme").unwrap().unwrap();
    assert_eq!(v["mode"], "light");
    assert!(settings::remove(c, "theme").unwrap());
    assert!(!settings::remove(c, "theme").unwrap());
}

// ---- folders ----------------------------------------------------------------------------------

#[test]
fn folder_upsert_is_idempotent_and_keeps_counters() {
    let (db, a1, f1, _a2, _f2) = two_accounts();
    let c = db.connection();
    messages::upsert(c, a1, f1, &header(1, "Hello", 10)).unwrap();
    let again = folders::upsert(
        c,
        a1,
        &RemoteFolder {
            delimiter: Some('.'),
            ..inbox()
        },
    )
    .unwrap();
    assert_eq!(again, f1);
    let f = folders::get(c, a1, f1).unwrap().unwrap();
    assert_eq!((f.total_count, f.unread_count), (1, 1));
    assert_eq!(f.delimiter.as_deref(), Some("."));
    assert_eq!(f.special_use, "inbox");
    assert_eq!(folders::list(c, a1).unwrap().len(), 1);
}

#[test]
fn folder_status_is_stored_per_account() {
    let (db, a1, f1, a2, _f2) = two_accounts();
    let c = db.connection();
    let status = FolderStatus {
        uid_validity: 7,
        uid_next: 42,
        highest_modseq: Some(99),
        exists: 0,
    };
    assert!(
        !folders::update_status(c, a2, f1, &status, 5).unwrap(),
        "account 2 cannot touch folder of 1"
    );
    assert!(folders::update_status(c, a1, f1, &status, 5).unwrap());
    let f = folders::get(c, a1, f1).unwrap().unwrap();
    assert_eq!(
        (
            f.uid_validity,
            f.uid_next,
            f.highest_modseq,
            f.last_synced_at
        ),
        (Some(7), Some(42), Some(99), Some(5))
    );
}

#[test]
fn folders_of_another_account_are_invisible_and_undeletable() {
    let (db, a1, f1, a2, _f2) = two_accounts();
    let c = db.connection();
    assert!(folders::get(c, a2, f1).unwrap().is_none());
    assert!(!folders::delete(c, a2, f1).unwrap());
    assert!(folders::get(c, a1, f1).unwrap().is_some());
    assert!(folders::delete(c, a1, f1).unwrap());
}

// ---- messages ---------------------------------------------------------------------------------

#[test]
fn list_is_newest_first_hides_snoozed_and_pages() {
    let (db, a1, f1, _a2, _f2) = two_accounts();
    let c = db.connection();
    for (uid, ts) in [(1, 100), (2, 300), (3, 200), (4, 400)] {
        messages::upsert(c, a1, f1, &header(uid, &format!("s{uid}"), ts)).unwrap();
    }
    let uids = |now, limit, offset| -> Vec<i64> {
        messages::list_by_folder(c, a1, f1, now, limit, offset)
            .unwrap()
            .iter()
            .map(|m| m.uid)
            .collect()
    };
    assert_eq!(uids(0, 10, 0), vec![4, 2, 3, 1]);
    assert_eq!(uids(0, 2, 1), vec![2, 3]);

    c.execute("UPDATE message SET snoozed_until = 1000 WHERE uid = 2", [])
        .unwrap();
    assert_eq!(uids(999, 10, 0), vec![4, 3, 1], "snoozed message is hidden");
    assert_eq!(
        uids(1000, 10, 0),
        vec![4, 2, 3, 1],
        "it returns when the snooze ends"
    );
}

#[test]
fn list_query_plan_uses_the_folder_date_index_without_sorting() {
    let (db, a1, f1, _a2, _f2) = two_accounts();
    let c = db.connection();
    let mut stmt = c
        .prepare(&format!("EXPLAIN QUERY PLAN {}", messages::LIST_SQL))
        .unwrap();
    let rows = stmt
        .query_map(rusqlite::params![a1, f1, 0, 10, 0], |r| {
            r.get::<_, String>(3)
        })
        .unwrap();
    let plan = rows.map(Result::unwrap).collect::<Vec<_>>().join(" ");
    assert!(
        plan.contains("message_folder_date") && !plan.contains("TEMP B-TREE"),
        "{plan}"
    );
}

#[test]
fn upsert_inserts_then_updates_by_folder_and_uid() {
    let (db, a1, f1, _a2, _f2) = two_accounts();
    let c = db.connection();
    let mut h = header(5, "First subject", 1000);
    h.flags.keywords = BTreeSet::from(["$label1".to_owned()]);
    let id = messages::upsert(c, a1, f1, &h).unwrap();
    let m = messages::get(c, a1, id).unwrap().unwrap();
    assert_eq!((m.account_id, m.folder_id, m.uid), (a1, f1, 5));
    assert_eq!(m.keywords, vec!["$label1".to_owned()]);
    assert_eq!(m.thread_key, "<m5@example.org>");
    assert!(!m.is_seen);

    h.subject = "Changed subject".into();
    h.date_ms = 2000;
    h.flags.seen = true;
    h.flags.answered = true;
    h.flags.keywords.clear();
    let same = messages::upsert(c, a1, f1, &h).unwrap();
    assert_eq!(same, id, "same (folder, uid) is the same row");
    let m = messages::get(c, a1, id).unwrap().unwrap();
    assert_eq!(
        (m.subject.as_str(), m.date_ts, m.is_seen, m.is_answered),
        ("Changed subject", 2000, true, true)
    );
    assert!(m.keywords.is_empty());
    let f = folders::get(c, a1, f1).unwrap().unwrap();
    assert_eq!(
        (f.total_count, f.unread_count),
        (1, 0),
        "triggers kept the counters right"
    );
}

#[test]
fn upsert_does_not_overwrite_seen_or_flagged_while_a_local_change_is_pending() {
    let (db, a1, f1, _a2, _f2) = two_accounts();
    let c = db.connection();
    let mut h = header(1, "Hello", 10);
    let id = messages::upsert(c, a1, f1, &h).unwrap();
    // The person marks it read and starred locally; the op is queued (pending_ops = 1).
    assert!(messages::update_flags(c, a1, id, FlagChange::Seen(true)).unwrap());
    assert!(messages::update_flags(c, a1, id, FlagChange::Flagged(true)).unwrap());
    c.execute("UPDATE message SET pending_ops = 1 WHERE id = ?1", [id])
        .unwrap();

    // The server still says unread and unflagged (and has a new subject).
    h.subject = "Hello again".into();
    messages::upsert(c, a1, f1, &h).unwrap();
    let m = messages::get(c, a1, id).unwrap().unwrap();
    assert!(
        m.is_seen && m.is_flagged,
        "local state must survive while pending_ops > 0"
    );
    assert_eq!(m.subject, "Hello again", "other headers still update");
    let f = folders::get(c, a1, f1).unwrap().unwrap();
    assert_eq!(f.unread_count, 0);

    // Once the queue has confirmed the change, the server wins again.
    c.execute("UPDATE message SET pending_ops = 0 WHERE id = ?1", [id])
        .unwrap();
    messages::upsert(c, a1, f1, &h).unwrap();
    let m = messages::get(c, a1, id).unwrap().unwrap();
    assert!(!m.is_seen && !m.is_flagged);
    assert_eq!(folders::get(c, a1, f1).unwrap().unwrap().unread_count, 1);
}

#[test]
fn upsert_keeps_snooze_category_and_snippet() {
    let (db, a1, f1, _a2, _f2) = two_accounts();
    let c = db.connection();
    let id = messages::upsert(c, a1, f1, &header(1, "Hello", 10)).unwrap();
    c.execute(
        "UPDATE message SET snoozed_until = 500, category = 'social', snippet = 'preview' WHERE id = ?1",
        [id],
    )
    .unwrap();
    messages::upsert(c, a1, f1, &header(1, "Hello", 10)).unwrap();
    let m = messages::get(c, a1, id).unwrap().unwrap();
    assert_eq!(
        (m.snoozed_until, m.category.as_str(), m.snippet.as_str()),
        (Some(500), "social", "preview")
    );
}

#[test]
fn update_flags_and_delete_work_in_the_owning_account() {
    let (db, a1, f1, _a2, _f2) = two_accounts();
    let c = db.connection();
    let id = messages::upsert(c, a1, f1, &header(1, "Hello", 10)).unwrap();
    messages::update_flags(c, a1, id, FlagChange::Seen(true)).unwrap();
    assert!(messages::get(c, a1, id).unwrap().unwrap().is_seen);
    assert_eq!(folders::get(c, a1, f1).unwrap().unwrap().unread_count, 0);
    assert!(messages::delete(c, a1, id).unwrap());
    assert!(messages::get(c, a1, id).unwrap().is_none());
    assert!(!messages::delete(c, a1, id).unwrap());
}

#[test]
fn account_2_cannot_list_read_update_or_delete_account_1_messages() {
    let (db, a1, f1, a2, f2) = two_accounts();
    let c = db.connection();
    let id = messages::upsert(c, a1, f1, &header(1, "Private", 10)).unwrap();

    // List: account 2 asking for account 1's folder id sees nothing.
    assert!(messages::list_by_folder(c, a2, f1, 0, 50, 0)
        .unwrap()
        .is_empty());
    // Read by id.
    assert!(messages::get(c, a2, id).unwrap().is_none());
    // Update.
    assert!(!messages::update_flags(c, a2, id, FlagChange::Seen(true)).unwrap());
    assert!(!messages::update_flags(c, a2, id, FlagChange::Flagged(true)).unwrap());
    // Delete.
    assert!(!messages::delete(c, a2, id).unwrap());
    // Upsert into account 1's folder as account 2 must fail and must not write.
    let err = messages::upsert(c, a2, f1, &header(2, "Injected", 20)).unwrap_err();
    assert!(matches!(err, StorageError::NotFound(_)), "{err}");
    // Upsert with a (folder, uid) that exists in account 1 must not overwrite it either.
    let err = messages::upsert(c, a2, f1, &header(1, "Overwritten", 20)).unwrap_err();
    assert!(matches!(err, StorageError::NotFound(_)), "{err}");

    let m = messages::get(c, a1, id).unwrap().unwrap();
    assert_eq!(
        (m.subject.as_str(), m.is_seen, m.is_flagged),
        ("Private", false, false)
    );
    assert_eq!(
        messages::list_by_folder(c, a1, f1, 0, 50, 0).unwrap().len(),
        1
    );
    assert!(messages::list_by_folder(c, a2, f2, 0, 50, 0)
        .unwrap()
        .is_empty());
}

// ---- thread keys ------------------------------------------------------------------------------

#[test]
fn thread_key_prefers_first_reference_then_message_id_then_subject() {
    let mut h = header(1, "Re: FWD: Fw: Lunch  ", 0);
    h.references = vec!["<root@x>".into(), "<mid@x>".into()];
    assert_eq!(thread_key(&h), "<root@x>");

    h.references.clear();
    assert_eq!(thread_key(&h), "<m1@example.org>");

    h.message_id = None;
    assert_eq!(thread_key(&h), "lunch");
    h.subject = "RE:re: Fwd:Planning MEETING".into();
    assert_eq!(thread_key(&h), "planning meeting");
    h.subject = "  Reunion notes".into();
    assert_eq!(
        thread_key(&h),
        "reunion notes",
        "only a real prefix is stripped"
    );
    h.subject = "Fwd:".into();
    assert_eq!(thread_key(&h), "");
}

#[test]
fn blank_subject_without_ids_gets_a_per_message_key() {
    let (db, a1, f1, _a2, _f2) = two_accounts();
    let c = db.connection();
    let mut a = header(1, "", 1);
    a.message_id = None;
    let mut b = header(2, "Re:", 2);
    b.message_id = None;
    let ia = messages::upsert(c, a1, f1, &a).unwrap();
    let ib = messages::upsert(c, a1, f1, &b).unwrap();
    let ka = messages::get(c, a1, ia).unwrap().unwrap().thread_key;
    let kb = messages::get(c, a1, ib).unwrap().unwrap().thread_key;
    assert_ne!(ka, kb);
}

// ---- the account_id rule ----------------------------------------------------------------------

/// Every SQL statement written as a raw string in the account-scoped repositories must mention
/// `account_id`. (`accounts.rs` and `settings.rs` are the documented exceptions.)
#[test]
fn every_query_in_folders_and_messages_filters_on_account_id() {
    for (name, src) in [
        ("folders.rs", include_str!("folders.rs")),
        ("messages.rs", include_str!("messages.rs")),
    ] {
        let code = src.split("#[cfg(test)]").next().unwrap_or(src);
        let mut statements = 0;
        for piece in code.split("r#\"").skip(1) {
            let sql = piece.split("\"#").next().unwrap_or_default();
            statements += 1;
            assert!(
                sql.contains("account_id"),
                "{name}: statement without account_id: {sql}"
            );
        }
        assert!(
            statements >= 3,
            "{name}: expected to find its SQL, found {statements}"
        );
        // Plain-string statements (single-line) must be checked too.
        for line in code.lines() {
            let t = line.trim();
            if (t.contains("\"SELECT ")
                || t.contains("\"UPDATE ")
                || t.contains("\"DELETE ")
                || t.contains("\"INSERT "))
                && !t.contains("r#\"")
                && !t.starts_with("//")
            {
                assert!(t.contains("account_id"), "{name}: {t}");
            }
        }
    }
}
