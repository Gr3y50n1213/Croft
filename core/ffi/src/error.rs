// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Every error the surface returns. All of them are fixed categories: no variant carries free
//! text, a path, an address or a row value. The only data in any of them is a certificate
//! fingerprint (a public SHA-256, shown to the person on purpose) and enum values.

use croft_engine::{ActionError, EngineError};
use croft_protocols::discovery::{DiscoveryError, VerifyError};
use croft_storage::StorageError;

use crate::types::KnownProvider;

/// What can go wrong when calling [`Core`](crate::Core).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Error)]
pub enum CoreError {
    /// The database was written by a newer version of Croft and was not modified.
    SchemaTooNew,
    /// The database could not be opened or read. `reason` is a fixed category; it never contains
    /// row data, SQL parameters or file paths.
    Storage {
        /// Fixed category.
        reason: StorageReason,
    },
    /// No account with this id.
    NoSuchAccount,
    /// No message with this id.
    NoSuchMessage,
    /// No folder with this id in the message's account.
    NoSuchFolder,
    /// The account has no Archive folder (special use `archive`). The screen then follows the
    /// folder rules in `docs/ux/server-contract.md`.
    NoArchiveFolder,
    /// The account has no Trash folder (special use `trash`). Same follow-up as above.
    NoTrashFolder,
    /// No such operation (or it finished and was cleaned up).
    NoSuchOperation,
    /// Too late to undo: the operation has started or its window has closed.
    TooLate,
    /// The server settings cannot be used (no host, plaintext to a real host, a settings record
    /// written by a newer version).
    BadSettings,
    /// The settings have no IMAP server. Croft cannot use a JMAP-only server yet.
    UnsupportedServer,
    /// The pinned certificate fingerprint is not 64 hex digits.
    BadFingerprint,
    /// The credential provider had no credential for the account. The provider was told
    /// (`sign_in_needed`) and the account stays in the needs-sign-in state.
    NoCredential,
    /// Something inside the core failed (a task panicked or stopped). Nothing the person can fix.
    Internal,
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
            Self::NoSuchAccount => f.write_str("no such account"),
            Self::NoSuchMessage => f.write_str("no such message"),
            Self::NoSuchFolder => f.write_str("no such folder"),
            Self::NoArchiveFolder => f.write_str("the account has no archive folder"),
            Self::NoTrashFolder => f.write_str("the account has no trash folder"),
            Self::NoSuchOperation => f.write_str("no such operation"),
            Self::TooLate => f.write_str("too late to undo"),
            Self::BadSettings => f.write_str("the server settings cannot be used"),
            Self::UnsupportedServer => f.write_str("the server type is not supported yet"),
            Self::BadFingerprint => f.write_str("the certificate fingerprint is not valid"),
            Self::NoCredential => f.write_str("no credential for the account"),
            Self::Internal => f.write_str("internal error"),
        }
    }
}

impl std::error::Error for CoreError {}

impl CoreError {
    pub(crate) fn storage(reason: StorageReason) -> Self {
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

impl From<rusqlite::Error> for CoreError {
    fn from(e: rusqlite::Error) -> Self {
        StorageError::Sqlite(e).into()
    }
}

impl From<ActionError> for CoreError {
    fn from(e: ActionError) -> Self {
        match e {
            ActionError::NoSuchMessage => Self::NoSuchMessage,
            ActionError::NoSuchFolder | ActionError::SameFolder => Self::NoSuchFolder,
            ActionError::NoSuchOperation => Self::NoSuchOperation,
            ActionError::TooLate => Self::TooLate,
            ActionError::Storage(e) => e.into(),
        }
    }
}

impl From<EngineError> for CoreError {
    fn from(e: EngineError) -> Self {
        match e {
            EngineError::Storage(e) => e.into(),
            EngineError::UnknownAccount(_) => Self::NoSuchAccount,
            EngineError::Backend(_) | EngineError::AccountExists(_) | EngineError::NoRuntime => {
                Self::Internal
            }
        }
    }
}

/// Why finding the server settings failed (`discover`). Fieldless: the probe-by-probe reasons are
/// shown by the progress observer, not here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Error)]
pub enum DiscoverError {
    /// The text is not an email address.
    InvalidAddress,
    /// Every place Croft looked was unreachable: most likely there is no network.
    Offline,
    /// The domain publishes nothing Croft can use. Next step: ask for the settings (S05).
    NothingFound,
    /// Croft could not start its name lookup on this device.
    ResolverUnavailable,
    /// Something inside the core failed.
    Internal,
}

impl std::fmt::Display for DiscoverError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::InvalidAddress => "not an email address",
            Self::Offline => "no network",
            Self::NothingFound => "no settings found",
            Self::ResolverUnavailable => "name lookup unavailable",
            Self::Internal => "internal error",
        })
    }
}

