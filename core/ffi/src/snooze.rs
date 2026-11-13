// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Snooze on this device (T36): a message put away until a wake time, then back in the Inbox,
//! unread. Without a Companion nothing reaches the server but the mark-unread at the wake time
//! (through the queue, like any other): other devices keep showing the message
//! (`docs/ux/server-contract.md`, "Snooze logic").

use croft_engine::Change;
use croft_storage::{accounts, messages};

use crate::core::{now_ms, Core};
use crate::error::CoreError;
use crate::list;
use crate::types::MessageRow;

/// A snoozed message for the Snoozed folder: its row and when it comes back.
#[derive(Clone, PartialEq, Eq, uniffi::Record)]
pub struct SnoozedMessage {
    /// The row.
    pub row: MessageRow,
    /// When it comes back, Unix milliseconds.
    pub wake_ms: i64,
}

impl std::fmt::Debug for SnoozedMessage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SnoozedMessage")
            .field("id", &self.row.id)
            .field("wake_ms", &self.wake_ms)
            .finish()
    }
}

impl Core {
    /// Sets (or clears) the snooze of each message; the folders touched are announced.
    fn set_snoozes(&self, ids: &[i64], until_ms: Option<i64>) -> Result<(), CoreError> {
        let found = self.inner.resolve(ids)?;
        self.inner.handle.read(|db| {
            for m in &found {
                messages::set_snooze(db.connection(), m.account_id, m.id, until_ms)?;
            }
            Ok::<_, CoreError>(())
        })?;
        for m in &found {
            self.inner.hub.apply(Change::Messages {
                account: m.account_id,
                folders: vec![m.folder_id],
            });
        }
        Ok(())
    }
}

#[uniffi::export]
impl Core {
    /// Snoozes messages until `until_ms`: they leave every list on this device until then (S29).
    /// [`CoreError::BadSnoozeTime`] for a time not in the future or more than a year ahead.
    pub fn snooze(&self, ids: Vec<i64>, until_ms: i64) -> Result<(), CoreError> {
        let now = now_ms();
        if until_ms <= now || until_ms - now > messages::MAX_SNOOZE_MS {
            return Err(CoreError::BadSnoozeTime);
        }
        self.set_snoozes(&ids, Some(until_ms))
    }

    /// Unsnoozes messages (Undo, or Unsnooze in the Snoozed folder): back in their folder at once,
    /// as they were.
    pub fn unsnooze(&self, ids: Vec<i64>) -> Result<(), CoreError> {
        self.set_snoozes(&ids, None)
    }

    /// The snoozed messages of `account`, or of every account (`None`), soonest first (the Snoozed
    /// folder).
    pub fn list_snoozed(&self, account: Option<i64>) -> Result<Vec<SnoozedMessage>, CoreError> {
        self.inner.handle.read(|db| {
            let conn = db.connection();
            let wanted: Vec<i64> = match account {
                Some(a) => vec![a],
                None => accounts::list(conn)?.into_iter().map(|a| a.id).collect(),
            };
            let mut out = Vec::new();
            for a in wanted {
                for (row, wake_ms) in list::snoozed_rows(conn, a, now_ms())? {
                    out.push(SnoozedMessage { row, wake_ms });
                }
            }
            out.sort_by_key(|m| (m.wake_ms, m.row.id));
            Ok(out)
        })
    }

    /// Wakes every message whose time has come: back in its folder, unread (queued like any
    /// mark-unread), with its Snoozed chip for a day. Returns their ids (for "Back in your inbox").
    /// Call it when the app starts, before the list first draws, and from the wake-up job.
    pub fn wake_due_snoozes(&self) -> Result<Vec<i64>, CoreError> {
        let now = now_ms();
        let woken = self.inner.handle.read(|db| {
            let conn = db.connection();
            let mut woken = Vec::new();
            for a in accounts::list(conn)? {
                for id in messages::due_snoozes(conn, a.id, now)? {
                    if messages::wake(conn, a.id, id, now)? {
                        woken.push((a.id, id));
                    }
                }
            }
            Ok::<_, CoreError>(woken)
        })?;
        let ids: Vec<i64> = woken.iter().map(|(_, id)| *id).collect();
        for &(account, id) in &woken {
            // Back unread; a message the engine no longer knows (removed meanwhile) just stays read.
            let _ = self.inner.handle.mark_seen(account, id, false);
        }
        if !ids.is_empty() {
            let found = self.inner.resolve(&ids).unwrap_or_default();
            for m in found {
                self.inner.hub.apply(Change::Messages {
                    account: m.account_id,
                    folders: vec![m.folder_id],
                });
            }
        }
        Ok(ids)
    }

    /// The soonest wake time of any snoozed message, if there is one (for the app's wake-up job).
    pub fn next_snooze_wake(&self) -> Result<Option<i64>, CoreError> {
        self.inner.handle.read(|db| {
            let conn = db.connection();
            let mut soonest: Option<i64> = None;
            for a in accounts::list(conn)? {
                if let Some(t) = messages::next_wake(conn, a.id)? {
                    soonest = Some(soonest.map_or(t, |s: i64| s.min(t)));
                }
            }
            Ok(soonest)
        })
    }
}
