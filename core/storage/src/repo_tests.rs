// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Tests for the connection layer and the repositories.

// Tests may unwrap: a panic is the failure report.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::BTreeSet;

use croft_protocols::{
    FlagChange, Flags, FolderStatus, Keyword, RemoteFolder, RemoteHeader, SpecialUse,
};
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
        list: Default::default(),
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

/// Every file in `dir` with its bytes, sorted by name: a byte-for-byte picture of the directory.
fn dir_snapshot(dir: &std::path::Path) -> Vec<(String, Vec<u8>)> {
    let mut files: Vec<(String, Vec<u8>)> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| {
            let e = e.unwrap();
            (
                e.file_name().to_string_lossy().into_owned(),
                std::fs::read(e.path()).unwrap(),
            )
        })
        .collect();
    files.sort();
    files
}

/// Asserts the same file names, and the same bytes in each file except `-shm` (SQLite's shared
/// memory index: it holds read-lock marks and is rebuilt from the `-wal`, never data).
fn assert_files_unchanged(before: &[(String, Vec<u8>)], dir: &std::path::Path) {
    let after = dir_snapshot(dir);
    let names = |v: &[(String, Vec<u8>)]| v.iter().map(|(n, _)| n.clone()).collect::<Vec<_>>();
    assert_eq!(names(&after), names(before), "directory listing changed");
    for ((name, was), (_, now)) in before.iter().zip(&after) {
        if !name.ends_with("-shm") {
            assert!(
                was == now,
                "{name} changed ({} -> {} bytes)",
                was.len(),
                now.len()
            );
        }
    }
}

/// Builds `<dir>/live.db` the way a crashed app leaves it: a WAL-mode database whose latest
/// changes (including `user_version`) exist only in a non-empty `-wal` file, never checkpointed
/// into the main file. The writer connection stays open (with auto-checkpoint off) while its files
/// are copied, so the copy is a snapshot of the crash state. `with_shm` also copies the `-shm`
/// index file, which a crash leaves next to the `-wal`.
fn live_wal_snapshot(
    dir: &std::path::Path,
    key: Option<&[u8; 32]>,
    user_version: u32,
    with_shm: bool,
) -> std::path::PathBuf {
    let src = tempfile::tempdir().unwrap();
    let src_db = src.path().join("live.db");
    let writer = Connection::open(&src_db).unwrap();
    if let Some(key) = key {
        let hex: String = key.iter().map(|b| format!("{b:02x}")).collect();
        writer
            .execute_batch(&format!("PRAGMA key = \"x'{hex}'\";"))
            .unwrap();
    }
    let mode: String = writer
        .pragma_update_and_check(None, "journal_mode", "WAL", |r| r.get(0))
        .unwrap();
    assert_eq!(mode, "wal");
    writer.pragma_update(None, "wal_autocheckpoint", 0).unwrap();
    writer
        .execute_batch(
            "CREATE TABLE from_the_future (x INTEGER); INSERT INTO from_the_future VALUES (1);",
        )
        .unwrap();
    writer
        .pragma_update(None, "user_version", user_version)
        .unwrap();

    let dst = dir.join("live.db");
    std::fs::copy(&src_db, &dst).unwrap();
    std::fs::copy(src.path().join("live.db-wal"), dir.join("live.db-wal")).unwrap();
    if with_shm {
        std::fs::copy(src.path().join("live.db-shm"), dir.join("live.db-shm")).unwrap();
    }
    drop(writer);
    assert!(
        std::fs::metadata(dir.join("live.db-wal")).unwrap().len() > 0,
        "the snapshot must carry a live -wal"
    );
    dst
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

/// Finding 1: refusing a database must not checkpoint (or delete) a leftover `-wal`.
#[test]
fn a_too_new_database_with_a_live_wal_is_refused_without_touching_any_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = live_wal_snapshot(dir.path(), None, latest_version() + 1, true);
    let before = dir_snapshot(dir.path());
    assert!(before.iter().any(|(n, _)| n == "live.db-wal"));

    let err = Db::open(&path, None).unwrap_err();
    assert!(
        matches!(
            err,
            StorageError::SchemaTooNew(SchemaTooNew { database, .. })
                if database == latest_version() + 1
        ),
        "{err}"
    );
    // Main file and -wal byte for byte, and no file added or removed.
    assert_files_unchanged(&before, dir.path());
}

