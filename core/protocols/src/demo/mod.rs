// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The sample mailbox ("Try with sample mail"): a [`MailBackend`] that serves a fixed set of
//! fictional messages from memory, and a [`MailSubmitter`] that never sends anything.
//!
//! App-store reviewers use it to reach every screen without a real account, so it must work with
//! no network at all. **It never opens a connection**: this module holds no host, no port, no
//! credential and no connector, and uses no socket API (a test reads this module's source and
//! fails if one appears; another runs every call on a runtime without an I/O driver, where any
//! socket would panic).
//!
//! # Changes stay on the device
//!
//! The "server" here is a copy in memory. Every change the engine confirms (read, star, move,
//! delete) is applied to that copy and reported as done, so the next sync agrees with what the
//! person did. Nothing is ever written anywhere else. When the app starts again, the copy is built
//! from what the device has stored ([`DemoBackend::resume`]), not from the built-in set, so a
//! sync after a restart does not undo anything either.
//!
//! # Sending
//!
//! [`DemoSubmitter::send`] accepts the message and sends nothing; [`DemoSubmitter::file_in_sent`]
//! puts the copy in the sample Sent folder, from where the next sync brings it to the device. So a
//! message written in the sample mailbox ends up in its local Sent folder. No SMTP, ever.

mod data;

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

pub use data::{ARCHIVE, DEMO_ADDRESS, DEMO_NAME, DRAFTS, INBOX, SENT, TRASH};

use crate::imap::text::{bracketed_message_id, decode_encoded_words, reference_ids};
use crate::smtp::data_lines;
use crate::{
    BackendError, BackendFuture, Capabilities, FlagChange, Flags, FolderStatus, MailBackend,
    MailSubmitter, MoveOutcome, RemoteFolder, RemoteHeader, RemoteSnapshot, Uid,
};

/// Reason given for a folder the sample mailbox does not have.
pub const REASON_NO_SUCH_FOLDER: &str = "The sample mailbox has no such folder";

/// What the device has stored for one folder of the sample mailbox, to rebuild the in-memory copy
/// after a restart ([`DemoBackend::resume`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DemoFolderState {
    /// The folder's path.
    pub path: String,
    /// True when the folder finished a sync (its UIDVALIDITY is stored). Then the stored messages
    /// are the whole folder. Otherwise the first sync was cut short: the built-in messages are kept
    /// and the stored ones are added to them.
    pub synced: bool,
    /// The UIDNEXT stored at the last sync, if any.
    pub uid_next: Option<Uid>,
    /// Every stored message (rows hidden while a move or delete waits included), with its flags.
    /// `references` may be empty: they are filled in from the built-in set by Message-ID.
    pub messages: Vec<RemoteHeader>,
}

/// One message in the in-memory copy.
#[derive(Debug, Clone)]
struct Stored {
    header: RemoteHeader,
    /// The whole message, when known (every built-in message, and every message filed in Sent
    /// during this run).
    source: Option<Arc<str>>,
}

#[derive(Debug, Clone)]
struct Folder {
    info: RemoteFolder,
    uid_next: Uid,
    messages: BTreeMap<Uid, Stored>,
}

impl Folder {
    fn status(&self) -> FolderStatus {
        FolderStatus {
            uid_validity: data::UID_VALIDITY,
            uid_next: self.uid_next,
            highest_modseq: None,
            exists: u32::try_from(self.messages.len()).unwrap_or(u32::MAX),
        }
    }

    /// Adds a message under the next UID and returns that UID.
    fn append(&mut self, mut stored: Stored) -> Uid {
        let uid = self.uid_next;
        self.uid_next = self.uid_next.saturating_add(1);
        stored.header.uid = uid;
        self.messages.insert(uid, stored);
        uid
    }
}

/// The sample mailbox. Cheap to build; every call answers from memory at once.
#[derive(Debug)]
pub struct DemoBackend {
    folders: Mutex<Vec<Folder>>,
}

