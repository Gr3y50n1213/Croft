// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The database key through `Core::open`. The Android build turns on the `sqlcipher` feature; the
//! default build refuses a key rather than store mail unencrypted. Both halves are tested here
//! (the encrypted half with `cargo test -p croft-ffi --features sqlcipher`).

use super::*;

/// Not a real key: 32 made-up bytes.
const KEY: [u8; 32] = [0x2a; 32];

fn open_with(db: &TempDb, key: Option<&[u8; 32]>) -> Result<Arc<Core>, CoreError> {
    Core::open(
        db.path.display().to_string(),
        key.map(|k| k.to_vec()),
        MemProvider::new(),
    )
}

#[cfg(not(feature = "sqlcipher"))]
#[test]
fn a_build_without_sqlcipher_refuses_a_key_and_creates_no_file() {
    let db = TempDb::new();
    let err = open_with(&db, Some(&KEY)).err().unwrap();
    assert_eq!(
        err,
        CoreError::storage(StorageReason::EncryptionUnavailable)
    );
    assert!(!db.path.exists());
}

#[cfg(feature = "sqlcipher")]
mod with_sqlcipher {
    use super::*;

    const OTHER_KEY: [u8; 32] = [0x2b; 32];

    /// The first 16 bytes of every plain SQLite file.
    const SQLITE_MAGIC: &[u8] = b"SQLite format 3\0";

    #[test]
    fn a_key_opens_an_encrypted_file_that_reopens_with_the_same_key() {
        let db = TempDb::new();
        {
            let core = open_with(&db, Some(&KEY)).unwrap();
            assert!(core.list_accounts().unwrap().is_empty());
        }
        let bytes = std::fs::read(&db.path).unwrap();
        assert!(bytes.len() >= 4096);
        assert!(
            !bytes.starts_with(SQLITE_MAGIC),
            "the file must not be plain SQLite"
        );
        let core = open_with(&db, Some(&KEY)).unwrap();
        assert!(core.list_accounts().unwrap().is_empty());
    }

    #[test]
    fn a_plain_sqlite_reader_cannot_read_it() {
        let db = TempDb::new();
        drop(open_with(&db, Some(&KEY)).unwrap());
        let plain = rusqlite::Connection::open(&db.path).unwrap();
        let read: Result<i64, _> =
            plain.query_row("SELECT count(*) FROM sqlite_master", [], |r| r.get(0));
        assert!(read.is_err(), "a plain reader must fail");
        // And without a key the core refuses it too.
        let err = open_with(&db, None).err().unwrap();
        assert_eq!(err, CoreError::storage(StorageReason::NotADatabase));
    }

    #[test]
    fn a_wrong_key_is_refused_and_the_file_is_left_unchanged() {
        let db = TempDb::new();
        drop(open_with(&db, Some(&KEY)).unwrap());
        let before = std::fs::read(&db.path).unwrap();
        let err = open_with(&db, Some(&OTHER_KEY)).err().unwrap();
        assert_eq!(err, CoreError::storage(StorageReason::NotADatabase));
        assert_eq!(std::fs::read(&db.path).unwrap(), before);
        // The right key still works.
        assert!(open_with(&db, Some(&KEY)).is_ok());
    }

    #[test]
    fn a_key_of_the_wrong_length_is_refused_before_a_file_exists() {
        let db = TempDb::new();
        let err = Core::open(
            db.path.display().to_string(),
            Some(vec![1, 2, 3]),
            MemProvider::new(),
        )
        .err()
        .unwrap();
        assert_eq!(err, CoreError::storage(StorageReason::InvalidKey));
        assert!(!db.path.exists());
    }

    #[test]
    fn an_account_is_stored_in_the_encrypted_file_without_its_password() {
        let db = TempDb::new();
        let provider = MemProvider::with_pending(PASSWORD);
        let core = Core::open(
            db.path.display().to_string(),
            Some(KEY.to_vec()),
            provider.clone(),
        )
        .unwrap();
        let account = block(core.add_account(
            FfiDiscovered {
                email: "alice@example.org".into(),
                imap: Some(ServerSettings {
                    host: "127.0.0.1".into(),
                    port: 1,
                    security: SecurityMode::Tls,
                    username: "alice".into(),
                }),
                smtp: None,
                jmap: None,
                source: DiscoverySource::Autoconfig,
                provider: None,
                auth_hint: AuthKind::Password,
            },
            "Alice".into(),
            None,
        ))
        .unwrap();
        assert_eq!(core.list_accounts().unwrap()[0].id, account);
        drop(core);
        let bytes = std::fs::read(&db.path).unwrap();
        assert!(!bytes
            .windows(PASSWORD.len())
            .any(|w| w == PASSWORD.as_bytes()));
    }
}