/// Same, when the `-shm` index is missing (a copied database): the read-only connection may have
/// to create the `-shm`, which holds no data; the main file and the `-wal` must still be intact.
#[test]
fn a_too_new_database_with_a_wal_but_no_shm_keeps_its_main_file_and_wal() {
    let dir = tempfile::tempdir().unwrap();
    let path = live_wal_snapshot(dir.path(), None, latest_version() + 1, false);
    let main = file_bytes(&path);
    let wal = file_bytes(&dir.path().join("live.db-wal"));

    let err = Db::open(&path, None).unwrap_err();
    assert!(matches!(err, StorageError::SchemaTooNew(_)), "{err}");
    assert_eq!(file_bytes(&path), main);
    assert_eq!(file_bytes(&dir.path().join("live.db-wal")), wal);
}

/// A leftover `-wal` on a database that is *not* too new is still recovered normally.
#[test]
fn a_current_database_with_a_live_wal_still_opens() {
    let dir = tempfile::tempdir().unwrap();
    let path = live_wal_snapshot(dir.path(), None, latest_version(), true);
    let db = Db::open(&path, None).unwrap();
    assert_eq!(db.schema_version().unwrap(), latest_version());
    let rows: i64 = db
        .connection()
        .query_row("SELECT count(*) FROM from_the_future", [], |r| r.get(0))
        .unwrap();
    assert_eq!(rows, 1, "data that only lived in the -wal must be kept");
}

/// Finding 2: many openers of one brand-new file at once.
#[test]
fn concurrent_opens_of_a_new_file_all_succeed_and_migrate_exactly_once() {
    const THREADS: usize = 4;
    for round in 0..20 {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("croft.db");
        let barrier = std::sync::Barrier::new(THREADS);
        let results: Vec<(usize, u32)> = std::thread::scope(|scope| {
            let handles: Vec<_> = (0..THREADS)
                .map(|_| {
                    scope.spawn(|| {
                        barrier.wait();
                        let db = Db::open(&path, None).unwrap_or_else(|e| {
                            panic!("round {round}: open failed: {e}");
                        });
                        (db.migrations_run_on_open(), db.schema_version().unwrap())
                    })
                })
                .collect();
            handles.into_iter().map(|h| h.join().unwrap()).collect()
        });
        for (_, version) in &results {
            assert_eq!(*version, latest_version(), "round {round}");
        }
        let ran: usize = results.iter().map(|(n, _)| n).sum();
        assert_eq!(
            ran,
            crate::MIGRATIONS.len(),
            "round {round}: each migration must run exactly once across all openers"
        );
    }
}

/// Finding 3: a migration that fails rolls back completely and says which step and SQLite code.
#[test]
fn a_failing_migration_rolls_back_and_leaves_the_file_at_version_0() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("croft.db");
    {
        // An existing object the migration also wants to create.
        let raw = Connection::open(&path).unwrap();
        raw.execute_batch("CREATE TABLE message_fts (x INTEGER);")
            .unwrap();
    }
    let err = Db::open(&path, None).unwrap_err();
    assert!(
        matches!(err, StorageError::Migration { version: 1, .. }),
        "{err}"
    );
    // Finding 6: the category and the SQLite code are in the text, nothing else.
    let text = err.to_string();
    assert!(text.contains("sqlite3 code"), "{text}");
    assert!(!text.contains("message_fts"), "{text}");

    let raw = Connection::open(&path).unwrap();
    let version: u32 = raw
        .pragma_query_value(None, "user_version", |r| r.get(0))
        .unwrap();
    assert_eq!(version, 0);
    let tables: Vec<String> = raw
        .prepare("SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(
        tables,
        vec!["message_fts".to_owned()],
        "no table of the failed migration may survive (no `account`)"
    );
}