/// The built-in set, as stored messages by folder path.
fn built_in(now_ms: i64) -> Vec<Folder> {
    let mut folders: Vec<Folder> = data::folders()
        .into_iter()
        .map(|info| Folder {
            info,
            uid_next: 1,
            messages: BTreeMap::new(),
        })
        .collect();
    for sample in data::samples() {
        let source: Arc<str> = Arc::from(sample.source(now_ms));
        let header = RemoteHeader {
            uid: 0,
            message_id: Some(sample.message_id()),
            references: sample.references(),
            subject: sample.subject.to_owned(),
            from_name: Some(sample.from.0.to_owned()),
            from_addr: Some(sample.from.1.to_owned()),
            date_ms: sample.date_ms(now_ms),
            size: u32::try_from(source.len()).unwrap_or(u32::MAX),
            flags: sample.flags(),
        };
        if let Some(folder) = folders.iter_mut().find(|f| f.info.path == sample.folder) {
            folder.append(Stored {
                header,
                source: Some(source),
            });
        }
    }
    folders
}

impl DemoBackend {
    /// The sample mailbox as it is first added: the built-in messages, dated relative to `now_ms`
    /// (Unix milliseconds).
    pub fn new(now_ms: i64) -> Self {
        Self {
            folders: Mutex::new(built_in(now_ms)),
        }
    }

    /// The sample mailbox as the device last stored it, so that a sync after a restart changes
    /// nothing the person did. `saved` is what the device has for each folder; a folder it has
    /// nothing for (or one whose first sync was cut short, see [`DemoFolderState::synced`]) starts
    /// from the built-in messages. Paths that are not sample folders are ignored.
    pub fn resume(now_ms: i64, saved: Vec<DemoFolderState>) -> Self {
        let mut folders = built_in(now_ms);
        // Built-in sources and references by Message-ID, to fill in what the device does not keep.
        let mut known: BTreeMap<String, Stored> = BTreeMap::new();
        for folder in &folders {
            for stored in folder.messages.values() {
                if let Some(id) = &stored.header.message_id {
                    known.insert(id.clone(), stored.clone());
                }
            }
        }
        for state in saved {
            let Some(folder) = folders.iter_mut().find(|f| f.info.path == state.path) else {
                continue;
            };
            if state.synced {
                folder.messages.clear();
            }
            for mut header in state.messages {
                let builtin = header.message_id.as_ref().and_then(|id| known.get(id));
                if header.references.is_empty() {
                    if let Some(b) = builtin {
                        header.references = b.header.references.clone();
                    }
                }
                let source = builtin.and_then(|b| b.source.clone());
                folder
                    .messages
                    .insert(header.uid, Stored { header, source });
            }
            let after_last = folder
                .messages
                .keys()
                .next_back()
                .map_or(1, |u| u.saturating_add(1));
            folder.uid_next = folder
                .uid_next
                .max(after_last)
                .max(state.uid_next.unwrap_or(1));
        }
        Self {
            folders: Mutex::new(folders),
        }
    }

    fn lock(&self) -> MutexGuard<'_, Vec<Folder>> {
        // Every change is one assignment under the lock, so a panic elsewhere leaves it whole.
        self.folders.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The whole message (RFC 5322 source) at `uid` in `folder`, for the message view. `None` for
    /// an unknown message, or for one written in the sample mailbox before the app last started
    /// (only its header is kept on the device).
    pub fn message_source(&self, folder: &str, uid: Uid) -> Option<String> {
        self.lock()
            .iter()
            .find(|f| f.info.path == folder)?
            .messages
            .get(&uid)?
            .source
            .as_deref()
            .map(str::to_owned)
    }

    /// A submitter for this mailbox: it sends nothing and files the copy in the sample Sent folder.
    pub fn submitter(self: &Arc<Self>) -> DemoSubmitter {
        DemoSubmitter {
            backend: self.clone(),
        }
    }

    fn with_folder<T>(
        &self,
        path: &str,
        f: impl FnOnce(&mut Folder) -> T,
    ) -> Result<T, BackendError> {
        let mut folders = self.lock();
        let folder = folders
            .iter_mut()
            .find(|x| x.info.path == path)
            .ok_or_else(|| BackendError::Permanent(REASON_NO_SUCH_FOLDER.to_owned()))?;
        Ok(f(folder))
    }

