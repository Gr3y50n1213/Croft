// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The records and enums the surface passes. Records that hold an address or login name print
//! `***` for it in `Debug`, so a stray log line cannot leak one.

use croft_protocols::discovery::{
    AuthHint, DiscoveredSettings, ImapServer, Probe, ProbeFailure, ProbeState, Provider,
    SmtpServer, Source,
};
use croft_protocols::Security;
use croft_storage::folders;

/// How the connection to a server is secured. Discovery never offers anything unencrypted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum SecurityMode {
    /// Implicit TLS from the first byte (IMAPS 993, SMTPS 465).
    Tls,
    /// STARTTLS (IMAP 143, submission 587); never falls back to plaintext.
    StartTls,
}

impl SecurityMode {
    pub(crate) fn from_security(s: Security) -> Self {
        match s {
            Security::Tls => Self::Tls,
            Security::StartTls => Self::StartTls,
            // Discovery never produces it. If one ever slipped in, the stricter choice makes the
            // connection fail instead of staying unencrypted.
            Security::PlaintextForTestsOnly => Self::Tls,
        }
    }

    pub(crate) fn to_security(self) -> Security {
        match self {
            Self::Tls => Security::Tls,
            Self::StartTls => Security::StartTls,
        }
    }
}

/// One server of a discovered or hand-entered setup.
#[derive(Clone, PartialEq, Eq, uniffi::Record)]
pub struct ServerSettings {
    /// Host name or IP address.
    pub host: String,
    /// TCP port.
    pub port: u16,
    /// How the connection is secured.
    pub security: SecurityMode,
    /// The login name, usually the whole address.
    pub username: String,
}

impl std::fmt::Debug for ServerSettings {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ServerSettings")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("security", &self.security)
            .field("username", &"***")
            .finish()
    }
}

/// A large provider Croft knows by domain.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum KnownProvider {
    /// Gmail.
    Gmail,
    /// Outlook.com and Microsoft 365.
    Outlook,
    /// iCloud Mail.
    ICloud,
    /// Fastmail.
    Fastmail,
}

impl From<Provider> for KnownProvider {
    fn from(p: Provider) -> Self {
        match p {
            Provider::Gmail => Self::Gmail,
            Provider::Outlook => Self::Outlook,
            Provider::ICloud => Self::ICloud,
            Provider::Fastmail => Self::Fastmail,
        }
    }
}

impl From<KnownProvider> for Provider {
    fn from(p: KnownProvider) -> Self {
        match p {
            KnownProvider::Gmail => Self::Gmail,
            KnownProvider::Outlook => Self::Outlook,
            KnownProvider::ICloud => Self::ICloud,
            KnownProvider::Fastmail => Self::Fastmail,
        }
    }
}

impl KnownProvider {
    /// The value stored in `account.provider`.
    pub(crate) fn db_name(self) -> &'static str {
        match self {
            Self::Gmail => "gmail",
            Self::Outlook => "outlook",
            Self::ICloud => "icloud",
            Self::Fastmail => "fastmail",
        }
    }
}

/// Where the settings came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum DiscoverySource {
    /// The built-in provider table.
    ProviderTable,
    /// An autoconfig file on the person's domain.
    Autoconfig,
    /// SRV records.
    Srv,
    /// `/.well-known/jmap`.
    JmapWellKnown,
}

impl From<Source> for DiscoverySource {
    fn from(s: Source) -> Self {
        match s {
            Source::ProviderTable => Self::ProviderTable,
            Source::Autoconfig => Self::Autoconfig,
            Source::Srv => Self::Srv,
            Source::JmapWellKnown => Self::JmapWellKnown,
        }
    }
}

impl From<DiscoverySource> for Source {
    fn from(s: DiscoverySource) -> Self {
        match s {
            DiscoverySource::ProviderTable => Self::ProviderTable,
            DiscoverySource::Autoconfig => Self::Autoconfig,
            DiscoverySource::Srv => Self::Srv,
            DiscoverySource::JmapWellKnown => Self::JmapWellKnown,
        }
    }
}

