// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The background check (T20): sync an account once within a deadline, then say which Inbox
//! messages deserve a notification ([`croft_mail_core::notify`]). The "notified up to" marker is
//! stored per account in the `setting` table and moves only when the app reports what it posted
//! ([`EngineHandle::mark_notified`]).

use std::time::Duration;

use croft_mail_core::notify::{self, Decision, InboxRow, Marker};
use croft_mail_core::AccountId;
use croft_storage::notify::{self as store, StoredMarker};
use croft_storage::{folders, Db, StorageError};

use crate::engine::EngineError;
use crate::handle::EngineHandle;
/// A message to notify about: what the notification shows (sender and subject only, never the
/// body) and what it opens.
#[derive(Clone, PartialEq, Eq)]
pub struct NewMail {
    /// The message's row id.
    pub message_id: i64,
    /// Its UID in the Inbox (passed back to [`EngineHandle::mark_notified`]).
    pub uid: u32,
    /// Sender display name.
    pub from_name: Option<String>,
    /// Sender address.
    pub from_addr: Option<String>,
    /// Subject.
    pub subject: String,
    /// Date, Unix milliseconds.
    pub date_ms: i64,
}

impl std::fmt::Debug for NewMail {
    // Sender and subject are personal: never printed.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NewMail")
            .field("message_id", &self.message_id)
            .field("uid", &self.uid)
            .finish()
    }
}

/// What one background check gave.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SyncOnce {
    /// The messages to notify about, oldest first (none on the first check: the baseline).
    pub new_mail: Vec<NewMail>,
    /// The deadline passed before the sync finished; the next check carries on.
    pub timed_out: bool,
}

/// The Inbox of an account: its id and UIDVALIDITY.
fn inbox(db: &Db, account: AccountId) -> Result<Option<(i64, u32)>, StorageError> {
    Ok(folders::list(db.connection(), account)?
        .into_iter()
        .find(|f| f.special_use == "inbox")
        .map(|f| {
            (
                f.id,
                f.uid_validity
                    .and_then(|v| u32::try_from(v).ok())
                    .unwrap_or(0),
            )
        }))
}

impl EngineHandle {
    /// One background check of `account`: due operations and a full sync, stopped at `deadline`
    /// (a timeout is not an error: `timed_out`), then the Inbox messages to notify about. The
    /// first check of an account (no marker yet), or one after the Inbox was renumbered, stores
    /// a baseline and returns none, so the mail already there never notifies. `demo` (the sample
    /// mailbox) never notifies. **Waits for the network**: for the background job only.
    pub async fn sync_once(
        &self,
        account: AccountId,
        deadline: Duration,
        demo: bool,
    ) -> Result<SyncOnce, EngineError> {
        let timed_out = match tokio::time::timeout(deadline, self.run_once(account, true)).await {
            Ok(r) => {
                r?;
                false
            }
            Err(_) => true,
        };
        // A sync cut short stored only some of the new headers (the newest first): deciding now
        // would move the marker past older new mail that arrives next time and never notifies.
        // Nothing is decided until a sync finishes.
        let new_mail = if timed_out {
            Vec::new()
        } else {
            let shared = self.shared().clone();
            let q = shared.lock();
            new_mail(q.store().db(), account, demo).map_err(EngineError::Storage)?
        };
        Ok(SyncOnce {
            new_mail,
            timed_out,
        })
    }

    /// The app posted notifications for these UIDs of the account's Inbox: the marker moves up
    /// to the highest of them (never back). When the Inbox was renumbered meanwhile (another
    /// UIDVALIDITY than the marker's) the UIDs belong to the old numbering and are ignored.
    pub fn mark_notified(&self, account: AccountId, uids: &[u32]) -> Result<(), EngineError> {
        let shared = self.shared().clone();
        let q = shared.lock();
        let conn = q.store().db().connection();
        let Some(stored) = store::marker(conn, account).map_err(EngineError::Storage)? else {
            return Ok(());
        };
        match inbox(q.store().db(), account).map_err(EngineError::Storage)? {
            Some((_, validity)) if validity == stored.uid_validity => {}
            _ => return Ok(()),
        }
        let next = notify::advance(
            Marker {
                uid_validity: stored.uid_validity,
                up_to: stored.up_to,
            },
            uids,
        );
        store::set_marker(
            conn,
            account,
            StoredMarker {
                uid_validity: next.uid_validity,
                up_to: next.up_to,
            },
            0,
        )
        .map_err(EngineError::Storage)
    }

    /// Whether `message_id` is still the Inbox message with `uid` of `account`. A notification's
    /// button and tap check this first: a row id can be reused once its message is gone, and a
    /// stale notification must never act on another message.
    pub fn notified_message_is(
        &self,
        account: AccountId,
        message_id: i64,
        uid: u32,
    ) -> Result<bool, EngineError> {
        let shared = self.shared().clone();
        let q = shared.lock();
        let db = q.store().db();
        let Some((folder, _)) = inbox(db, account).map_err(EngineError::Storage)? else {
            return Ok(false);
        };
        store::is_message(db.connection(), account, folder, message_id, uid)
            .map_err(EngineError::Storage)
    }

    /// Forgets the marker of a removed account.
    pub fn forget_notified(&self, account: AccountId) -> Result<(), EngineError> {
        let shared = self.shared().clone();
        let q = shared.lock();
        store::remove_marker(q.store().db().connection(), account)
            .map(|_| ())
            .map_err(EngineError::Storage)
    }
}

/// The decision for the account's Inbox, storing a baseline when there is no usable marker.
fn new_mail(db: &Db, account: AccountId, demo: bool) -> Result<Vec<NewMail>, StorageError> {
    let Some((folder, uid_validity)) = inbox(db, account)? else {
        return Ok(Vec::new());
    };
    let marker = store::marker(db.connection(), account)?.map(|m| Marker {
        uid_validity: m.uid_validity,
        up_to: m.up_to,
    });
    // Only rows above the marker are read (the whole Inbox only to find a baseline's newest UID).
    let above = marker
        .filter(|m| m.uid_validity == uid_validity)
        .map_or(0, |m| m.up_to);
    let rows: Vec<InboxRow> = store::rows_above(db.connection(), account, folder, above)?
        .into_iter()
        .map(|(id, uid, seen)| InboxRow { id, uid, seen })
        .collect();
    match notify::decide(&rows, uid_validity, marker, demo) {
        Decision::Baseline(m) => {
            store::set_marker(
                db.connection(),
                account,
                StoredMarker {
                    uid_validity: m.uid_validity,
                    up_to: m.up_to,
                },
                0,
            )?;
            Ok(Vec::new())
        }
        Decision::Notify(rows) => {
            let mut out = Vec::with_capacity(rows.len());
            for r in rows {
                if let Some((from_name, from_addr, subject, date_ms)) =
                    store::shown(db.connection(), account, r.id)?
                {
                    out.push(NewMail {
                        message_id: r.id,
                        uid: r.uid,
                        from_name,
                        from_addr,
                        subject,
                        date_ms,
                    });
                }
            }
            Ok(out)
        }
    }
}
