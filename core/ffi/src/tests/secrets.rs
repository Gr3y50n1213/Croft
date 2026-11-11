// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! No password may come out of the surface: not in an error, not in a returned value, not in a
//! log line, not in the database file.

use std::sync::{Mutex, OnceLock};

use super::*;

const WRONG: &str = "typed-wrong-pw-Q9-5520";

struct Capture;

fn log_lines() -> &'static Mutex<Vec<String>> {
    static LINES: OnceLock<Mutex<Vec<String>>> = OnceLock::new();
    LINES.get_or_init(Mutex::default)
}

impl log::Log for Capture {
    fn enabled(&self, _: &log::Metadata<'_>) -> bool {
        true
    }
    fn log(&self, record: &log::Record<'_>) {
        log_lines()
            .lock()
            .unwrap()
            .push(format!("{}", record.args()));
    }
    fn flush(&self) {}
}

fn all_core_errors() -> Vec<CoreError> {
    let mut v = vec![
        CoreError::SchemaTooNew,
        CoreError::NoSuchAccount,
        CoreError::NoSuchMessage,
        CoreError::NoSuchFolder,
        CoreError::NoArchiveFolder,
        CoreError::NoTrashFolder,
        CoreError::NoSuchOperation,
        CoreError::TooLate,
        CoreError::BadSettings,
        CoreError::UnsupportedServer,
        CoreError::BadFingerprint,
        CoreError::NoCredential,
        CoreError::Internal,
    ];
    for reason in [
        StorageReason::Database,
        StorageReason::Migration,
        StorageReason::Json,
        StorageReason::NotFound,
        StorageReason::OutOfRange,
        StorageReason::InvalidKey,
        StorageReason::NotADatabase,
        StorageReason::EncryptionUnavailable,
        StorageReason::WalUnavailable,
        StorageReason::LockPoisoned,
    ] {
        v.push(CoreError::Storage { reason });
    }
    v
}

fn all_verify_failures() -> Vec<VerifyFailure> {
    let mut v = vec![
        VerifyFailure::WrongPassword,
        VerifyFailure::ConnectionIntercepted,
        VerifyFailure::CertificateInvalid,
        VerifyFailure::CannotConnect,
        VerifyFailure::NoSecureConnection,
        VerifyFailure::ServerProblem,
        VerifyFailure::JmapOnly,
        VerifyFailure::BadSettings,
        VerifyFailure::Internal,
        VerifyFailure::UntrustedCertificate {
            fingerprint: "AA:BB".into(),
        },
        VerifyFailure::CertificateChanged {
            fingerprint: "CC:DD".into(),
        },
    ];
    for provider in [
        KnownProvider::Gmail,
        KnownProvider::Outlook,
        KnownProvider::ICloud,
        KnownProvider::Fastmail,
    ] {
        v.push(VerifyFailure::NeedsAppPasswordOrOAuth { provider });
    }
    v
}

fn all_discover_errors() -> [DiscoverError; 5] {
    [
        DiscoverError::InvalidAddress,
        DiscoverError::Offline,
        DiscoverError::NothingFound,
        DiscoverError::ResolverUnavailable,
        DiscoverError::Internal,
    ]
}

