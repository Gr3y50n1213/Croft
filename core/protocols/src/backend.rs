// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The one trait the rest of Croft talks to.

use crate::types::*;
use std::future::Future;
use std::pin::Pin;

/// Boxed future so the trait can be used as `dyn MailBackend` without extra crates.
pub type BackendFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T, BackendError>> + Send + 'a>>;

/// Everything the sync engine needs from a mail server, whatever the protocol.
///
/// Implementations must be cheap to call repeatedly and must not hold the person's password in
/// logs or error strings. They return [`BackendError`] so the queue can tell retryable failures
/// from permanent ones.
///
/// Every method is safe to call twice with the same arguments (see "Safe to repeat" in
/// `docs/ux/server-contract.md`): a flag set twice is harmless, and a message that is already gone
/// is simply absent from the answer instead of being an error.
///
/// Message-ID strings are passed exactly as [`RemoteHeader::message_id`] gives them, angle
/// brackets included.
pub trait MailBackend: Send + Sync {
    /// What the server can do. Read once per connection; the first call connects if needed.
    fn capabilities(&self) -> BackendFuture<'_, Capabilities>;

    /// Lists every folder.
    fn list_folders(&self) -> BackendFuture<'_, Vec<RemoteFolder>>;

    /// Counters for one folder. A different `uid_validity` from the one the caller last saw means
    /// every cached UID in the folder is meaningless.
    fn folder_status<'a>(&'a self, folder: &'a str) -> BackendFuture<'a, FolderStatus>;

    /// Flags for every UID in a folder. With CONDSTORE/QRESYNC or JMAP state strings an adapter
    /// may answer from deltas, but the result is always the full picture.
    fn snapshot<'a>(&'a self, folder: &'a str) -> BackendFuture<'a, RemoteSnapshot>;

    /// Headers for the given UIDs, newest first. UIDs that no longer exist are left out.
    fn fetch_headers<'a>(
        &'a self,
        folder: &'a str,
        uids: &'a [Uid],
    ) -> BackendFuture<'a, Vec<RemoteHeader>>;

    /// Finds a message in a folder by its Message-ID header: the UID of the newest match, or
    /// `None`. Used when a server has no UIDPLUS (so a move gave no new UID) and after a
    /// UIDVALIDITY change. An adapter may return `None` when several different messages match
    /// (the JMAP adapter does), so a caller never acts on a guess.
    fn find_by_message_id<'a>(
        &'a self,
        folder: &'a str,
        message_id: &'a str,
    ) -> BackendFuture<'a, Option<Uid>>;

    /// Sets or clears a flag on messages. UIDs that no longer exist are ignored.
    fn set_flag<'a>(
        &'a self,
        folder: &'a str,
        uids: &'a [Uid],
        change: FlagChange,
    ) -> BackendFuture<'a, ()>;

    /// Moves messages to another folder (archive, trash, spam and Move to all use this).
    ///
    /// The answer maps each source UID to its new UID in `dest`, from the `COPYUID` response. It
    /// is empty when the server gave none. A source UID that no longer exists is absent from the
    /// answer, not an error, so a retried move after a lost reply is harmless.
    fn move_to<'a>(
        &'a self,
        folder: &'a str,
        uids: &'a [Uid],
        dest: &'a str,
    ) -> BackendFuture<'a, MoveOutcome>;

    /// Deletes messages permanently.
    ///
    /// With UIDPLUS this sets `\Deleted` and sends `UID EXPUNGE` for just those UIDs. Without it
    /// a plain `EXPUNGE` would also remove other messages already marked `\Deleted`, so the
    /// adapter first checks that the folder has none and otherwise returns
    /// [`BackendError::Permanent`] without changing anything.
    fn delete_forever<'a>(&'a self, folder: &'a str, uids: &'a [Uid]) -> BackendFuture<'a, ()>;

    /// The whole message at `uid` in `folder`, as the server stores it (RFC 5322 bytes), for
    /// reading it and for its attachments: Croft fetches a message once and keeps every part from
    /// that copy, so there is no separate attachment download.
    ///
    /// **Never marks the message read**: IMAP uses `BODY.PEEK[]` on a folder opened read-only
    /// (`EXAMINE`), JMAP downloads the message's blob. A message the server no longer has is
    /// [`RawMessage::Gone`], not an error. One larger than [`MAX_MESSAGE_BYTES`] is
    /// [`RawMessage::TooLarge`] and nothing of it is downloaded. Errors keep the usual split
    /// (`Transient`, `Auth`, `Permanent`) and never contain message content.
    ///
    /// The default answers `Permanent`: an adapter (or a test stand-in) that cannot fetch bodies
    /// says so instead of failing to compile. Every real adapter (IMAP, JMAP, the sample mailbox)
    /// implements it.
    fn fetch_body<'a>(&'a self, folder: &'a str, uid: Uid) -> BackendFuture<'a, RawMessage> {
        let _ = (folder, uid);
        Box::pin(async {
            Err(BackendError::Permanent(
                "this server adapter cannot fetch messages".into(),
            ))
        })
    }
}
