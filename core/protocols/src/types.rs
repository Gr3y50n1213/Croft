// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Protocol-neutral mail vocabulary.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

/// IMAP UID, or the numeric part of a JMAP id mapped by the adapter.
pub type Uid = u32;

/// What a folder is for, from RFC 6154 SPECIAL-USE attributes or well-known names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum SpecialUse {
    /// The main inbox.
    Inbox,
    /// Sent mail.
    Sent,
    /// Drafts.
    Drafts,
    /// Deleted mail.
    Trash,
    /// Spam.
    Junk,
    /// Archive (not All Mail).
    Archive,
    /// Gmail-style All Mail.
    All,
    /// Starred.
    Flagged,
    /// Anything else.
    Other,
}

impl SpecialUse {
    /// Reads an IMAP LIST attribute such as `\Sent`. Unknown attributes return `None`.
    pub fn from_attribute(attr: &str) -> Option<Self> {
        match attr.to_ascii_lowercase().as_str() {
            "\\inbox" => Some(Self::Inbox),
            "\\sent" => Some(Self::Sent),
            "\\drafts" => Some(Self::Drafts),
            "\\trash" => Some(Self::Trash),
            "\\junk" => Some(Self::Junk),
            "\\archive" => Some(Self::Archive),
            "\\all" => Some(Self::All),
            "\\flagged" => Some(Self::Flagged),
            _ => None,
        }
    }

    /// Guesses from the folder name when the server gives no attribute (many self-hosted setups).
    pub fn from_name(path: &str) -> Self {
        let leaf = path
            .rsplit(['/', '.'])
            .next()
            .unwrap_or(path)
            .to_ascii_lowercase();
        match leaf.as_str() {
            "inbox" => Self::Inbox,
            "sent" | "sent items" | "sent mail" | "sent messages" => Self::Sent,
            "drafts" | "draft" => Self::Drafts,
            "trash" | "deleted items" | "deleted messages" | "bin" => Self::Trash,
            "junk" | "spam" | "junk e-mail" | "junk email" => Self::Junk,
            "archive" | "archives" => Self::Archive,
            "all mail" => Self::All,
            _ => Self::Other,
        }
    }

    /// The text stored in the `folder.special_use` column.
    pub fn as_db_str(self) -> &'static str {
        match self {
            Self::Inbox => "inbox",
            Self::Sent => "sent",
            Self::Drafts => "drafts",
            Self::Trash => "trash",
            Self::Junk => "junk",
            Self::Archive => "archive",
            Self::All => "all",
            Self::Flagged => "flagged",
            Self::Other => "other",
        }
    }
}

/// A folder as the server reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteFolder {
    /// Full path, with the server's own delimiter.
    pub path: String,
    /// Hierarchy delimiter, when the server has one.
    pub delimiter: Option<char>,
    /// What the folder is for.
    pub special_use: SpecialUse,
    /// False for `\Noselect` containers.
    pub selectable: bool,
}

/// Message state shared by every protocol.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Flags {
    /// `\Seen`.
    pub seen: bool,
    /// `\Flagged` (starred).
    pub flagged: bool,
    /// `\Answered`.
    pub answered: bool,
    /// `\Draft`.
    pub draft: bool,
    /// `\Deleted`, used only for servers without a Trash move.
    pub deleted: bool,
    /// Custom keywords and labels.
    pub keywords: BTreeSet<String>,
}

/// A change to apply to [`Flags`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlagChange {
    /// Set or clear `\Seen`.
    Seen(bool),
    /// Set or clear `\Flagged`.
    Flagged(bool),
}

impl Flags {
    /// Applies one change.
    pub fn apply(&mut self, change: FlagChange) {
        match change {
            FlagChange::Seen(v) => self.seen = v,
            FlagChange::Flagged(v) => self.flagged = v,
        }
    }
}

/// Folder state used to decide how much to fetch.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FolderStatus {
    /// UIDVALIDITY: if this changes, every cached UID in the folder is meaningless.
    pub uid_validity: u32,
    /// The UID the next arriving message will get.
    pub uid_next: u32,
    /// HIGHESTMODSEQ when the server supports CONDSTORE.
    pub highest_modseq: Option<u64>,
    /// Number of messages.
    pub exists: u32,
}