#[test]
fn no_password_in_any_error_value_returned_string_log_line_or_database_file() {
    let _ = log::set_logger(&Capture);
    log::set_max_level(log::LevelFilter::Debug);

    let mut seen: Vec<String> = Vec::new();
    let mut note = |what: &str, text: String| seen.push(format!("[{what}] {text}"));

    // Every error value, every message and key.
    for e in all_core_errors() {
        note("core error", format!("{e} {e:?}"));
    }
    for e in all_verify_failures() {
        note(
            "verify failure",
            format!(
                "{e} {e:?} {} {}",
                verify_failure_message_key(e.clone()),
                verify_failure_default_message(e.clone())
            ),
        );
    }
    for e in all_discover_errors() {
        note(
            "discover error",
            format!(
                "{e} {e:?} {} {}",
                discover_error_message_key(e),
                discover_error_default_message(e)
            ),
        );
    }
    // The credential types print nothing secret even when asked to.
    note("credential", format!("{:?}", password_cred(PASSWORD)));
    note(
        "credential",
        format!(
            "{:?}",
            FfiCredential::oauth2("alice".into(), PASSWORD.as_bytes().to_vec())
        ),
    );

    // A real run with the right and the wrong password.
    let server = start_server();
    for i in 1..=3 {
        server.add_message(
            "INBOX",
            &raw(&format!("m{i}@example.org"), &format!("Hello {i}")),
        );
    }
    let db = TempDb::new();
    let provider = MemProvider::new();
    let core = db.open(&provider);
    let found = discovered_for(&server);
    let pin = Some(fingerprint_of(&server));

    note("discovered", format!("{found:?}"));
    for (cred, pin) in [
        (password_cred(WRONG), None),
        (password_cred(WRONG), pin.clone()),
        (password_cred(PASSWORD), None),
        (
            password_cred(PASSWORD),
            Some("not a fingerprint".to_owned()),
        ),
        (
            password_cred(PASSWORD),
            Some(CertFingerprint::of_der(b"x").to_string()),
        ),
    ] {
        let result = block(core.verify(found.clone(), cred, pin));
        note("verify", format!("{result:?}"));
        if let Err(e) = result {
            note(
                "verify text",
                format!("{e} {}", verify_failure_default_message(e.clone())),
            );
        }
    }
    block(core.verify(found.clone(), password_cred(PASSWORD), pin.clone())).unwrap();

    let account =
        block(core.add_account(found, "Alice".into(), pin, None, password_cred(PASSWORD)))
            .unwrap()
            .id;
    provider.store(account, password_cred(PASSWORD));
    let rig_scope = |core: &Core| {
        let folders = core.list_folders(account).unwrap();
        let inbox = folders.iter().find(|f| f.path == "INBOX").map(|f| f.id);
        (folders, inbox)
    };
    block(wait_for(&core, "mail", || {
        rig_scope(&core).1.is_some_and(|id| {
            core.list_messages(
                MessageScope::Folder {
                    account,
                    folder_id: id,
                },
                None,
                10,
            )
            .unwrap()
            .rows
            .len()
                == 3
        })
    }));
    let (folders, inbox) = rig_scope(&core);
    let scope = MessageScope::Folder {
        account,
        folder_id: inbox.unwrap(),
    };
    note("accounts", format!("{:?}", core.list_accounts().unwrap()));
    note("folders", format!("{folders:?}"));
    let page = core.list_messages(scope, None, 10).unwrap();
    note("rows", format!("{page:?}"));
    note(
        "unified",
        format!(
            "{:?}",
            core.list_messages(MessageScope::UnifiedInbox, None, 10)
                .unwrap()
        ),
    );
    note(
        "batch",
        format!("{:?}", block(core.next_change(0)).unwrap()),
    );

    // Errors from calls that failed.
    note("action error", format!("{:?}", core.archive(vec![987_654])));
    note(
        "action error",
        format!("{:?}", core.move_to(vec![page.rows[0].id], 987_654)),
    );
    note(
        "remove error",
        format!("{:?}", block(core.remove_account(987_654))),
    );

    // A wrong-password account too, and the calls around it.
    let provider2 = MemProvider::new();
    let db2 = TempDb::new();
    let core2 = db2.open(&provider2);
    let bad = block(core2.add_account(
        discovered_for(&server),
        "Bob".into(),
        Some(fingerprint_of(&server)),
        None,
        password_cred(WRONG),
    ))
    .unwrap()
    .id;
    provider2.store(bad, password_cred(WRONG));
    block(wait_for(&core2, "sign-in needed", || {
        !provider2.sign_ins().is_empty()
    }));
    note(
        "accounts 2",
        format!("{:?}", core2.list_accounts().unwrap()),
    );
    note(
        "signed_in",
        format!("{:?}", block(core2.signed_in(987_654))),
    );
    let _ = bad;

    // The database: the settings row and the whole file (and its write-ahead log).
    let config: String = {
        let d = Db::open(&db.path, None).unwrap();
        d.connection()
            .query_row("SELECT config_json FROM account", [], |r| r.get(0))
            .unwrap()
    };
    note("config_json", config);
    drop(core);
    drop(core2);
    let mut files = 0;
    for dbfile in [&db, &db2] {
        for suffix in ["", "-wal", "-shm"] {
            let mut p = dbfile.path.clone().into_os_string();
            p.push(suffix);
            if let Ok(bytes) = std::fs::read(&p) {
                files += 1;
                let text = String::from_utf8_lossy(&bytes).into_owned();
                note(&format!("database file {suffix}"), text);
            }
        }
    }
    assert!(files >= 2);

    // Search it all, and the log lines of the whole test process.
    note("log", log_lines().lock().unwrap().join("\n"));
    for (what, text) in seen.iter().map(|s| (s.split(']').next().unwrap_or(""), s)) {
        for secret in [PASSWORD, WRONG] {
            assert!(!text.contains(secret), "secret in {what}]");
        }
    }
    // The search itself works: a string that holds the secret is caught.
    assert!(format!("x {PASSWORD} y").contains(PASSWORD));
}