    /// Files a sent message in Sent once (by Message-ID). Returns its UID there.
    fn file_sent(&self, message_id: Option<&str>, mime: &[u8]) -> Result<Uid, BackendError> {
        let mut header = parse_header(mime);
        if header.message_id.is_none() {
            header.message_id = message_id.map(bracketed_message_id);
        }
        header.flags = Flags {
            seen: true,
            ..Flags::default()
        };
        let source: Option<Arc<str>> = std::str::from_utf8(mime).ok().map(Arc::from);
        self.with_folder(SENT, |sent| {
            let wanted = header.message_id.clone();
            if let Some(id) = &wanted {
                if let Some((uid, _)) = sent
                    .messages
                    .iter()
                    .find(|(_, m)| m.header.message_id.as_deref() == Some(id.as_str()))
                {
                    return *uid;
                }
            }
            sent.append(Stored { header, source })
        })
    }
}

/// The few header fields a list row needs, read from a message written in the sample mailbox.
fn parse_header(mime: &[u8]) -> RemoteHeader {
    let mut fields: Vec<(String, String)> = Vec::new();
    let mut raw = Vec::new();
    for line in data_lines(mime) {
        if line.is_empty() {
            break;
        }
        raw.extend_from_slice(line);
        raw.extend_from_slice(b"\r\n");
        let text = String::from_utf8_lossy(line);
        if text.starts_with([' ', '\t']) {
            if let Some((_, value)) = fields.last_mut() {
                value.push(' ');
                value.push_str(text.trim());
            }
        } else if let Some((name, value)) = text.split_once(':') {
            fields.push((name.trim().to_ascii_lowercase(), value.trim().to_owned()));
        }
    }
    let field = |name: &str| {
        fields
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.clone())
    };
    let (from_name, from_addr) = field("from").map_or((None, None), |v| split_mailbox(&v));
    let date_ms = field("date")
        .and_then(|d| chrono::DateTime::parse_from_rfc2822(&d).ok())
        .map(|d| d.timestamp_millis())
        .unwrap_or_else(now_ms);
    RemoteHeader {
        uid: 0,
        message_id: field("message-id")
            .filter(|v| !v.is_empty())
            .map(|v| bracketed_message_id(&v)),
        references: reference_ids(&raw),
        subject: field("subject")
            .map(|s| decode_encoded_words(&s))
            .unwrap_or_default(),
        from_name,
        from_addr,
        date_ms,
        size: u32::try_from(mime.len()).unwrap_or(u32::MAX),
        flags: Flags::default(),
    }
}

/// `Name <addr>` or a bare address, as (name, address).
fn split_mailbox(value: &str) -> (Option<String>, Option<String>) {
    match (value.rfind('<'), value.rfind('>')) {
        (Some(open), Some(close)) if open < close => {
            let name = decode_encoded_words(value[..open].trim().trim_matches('"'));
            let addr = value[open + 1..close].trim().to_owned();
            ((!name.is_empty()).then_some(name), Some(addr))
        }
        _ => (None, Some(value.trim().to_owned())),
    }
}

/// The time now, for a written message with no readable Date.
fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|d| i64::try_from(d.as_millis()).ok())
        .unwrap_or(0)
}

