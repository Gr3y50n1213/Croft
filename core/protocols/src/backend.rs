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
pub trait MailBackend: Send + Sync {
    /// Lists every folder.
    fn list_folders(&self) -> BackendFuture<'_, Vec<RemoteFolder>>;

    /// Counters for one folder.
    fn folder_status<'a>(&'a self, folder: &'a str) -> BackendFuture<'a, FolderStatus>;

    /// Flags for every UID in a folder. With CONDSTORE/QRESYNC or JMAP state strings an adapter
    /// may answer from deltas, but the result is always the full picture.
    fn snapshot<'a>(&'a self, folder: &'a str) -> BackendFuture<'a, RemoteSnapshot>;

    /// Headers for the given UIDs, newest first.
    fn fetch_headers<'a>(
        &'a self,
        folder: &'a str,
        uids: &'a [Uid],
    ) -> BackendFuture<'a, Vec<RemoteHeader>>;

    /// Sets or clears a flag on messages.
    fn set_flag<'a>(
        &'a self,
        folder: &'a str,
        uids: &'a [Uid],
        change: FlagChange,
    ) -> BackendFuture<'a, ()>;

    /// Moves messages to another folder (archive, trash, spam and Move to all use this).
    fn move_to<'a>(
        &'a self,
        folder: &'a str,
        uids: &'a [Uid],
        dest: &'a str,
    ) -> BackendFuture<'a, ()>;

    /// Deletes messages permanently.
    fn delete_forever<'a>(&'a self, folder: &'a str, uids: &'a [Uid]) -> BackendFuture<'a, ()>;
}