/// What kind of secret the server wants, as far as discovery can tell.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum AuthKind {
    /// An ordinary password.
    Password,
    /// An app-specific password.
    AppPassword,
    /// Sign-in through the provider's page (OAuth 2).
    OAuth2,
    /// OAuth 2, or an app password as the fallback.
    OAuth2OrAppPassword,
}

impl From<AuthHint> for AuthKind {
    fn from(a: AuthHint) -> Self {
        match a {
            AuthHint::Password => Self::Password,
            AuthHint::AppPassword => Self::AppPassword,
            AuthHint::OAuth2 => Self::OAuth2,
            AuthHint::OAuth2OrAppPassword => Self::OAuth2OrAppPassword,
        }
    }
}

impl From<AuthKind> for AuthHint {
    fn from(a: AuthKind) -> Self {
        match a {
            AuthKind::Password => Self::Password,
            AuthKind::AppPassword => Self::AppPassword,
            AuthKind::OAuth2 => Self::OAuth2,
            AuthKind::OAuth2OrAppPassword => Self::OAuth2OrAppPassword,
        }
    }
}

/// What `discover` found for an address; passed back to `verify` and `add_account`. It holds no
/// secret.
#[derive(Clone, PartialEq, Eq, uniffi::Record)]
pub struct FfiDiscovered {
    /// The address that was looked up (the account's own address).
    pub email: String,
    /// The IMAP server. `None` only when the domain offers JMAP and nothing else.
    pub imap: Option<ServerSettings>,
    /// The SMTP submission server, when one was found (kept for sending).
    pub smtp: Option<ServerSettings>,
    /// A JMAP URL, when the domain publishes one.
    pub jmap: Option<String>,
    /// Which probe produced these settings.
    pub source: DiscoverySource,
    /// The provider, when the address is in the built-in table.
    pub provider: Option<KnownProvider>,
    /// What kind of secret to ask for.
    pub auth_hint: AuthKind,
}

impl std::fmt::Debug for FfiDiscovered {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FfiDiscovered")
            .field("email", &"***")
            .field("imap", &self.imap)
            .field("smtp", &self.smtp)
            .field("jmap", &self.jmap)
            .field("source", &self.source)
            .field("provider", &self.provider)
            .field("auth_hint", &self.auth_hint)
            .finish()
    }
}

impl FfiDiscovered {
    pub(crate) fn from_settings(email: &str, s: &DiscoveredSettings) -> Self {
        Self {
            email: email.to_owned(),
            imap: s.imap.as_ref().map(|i| ServerSettings {
                host: i.host.clone(),
                port: i.port,
                security: SecurityMode::from_security(i.security),
                username: i.username.clone(),
            }),
            smtp: s.smtp.as_ref().map(|i| ServerSettings {
                host: i.host.clone(),
                port: i.port,
                security: SecurityMode::from_security(i.security),
                username: i.username.clone(),
            }),
            jmap: s.jmap.clone(),
            source: s.source.into(),
            provider: s.provider.map(Into::into),
            auth_hint: s.auth_hint.into(),
        }
    }

    pub(crate) fn to_settings(&self) -> DiscoveredSettings {
        DiscoveredSettings {
            imap: self.imap.as_ref().map(|i| ImapServer {
                host: i.host.clone(),
                port: i.port,
                security: i.security.to_security(),
                username: i.username.clone(),
            }),
            smtp: self.smtp.as_ref().map(|i| SmtpServer {
                host: i.host.clone(),
                port: i.port,
                security: i.security.to_security(),
                username: i.username.clone(),
            }),
            jmap: self.jmap.clone(),
            source: self.source.into(),
            provider: self.provider.map(Into::into),
            auth_hint: self.auth_hint.into(),
        }
    }
}

/// One of the places discovery looks, in order of preference (S03 shows them as a list).
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum DiscoveryProbe {
    /// The built-in table of large providers.
    ProviderTable,
    /// `autoconfig.<domain>`.
    AutoconfigSubdomain,
    /// `<domain>/.well-known/autoconfig`.
    AutoconfigWellKnown,
    /// SRV records.
    Srv,
    /// `<domain>/.well-known/jmap`.
    JmapWellKnown,
}

