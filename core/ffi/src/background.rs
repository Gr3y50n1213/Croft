// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The periodic background check (T20) for the app's WorkManager job: one account at a time,
//! within a deadline, answering what to notify about. Problems are flags, so the job decides
//! between "done" and "retry" without parsing errors.

use std::time::Duration;

use croft_engine::EngineError;
use croft_protocols::BackendError;

use crate::core::Core;
use crate::error::CoreError;
use crate::settings::is_demo;

/// A message to notify about: sender and subject only (never the body), and what it opens.
#[derive(Clone, PartialEq, Eq, uniffi::Record)]
pub struct NewMailInfo {
    /// The message's row id (the notification's tap opens it).
    pub message_id: i64,
    /// Its UID in the Inbox: pass it back to [`Core::mark_notified`] once posted.
    pub uid: u32,
    /// Sender display name, if any.
    pub sender_name: Option<String>,
    /// Sender address.
    pub sender_address: Option<String>,
    /// Subject.
    pub subject: String,
    /// Date, Unix milliseconds.
    pub date_ms: i64,
}

/// What one background check of an account gave.
#[derive(Clone, PartialEq, Eq, uniffi::Record)]
pub struct BackgroundCheck {
    /// The messages to notify about, oldest first. Empty on an account's first check.
    pub new_mail: Vec<NewMailInfo>,
    /// The deadline passed before the sync finished (not a failure; the next check goes on).
    pub timed_out: bool,
    /// The server or the network failed for now: the job may ask to be retried.
    pub transient_failure: bool,
    /// The account needs the person to sign in again: nothing was asked of the server.
    pub needs_sign_in: bool,
}

#[uniffi::export]
impl Core {
    /// One background check of `account_id`: due changes, a full sync, then the Inbox messages to
    /// notify about, all within `deadline_ms`. The sample mailbox and paused accounts are not
    /// synced and notify nothing. Async; for the background job only.
    pub async fn sync_once(
        &self,
        account_id: i64,
        deadline_ms: u64,
    ) -> Result<BackgroundCheck, CoreError> {
        let account = self
            .inner
            .handle
            .read(|db| croft_storage::accounts::get(db.connection(), account_id))?
            .ok_or(CoreError::NoSuchAccount)?;
        let mut out = BackgroundCheck {
            new_mail: Vec::new(),
            timed_out: false,
            transient_failure: false,
            needs_sign_in: false,
        };
        if is_demo(&account.config, &account.email) {
            return Ok(out);
        }
        if !self.inner.backends().contains_key(&account_id) {
            // Not running (settings unusable, or removed meanwhile): nothing to check.
            return Ok(out);
        }
        if self.inner.handle.needs_sign_in(account_id) {
            out.needs_sign_in = true;
            return Ok(out);
        }
        let handle = self.inner.handle.clone();
        let deadline = Duration::from_millis(deadline_ms);
        let task = self.spawn(
            async move { handle.sync_once(account_id, deadline, false).await },
            true,
        );
        match task.await.map_err(|_| CoreError::Internal)? {
            Ok(r) => {
                out.timed_out = r.timed_out;
                out.new_mail = r
                    .new_mail
                    .into_iter()
                    .map(|m| NewMailInfo {
                        message_id: m.message_id,
                        uid: m.uid,
                        sender_name: m.from_name,
                        sender_address: m.from_addr,
                        subject: m.subject,
                        date_ms: m.date_ms,
                    })
                    .collect();
            }
            Err(EngineError::Backend(BackendError::Auth(_))) => out.needs_sign_in = true,
            Err(EngineError::Backend(BackendError::Transient(_))) => out.transient_failure = true,
            Err(EngineError::Backend(BackendError::Permanent(_))) => {}
            Err(EngineError::Storage(e)) => return Err(e.into()),
            Err(_) => {}
        }
        Ok(out)
    }

    /// Whether `message_id` is still the Inbox message with `uid` of the account. A
    /// notification's buttons and tap check this before acting: a deleted message's row id can be
    /// given to a new message, and a stale notification must never act on that one.
    pub fn notified_message_is(
        &self,
        account_id: i64,
        message_id: i64,
        uid: u32,
    ) -> Result<bool, CoreError> {
        self.inner
            .handle
            .notified_message_is(account_id, message_id, uid)
            .map_err(|e| match e {
                EngineError::Storage(s) => s.into(),
                _ => CoreError::Internal,
            })
    }

    /// The app posted notifications for these Inbox UIDs of the account: they are never offered
    /// again (the marker moves up to the highest; never back).
    pub fn mark_notified(&self, account_id: i64, uids: Vec<u32>) -> Result<(), CoreError> {
        self.inner
            .handle
            .mark_notified(account_id, &uids)
            .map_err(|e| match e {
                EngineError::Storage(s) => s.into(),
                _ => CoreError::Internal,
            })
    }
}
