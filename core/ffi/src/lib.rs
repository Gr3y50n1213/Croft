// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The Croft FFI surface: the only Rust API the apps call.
//!
//! The surface is deliberately small and versioned. [`FFI_API_VERSION`] (exposed as
//! `ffi_api_version()`) must be bumped on every breaking change to anything exported here: a
//! removed or renamed function, object, field or variant, or a changed type. Adding things is not
//! breaking. The apps check the version at start-up and refuse to run against one they do not
//! know.
//!
//! Errors never carry row data, SQL parameters or file paths: see [`CoreError`].

use std::sync::{Arc, Mutex};

use croft_storage::{folders, Db, StorageError};

uniffi::setup_scaffolding!();

/// Version of this FFI surface. Version 2 replaced the free-text storage reason with `StorageReason`. Bump on any breaking change to what is exported.
pub const FFI_API_VERSION: u32 = 2;

/// The version of the Croft core library, for example `0.1.0`.
#[uniffi::export]
pub fn croft_version() -> String {
    env!("CARGO_PKG_VERSION").to_owned()
}

/// The version of the FFI surface itself ([`FFI_API_VERSION`]).
#[uniffi::export]
pub fn ffi_api_version() -> u32 {
    FFI_API_VERSION
}

/// What can go wrong when calling [`Core`].
#[derive(Debug, uniffi::Error)]
pub enum CoreError {
    /// The database was written by a newer version of Croft and was not modified.
    SchemaTooNew,
    /// The database could not be opened or read. `reason` is a fixed category; it never contains
    /// row data, SQL parameters or file paths.
    Storage {
        /// Fixed category.
        reason: StorageReason,
    },
}

/// Why a storage call failed. A fixed set of categories, never free text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum StorageReason {
    /// A SQLite error.
    Database,
    /// A migration failed.
    Migration,
    /// A stored JSON value could not be read.
    Json,
    /// A row was not found.
    NotFound,
    /// A value was out of range.
    OutOfRange,
    /// The encryption key has the wrong form.
    InvalidKey,
    /// The file is not a database, or the key is wrong.
    NotADatabase,
    /// This build has no database encryption.
    EncryptionUnavailable,
    /// Write-ahead logging is unavailable.
    WalUnavailable,
    /// An internal lock was poisoned.
    LockPoisoned,
}

impl StorageReason {
    fn as_str(self) -> &'static str {
        match self {
            Self::Database => "database",
            Self::Migration => "migration",
            Self::Json => "json",
            Self::NotFound => "not found",
            Self::OutOfRange => "out of range",
            Self::InvalidKey => "invalid key",
            Self::NotADatabase => "not a database",
            Self::EncryptionUnavailable => "encryption unavailable",
            Self::WalUnavailable => "wal unavailable",
            Self::LockPoisoned => "lock poisoned",
        }
    }
}

impl std::fmt::Display for CoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::SchemaTooNew => f.write_str("the database was written by a newer version"),
            Self::Storage { reason } => write!(f, "storage error ({})", reason.as_str()),
        }
    }
}

impl std::error::Error for CoreError {}

impl CoreError {
    fn storage(reason: StorageReason) -> Self {
        Self::Storage { reason }
    }
}

impl From<StorageError> for CoreError {
    fn from(e: StorageError) -> Self {
        let reason = match e {
            StorageError::SchemaTooNew(_) => return Self::SchemaTooNew,
            StorageError::Sqlite(_) => StorageReason::Database,
            StorageError::Migration { .. } => StorageReason::Migration,
            StorageError::Json(_) => StorageReason::Json,
            StorageError::NotFound(_) => StorageReason::NotFound,
            StorageError::OutOfRange(_) => StorageReason::OutOfRange,
            StorageError::InvalidKey => StorageReason::InvalidKey,
            StorageError::WrongKeyOrNotADatabase => StorageReason::NotADatabase,
            StorageError::EncryptionUnavailable => StorageReason::EncryptionUnavailable,
            StorageError::WalUnavailable => StorageReason::WalUnavailable,
        };
        Self::storage(reason)
    }
}

/// One folder as the apps see it.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct FolderInfo {
    /// Row id.
    pub id: i64,
    /// Full folder path with the server's delimiter.
    pub path: String,
    /// Special use such as `inbox`, `sent` or `trash`; an empty-use folder has the stored default.
    pub special_use: String,
    /// Unread messages.
    pub unread_count: u32,
    /// All messages.
    pub total_count: u32,
}