impl From<Probe> for DiscoveryProbe {
    fn from(p: Probe) -> Self {
        match p {
            Probe::ProviderTable => Self::ProviderTable,
            Probe::AutoconfigSubdomain => Self::AutoconfigSubdomain,
            Probe::AutoconfigWellKnown => Self::AutoconfigWellKnown,
            Probe::Srv => Self::Srv,
            Probe::JmapWellKnown => Self::JmapWellKnown,
        }
    }
}

/// Why one probe found nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum ProbeFailureReason {
    /// The domain does not publish this.
    NotPublished,
    /// The request took too long.
    TimedOut,
    /// The network or the name service could not be reached.
    Unreachable,
    /// The server's certificate was not accepted.
    CertificateProblem,
    /// The answer was not in a form Croft understands.
    Unusable,
    /// The answer was too large.
    TooLarge,
    /// The server sent Croft somewhere it does not follow.
    BadRedirect,
    /// The domain offers only an unencrypted connection.
    OnlyUnencrypted,
    /// Something inside Croft went wrong.
    Internal,
}

impl From<ProbeFailure> for ProbeFailureReason {
    fn from(p: ProbeFailure) -> Self {
        match p {
            ProbeFailure::NotPublished => Self::NotPublished,
            ProbeFailure::TimedOut => Self::TimedOut,
            ProbeFailure::Unreachable => Self::Unreachable,
            ProbeFailure::CertificateProblem => Self::CertificateProblem,
            ProbeFailure::Unusable => Self::Unusable,
            ProbeFailure::TooLarge => Self::TooLarge,
            ProbeFailure::BadRedirect => Self::BadRedirect,
            ProbeFailure::OnlyUnencrypted => Self::OnlyUnencrypted,
            ProbeFailure::Internal => Self::Internal,
        }
    }
}

/// Where one probe stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum ProbeProgress {
    /// Not started yet.
    Waiting,
    /// Asking the network.
    Running,
    /// Returned usable settings.
    Found,
    /// Looked and found nothing usable.
    Failed {
        /// Why.
        reason: ProbeFailureReason,
    },
    /// Not needed.
    Skipped,
}

impl From<ProbeState> for ProbeProgress {
    fn from(s: ProbeState) -> Self {
        match s {
            ProbeState::Waiting => Self::Waiting,
            ProbeState::Running => Self::Running,
            ProbeState::Found => Self::Found,
            ProbeState::Failed(r) => Self::Failed { reason: r.into() },
            ProbeState::Skipped => Self::Skipped,
        }
    }
}

/// Sync state of an account, for the status chip.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum AccountStatus {
    /// Healthy.
    Ok,
    /// A sync is running.
    Syncing,
    /// No connection.
    Offline,
    /// The server refused the credential, or none is stored. The account does nothing until
    /// `signed_in`.
    NeedsSignIn,
    /// The account cannot run (unusable stored settings).
    Error,
}

/// One account as the account list shows it.
#[derive(Clone, PartialEq, Eq, uniffi::Record)]
pub struct AccountInfo {
    /// Row id.
    pub id: i64,
    /// Name shown in the list.
    pub label: String,
    /// The account's address. Shown on screen; never logged or put in an error.
    pub email: String,
    /// Status chip state.
    pub status: AccountStatus,
    /// When a folder of the account last synced successfully (the newest `last_synced_at` of its
    /// folders), Unix milliseconds. `None` until the first folder has synced. S13 and S19 show it
    /// as "Last synced".
    pub last_synced_ms: Option<i64>,
    /// True for the sample mailbox (`add_demo_account`): fictional mail served from the app, no
    /// server. Every change to it stays on this device, so its message rows show "Not saved to
    /// server" (the read-only mailbox rule in `docs/ux/server-contract.md`), and nothing written
    /// in it is ever sent.
    pub is_demo: bool,
}

impl std::fmt::Debug for AccountInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AccountInfo")
            .field("id", &self.id)
            .field("label", &"***")
            .field("email", &"***")
            .field("status", &self.status)
            .field("last_synced_ms", &self.last_synced_ms)
            .field("is_demo", &self.is_demo)
            .finish()
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