impl std::error::Error for DiscoverError {}

impl From<&DiscoveryError> for DiscoverError {
    fn from(e: &DiscoveryError) -> Self {
        match e {
            DiscoveryError::InvalidAddress => Self::InvalidAddress,
            DiscoveryError::Offline { .. } => Self::Offline,
            DiscoveryError::NothingFound { .. } => Self::NothingFound,
            DiscoveryError::ResolverUnavailable => Self::ResolverUnavailable,
        }
    }
}

impl DiscoverError {
    fn key(self) -> &'static str {
        match self {
            Self::InvalidAddress => "discover.invalid_address",
            Self::Offline => "discover.offline",
            Self::NothingFound => "discover.nothing_found",
            Self::ResolverUnavailable => "discover.resolver_unavailable",
            Self::Internal => "discover.internal",
        }
    }

    fn default_message(self) -> &'static str {
        match self {
            Self::InvalidAddress => DiscoveryError::InvalidAddress.message(),
            Self::Offline => DiscoveryError::Offline { reasons: vec![] }.message(),
            Self::NothingFound => DiscoveryError::NothingFound { reasons: vec![] }.message(),
            Self::ResolverUnavailable => DiscoveryError::ResolverUnavailable.message(),
            Self::Internal => "Something went wrong inside Croft. Try again.",
        }
    }
}

/// The key the UI looks up in its own translations for a discovery error, for example
/// `discover.offline`.
#[uniffi::export]
pub fn discover_error_message_key(error: DiscoverError) -> String {
    error.key().to_owned()
}

/// The English text of a discovery error from the protocol layer (fixed wording, no data): the
/// default when the UI has no translation for the key.
#[uniffi::export]
pub fn discover_error_default_message(error: DiscoverError) -> String {
    error.default_message().to_owned()
}

/// Why signing in with the discovered settings failed (`verify`). Mirrors
/// `croft_protocols::discovery::VerifyError`. The fingerprints are public SHA-256 values of a
/// certificate (upper-case hex pairs separated by colons) for the person to compare; they are the
/// only strings in any error.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Error)]
pub enum VerifyFailure {
    /// The server rejected the address or password.
    WrongPassword,
    /// The server rejected the sign-in and its provider does not accept ordinary passwords from
    /// apps. The text names the provider's way in.
    NeedsAppPasswordOrOAuth {
        /// The provider.
        provider: KnownProvider,
    },
    /// The certificate does not chain to a trusted authority (self-signed, for example). Show the
    /// fingerprint and offer to trust it once; nothing secret was sent.
    UntrustedCertificate {
        /// SHA-256 of the server's certificate.
        fingerprint: String,
    },
    /// A pinned certificate changed.
    CertificateChanged {
        /// SHA-256 of the certificate the server presented now.
        fingerprint: String,
    },
    /// A well-known provider's certificate could not be verified: the connection is being
    /// intercepted. Cannot be trusted.
    ConnectionIntercepted,
    /// The certificate is expired, made for another name or faulty in another way.
    CertificateInvalid,
    /// The server could not be reached.
    CannotConnect,
    /// The server does not offer encryption Croft accepts.
    NoSecureConnection,
    /// The server answered but would not let Croft in.
    ServerProblem,
    /// The server offers JMAP and no IMAP, and Croft cannot sign in over JMAP yet.
    JmapOnly,
    /// The settings cannot be used, or the pinned fingerprint is malformed.
    BadSettings,
    /// Something inside the core failed.
    Internal,
}

impl std::fmt::Display for VerifyFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.key())
    }
}