impl From<folders::Folder> for FolderInfo {
    fn from(f: folders::Folder) -> Self {
        Self {
            id: f.id,
            path: f.path,
            special_use: f.special_use,
            unread_count: clamp_count(f.unread_count),
            total_count: clamp_count(f.total_count),
        }
    }
}

/// A database count as `u32`: negative values (which triggers never produce) become 0 and
/// oversized ones saturate.
fn clamp_count(n: i64) -> u32 {
    u32::try_from(n.max(0)).unwrap_or(u32::MAX)
}

/// The Croft core as one shared object.
///
/// UniFFI objects are shared across threads, so the single database connection sits behind a
/// mutex; calls from different threads take turns.
#[derive(uniffi::Object)]
pub struct Core {
    db: Mutex<Db>,
}

#[uniffi::export]
impl Core {
    /// Opens (creating it if needed) the database at `db_path` and migrates it. The database is
    /// not encrypted yet: the key arrives with the keystore task.
    #[uniffi::constructor]
    pub fn open(db_path: String) -> Result<Arc<Self>, CoreError> {
        let db = Db::open(std::path::Path::new(&db_path), None)?;
        Ok(Arc::new(Self { db: Mutex::new(db) }))
    }

    /// Every folder of the account, ordered by path.
    pub fn list_folders(&self, account_id: i64) -> Result<Vec<FolderInfo>, CoreError> {
        let db = self
            .db
            .lock()
            .map_err(|_| CoreError::storage(StorageReason::LockPoisoned))?;
        let folders = folders::list(db.connection(), account_id)?;
        Ok(folders.into_iter().map(FolderInfo::from).collect())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use croft_protocols::{RemoteFolder, SpecialUse};
    use croft_storage::accounts::{self, NewAccount, Protocol};

    fn new_account(n: i64) -> NewAccount {
        NewAccount {
            label: format!("Account {n}"),
            email: format!("user{n}@example.org"),
            protocol: Protocol::Imap,
            provider: "generic".to_owned(),
            config: serde_json::json!({}),
            color: None,
            position: n,
        }
    }

    fn folder(path: &str, special_use: SpecialUse) -> RemoteFolder {
        RemoteFolder {
            path: path.to_owned(),
            delimiter: Some('/'),
            special_use,
            selectable: true,
        }
    }

    #[test]
    fn versions() {
        assert_eq!(croft_version(), env!("CARGO_PKG_VERSION"));
        assert_eq!(ffi_api_version(), 2);
    }

    #[test]
    fn open_and_list_on_a_fresh_database() {
        let dir = tempfile::tempdir().unwrap();
        let core = Core::open(dir.path().join("croft.db").display().to_string()).unwrap();
        assert!(core.list_folders(1).unwrap().is_empty());
    }

    #[test]
    fn lists_folders_of_the_requested_account_only() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("croft.db");
        {
            let db = Db::open(&path, None).unwrap();
            let conn = db.connection();
            for n in 1..=2 {
                accounts::insert(conn, &new_account(n), 0).unwrap();
            }
            folders::upsert(conn, 1, &folder("INBOX", SpecialUse::Inbox)).unwrap();
            folders::upsert(conn, 2, &folder("Other", SpecialUse::Other)).unwrap();
        }
        let core = Core::open(path.display().to_string()).unwrap();
        let listed = core.list_folders(1).unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].path, "INBOX");
        assert_eq!(listed[0].special_use, "inbox");
        assert_eq!((listed[0].unread_count, listed[0].total_count), (0, 0));
        assert!(core.list_folders(3).unwrap().is_empty());
    }

    #[test]
    fn a_non_database_file_is_a_storage_error_without_the_path() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("jane-doe-mail.db");
        std::fs::write(&path, vec![b'x'; 4096]).unwrap();
        let err = Core::open(path.display().to_string()).err().unwrap();
        let text = format!("{err} {err:?}");
        assert!(
            matches!(
                err,
                CoreError::Storage {
                    reason: StorageReason::NotADatabase
                }
            ),
            "{text}"
        );
        assert!(!text.contains("jane-doe"), "{text}");
    }

    #[test]
    fn a_newer_schema_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("croft.db");
        {
            let db = Db::open(&path, None).unwrap();
            db.connection()
                .pragma_update(None, "user_version", 9999)
                .unwrap();
        }
        let err = Core::open(path.display().to_string()).err().unwrap();
        assert!(matches!(err, CoreError::SchemaTooNew));
    }
}