/// Header-level data needed to draw a list row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteHeader {
    /// UID in the folder.
    pub uid: Uid,
    /// RFC 5322 Message-ID.
    pub message_id: Option<String>,
    /// References and In-Reply-To ids, oldest first.
    pub references: Vec<String>,
    /// Subject, decoded.
    pub subject: String,
    /// Sender display name.
    pub from_name: Option<String>,
    /// Sender address.
    pub from_addr: Option<String>,
    /// Date as Unix milliseconds.
    pub date_ms: i64,
    /// RFC 822 size in bytes.
    pub size: u32,
    /// Flags at fetch time.
    pub flags: Flags,
}

/// What a move produced: where each moved message ended up.
///
/// IMAP servers report this in the `COPYUID` response code (RFC 4315 UIDPLUS, RFC 6851 MOVE). The
/// sync engine uses it to re-point later queued operations on the same message at its new UID.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MoveOutcome {
    /// Source UID to the UID the message has in the destination folder. Empty when the server gave
    /// no `COPYUID` (no UIDPLUS), and a source UID that no longer existed when the move ran is
    /// simply absent, so repeating a move is never an error.
    pub dest_uids: BTreeMap<Uid, Uid>,
}

/// What the server can do, read once per connection. The screens use it to decide what is enabled
/// and the adapter uses it to pick the best command (MOVE or COPY + expunge, deltas or a full scan).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Capabilities {
    /// RFC 6851 `MOVE`.
    pub move_cmd: bool,
    /// RFC 4315 `UIDPLUS`: `UID EXPUNGE` and `COPYUID` / `APPENDUID` codes.
    pub uidplus: bool,
    /// RFC 6154 `SPECIAL-USE`: the server labels Sent, Trash and the rest.
    pub special_use: bool,
    /// RFC 2177 `IDLE`: the server can push changes.
    pub idle: bool,
    /// RFC 7162 `CONDSTORE`: per-message modification sequences.
    pub condstore: bool,
    /// RFC 7162 `QRESYNC`: resynchronise with a list of vanished UIDs.
    pub qresync: bool,
}

/// Server-side picture of a folder: every UID with its flags.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RemoteSnapshot {
    /// Folder counters.
    pub status: FolderStatus,
    /// Flags by UID.
    pub messages: BTreeMap<Uid, Flags>,
}

/// Why a backend call failed. The queue uses the split between transient and permanent errors to
/// decide between retrying and showing "Couldn't archive".
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BackendError {
    /// No connection, timeout, server busy. Retry later.
    Transient(String),
    /// Credentials rejected or token expired. The account needs the person's attention.
    Auth(String),
    /// The server refused this request and will refuse it again (no such folder, over quota).
    Permanent(String),
}

impl BackendError {
    /// True when retrying later can succeed.
    pub fn is_retryable(&self) -> bool {
        matches!(self, Self::Transient(_))
    }
}

impl fmt::Display for BackendError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Transient(m) => write!(f, "temporary problem: {m}"),
            Self::Auth(m) => write!(f, "sign-in needed: {m}"),
            Self::Permanent(m) => write!(f, "server refused: {m}"),
        }
    }
}

impl std::error::Error for BackendError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn special_use_from_attribute_ignores_case() {
        assert_eq!(SpecialUse::from_attribute("\\Sent"), Some(SpecialUse::Sent));
        assert_eq!(
            SpecialUse::from_attribute("\\TRASH"),
            Some(SpecialUse::Trash)
        );
        assert_eq!(SpecialUse::from_attribute("\\HasNoChildren"), None);
    }

    #[test]
    fn special_use_from_name_handles_common_self_hosted_layouts() {
        assert_eq!(SpecialUse::from_name("INBOX"), SpecialUse::Inbox);
        assert_eq!(SpecialUse::from_name("INBOX.Sent"), SpecialUse::Sent);
        assert_eq!(
            SpecialUse::from_name("INBOX/Deleted Items"),
            SpecialUse::Trash
        );
        assert_eq!(SpecialUse::from_name("Junk E-mail"), SpecialUse::Junk);
        assert_eq!(SpecialUse::from_name("Projects/2026"), SpecialUse::Other);
    }

    #[test]
    fn only_transient_errors_retry() {
        assert!(BackendError::Transient("timeout".into()).is_retryable());
        assert!(!BackendError::Auth("expired".into()).is_retryable());
        assert!(!BackendError::Permanent("no such folder".into()).is_retryable());
    }

    #[test]
    fn flag_changes_apply() {
        let mut f = Flags::default();
        f.apply(FlagChange::Seen(true));
        f.apply(FlagChange::Flagged(true));
        assert!(f.seen && f.flagged);
        f.apply(FlagChange::Seen(false));
        assert!(!f.seen && f.flagged);
    }
}