/// Finding 4: a key that is refused must not leave a file behind.
#[cfg(not(feature = "sqlcipher"))]
#[test]
fn a_refused_key_creates_no_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("croft.db");
    let err = Db::open(&path, Some(&[7u8; 32])).unwrap_err();
    assert!(matches!(err, StorageError::EncryptionUnavailable), "{err}");
    assert!(!path.exists(), "no file may exist after a refused key");
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
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
        let path = dir.path().join("croft.db");
        for bad in [&[1u8; 5][..], &[1u8; 33][..], &[][..]] {
            let err = Db::open(&path, Some(bad)).unwrap_err();
            assert!(matches!(err, StorageError::InvalidKey), "{err}");
            // Finding 4: refused before the file is created.
            assert!(!path.exists(), "no file may exist after InvalidKey");
        }
    }

    /// Finding 1, wrong-key path: an encrypted database with a live `-wal` must not be
    /// checkpointed or changed by a failed open.
    #[test]
    fn wrong_key_with_a_live_wal_touches_no_file() {
        let dir = tempfile::tempdir().unwrap();
        let key = [7u8; 32];
        live_wal_snapshot(dir.path(), Some(&key), latest_version(), true);
        let path = dir.path().join("live.db");
        let before = dir_snapshot(dir.path());

        for bad in [Some(&[8u8; 32][..]), None] {
            let err = Db::open(&path, bad).unwrap_err();
            assert!(matches!(err, StorageError::WrongKeyOrNotADatabase), "{err}");
            assert_files_unchanged(&before, dir.path());
        }
    }

    /// Finding 1, too-new path with a key and a live `-wal`.
    #[test]
    fn too_new_encrypted_database_with_a_live_wal_touches_no_file() {
        let dir = tempfile::tempdir().unwrap();
        let key = [7u8; 32];
        live_wal_snapshot(dir.path(), Some(&key), latest_version() + 1, true);
        let path = dir.path().join("live.db");
        let before = dir_snapshot(dir.path());

        let err = Db::open(&path, Some(&key)).unwrap_err();
        assert!(matches!(err, StorageError::SchemaTooNew(_)), "{err}");
        assert_files_unchanged(&before, dir.path());
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

/// Each case first proves that the *raw* error text contains the marker (so the test can fail),
/// then that the crate error's `Display` and `Debug` do not.
#[test]
fn errors_do_not_leak_values() {
    const MARKER: &str = "hunter2-marker";
    let check = |raw: &dyn std::fmt::Display, err: StorageError| {
        let raw = raw.to_string();
        assert!(raw.contains(MARKER), "test is vacuous, raw error: {raw}");
        for text in [err.to_string(), format!("{err:?}")] {
            assert!(!text.contains(MARKER), "leaked: {text}");
            assert!(!text.contains("secret.person"), "leaked: {text}");
        }
    };
    let db = Db::open_in_memory().unwrap();
    let c = db.connection();

    // 1. SqlInputError: SQLite echoes the offending token and the error carries the whole SQL.
    let raw = c
        .prepare("SELECT secret.person@example.org FROM hunter2-marker WHERE")
        .unwrap_err();
    assert!(
        matches!(raw, rusqlite::Error::SqlInputError { .. }),
        "{raw:?}"
    );
    // (The full Debug of the raw error includes the SQL text.)
    assert!(format!("{raw:?}").contains(MARKER));
    assert!(format!("{raw:?}").contains("secret.person"));
    let raw_text = format!("{raw:?}");
    check(&raw_text, StorageError::from(raw));

    // 2. FromSqlConversionFailure / InvalidColumnType: a row holding the marker, read as a number.
    c.execute_batch(&format!(
        "CREATE TEMP TABLE leak (\"{MARKER}\" TEXT); INSERT INTO leak VALUES ('{MARKER}');"
    ))
    .unwrap();
    let raw = c
        .query_row(&format!("SELECT \"{MARKER}\" FROM leak"), [], |r| {
            r.get::<_, i64>(0)
        })
        .unwrap_err();
    assert!(
        matches!(
            raw,
            rusqlite::Error::InvalidColumnType(..) | rusqlite::Error::FromSqlConversionFailure(..)
        ),
        "{raw:?}"
    );
    let raw_text = raw.to_string();
    check(&raw_text, StorageError::from(raw));

    // 3. SqliteFailure whose message quotes an identifier ("UNIQUE constraint failed: <table>.<col>").
    c.execute_batch(
        "CREATE TEMP TABLE \"hunter2-marker\" (v TEXT UNIQUE); INSERT INTO \"hunter2-marker\" VALUES ('x');",
    )
    .unwrap();
    let raw = c
        .execute("INSERT INTO \"hunter2-marker\" VALUES ('x')", [])
        .unwrap_err();
    let raw_text = raw.to_string();
    check(&raw_text, StorageError::from(raw));

    // 4. The original check: a CHECK violation on `account` with a personal address in the row.
    // SQLite's text only names the constraint here, so there is no vacuity guard: the values must
    // simply not show up in the crate error.
    let raw = c
        .execute(
            "INSERT INTO account (label,email,protocol,config_json,created_at) VALUES ('x','secret.person@example.org','hunter2-marker','{}',0)",
            [],
        )
        .unwrap_err();
    let err = StorageError::from(raw);
    for text in [err.to_string(), format!("{err:?}")] {
        assert!(
            !text.contains(MARKER) && !text.contains("secret.person"),
            "{text}"
        );
    }

    // 5. A JSON error whose serde text quotes the value: `invalid type: string "..."`.
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(&format!(
        "CREATE TABLE setting (key TEXT, value_json TEXT, updated_at INTEGER); INSERT INTO setting VALUES ('k', '\"{MARKER}\"', 0);"
    ))
    .unwrap();
    let raw = serde_json::from_str::<u32>(&format!("\"{MARKER}\"")).unwrap_err();
    let err = settings::get::<u32>(&conn, "k").unwrap_err();
    check(&raw, err);
    // Syntax errors too (the original check).
    let bad = settings::get::<serde_json::Value>(
        &{
            let c = Connection::open_in_memory().unwrap();
            c.execute_batch("CREATE TABLE setting (key TEXT, value_json TEXT, updated_at INTEGER); INSERT INTO setting VALUES ('k', '{\"password\": hunter2', 0);").unwrap();
            c
        },
        "k",
    )
    .unwrap_err();
    assert!(!bad.to_string().contains("hunter2"));
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
fn account_settings_are_replaced_in_place() {
    let (db, a1, f1, a2, _f2) = two_accounts();
    let c = db.connection();
    let message = messages::upsert(c, a1, f1, &header(1, "Hello", 10)).unwrap();
    let before = accounts::get(c, a1).unwrap().unwrap();
    let config = serde_json::json!({ "port": 143, "pinned": "AA" });
    assert!(accounts::set_settings(c, a1, "gmail", &config).unwrap());
    let after = accounts::get(c, a1).unwrap().unwrap();
    assert_eq!((after.provider.as_str(), &after.config), ("gmail", &config));
    // Only the settings changed: the id, the address, the position and the account's rows stay.
    assert_eq!(
        (
            after.id,
            &after.email,
            &after.label,
            after.position,
            after.created_at
        ),
        (
            before.id,
            &before.email,
            &before.label,
            before.position,
            before.created_at
        )
    );
    assert!(messages::get(c, a1, message).unwrap().is_some());
    // Another account is untouched, and a missing one is reported.
    assert_eq!(accounts::get(c, a2).unwrap().unwrap().config["port"], 993);
    assert!(!accounts::set_settings(c, 999, "generic", &config).unwrap());
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
///
/// This is a text scan and only a cheap tripwire: a statement can mention `account_id` and still be
/// wrong. The behavioural cross-account tests above (`account_2_cannot_list_read_update_or_delete_
/// account_1_messages`, `folders_of_another_account_are_invisible_and_undeletable`) are the real
/// guard.
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

// ---- helpers for the sync engine (T04) ----------------------------------------------------------

#[test]
fn folders_are_found_by_path_only_in_their_account() {
    let (db, a1, f1, a2, f2) = two_accounts();
    let c = db.connection();
    assert_eq!(
        folders::find_by_path(c, a1, "INBOX").unwrap().map(|f| f.id),
        Some(f1)
    );
    assert_eq!(
        folders::find_by_path(c, a2, "INBOX").unwrap().map(|f| f.id),
        Some(f2)
    );
    assert!(folders::find_by_path(c, a1, "Archive").unwrap().is_none());
}

#[test]
fn folder_flags_and_find_by_uid_read_hidden_rows_too() {
    let (db, a1, f1, a2, _f2) = two_accounts();
    let c = db.connection();
    let mut h = header(3, "Hello", 10);
    h.flags.seen = true;
    h.flags.keywords.insert("$Work".into());
    let id = messages::upsert(c, a1, f1, &h).unwrap();
    messages::upsert(c, a1, f1, &header(4, "Other", 20)).unwrap();
    c.execute("UPDATE message SET local_hidden = 1 WHERE id = ?1", [id])
        .unwrap();

    let flags = messages::folder_flags(c, a1, f1).unwrap();
    assert_eq!(flags.keys().copied().collect::<Vec<_>>(), vec![3, 4]);
    assert!(flags[&3].seen && flags[&3].keywords.contains("$Work"));
    assert!(messages::folder_flags(c, a2, f1).unwrap().is_empty());

    let m = messages::find_by_uid(c, a1, f1, 3).unwrap().unwrap();
    assert_eq!((m.id, m.local_hidden), (id, true));
    assert!(messages::find_by_uid(c, a2, f1, 3).unwrap().is_none());
    assert!(messages::find_by_uid(c, a1, f1, 99).unwrap().is_none());
}

#[test]
fn set_server_flags_keeps_local_seen_and_flagged_while_pending() {
    let (db, a1, f1, a2, _f2) = two_accounts();
    let c = db.connection();
    let id = messages::upsert(c, a1, f1, &header(1, "Hello", 10)).unwrap();
    let server = Flags {
        seen: true,
        flagged: true,
        answered: true,
        ..Flags::default()
    };
    assert!(!messages::set_server_flags(c, a2, f1, 1, &server).unwrap());
    c.execute("UPDATE message SET pending_ops = 1 WHERE id = ?1", [id])
        .unwrap();
    assert!(messages::set_server_flags(c, a1, f1, 1, &server).unwrap());
    let m = messages::get(c, a1, id).unwrap().unwrap();
    assert!(!m.is_seen && !m.is_flagged && m.is_answered);
    c.execute("UPDATE message SET pending_ops = 0 WHERE id = ?1", [id])
        .unwrap();
    messages::set_server_flags(c, a1, f1, 1, &server).unwrap();
    let m = messages::get(c, a1, id).unwrap().unwrap();
    assert!(m.is_seen && m.is_flagged);
    assert_eq!(folders::get(c, a1, f1).unwrap().unwrap().unread_count, 0);
}

#[test]
fn delete_uids_and_delete_all_stay_in_the_account_and_folder() {
    let (db, a1, f1, a2, f2) = two_accounts();
    let c = db.connection();
    for uid in 1..=3 {
        messages::upsert(c, a1, f1, &header(uid, "x", 10)).unwrap();
        messages::upsert(c, a2, f2, &header(uid, "x", 10)).unwrap();
    }
    assert_eq!(messages::delete_uids(c, a2, f1, &[1]).unwrap(), 0);
    assert_eq!(messages::delete_uids(c, a1, f1, &[1, 9]).unwrap(), 1);
    assert_eq!(messages::folder_flags(c, a1, f1).unwrap().len(), 2);
    assert_eq!(messages::delete_all_in_folder(c, a2, f1).unwrap(), 0);
    assert_eq!(messages::delete_all_in_folder(c, a1, f1).unwrap(), 2);
    assert_eq!(folders::get(c, a1, f1).unwrap().unwrap().total_count, 0);
    assert_eq!(messages::folder_flags(c, a2, f2).unwrap().len(), 3);
}

#[test]
fn release_local_state_clears_stuck_rows_except_the_ones_kept() {
    let (db, a1, f1, a2, f2) = two_accounts();
    let c = db.connection();
    for uid in 1..=3 {
        messages::upsert(c, a1, f1, &header(uid, "x", 10)).unwrap();
    }
    messages::upsert(c, a2, f2, &header(1, "x", 10)).unwrap();
    c.execute(
        "UPDATE message SET local_hidden = 1, pending_ops = 2 WHERE uid IN (1, 2)",
        [],
    )
    .unwrap();
    assert_eq!(
        folders::get(c, a1, f1).unwrap().unwrap().total_count,
        1,
        "hidden rows are not counted"
    );
    let keep: BTreeSet<u32> = [2].into_iter().collect();
    assert_eq!(messages::release_local_state(c, a1, f1, &keep).unwrap(), 1);
    let row = |uid| messages::find_by_uid(c, a1, f1, uid).unwrap().unwrap();
    assert_eq!((row(1).local_hidden, row(1).pending_ops), (false, 0));
    assert_eq!((row(2).local_hidden, row(2).pending_ops), (true, 2));
    assert_eq!(folders::get(c, a1, f1).unwrap().unwrap().total_count, 2);
    let other = messages::find_by_uid(c, a2, f2, 1).unwrap().unwrap();
    assert_eq!(
        (other.local_hidden, other.pending_ops),
        (true, 2),
        "another account is untouched"
    );
}

#[test]
fn a_reset_stores_the_validity_alone_and_the_row_helpers_count_what_they_say() {
    let (db, a1, f1, _, _) = two_accounts();
    let c = db.connection();
    let status = FolderStatus {
        uid_validity: 7,
        uid_next: 10,
        highest_modseq: Some(5),
        exists: 3,
    };
    assert!(folders::update_status(c, a1, f1, &status, 5).unwrap());
    assert!(folders::set_validity_only(c, a1, f1, 8).unwrap());
    let f = folders::get(c, a1, f1).unwrap().unwrap();
    assert_eq!(
        (
            f.uid_validity,
            f.uid_next,
            f.highest_modseq,
            f.last_synced_at
        ),
        (Some(8), None, None, Some(5))
    );
    assert!(!folders::set_validity_only(c, a1 + 100, f1, 9).unwrap());

    for uid in [3, 1, 2] {
        let mut h = header(uid, "s", 1_000);
        h.message_id = Some(if uid == 3 { "<other@x>" } else { "<dup@x>" }.into());
        messages::upsert(c, a1, f1, &h).unwrap();
    }
    assert_eq!(messages::count_in_folder(c, a1, f1).unwrap(), 3);
    assert_eq!(messages::uids_in_folder(c, a1, f1).unwrap(), vec![1, 2, 3]);
    assert_eq!(
        messages::count_other_copies(c, a1, f1, "<dup@x>", 1).unwrap(),
        1
    );
    assert_eq!(
        messages::count_other_copies(c, a1, f1, "<dup@x>", 3).unwrap(),
        2
    );
    assert_eq!(
        messages::count_other_copies(c, a1, f1, "<nothing@x>", 3).unwrap(),
        0
    );
    assert_eq!(messages::count_in_folder(c, a1 + 100, f1).unwrap(), 0);
    assert_eq!(
        messages::uids_with_message_ids(c, a1, f1, &["<dup@x>", "<nothing@x>"])
            .unwrap()
            .into_iter()
            .collect::<Vec<_>>(),
        vec![1, 2]
    );
    assert!(
        messages::uids_with_message_ids(c, a1 + 100, f1, &["<dup@x>"])
            .unwrap()
            .is_empty()
    );
}

// ---- T35: inbox categories ----

fn bulk_header(uid: u32, from: &str) -> RemoteHeader {
    let mut h = header(uid, "Big sale", 1_000 + i64::from(uid));
    h.from_addr = Some(from.into());
    h.list.unsubscribe = Some("<https://shop.example/u>".into());
    h.list.one_click = true;
    h
}

fn category_of(c: &Connection, id: i64) -> String {
    c.query_row("SELECT category FROM message WHERE id = ?1", [id], |r| {
        r.get(0)
    })
    .unwrap()
}

#[test]
fn a_new_message_is_classified_once_and_a_sync_keeps_the_persons_choice() {
    let (db, a1, f1, _, _) = two_accounts();
    let c = db.connection();
    let id = messages::upsert(c, a1, f1, &bulk_header(1, "deals@shop.example")).unwrap();
    assert_eq!(category_of(c, id), "promotions");
    let (unsub, one_click): (Option<String>, bool) = c
        .query_row(
            "SELECT list_unsubscribe, list_one_click FROM message WHERE id = ?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(unsub.as_deref(), Some("<https://shop.example/u>"));
    assert!(one_click);
    // Moved to Primary by the person; the next sync of the same message keeps that.
    assert!(
        messages::set_category(c, a1, id, croft_mail_core::category::Category::Primary).unwrap()
    );
    messages::upsert(c, a1, f1, &bulk_header(1, "deals@shop.example")).unwrap();
    assert_eq!(category_of(c, id), "primary");
}

#[test]
fn a_rule_refiles_the_senders_mail_in_its_account_only_and_classifies_new_mail() {
    use croft_mail_core::category::Category;
    let (db, a1, f1, a2, f2) = two_accounts();
    let c = db.connection();
    let mine = messages::upsert(c, a1, f1, &bulk_header(1, "Deals@Shop.Example")).unwrap();
    let theirs = messages::upsert(c, a2, f2, &bulk_header(1, "deals@shop.example")).unwrap();
    assert_eq!(
        messages::add_category_rule(c, a1, "deals@shop.example", Category::Primary).unwrap(),
        1
    );
    assert_eq!(category_of(c, mine), "primary");
    assert_eq!(
        category_of(c, theirs),
        "promotions",
        "another account is untouched"
    );
    let next = messages::upsert(c, a1, f1, &bulk_header(2, "deals@shop.example")).unwrap();
    assert_eq!(category_of(c, next), "primary", "new mail follows the rule");
    assert_eq!(messages::category_rules(c, a2).unwrap(), vec![]);
    // Deleting the account deletes its rules.
    c.execute("DELETE FROM account WHERE id = ?1", [a1])
        .unwrap();
    assert_eq!(messages::category_rules(c, a1).unwrap(), vec![]);
}

#[test]
fn unread_counts_per_category_leave_out_read_hidden_and_snoozed_mail() {
    use croft_mail_core::category::Category;
    let (db, a1, f1, _, _) = two_accounts();
    let c = db.connection();
    messages::upsert(c, a1, f1, &bulk_header(1, "deals@shop.example")).unwrap();
    let read = messages::upsert(c, a1, f1, &bulk_header(2, "deals@shop.example")).unwrap();
    let snoozed = messages::upsert(c, a1, f1, &bulk_header(3, "deals@shop.example")).unwrap();
    messages::upsert(c, a1, f1, &header(4, "Hello", 4)).unwrap();
    c.execute("UPDATE message SET is_seen = 1 WHERE id = ?1", [read])
        .unwrap();
    c.execute(
        "UPDATE message SET snoozed_until = 10000 WHERE id = ?1",
        [snoozed],
    )
    .unwrap();
    let counts = messages::unread_by_category(c, a1, f1, 5_000).unwrap();
    assert_eq!(
        counts,
        vec![
            (Category::Primary, 1),
            (Category::Updates, 0),
            (Category::Promotions, 1),
            (Category::Social, 0)
        ]
    );
}

// ---- T36: snooze on this device ----

#[test]
fn a_snooze_hides_until_its_time_then_wakes_once_with_its_mark_and_stays_in_its_account() {
    let (db, a1, f1, a2, f2) = two_accounts();
    let c = db.connection();
    let m1 = messages::upsert(c, a1, f1, &header(1, "Later", 1_000)).unwrap();
    let m2 = messages::upsert(c, a2, f2, &header(1, "Other", 1_000)).unwrap();
    assert!(messages::set_snooze(c, a1, m1, Some(10_000)).unwrap());
    assert!(
        !messages::set_snooze(c, a2, m1, Some(10_000)).unwrap(),
        "not that account's message"
    );
    messages::set_snooze(c, a2, m2, Some(20_000)).unwrap();
    assert!(
        messages::list_by_folder(c, a1, f1, 5_000, 50, 0)
            .unwrap()
            .is_empty(),
        "hidden until 10 000"
    );
    assert_eq!(messages::next_wake(c, a1).unwrap(), Some(10_000));
    assert!(
        messages::due_snoozes(c, a1, 9_999).unwrap().is_empty(),
        "never early"
    );
    assert_eq!(messages::due_snoozes(c, a1, 10_000).unwrap(), vec![m1]);
    assert!(messages::wake(c, a1, m1, 10_000).unwrap());
    assert!(!messages::wake(c, a1, m1, 10_001).unwrap(), "once");
    let back: Option<i64> = c
        .query_row(
            "SELECT snooze_back_at FROM message WHERE id = ?1",
            [m1],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(back, Some(10_000));
    assert_eq!(
        messages::list_by_folder(c, a1, f1, 10_000, 50, 0)
            .unwrap()
            .len(),
        1
    );
    assert_eq!(messages::next_wake(c, a1).unwrap(), None);
    assert_eq!(
        messages::next_wake(c, a2).unwrap(),
        Some(20_000),
        "the other account keeps its own"
    );
    // Snoozing again clears the mark; unsnoozing brings it back at once.
    messages::set_snooze(c, a1, m1, Some(30_000)).unwrap();
    let back: Option<i64> = c
        .query_row(
            "SELECT snooze_back_at FROM message WHERE id = ?1",
            [m1],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(back, None);
    messages::set_snooze(c, a1, m1, None).unwrap();
    assert_eq!(
        messages::list_by_folder(c, a1, f1, 11_000, 50, 0)
            .unwrap()
            .len(),
        1
    );
}

/// The pin and follow-up columns of a message: (is_pinned, follows_up, keywords_json).
fn marks(c: &Connection, id: i64) -> (bool, bool, String) {
    c.query_row(
        "SELECT is_pinned, follows_up, keywords_json FROM message WHERE id = ?1",
        [id],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )
    .unwrap()
}

#[test]
fn pin_and_follow_up_columns_follow_the_keywords_in_their_account_and_a_sync_keeps_a_queued_change()
{
    let (db, a1, f1, a2, _) = two_accounts();
    let c = db.connection();
    let mut h = header(1, "Keep", 1_000);
    h.flags.keywords = BTreeSet::from(["$pinned".to_owned(), "$Work".to_owned()]);
    let m = messages::upsert(c, a1, f1, &h).unwrap();
    assert_eq!(
        marks(c, m),
        (true, false, r#"["$Work","$pinned"]"#.to_owned()),
        "a pin from the server, in any case"
    );

    assert!(
        messages::update_flags(c, a1, m, FlagChange::Keyword(Keyword::FollowUp, true)).unwrap()
    );
    assert!(
        !messages::update_flags(c, a2, m, FlagChange::Keyword(Keyword::FollowUp, false)).unwrap(),
        "not that account's message"
    );
    assert_eq!(
        marks(c, m),
        (true, true, r#"["$FollowUp","$Work","$pinned"]"#.to_owned())
    );
    messages::update_flags(c, a1, m, FlagChange::Keyword(Keyword::Pinned, false)).unwrap();
    assert_eq!(
        marks(c, m),
        (false, true, r#"["$FollowUp","$Work"]"#.to_owned()),
        "every spelling of the keyword goes"
    );

    // While a change waits in the queue, a sync that has not seen it yet does not undo it.
    c.execute("UPDATE message SET pending_ops = 1 WHERE id = ?1", [m])
        .unwrap();
    messages::upsert(c, a1, f1, &header(1, "Keep", 1_000)).unwrap();
    assert_eq!(
        marks(c, m),
        (false, true, r#"["$FollowUp","$Work"]"#.to_owned())
    );
    // Once it is done, the server's state is taken.
    c.execute("UPDATE message SET pending_ops = 0 WHERE id = ?1", [m])
        .unwrap();
    messages::upsert(c, a1, f1, &header(1, "Keep", 1_000)).unwrap();
    assert_eq!(marks(c, m), (false, false, "[]".to_owned()));
}
