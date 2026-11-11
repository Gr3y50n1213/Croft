// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The sample mailbox (`croft_protocols::demo`) in the engine: rebuilding its in-memory copy from
//! what the device stored ([`resume_demo`]), and its submitter
//! ([`EngineHandle::demo_submitter`]), which stores the copy of a sent message on the device
//! before the Outbox lets go of the message.
//!
//! Why the submitter writes to the device: the sample mailbox's "server" is memory. A sent
//! message's copy would otherwise exist only there until the next sync of Sent, and an app killed
//! in between would rebuild Sent without it ([`resume_demo`] reads the device). The row is written
//! inside `file_in_sent`, before the engine marks the Outbox entry filed, so at every moment the
//! message is either still in the Outbox (and filed again after a restart, once: the copy is found
//! by Message-ID) or stored in the local Sent folder.

use std::fmt;
use std::sync::Arc;

use croft_mail_core::AccountId;
use croft_protocols::demo::SENT;
use croft_protocols::{
    BackendError, BackendFuture, DemoBackend, DemoFolderState, MailSubmitter, RemoteHeader,
};
use croft_storage::{folders, messages, Db, StorageError};

use crate::engine::Shared;
use crate::handle::EngineHandle;

/// Reason given when the copy of a sent message could not be stored on the device (the engine
/// tries the copy again later).
pub const REASON_COPY_NOT_STORED: &str = "The copy for Sent could not be stored on this device";

/// The sample mailbox of `account` as the device stored it, for after a restart: every folder with
/// its rows (rows hidden while a move or delete waits included) and their flags, so that a sync
/// changes nothing the person did. A folder the device has nothing for starts from the built-in
/// messages (see [`DemoBackend::resume`]). `now_ms` dates those built-in messages.
///
/// Reads the database: call it from a worker thread, not the UI thread.
pub fn resume_demo(db: &Db, account: AccountId, now_ms: i64) -> Result<DemoBackend, StorageError> {
    let conn = db.connection();
    let mut saved = Vec::new();
    for f in folders::list(conn, account)? {
        let mut headers = Vec::new();
        for (uid, flags) in messages::folder_flags(conn, account, f.id)? {
            if let Some(m) = messages::find_by_uid(conn, account, f.id, uid)? {
                headers.push(RemoteHeader {
                    uid,
                    message_id: m.message_id_hdr,
                    references: Vec::new(),
                    subject: m.subject,
                    from_name: m.from_name,
                    from_addr: m.from_addr,
                    date_ms: m.date_ts,
                    size: u32::try_from(m.size).unwrap_or(0),
                    flags,
                });
            }
        }
        saved.push(DemoFolderState {
            path: f.path,
            synced: f.uid_validity.is_some(),
            uid_next: f.uid_next.and_then(|n| u32::try_from(n).ok()),
            messages: headers,
        });
    }
    Ok(DemoBackend::resume(now_ms, saved))
}

impl EngineHandle {
    /// The submitter for a sample-mailbox account: it sends nothing, files the copy in the sample
    /// Sent folder, and stores that copy's row in the account's local Sent folder before the Outbox
    /// lets go of the message (see the module documentation). Pass it to
    /// [`crate::Engine::add_account_with_submitter`] with the same `backend`.
    pub fn demo_submitter(
        &self,
        account: AccountId,
        backend: Arc<DemoBackend>,
    ) -> Arc<dyn MailSubmitter> {
        Arc::new(DemoLocalSubmitter {
            shared: self.shared().clone(),
            account,
            backend,
        })
    }
}

/// See [`EngineHandle::demo_submitter`].
struct DemoLocalSubmitter {
    shared: Arc<Shared>,
    account: AccountId,
    backend: Arc<DemoBackend>,
}

impl fmt::Debug for DemoLocalSubmitter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DemoLocalSubmitter")
            .field("account", &self.account)
            .finish_non_exhaustive()
    }
}

impl DemoLocalSubmitter {
    /// Stores the copy's row in the local Sent folder (created if the first sync has not made it
    /// yet), in one transaction.
    fn store(&self, header: &RemoteHeader) -> Result<(), StorageError> {
        let account = self.account;
        let sent = self.backend.folder(SENT);
        let created = {
            let mut q = self.shared.lock();
            q.store_mut().db_mut().transaction(|tx| {
                let (folder_id, created) = match folders::find_by_path(tx, account, SENT)? {
                    Some(f) => (f.id, false),
                    None => match &sent {
                        Some(f) => (folders::upsert(tx, account, f)?, true),
                        None => return Err(StorageError::NotFound("folder")),
                    },
                };
                messages::upsert(tx, account, folder_id, header)?;
                Ok(created)
            })?
        };
        if created {
            self.shared.emit(crate::Change::Folders { account });
        }
        self.shared.emit_folders(account, &[SENT]);
        Ok(())
    }
}

impl MailSubmitter for DemoLocalSubmitter {
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

    /// Files the copy in the sample Sent folder (once per Message-ID) and stores its row on the
    /// device. A failure to store is temporary: the Outbox keeps the message and tries again.
    fn file_in_sent<'a>(
        &'a self,
        message_id: Option<&'a str>,
        mime: &'a [u8],
    ) -> BackendFuture<'a, ()> {
        Box::pin(async move {
            let header = self.backend.file_sent_copy(message_id, mime)?;
            self.store(&header).map_err(|_| {
                log::warn!(
                    "account {}: could not store a sent copy of the sample mailbox",
                    self.account
                );
                BackendError::Transient(REASON_COPY_NOT_STORED.to_owned())
            })
        })
    }
}