impl std::error::Error for VerifyFailure {}

impl VerifyFailure {
    pub(crate) fn from_verify_error(e: &VerifyError) -> Self {
        match e {
            VerifyError::WrongPassword => Self::WrongPassword,
            VerifyError::NeedsAppPasswordOrOAuth(p) => Self::NeedsAppPasswordOrOAuth {
                provider: (*p).into(),
            },
            VerifyError::UntrustedCertificate { fingerprint } => Self::UntrustedCertificate {
                fingerprint: fingerprint.to_string(),
            },
            VerifyError::CertificateChanged { fingerprint } => Self::CertificateChanged {
                fingerprint: fingerprint.to_string(),
            },
            VerifyError::ConnectionIntercepted => Self::ConnectionIntercepted,
            VerifyError::CertificateInvalid => Self::CertificateInvalid,
            VerifyError::CannotConnect => Self::CannotConnect,
            VerifyError::NoSecureConnection => Self::NoSecureConnection,
            VerifyError::ServerProblem => Self::ServerProblem,
            VerifyError::JmapOnly => Self::JmapOnly,
            VerifyError::BadSettings => Self::BadSettings,
        }
    }

    /// The translation key. Fixed text without data.
    pub(crate) fn key(&self) -> &'static str {
        match self {
            Self::WrongPassword => "verify.wrong_password",
            Self::NeedsAppPasswordOrOAuth { provider } => match provider {
                KnownProvider::Gmail => "verify.needs_app_password.gmail",
                KnownProvider::Outlook => "verify.needs_app_password.outlook",
                KnownProvider::ICloud => "verify.needs_app_password.icloud",
                KnownProvider::Fastmail => "verify.needs_app_password.fastmail",
            },
            Self::UntrustedCertificate { .. } => "verify.untrusted_certificate",
            Self::CertificateChanged { .. } => "verify.certificate_changed",
            Self::ConnectionIntercepted => "verify.connection_intercepted",
            Self::CertificateInvalid => "verify.certificate_invalid",
            Self::CannotConnect => "verify.cannot_connect",
            Self::NoSecureConnection => "verify.no_secure_connection",
            Self::ServerProblem => "verify.server_problem",
            Self::JmapOnly => "verify.jmap_only",
            Self::BadSettings => "verify.bad_settings",
            Self::Internal => "verify.internal",
        }
    }

    /// The English text from the protocol layer, which the UI shows when it has no translation.
    pub(crate) fn default_message(&self) -> &'static str {
        use croft_protocols::CertFingerprint;
        let zero = CertFingerprint::from_bytes([0; 32]);
        match self {
            Self::WrongPassword => VerifyError::WrongPassword.message(),
            Self::NeedsAppPasswordOrOAuth { provider } => {
                VerifyError::NeedsAppPasswordOrOAuth((*provider).into()).message()
            }
            Self::UntrustedCertificate { .. } => {
                VerifyError::UntrustedCertificate { fingerprint: zero }.message()
            }
            Self::CertificateChanged { .. } => {
                VerifyError::CertificateChanged { fingerprint: zero }.message()
            }
            Self::ConnectionIntercepted => VerifyError::ConnectionIntercepted.message(),
            Self::CertificateInvalid => VerifyError::CertificateInvalid.message(),
            Self::CannotConnect => VerifyError::CannotConnect.message(),
            Self::NoSecureConnection => VerifyError::NoSecureConnection.message(),
            Self::ServerProblem => VerifyError::ServerProblem.message(),
            Self::JmapOnly => VerifyError::JmapOnly.message(),
            Self::BadSettings => VerifyError::BadSettings.message(),
            Self::Internal => "Something went wrong inside Croft. Try again.",
        }
    }
}

/// The key the UI looks up in its own translations for a sign-in failure, for example
/// `verify.wrong_password`.
#[uniffi::export]
pub fn verify_failure_message_key(failure: VerifyFailure) -> String {
    failure.key().to_owned()
}

/// The English text for a sign-in failure (from the protocol layer, fixed wording with no data):
/// the default when the UI has no translation for the key.
#[uniffi::export]
pub fn verify_failure_default_message(failure: VerifyFailure) -> String {
    failure.default_message().to_owned()
}
