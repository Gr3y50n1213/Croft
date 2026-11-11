// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The crate error type.
//!
//! Messages never contain message bodies, passwords, row values or full email addresses. SQLite and
//! JSON errors are reduced to a category before they are shown or debug-printed.

use std::fmt;

use crate::SchemaTooNew;

/// Result alias for this crate.
pub type Result<T, E = StorageError> = std::result::Result<T, E>;

/// Everything that can go wrong in the storage layer.
pub enum StorageError {
    /// SQLite reported an error. Only its category is shown, never its message text.
    Sqlite(rusqlite::Error),
    /// A schema migration failed; the database was rolled back to the version before it.
    Migration {
        /// Schema version the migration was trying to reach.
        version: u32,
        /// The underlying SQLite error.
        cause: rusqlite::Error,
    },
    /// The database was written by a newer app. It was not modified.
    SchemaTooNew(SchemaTooNew),
    /// A JSON document could not be read or written. Only the category is kept.
    Json(&'static str),
    /// The row does not exist for this account (or the id belongs to another account).
    NotFound(&'static str),
    /// A number does not fit the column type.
    OutOfRange(&'static str),
    /// The encryption key is not exactly 32 bytes.
    InvalidKey,
    /// The key is wrong or the file is not a database.
    WrongKeyOrNotADatabase,
    /// A key was given but this build has no SQLCipher; refusing to store mail unencrypted.
    EncryptionUnavailable,
    /// WAL journal mode could not be enabled on the database file.
    WalUnavailable,
}

fn sqlite_summary(err: &rusqlite::Error) -> String {
    match err {
        rusqlite::Error::SqliteFailure(e, _) => format!("{:?}", e.code),
        rusqlite::Error::QueryReturnedNoRows => "NoRows".to_owned(),
        _ => "Other".to_owned(),
    }
}

impl fmt::Display for StorageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Sqlite(e) => write!(f, "database error ({})", sqlite_summary(e)),
            Self::Migration { version, cause } => write!(
                f,
                "migration to schema version {version} failed ({})",
                sqlite_summary(cause)
            ),
            Self::SchemaTooNew(e) => e.fmt(f),
            Self::Json(kind) => write!(f, "invalid JSON document ({kind})"),
            Self::NotFound(what) => write!(f, "{what} not found"),
            Self::OutOfRange(what) => write!(f, "{what} is out of range"),
            Self::InvalidKey => f.write_str("the database key must be exactly 32 bytes"),
            Self::WrongKeyOrNotADatabase => {
                f.write_str("wrong database key, or the file is not a Croft database")
            }
            Self::EncryptionUnavailable => {
                f.write_str("this build has no database encryption (cargo feature sqlcipher)")
            }
            Self::WalUnavailable => f.write_str("could not enable WAL journal mode"),
        }
    }
}

// Debug goes through Display so that "{:?}" in a log line cannot leak what SQLite put in a message.
impl fmt::Debug for StorageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "StorageError({self})")
    }
}

impl std::error::Error for StorageError {}

impl From<rusqlite::Error> for StorageError {
    fn from(e: rusqlite::Error) -> Self {
        Self::Sqlite(e)
    }
}

impl From<SchemaTooNew> for StorageError {
    fn from(e: SchemaTooNew) -> Self {
        Self::SchemaTooNew(e)
    }
}

impl From<serde_json::Error> for StorageError {
    fn from(e: serde_json::Error) -> Self {
        use serde_json::error::Category;
        Self::Json(match e.classify() {
            Category::Io => "io",
            Category::Syntax => "syntax",
            Category::Data => "data",
            Category::Eof => "unexpected end",
        })
    }
}