impl MailBackend for DemoBackend {
    fn capabilities(&self) -> BackendFuture<'_, Capabilities> {
        Box::pin(async {
            Ok(Capabilities {
                move_cmd: true,
                uidplus: true,
                special_use: true,
                ..Capabilities::default()
            })
        })
    }

    fn list_folders(&self) -> BackendFuture<'_, Vec<RemoteFolder>> {
        Box::pin(async { Ok(self.lock().iter().map(|f| f.info.clone()).collect()) })
    }

    fn folder_status<'a>(&'a self, folder: &'a str) -> BackendFuture<'a, FolderStatus> {
        Box::pin(async move { self.with_folder(folder, |f| f.status()) })
    }

    fn snapshot<'a>(&'a self, folder: &'a str) -> BackendFuture<'a, RemoteSnapshot> {
        Box::pin(async move {
            self.with_folder(folder, |f| RemoteSnapshot {
                status: f.status(),
                messages: f
                    .messages
                    .iter()
                    .map(|(uid, m)| (*uid, m.header.flags.clone()))
                    .collect(),
            })
        })
    }

    fn fetch_headers<'a>(
        &'a self,
        folder: &'a str,
        uids: &'a [Uid],
    ) -> BackendFuture<'a, Vec<RemoteHeader>> {
        Box::pin(async move {
            self.with_folder(folder, |f| {
                let mut out: Vec<RemoteHeader> = uids
                    .iter()
                    .filter_map(|uid| f.messages.get(uid))
                    .map(|m| m.header.clone())
                    .collect();
                out.sort_by(|a, b| b.date_ms.cmp(&a.date_ms).then(b.uid.cmp(&a.uid)));
                out.dedup_by_key(|h| h.uid);
                out
            })
        })
    }

    fn find_by_message_id<'a>(
        &'a self,
        folder: &'a str,
        message_id: &'a str,
    ) -> BackendFuture<'a, Option<Uid>> {
        Box::pin(async move {
            self.with_folder(folder, |f| {
                f.messages
                    .iter()
                    .rev()
                    .find(|(_, m)| m.header.message_id.as_deref() == Some(message_id))
                    .map(|(uid, _)| *uid)
            })
        })
    }

    fn set_flag<'a>(
        &'a self,
        folder: &'a str,
        uids: &'a [Uid],
        change: FlagChange,
    ) -> BackendFuture<'a, ()> {
        Box::pin(async move {
            self.with_folder(folder, |f| {
                for uid in uids {
                    if let Some(m) = f.messages.get_mut(uid) {
                        m.header.flags.apply(change);
                    }
                }
            })
        })
    }

    fn move_to<'a>(
        &'a self,
        folder: &'a str,
        uids: &'a [Uid],
        dest: &'a str,
    ) -> BackendFuture<'a, MoveOutcome> {
        Box::pin(async move {
            let mut folders = self.lock();
            let no_folder = || BackendError::Permanent(REASON_NO_SUCH_FOLDER.to_owned());
            let src = folders
                .iter()
                .position(|f| f.info.path == folder)
                .ok_or_else(no_folder)?;
            let dst = folders
                .iter()
                .position(|f| f.info.path == dest)
                .ok_or_else(no_folder)?;
            let mut outcome = MoveOutcome::default();
            for uid in uids {
                let Some(stored) = folders.get_mut(src).and_then(|f| f.messages.remove(uid)) else {
                    continue;
                };
                if let Some(d) = folders.get_mut(dst) {
                    outcome.dest_uids.insert(*uid, d.append(stored));
                }
            }
            Ok(outcome)
        })
    }

    fn delete_forever<'a>(&'a self, folder: &'a str, uids: &'a [Uid]) -> BackendFuture<'a, ()> {
        Box::pin(async move {
            self.with_folder(folder, |f| {
                for uid in uids {
                    f.messages.remove(uid);
                }
            })
        })
    }
}

/// "Sends" from the sample mailbox: nothing leaves the device. The message is accepted and its
/// copy goes into the sample Sent folder, so the person sees it there after the next sync.
#[derive(Debug, Clone)]
pub struct DemoSubmitter {
    backend: Arc<DemoBackend>,
}

impl MailSubmitter for DemoSubmitter {
    /// Accepts the message and sends nothing.
    fn send<'a>(
        &'a self,
        _envelope_from: &'a str,
        _recipients: &'a [String],
        _mime: &'a [u8],
    ) -> BackendFuture<'a, ()> {
        Box::pin(async { Ok(()) })
    }

    fn files_sent_itself(&self) -> bool {
        false
    }

    /// Puts the copy in the sample Sent folder, once per Message-ID.
    fn file_in_sent<'a>(
        &'a self,
        message_id: Option<&'a str>,
        mime: &'a [u8],
    ) -> BackendFuture<'a, ()> {
        Box::pin(async move { self.backend.file_sent(message_id, mime).map(|_| ()) })
    }
}

#[cfg(test)]
mod tests;