/// Which messages a list shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum MessageScope {
    /// One folder of one account.
    Folder {
        /// The account.
        account: i64,
        /// The folder's row id.
        folder_id: i64,
    },
    /// The Inbox of every account (folders with special use `inbox`) merged by date.
    UnifiedInbox,
    /// Starred (flagged) messages across the folders of one account, or of every account when
    /// `account` is `None`, merged by date. Messages in Trash and Junk and rows hidden while a
    /// move or delete waits for the server or by a snooze are left out, as in every other list.
    Starred {
        /// The account, or `None` for all accounts.
        account: Option<i64>,
    },
}

/// Where the next page starts: just after the row with this date and id. Opaque to the app; pass
/// back the `next` of the previous page.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Record)]
pub struct RowCursor {
    /// The last row's date, Unix milliseconds.
    pub date_ms: i64,
    /// The last row's id.
    pub id: i64,
}

/// One list row, shaped for S10.
#[derive(Clone, PartialEq, Eq, uniffi::Record)]
pub struct MessageRow {
    /// Row id (local; what the actions take).
    pub id: i64,
    /// The account.
    pub account: i64,
    /// The folder's row id.
    pub folder: i64,
    /// Sender display name, if the message has one.
    pub sender_name: Option<String>,
    /// Sender address.
    pub sender_address: Option<String>,
    /// Subject.
    pub subject: String,
    /// Preview text.
    pub snippet: String,
    /// Date, Unix milliseconds.
    pub date_ms: i64,
    /// Read.
    pub seen: bool,
    /// Starred.
    pub flagged: bool,
    /// Has attachments.
    pub has_attachments: bool,
    /// A local change has not been confirmed by the server yet.
    pub pending: bool,
    /// How many messages share this row's conversation key in this row's folder (this one
    /// included; rows hidden by a waiting move or delete are not counted). 1 when alone.
    pub thread_count: u32,
    /// Who the message was sent to (display names, else addresses), for the Sent folder's rows.
    /// Read from the stored `to_json`; empty when the sync stored none.
    pub recipients: Vec<String>,
}

impl std::fmt::Debug for MessageRow {
    /// Message content and addresses are not printed.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MessageRow")
            .field("id", &self.id)
            .field("account", &self.account)
            .field("folder", &self.folder)
            .field("date_ms", &self.date_ms)
            .finish_non_exhaustive()
    }
}

/// One page of a list, newest first.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct MessagePage {
    /// The rows.
    pub rows: Vec<MessageRow>,
    /// Pass this as `before` for the next page; `None` when this was the last.
    pub next: Option<RowCursor>,
}

/// A folder of an account, in a change notice.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, uniffi::Record)]
pub struct FolderRef {
    /// The account.
    pub account: i64,
    /// The folder's row id.
    pub folder_id: i64,
}

/// Why a queued change was given up. Fixed categories.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum FailureReason {
    /// The message could not be found after it was moved.
    LostAfterMove,
    /// The folder was renumbered on the server and the message could not be found again.
    LostAfterReset,
    /// Two copies of the message exist and the core could not tell which to change.
    AmbiguousCopy,
    /// The server refused the change.
    Rejected,
}

/// A change that failed and was undone locally: the screen shows "Couldn't archive" or similar.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Record)]
pub struct OpFailure {
    /// The account.
    pub account: i64,
    /// The operation id the failed call returned.
    pub op_id: u64,
    /// Why.
    pub reason: FailureReason,
}

/// What changed since the `after` the app passed to `next_change`. The app re-runs the queries of
/// what it shows.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ChangeBatch {
    /// Pass this as `after` next time.
    pub seq: u64,
    /// True when the app was too far behind (or from another run) to be told exactly: re-run
    /// everything. The other lists may then be incomplete.
    pub everything: bool,
    /// Folders whose rows or counters changed.
    pub folders: Vec<FolderRef>,
    /// Accounts whose entry, status or folder list changed (also accounts added or removed).
    pub accounts: Vec<i64>,
    /// Changes that failed.
    pub failures: Vec<OpFailure>,
}
