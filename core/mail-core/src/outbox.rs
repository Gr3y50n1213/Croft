// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The Outbox: messages the person has sent but the server has not yet taken.
//!
//! This is the pure state machine (no I/O, no clock: time is passed in as Unix milliseconds), in
//! the style of [`OpQueue`](crate::ops::OpQueue). [`crate::outbox_store`] adds persistence.
//!
//! The machine tracks everything about a message except its bytes. The MIME text is opaque to
//! every rule here, and a few large attachments held in memory (and copied for rollback) would
//! cost far more than they are worth, so the bytes live in the store and are read when a message
//! is handed out.
//!
//! # States
//!
//! ```text
//!            undo (inside the window)
//!   queue ──► Queued ───────────────────────────────► (removed)
//!               │ take_due                  edit / discard
//!               ▼                           (Queued or Failed)
//!            Sending ── failed(Transient) ──► Queued   (later, with back-off)
//!               │   ╲── failed(Auth) ───────► Queued   (account paused)
//!               │    ╲─ failed(Permanent) ──► Failed ──► retry ──► Queued
//!               │ submitted                    ▲
//!               ▼                              │ restart while Sending
//!           Submitted ── take_filing ──► Filing ── filed ──► Sent (removed)
//!                 ▲                        │
//!                 └── filing_failed(Transient / Auth) / restart while Filing
//! ```
//!
//! * **Queued**: waiting for its time (`send_at`: the end of the Undo window, or the retry time)
//!   or for the network.
//! * **Sending**: handed out; the server's answer is awaited. Written to disk *before* the
//!   message is submitted (see [`crate::outbox_store::PersistentOutbox::take_due`]).
//! * **Submitted**: the server accepted the message. What is left is the copy in the Sent folder,
//!   unless the server files it itself. This is its own state so a crash between the two steps
//!   neither sends the message again nor forgets the copy.
//! * **Filing**: the copy is being appended to Sent.
//! * **Sent**: finished; the row is removed by the persistent layer.
//! * **Failed**: rejected or out of retries, with a reason the Outbox screen shows. Retry, Edit
//!   (back to a draft) and Discard are the three ways out.
//!
//! # Never twice
//!
//! The one dangerous window is a crash after the server accepted the message and before the row
//! was marked [`Submitted`](OutboxState::Submitted). A message found in [`Sending`](OutboxState::Sending)
//! after a restart is therefore **not** sent again: [`Outbox::recover_after_restart`] moves it to
//! [`Failed`](OutboxState::Failed) with [`REASON_MAY_NOT_HAVE_BEEN_SENT`] and the person decides,
//! after looking in Sent. This trades a rare manual Retry for never sending a message twice. The
//! SMTP adapter makes the same choice when the server's answer to a message is lost
//! ([`croft_protocols::smtp::MAYBE_SENT_REASON`]). A message in [`Filing`](OutboxState::Filing)
//! after a restart goes back to Submitted: appending again is harmless (the engine looks for the
//! Message-ID in Sent first).
//!
//! # Retry, pause, offline
//!
//! A transient failure retries with the same back-off as [`crate::ops`] ([`backoff_ms`]) up to
//! [`MAX_ATTEMPTS`] times, and holds the account's other messages until then. A sign-in failure
//! (`Auth`) puts the message back in the queue without counting the attempt and pauses the
//! account's outgoing mail until [`Outbox::resume`]. While an account is offline nothing is
//! handed out for it, so being offline never uses up an attempt.

use croft_protocols::smtp::MAYBE_SENT_REASON;
use croft_protocols::BackendError;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

pub use crate::ops::{backoff_ms, AccountId, MAX_ATTEMPTS};

/// Identifier of an Outbox message. Increasing, and never reused (the store keeps a high-water
/// mark), so an old Undo button can never act on a newer message.
pub type OutboxId = u64;

/// Shown when a message was being sent when the app stopped, so nobody can say whether the server
/// took it. The same wording as the SMTP adapter uses when its connection drops after the message
/// body.
pub const REASON_MAY_NOT_HAVE_BEEN_SENT: &str = MAYBE_SENT_REASON;

/// Reason recorded when the copy in Sent could not be saved and Croft gave up on it.
pub const REASON_NOT_FILED: &str = "Sent, but a copy could not be saved to the Sent folder";

/// Reason recorded when a stored message cannot be read back.
pub const REASON_UNREADABLE: &str = "This message could not be read and was not sent";

/// Where a message is in its life. See the module notes for the diagram.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutboxState {
    /// Waiting for `send_at` or for the network.
    Queued,
    /// Handed to the SMTP sender; the answer is awaited.
    Sending,
    /// The server accepted it; the copy in Sent is still to be saved.
    Submitted,
    /// The copy is being appended to Sent.
    Filing,
    /// Done. Removed by the persistent layer.
    Sent,
    /// Rejected or out of retries. Retry, Edit or Discard.
    Failed,
}

impl OutboxState {
    /// True for the states the Outbox screen (S26) lists: Queued, Sending and Failed. A message
    /// that the server has accepted is not listed, even while its copy is being filed.
    pub fn shows_in_outbox(self) -> bool {
        matches!(self, Self::Queued | Self::Sending | Self::Failed)
    }
}

/// What the caller supplies for a new message. The bytes go to the store separately.
#[derive(Clone, PartialEq, Eq)]
pub struct NewMessage {
    /// The account that sends it.
    pub account: AccountId,
    /// The identity it is sent as (`identity.id`), if any.
    pub identity: Option<i64>,
    /// The envelope sender: a bare address.
    pub envelope_from: String,
    /// The envelope recipients: bare addresses, To, Cc and Bcc together. Bcc recipients are here
    /// and not in the headers.
    pub recipients: Vec<String>,
}

impl fmt::Debug for NewMessage {
    /// Never prints addresses: only the account, identity and the number of recipients.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NewMessage")
            .field("account", &self.account)
            .field("identity", &self.identity)
            .field("recipients", &self.recipients.len())
            .finish()
    }
}

/// One message in the Outbox, without its bytes.
#[derive(Clone, PartialEq, Eq)]
pub struct OutboxEntry {
    /// Id, for Undo, Retry, Edit and Discard.
    pub id: OutboxId,
    /// Owning account.
    pub account: AccountId,
    /// The identity it is sent as.
    pub identity: Option<i64>,
    /// Envelope sender.
    pub envelope_from: String,
    /// Envelope recipients.
    pub recipients: Vec<String>,
    /// Current state.
    pub state: OutboxState,
    /// Earliest time the next step may start: the end of the Undo window for a new message, the
    /// retry time after a transient failure.
    pub send_at_ms: i64,
    /// Times it has been handed out (for filing: times filing was tried).
    pub attempts: u32,
    /// The reason it failed, or the last problem, in plain words. Never contains server text or
    /// an address.
    pub last_error: Option<String>,
}

impl fmt::Debug for OutboxEntry {
    /// Never prints addresses: ids, state, attempts and the recipient count only.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OutboxEntry")
            .field("id", &self.id)
            .field("account", &self.account)
            .field("identity", &self.identity)
            .field("state", &self.state)
            .field("recipients", &self.recipients.len())
            .field("send_at_ms", &self.send_at_ms)
            .field("attempts", &self.attempts)
            .field("has_error", &self.last_error.is_some())
            .finish()
    }
}

impl OutboxEntry {
    /// True for a failed message that the server may have accepted: the person should look in
    /// Sent before choosing Retry.
    pub fn may_have_been_sent(&self) -> bool {
        self.state == OutboxState::Failed
            && self.last_error.as_deref() == Some(REASON_MAY_NOT_HAVE_BEEN_SENT)
    }
}

/// Why [`Outbox::undo`] refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UndoError {
    /// No such message.
    NotFound,
    /// The Undo window has closed, or the message already started to go out.
    TooLate,
}

/// Why Retry, Edit or Discard refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActionError {
    /// No such message.
    NotFound,
    /// The message is not in a state that allows it (for example, it is already being sent: it
    /// cannot be taken back).
    WrongState,
}

/// What [`Outbox::failed`] decided.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SendRejection {
    /// Will be tried again at this time.
    RetryAt(i64),
    /// Gave up. The message is Failed with a reason; tell the person once.
    Failed,
    /// Sign-in problem: the account's outgoing mail is paused until [`Outbox::resume`].
    AccountNeedsAttention,
}

/// What [`Outbox::filing_failed`] decided. The message was sent in every case: this is only about
/// its copy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilingRejection {
    /// Will be tried again at this time.
    RetryAt(i64),
    /// Gave up on the copy ([`REASON_NOT_FILED`]). The row is finished; tell the person.
    GaveUp,
    /// Sign-in problem: paused until [`Outbox::resume`].
    AccountNeedsAttention,
}

/// The Outbox. Lives in memory; [`crate::outbox_store::PersistentOutbox`] keeps it in a store.
#[derive(Default, Clone)]
pub struct Outbox {
    entries: BTreeMap<OutboxId, OutboxEntry>,
    next_id: OutboxId,
    /// Accounts waiting for the person to sign in again.
    paused: BTreeSet<AccountId>,
    /// Accounts with no connection: nothing is handed out, so no attempt is used up.
    offline: BTreeSet<AccountId>,
    /// After a transient SMTP failure, nothing more is sent for the account before this time.
    send_holds: BTreeMap<AccountId, i64>,
    /// The same for filing copies in Sent (a different server, so a separate hold).
    file_holds: BTreeMap<AccountId, i64>,
}

impl fmt::Debug for Outbox {
    /// Never prints messages: counts only.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Outbox")
            .field("entries", &self.entries.len())
            .field("next_id", &self.next_id)
            .field("paused_accounts", &self.paused.len())
            .field("offline_accounts", &self.offline.len())
            .finish()
    }
}

impl Outbox {
    /// An empty Outbox.
    pub fn new() -> Self {
        Self {
            next_id: 1,
            ..Self::default()
        }
    }

    /// Rebuilds the Outbox from stored entries. The next id is one above the largest of `max_id`
    /// (the largest id the store ever held, which may belong to a row that is gone) and the ids
    /// given. Offline flags, pauses and back-off are not stored: the engine sets them again.
    ///
    /// Entries still in [`OutboxState::Sending`] are kept as they are; call
    /// [`Outbox::recover_after_restart`] to deal with them.
    pub fn from_parts(entries: Vec<OutboxEntry>, max_id: OutboxId) -> Self {
        let next_id = entries
            .iter()
            .map(|e| e.id)
            .max()
            .unwrap_or(0)
            .max(max_id)
            .saturating_add(1);
        Self {
            entries: entries.into_iter().map(|e| (e.id, e)).collect(),
            next_id: next_id.max(1),
            ..Self::default()
        }
    }

    /// Looks one message up.
    pub fn get(&self, id: OutboxId) -> Option<&OutboxEntry> {
        self.entries.get(&id)
    }

    /// Every message, in id order.
    pub fn iter(&self) -> impl Iterator<Item = &OutboxEntry> {
        self.entries.values()
    }

    /// The messages the Outbox screen lists for an account (Queued, Sending, Failed), oldest first.
    pub fn visible(&self, account: AccountId) -> Vec<&OutboxEntry> {
        self.entries
            .values()
            .filter(|e| e.account == account && e.state.shows_in_outbox())
            .collect()
    }

    /// Number of messages that are not finished.
    pub fn unfinished(&self) -> usize {
        self.entries
            .values()
            .filter(|e| e.state != OutboxState::Sent)
            .count()
    }

    /// Records a message as queued and returns its id: it becomes due when the Undo window ends,
    /// `now_ms + undo_window_ms`. Keep the id for Undo.
    pub fn queue(&mut self, message: NewMessage, now_ms: i64, undo_window_ms: i64) -> OutboxId {
        let id = self.next_id;
        self.next_id = self.next_id.saturating_add(1);
        self.entries.insert(
            id,
            OutboxEntry {
                id,
                account: message.account,
                identity: message.identity,
                envelope_from: message.envelope_from,
                recipients: message.recipients,
                state: OutboxState::Queued,
                send_at_ms: now_ms.saturating_add(undo_window_ms.max(0)),
                attempts: 0,
                last_error: None,
            },
        );
        id
    }

    /// Undo: takes a message back before anything reaches the server. Only a [`Queued`](OutboxState::Queued)
    /// message that is not yet due can be undone; the entry is removed and returned so the caller
    /// can reopen the draft. Once `now_ms` reaches `send_at`, the window is closed.
    pub fn undo(&mut self, id: OutboxId, now_ms: i64) -> Result<OutboxEntry, UndoError> {
        let entry = self.entries.get(&id).ok_or(UndoError::NotFound)?;
        if entry.state != OutboxState::Queued || now_ms >= entry.send_at_ms {
            return Err(UndoError::TooLate);
        }
        self.entries.remove(&id).ok_or(UndoError::NotFound)
    }

    /// True when the account's outgoing mail may be handed out at `now_ms`: it is online, not
    /// waiting for a sign-in, and past any back-off.
    fn may_send(&self, account: AccountId, now_ms: i64) -> bool {
        !self.paused.contains(&account)
            && !self.offline.contains(&account)
            && self.send_holds.get(&account).is_none_or(|t| now_ms >= *t)
    }

    fn may_file(&self, account: AccountId, now_ms: i64) -> bool {
        !self.paused.contains(&account)
            && !self.offline.contains(&account)
            && self.file_holds.get(&account).is_none_or(|t| now_ms >= *t)
    }

    /// Ids of messages that [`Outbox::take_due`] would hand out now, oldest first, at most `limit`.
    pub fn due(&self, now_ms: i64, limit: usize) -> Vec<OutboxId> {
        self.due_matching(None, now_ms, limit)
    }

    /// Like [`Outbox::due`], for one account only: what [`Outbox::take_due_for`] would hand out.
    pub fn due_for(&self, account: AccountId, now_ms: i64, limit: usize) -> Vec<OutboxId> {
        self.due_matching(Some(account), now_ms, limit)
    }

    fn due_matching(&self, only: Option<AccountId>, now_ms: i64, limit: usize) -> Vec<OutboxId> {
        let mut due: Vec<&OutboxEntry> = self
            .entries
            .values()
            .filter(|e| {
                only.is_none_or(|a| a == e.account)
                    && e.state == OutboxState::Queued
                    && e.send_at_ms <= now_ms
                    && self.may_send(e.account, now_ms)
            })
            .collect();
        due.sort_by_key(|e| (e.send_at_ms, e.id));
        due.into_iter().take(limit).map(|e| e.id).collect()
    }

    /// Hands out up to `limit` messages that are due (Queued, `send_at` reached, account online,
    /// not paused, not backing off), oldest first. Each becomes [`Sending`](OutboxState::Sending)
    /// and its attempt count goes up by one. A message is handed out once: until the caller
    /// reports the outcome it is Sending and is never returned again.
    pub fn take_due(&mut self, now_ms: i64, limit: usize) -> Vec<OutboxEntry> {
        let ids = self.due(now_ms, limit);
        self.take(&ids)
    }

    /// Like [`Outbox::take_due`], but only for `account`: another account's messages are never
    /// handed out, whatever their state.
    pub fn take_due_for(
        &mut self,
        account: AccountId,
        now_ms: i64,
        limit: usize,
    ) -> Vec<OutboxEntry> {
        let ids = self.due_for(account, now_ms, limit);
        self.take(&ids)
    }

    /// Marks the given Queued messages as Sending (attempt count up by one) and returns them.
    /// Ids that are not Queued are skipped. [`Outbox::due`] says which ids may be taken.
    pub fn take(&mut self, ids: &[OutboxId]) -> Vec<OutboxEntry> {
        let mut out = Vec::new();
        for id in ids {
            if let Some(e) = self
                .entries
                .get_mut(id)
                .filter(|e| e.state == OutboxState::Queued)
            {
                e.state = OutboxState::Sending;
                e.attempts += 1;
                out.push(e.clone());
            }
        }
        out
    }

    /// The stored bytes of a message cannot be found or read. A Queued message becomes Failed
    /// with [`REASON_UNREADABLE`] (it cannot be sent, but the person is told); a Submitted one is
    /// finished without its copy in Sent ([`REASON_NOT_FILED`]). Other states are left alone.
    pub fn give_up_unreadable(&mut self, id: OutboxId) {
        if let Some(e) = self.entries.get_mut(&id) {
            match e.state {
                OutboxState::Queued => {
                    e.state = OutboxState::Failed;
                    e.last_error = Some(REASON_UNREADABLE.to_owned());
                }
                OutboxState::Submitted => {
                    e.state = OutboxState::Sent;
                    e.last_error = Some(REASON_NOT_FILED.to_owned());
                }
                _ => {}
            }
        }
    }

    /// The server accepted the message. It becomes [`Submitted`](OutboxState::Submitted): the
    /// copy in Sent is next (or, when the server files it itself, call [`Outbox::filed`] at once).
    /// Returns false when the message is not Sending.
    pub fn submitted(&mut self, id: OutboxId, now_ms: i64) -> bool {
        let Some(e) = self
            .entries
            .get_mut(&id)
            .filter(|e| e.state == OutboxState::Sending)
        else {
            return false;
        };
        e.state = OutboxState::Submitted;
        e.attempts = 0;
        e.send_at_ms = now_ms;
        e.last_error = None;
        self.send_holds.remove(&e.account);
        true
    }

    /// Sending failed. Decides between retrying, giving up and pausing the account. `None` when
    /// the message is not Sending.
    ///
    /// - `Transient`: back to Queued with the back-off time, until [`MAX_ATTEMPTS`] attempts have
    ///   been used; then Failed. The account's other messages wait for the same time.
    /// - `Auth`: back to Queued, the attempt not counted, the account paused. Nothing reached the
    ///   server (a sign-in failure comes before any message is sent).
    /// - `Permanent`: Failed with the adapter's reason.
    pub fn failed(
        &mut self,
        id: OutboxId,
        error: &BackendError,
        now_ms: i64,
    ) -> Option<SendRejection> {
        let e = self
            .entries
            .get_mut(&id)
            .filter(|e| e.state == OutboxState::Sending)?;
        match error {
            BackendError::Auth(m) => {
                e.state = OutboxState::Queued;
                e.attempts = e.attempts.saturating_sub(1);
                e.last_error = Some(m.clone());
                self.paused.insert(e.account);
                Some(SendRejection::AccountNeedsAttention)
            }
            BackendError::Transient(m) if e.attempts < MAX_ATTEMPTS => {
                let at = now_ms.saturating_add(backoff_ms(e.attempts));
                e.state = OutboxState::Queued;
                e.send_at_ms = at;
                e.last_error = Some(m.clone());
                let hold = self.send_holds.entry(e.account).or_insert(at);
                *hold = (*hold).max(at);
                Some(SendRejection::RetryAt(at))
            }
            BackendError::Transient(m) => {
                e.state = OutboxState::Failed;
                e.last_error = Some(format!("Gave up after {} tries: {m}", e.attempts));
                Some(SendRejection::Failed)
            }
            BackendError::Permanent(m) => {
                e.state = OutboxState::Failed;
                e.last_error = Some(m.clone());
                Some(SendRejection::Failed)
            }
        }
    }

    /// Hands out up to `limit` messages whose copy in Sent is due to be saved (Submitted, time
    /// reached, account online, not paused, not backing off). Each becomes
    /// [`Filing`](OutboxState::Filing) and its attempt count goes up by one.
    pub fn take_filing(&mut self, now_ms: i64, limit: usize) -> Vec<OutboxEntry> {
        let ids = self.filing_due(now_ms, limit);
        self.take_for_filing(&ids)
    }

    /// Like [`Outbox::take_filing`], but only for `account`.
    pub fn take_filing_for(
        &mut self,
        account: AccountId,
        now_ms: i64,
        limit: usize,
    ) -> Vec<OutboxEntry> {
        let ids = self.filing_due_for(account, now_ms, limit);
        self.take_for_filing(&ids)
    }

    /// Ids of messages that [`Outbox::take_filing`] would hand out now, oldest first.
    pub fn filing_due(&self, now_ms: i64, limit: usize) -> Vec<OutboxId> {
        self.filing_due_matching(None, now_ms, limit)
    }

    /// Like [`Outbox::filing_due`], for one account only.
    pub fn filing_due_for(&self, account: AccountId, now_ms: i64, limit: usize) -> Vec<OutboxId> {
        self.filing_due_matching(Some(account), now_ms, limit)
    }

    fn filing_due_matching(
        &self,
        only: Option<AccountId>,
        now_ms: i64,
        limit: usize,
    ) -> Vec<OutboxId> {
        let mut ids: Vec<(i64, OutboxId)> = self
            .entries
            .values()
            .filter(|e| {
                only.is_none_or(|a| a == e.account)
                    && e.state == OutboxState::Submitted
                    && e.send_at_ms <= now_ms
                    && self.may_file(e.account, now_ms)
            })
            .map(|e| (e.send_at_ms, e.id))
            .collect();
        ids.sort_unstable();
        ids.into_iter().take(limit).map(|(_, id)| id).collect()
    }

    /// Marks the given Submitted messages as Filing (try count up by one) and returns them. Ids
    /// that are not Submitted are skipped.
    pub fn take_for_filing(&mut self, ids: &[OutboxId]) -> Vec<OutboxEntry> {
        let mut out = Vec::new();
        for id in ids {
            if let Some(e) = self
                .entries
                .get_mut(id)
                .filter(|e| e.state == OutboxState::Submitted)
            {
                e.state = OutboxState::Filing;
                e.attempts += 1;
                out.push(e.clone());
            }
        }
        out
    }

    /// The copy is in Sent (or the server files it itself): the message is finished. Accepts a
    /// Filing message, or a Submitted one when no copy is needed. Returns false otherwise.
    pub fn filed(&mut self, id: OutboxId) -> bool {
        let Some(e) = self
            .entries
            .get_mut(&id)
            .filter(|e| matches!(e.state, OutboxState::Filing | OutboxState::Submitted))
        else {
            return false;
        };
        e.state = OutboxState::Sent;
        e.last_error = None;
        self.file_holds.remove(&e.account);
        true
    }

    /// Saving the copy failed. The message itself was sent, so it never goes back to Queued and
    /// is never sent again. `None` when the message is not Filing.
    ///
    /// - `Transient`: back to Submitted with the back-off time, until [`MAX_ATTEMPTS`] tries.
    /// - `Auth`: back to Submitted, the try not counted, the account paused.
    /// - `Permanent`, or out of tries: the message is finished with [`REASON_NOT_FILED`]; the
    ///   caller tells the person once.
    pub fn filing_failed(
        &mut self,
        id: OutboxId,
        error: &BackendError,
        now_ms: i64,
    ) -> Option<FilingRejection> {
        let e = self
            .entries
            .get_mut(&id)
            .filter(|e| e.state == OutboxState::Filing)?;
        match error {
            BackendError::Auth(_) => {
                e.state = OutboxState::Submitted;
                e.attempts = e.attempts.saturating_sub(1);
                self.paused.insert(e.account);
                Some(FilingRejection::AccountNeedsAttention)
            }
            BackendError::Transient(_) if e.attempts < MAX_ATTEMPTS => {
                let at = now_ms.saturating_add(backoff_ms(e.attempts));
                e.state = OutboxState::Submitted;
                e.send_at_ms = at;
                let hold = self.file_holds.entry(e.account).or_insert(at);
                *hold = (*hold).max(at);
                Some(FilingRejection::RetryAt(at))
            }
            BackendError::Transient(_) | BackendError::Permanent(_) => {
                e.state = OutboxState::Sent;
                e.last_error = Some(REASON_NOT_FILED.to_owned());
                Some(FilingRejection::GaveUp)
            }
        }
    }

    /// Retry on a Failed message: back to Queued, due now, with a fresh set of attempts. After
    /// [`REASON_MAY_NOT_HAVE_BEEN_SENT`] this is the person's decision that it was not sent.
    pub fn retry(&mut self, id: OutboxId, now_ms: i64) -> Result<(), ActionError> {
        let e = self.entries.get_mut(&id).ok_or(ActionError::NotFound)?;
        if e.state != OutboxState::Failed {
            return Err(ActionError::WrongState);
        }
        e.state = OutboxState::Queued;
        e.send_at_ms = now_ms;
        e.attempts = 0;
        e.last_error = None;
        Ok(())
    }

    /// Gives up on a message that is Sending without knowing whether the server took it: it
    /// becomes Failed with [`REASON_MAY_NOT_HAVE_BEEN_SENT`]. It never goes back to Queued.
    pub fn abandon(&mut self, id: OutboxId) -> Result<(), ActionError> {
        let e = self.entries.get_mut(&id).ok_or(ActionError::NotFound)?;
        if e.state != OutboxState::Sending {
            return Err(ActionError::WrongState);
        }
        e.state = OutboxState::Failed;
        e.last_error = Some(REASON_MAY_NOT_HAVE_BEEN_SENT.to_owned());
        Ok(())
    }

    /// Edit: takes a Queued or Failed message out of the Outbox so it can become a draft again.
    /// The entry is removed and returned (the persistent layer returns the bytes with it). A
    /// message that is being sent, or has been, cannot be taken back.
    pub fn edit_back_to_draft(&mut self, id: OutboxId) -> Result<OutboxEntry, ActionError> {
        self.take_out(id)
    }

    /// Discard: drops a Queued or Failed message.
    pub fn discard(&mut self, id: OutboxId) -> Result<OutboxEntry, ActionError> {
        self.take_out(id)
    }

    fn take_out(&mut self, id: OutboxId) -> Result<OutboxEntry, ActionError> {
        let state = self.entries.get(&id).ok_or(ActionError::NotFound)?.state;
        if !matches!(state, OutboxState::Queued | OutboxState::Failed) {
            return Err(ActionError::WrongState);
        }
        self.entries.remove(&id).ok_or(ActionError::NotFound)
    }

    /// After a crash or kill. A message that was Sending **is not sent again**: the server may
    /// have taken it, so it becomes Failed with [`REASON_MAY_NOT_HAVE_BEEN_SENT`] and the person
    /// decides. A message that was Filing goes back to Submitted. Returns the ids that became
    /// Failed, so the caller can tell the person once.
    pub fn recover_after_restart(&mut self) -> Vec<OutboxId> {
        let mut failed = Vec::new();
        for e in self.entries.values_mut() {
            match e.state {
                OutboxState::Sending => {
                    e.state = OutboxState::Failed;
                    e.last_error = Some(REASON_MAY_NOT_HAVE_BEEN_SENT.to_owned());
                    failed.push(e.id);
                }
                OutboxState::Filing => e.state = OutboxState::Submitted,
                _ => {}
            }
        }
        failed
    }

    /// Records whether the account has a connection. While offline nothing is handed out for it,
    /// so no message uses up an attempt just because the network is down.
    pub fn set_offline(&mut self, account: AccountId, offline: bool) {
        if offline {
            self.offline.insert(account);
        } else {
            self.offline.remove(&account);
        }
    }

    /// True while the account has no connection.
    pub fn is_offline(&self, account: AccountId) -> bool {
        self.offline.contains(&account)
    }

    /// Lets an account's outgoing mail run again after the person signed in.
    pub fn resume(&mut self, account: AccountId) {
        self.paused.remove(&account);
    }

    /// True while the account is waiting for the person to sign in.
    pub fn is_paused(&self, account: AccountId) -> bool {
        self.paused.contains(&account)
    }

    /// When the engine should next wake up to send or file something, or `None` if nothing is
    /// waiting (or everything waiting is held up by an offline or paused account, which wakes the
    /// engine by other means).
    pub fn next_wake_ms(&self) -> Option<i64> {
        self.next_wake_matching(None)
    }

    /// Like [`Outbox::next_wake_ms`], for one account only. This is what an account's task sleeps
    /// until. It follows the same rules as [`Outbox::take_due_for`] and
    /// [`Outbox::take_filing_for`], so it is never earlier than the time at which they would hand
    /// something out (a task that woke at a time where nothing can be handed out would spin):
    /// `None` while the account is paused (waiting for a sign-in) or offline, and a time never
    /// before the account's back-off hold.
    pub fn next_wake_ms_for(&self, account: AccountId) -> Option<i64> {
        self.next_wake_matching(Some(account))
    }

    fn next_wake_matching(&self, only: Option<AccountId>) -> Option<i64> {
        self.entries
            .values()
            .filter(|e| only.is_none_or(|a| a == e.account))
            .filter(|e| self.may_send_ignoring_time(e.account))
            .filter_map(|e| {
                let hold = match e.state {
                    OutboxState::Queued => self.send_holds.get(&e.account),
                    OutboxState::Submitted => self.file_holds.get(&e.account),
                    _ => return None,
                };
                Some(e.send_at_ms.max(hold.copied().unwrap_or(i64::MIN)))
            })
            .min()
    }

    fn may_send_ignoring_time(&self, account: AccountId) -> bool {
        !self.paused.contains(&account) && !self.offline.contains(&account)
    }

    /// Ends the account's back-off, for when the connection came back (a connection that was lost
    /// says nothing about how long the server needs). The hold is dropped, and so is the retry time
    /// of each message that was already tried (a Queued one handed out before, a Submitted one
    /// whose copy was tried before): they are due at `now_ms`. A message that has not been tried
    /// keeps its own time, so **an Undo window is never shortened**. Attempts already used stay
    /// used. Mirrors [`OpQueue::clear_hold`](crate::ops::OpQueue::clear_hold).
    pub fn clear_hold(&mut self, account: AccountId, now_ms: i64) {
        self.send_holds.remove(&account);
        self.file_holds.remove(&account);
        for e in self.entries.values_mut().filter(|e| {
            e.account == account
                && e.attempts > 0
                && matches!(e.state, OutboxState::Queued | OutboxState::Submitted)
        }) {
            e.send_at_ms = e.send_at_ms.min(now_ms);
        }
    }

    /// Drops finished ([`Sent`](OutboxState::Sent)) messages and returns their ids.
    pub fn purge_finished(&mut self) -> Vec<OutboxId> {
        let done: Vec<OutboxId> = self
            .entries
            .values()
            .filter(|e| e.state == OutboxState::Sent)
            .map(|e| e.id)
            .collect();
        for id in &done {
            self.entries.remove(id);
        }
        done
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    const A: AccountId = 1;
    const B: AccountId = 2;
    const WINDOW: i64 = 5_000;

    fn msg(account: AccountId) -> NewMessage {
        NewMessage {
            account,
            identity: Some(7),
            envelope_from: "me@example.org".into(),
            recipients: vec!["you@example.org".into()],
        }
    }

    fn transient() -> BackendError {
        BackendError::Transient("down".into())
    }

    fn state(o: &Outbox, id: OutboxId) -> OutboxState {
        o.get(id).unwrap().state
    }

    #[test]
    fn a_new_message_is_due_when_the_undo_window_ends() {
        let mut o = Outbox::new();
        let id = o.queue(msg(A), 1_000, WINDOW);
        assert_eq!(o.get(id).unwrap().send_at_ms, 6_000);
        assert!(o.take_due(5_999, 10).is_empty());
        let due = o.take_due(6_000, 10);
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].id, id);
        assert_eq!(state(&o, id), OutboxState::Sending);
        assert_eq!(o.get(id).unwrap().attempts, 1);
    }

    #[test]
    fn undo_inside_the_window_means_it_is_never_handed_out() {
        let mut o = Outbox::new();
        let id = o.queue(msg(A), 0, WINDOW);
        let removed = o.undo(id, 4_999).unwrap();
        assert_eq!(removed.id, id);
        assert!(o.get(id).is_none());
        assert!(o.take_due(1_000_000, 10).is_empty());
        assert_eq!(o.undo(id, 4_999), Err(UndoError::NotFound));
    }

    #[test]
    fn undo_is_too_late_once_the_window_closed_or_sending_began() {
        let mut o = Outbox::new();
        let id = o.queue(msg(A), 0, WINDOW);
        assert_eq!(o.undo(id, 5_000), Err(UndoError::TooLate));
        assert_eq!(o.undo(id, 9_000), Err(UndoError::TooLate));
        assert_eq!(
            state(&o, id),
            OutboxState::Queued,
            "a refused undo changes nothing"
        );
        o.take_due(5_000, 10);
        assert_eq!(o.undo(id, 5_001), Err(UndoError::TooLate));
        assert_eq!(state(&o, id), OutboxState::Sending);
    }

    #[test]
    fn a_zero_window_is_due_at_once() {
        let mut o = Outbox::new();
        let id = o.queue(msg(A), 100, 0);
        assert_eq!(o.take_due(100, 10).len(), 1);
        assert_eq!(state(&o, id), OutboxState::Sending);
        // A negative window (a bad setting) is treated as zero, never as a time in the past.
        let id2 = o.queue(msg(A), 100, -50);
        assert_eq!(o.get(id2).unwrap().send_at_ms, 100);
    }

    #[test]
    fn a_message_is_handed_out_exactly_once() {
        let mut o = Outbox::new();
        o.queue(msg(A), 0, 0);
        assert_eq!(o.take_due(0, 10).len(), 1);
        assert!(o.take_due(0, 10).is_empty());
        assert!(o.take_due(1_000_000, 10).is_empty());
    }

    #[test]
    fn offline_hands_out_nothing_and_counts_no_attempt_then_exactly_once_when_online() {
        let mut o = Outbox::new();
        let id = o.queue(msg(A), 0, WINDOW);
        o.set_offline(A, true);
        for t in [5_000, 60_000, 3_600_000] {
            assert!(o.take_due(t, 10).is_empty());
        }
        assert_eq!(o.get(id).unwrap().attempts, 0);
        assert_eq!(state(&o, id), OutboxState::Queued);
        assert_eq!(
            o.next_wake_ms(),
            None,
            "offline: the engine wakes on reconnect"
        );
        o.set_offline(A, false);
        assert_eq!(o.next_wake_ms(), Some(5_000));
        assert_eq!(o.take_due(3_700_000, 10).len(), 1);
        assert!(o.take_due(3_700_000, 10).is_empty());
        assert_eq!(o.get(id).unwrap().attempts, 1);
    }

    #[test]
    fn one_account_offline_does_not_hold_back_another() {
        let mut o = Outbox::new();
        let a = o.queue(msg(A), 0, 0);
        let b = o.queue(msg(B), 0, 0);
        o.set_offline(A, true);
        let due = o.take_due(0, 10);
        assert_eq!(due.iter().map(|e| e.id).collect::<Vec<_>>(), vec![b]);
        assert_eq!(state(&o, a), OutboxState::Queued);
    }

    #[test]
    fn the_limit_and_the_order_are_respected() {
        let mut o = Outbox::new();
        let first = o.queue(msg(A), 0, 100);
        let second = o.queue(msg(A), 0, 50);
        let third = o.queue(msg(A), 0, 100);
        let due = o.take_due(1_000, 2);
        assert_eq!(
            due.iter().map(|e| e.id).collect::<Vec<_>>(),
            vec![second, first],
            "earliest send_at first, then by id"
        );
        assert_eq!(o.take_due(1_000, 2)[0].id, third);
    }

    #[test]
    fn a_transient_failure_backs_off_like_the_operation_queue_and_holds_the_account() {
        let mut o = Outbox::new();
        let a = o.queue(msg(A), 0, 0);
        let other = o.queue(msg(A), 0, 0);
        let b = o.queue(msg(B), 0, 0);
        let first = o.take_due(0, 1);
        assert_eq!(first[0].id, a);
        assert_eq!(
            o.failed(a, &transient(), 1_000),
            Some(SendRejection::RetryAt(1_000 + backoff_ms(1)))
        );
        assert_eq!(state(&o, a), OutboxState::Queued);
        // The same account waits, another account does not.
        let due = o.take_due(1_001, 10);
        assert_eq!(due.iter().map(|e| e.id).collect::<Vec<_>>(), vec![b]);
        assert_eq!(state(&o, other), OutboxState::Queued);
        // After the back-off the account's messages are handed out again; the one that failed
        // goes behind the one that was waiting.
        let due = o.take_due(1_000 + backoff_ms(1), 10);
        assert_eq!(due.iter().map(|e| e.id).collect::<Vec<_>>(), vec![other, a]);
        assert_eq!(o.get(a).unwrap().attempts, 2);
    }

    #[test]
    fn transient_failures_give_up_after_max_attempts_with_a_reason() {
        let mut o = Outbox::new();
        let id = o.queue(msg(A), 0, 0);
        let mut now = 0;
        for attempt in 1..=MAX_ATTEMPTS {
            let due = o.take_due(now, 1);
            assert_eq!(due.len(), 1, "attempt {attempt}");
            let r = o.failed(id, &transient(), now).unwrap();
            if attempt < MAX_ATTEMPTS {
                let SendRejection::RetryAt(at) = r else {
                    panic!("attempt {attempt}: {r:?}")
                };
                now = at;
            } else {
                assert_eq!(r, SendRejection::Failed);
            }
        }
        let e = o.get(id).unwrap();
        assert_eq!(e.state, OutboxState::Failed);
        assert_eq!(e.last_error.as_deref(), Some("Gave up after 8 tries: down"));
        assert!(o.take_due(i64::MAX / 2, 10).is_empty());
    }

    #[test]
    fn a_sign_in_failure_pauses_the_account_and_costs_no_attempt() {
        let mut o = Outbox::new();
        let id = o.queue(msg(A), 0, 0);
        let other = o.queue(msg(B), 0, 0);
        o.take_due(0, 1);
        let r = o.failed(id, &BackendError::Auth("sign in".into()), 10);
        assert_eq!(r, Some(SendRejection::AccountNeedsAttention));
        assert!(o.is_paused(A));
        let e = o.get(id).unwrap();
        assert_eq!((e.state, e.attempts), (OutboxState::Queued, 0));
        // Nothing for A, however long we wait; B is unaffected.
        assert_eq!(
            o.take_due(1_000_000, 10)
                .iter()
                .map(|e| e.id)
                .collect::<Vec<_>>(),
            vec![other]
        );
        assert_eq!(o.next_wake_ms(), None);
        o.resume(A);
        assert_eq!(o.take_due(1_000_000, 10).len(), 1);
    }

    #[test]
    fn a_permanent_refusal_fails_with_the_reason_and_retry_edit_discard_work() {
        let mut o = Outbox::new();
        let id = o.queue(msg(A), 0, 0);
        o.take_due(0, 1);
        let refusal = BackendError::Permanent("The server refused a recipient address".into());
        assert_eq!(o.failed(id, &refusal, 5), Some(SendRejection::Failed));
        let e = o.get(id).unwrap();
        assert_eq!(e.state, OutboxState::Failed);
        assert_eq!(
            e.last_error.as_deref(),
            Some("The server refused a recipient address")
        );
        assert!(!e.may_have_been_sent());

        // Retry: queued again, due now, fresh attempts.
        o.retry(id, 100).unwrap();
        let e = o.get(id).unwrap();
        assert_eq!(
            (e.state, e.attempts, e.send_at_ms, e.last_error.clone()),
            (OutboxState::Queued, 0, 100, None)
        );
        assert_eq!(o.retry(id, 100), Err(ActionError::WrongState));
        assert_eq!(o.take_due(100, 1).len(), 1);

        // Edit and discard are refused while sending, allowed when failed.
        assert_eq!(
            o.edit_back_to_draft(id).unwrap_err(),
            ActionError::WrongState
        );
        assert_eq!(o.discard(id).unwrap_err(), ActionError::WrongState);
        o.failed(id, &refusal, 200);
        let back = o.edit_back_to_draft(id).unwrap();
        assert_eq!(back.id, id);
        assert!(o.get(id).is_none());
        assert_eq!(o.discard(id).unwrap_err(), ActionError::NotFound);
        let id2 = o.queue(msg(A), 0, 0);
        o.discard(id2).unwrap();
        assert_eq!(o.unfinished(), 0);
    }

    #[test]
    fn a_queued_message_waiting_for_the_network_can_be_edited_or_discarded() {
        let mut o = Outbox::new();
        let id = o.queue(msg(A), 0, 0);
        o.set_offline(A, true);
        assert!(o.edit_back_to_draft(id).is_ok());
    }

    #[test]
    fn submitted_then_filed_finishes_the_message() {
        let mut o = Outbox::new();
        let id = o.queue(msg(A), 0, 0);
        o.take_due(0, 1);
        assert!(o.submitted(id, 10));
        assert_eq!(state(&o, id), OutboxState::Submitted);
        assert!(!o.get(id).unwrap().state.shows_in_outbox());
        assert!(o.take_due(1_000_000, 10).is_empty(), "never sent twice");
        assert!(
            !o.submitted(id, 11),
            "only a Sending message can be submitted"
        );
        let filing = o.take_filing(10, 10);
        assert_eq!(filing.len(), 1);
        assert_eq!(state(&o, id), OutboxState::Filing);
        assert!(o.take_filing(10, 10).is_empty(), "handed out once");
        assert!(o.filed(id));
        assert_eq!(state(&o, id), OutboxState::Sent);
        assert_eq!(o.purge_finished(), vec![id]);
        assert_eq!(o.unfinished(), 0);
    }

    #[test]
    fn when_the_server_files_sent_itself_filed_follows_submitted_directly() {
        let mut o = Outbox::new();
        let id = o.queue(msg(A), 0, 0);
        o.take_due(0, 1);
        o.submitted(id, 1);
        assert!(o.filed(id));
        assert_eq!(state(&o, id), OutboxState::Sent);
        assert!(!o.filed(id));
    }

    #[test]
    fn a_failed_copy_never_resends_the_message() {
        let mut o = Outbox::new();
        let id = o.queue(msg(A), 0, 0);
        o.take_due(0, 1);
        o.submitted(id, 0);
        o.take_filing(0, 1);
        assert_eq!(
            o.filing_failed(id, &transient(), 100),
            Some(FilingRejection::RetryAt(100 + backoff_ms(1)))
        );
        assert_eq!(state(&o, id), OutboxState::Submitted);
        assert!(
            o.take_due(10_000_000, 10).is_empty(),
            "the message is not queued again"
        );
        assert!(o.take_filing(100, 10).is_empty(), "backing off");
        assert_eq!(o.take_filing(100 + backoff_ms(1), 10).len(), 1);
        // Sign-in problem pauses without counting.
        assert_eq!(
            o.filing_failed(id, &BackendError::Auth("x".into()), 200),
            Some(FilingRejection::AccountNeedsAttention)
        );
        assert!(o.is_paused(A));
        assert_eq!(o.get(id).unwrap().attempts, 1);
        assert!(o.take_filing(10_000_000, 10).is_empty());
        o.resume(A);
        assert_eq!(o.take_filing(10_000_000, 10).len(), 1);
        // A refusal gives up on the copy, and the message is finished, not failed.
        assert_eq!(
            o.filing_failed(id, &BackendError::Permanent("no folder".into()), 300),
            Some(FilingRejection::GaveUp)
        );
        let e = o.get(id).unwrap();
        assert_eq!(e.state, OutboxState::Sent);
        assert_eq!(e.last_error.as_deref(), Some(REASON_NOT_FILED));
    }

    #[test]
    fn filing_gives_up_after_max_attempts() {
        let mut o = Outbox::new();
        let id = o.queue(msg(A), 0, 0);
        o.take_due(0, 1);
        o.submitted(id, 0);
        let mut now = 0;
        for attempt in 1..=MAX_ATTEMPTS {
            assert_eq!(o.take_filing(now, 1).len(), 1, "attempt {attempt}");
            match o.filing_failed(id, &transient(), now).unwrap() {
                FilingRejection::RetryAt(at) => now = at,
                FilingRejection::GaveUp => assert_eq!(attempt, MAX_ATTEMPTS),
                other => panic!("{other:?}"),
            }
        }
        assert_eq!(state(&o, id), OutboxState::Sent);
    }

    #[test]
    fn a_restart_while_sending_never_resends_it() {
        let mut o = Outbox::new();
        let sending = o.queue(msg(A), 0, 0);
        let filing = o.queue(msg(A), 0, 0);
        let queued = o.queue(msg(A), 0, 10_000);
        let submitted = o.queue(msg(A), 0, 0);
        o.take_due(0, 3);
        o.submitted(filing, 0);
        o.take_filing(0, 1);
        o.submitted(submitted, 0);
        // `sending` was handed out and never reported; `filing` was mid-append.
        assert_eq!(state(&o, sending), OutboxState::Sending);
        assert_eq!(state(&o, filing), OutboxState::Filing);

        let mut restarted = Outbox::from_parts(o.iter().cloned().collect(), 0);
        let failed = restarted.recover_after_restart();
        assert_eq!(failed, vec![sending]);
        let e = restarted.get(sending).unwrap();
        assert_eq!(e.state, OutboxState::Failed);
        assert_eq!(e.last_error.as_deref(), Some(REASON_MAY_NOT_HAVE_BEEN_SENT));
        assert!(e.may_have_been_sent());
        assert_eq!(state(&restarted, filing), OutboxState::Submitted);
        assert_eq!(state(&restarted, queued), OutboxState::Queued);

        // Whatever the time, the message that may have been sent is not handed out again.
        let due: Vec<OutboxId> = restarted
            .take_due(i64::MAX / 2, 100)
            .iter()
            .map(|e| e.id)
            .collect();
        assert_eq!(due, vec![queued]);
        // The person looked in Sent and chose Retry.
        restarted.retry(sending, 50).unwrap();
        assert_eq!(state(&restarted, sending), OutboxState::Queued);
    }

    #[test]
    fn ids_are_never_reused_even_after_the_newest_row_is_gone() {
        let mut o = Outbox::new();
        let id = o.queue(msg(A), 0, 10_000);
        o.undo(id, 1).unwrap();
        assert_eq!(o.queue(msg(A), 0, 10_000), id + 1);
        // After a restart the store's high-water mark keeps the count going.
        let mut r = Outbox::from_parts(Vec::new(), 41);
        assert_eq!(r.queue(msg(A), 0, 0), 42);
    }

    #[test]
    fn the_outbox_screen_lists_queued_sending_and_failed_only() {
        let mut o = Outbox::new();
        let queued = o.queue(msg(A), 0, 10_000);
        let sending = o.queue(msg(A), 0, 0);
        let failed = o.queue(msg(A), 0, 0);
        let submitted = o.queue(msg(A), 0, 0);
        let other = o.queue(msg(B), 0, 0);
        o.take_due(0, 3);
        // Order of handing out: sending, failed, submitted (ids 2, 3, 4).
        o.failed(failed, &BackendError::Permanent("no".into()), 1);
        o.submitted(submitted, 1);
        let ids: Vec<_> = o.visible(A).iter().map(|e| e.id).collect();
        assert_eq!(ids, vec![queued, sending, failed]);
        assert_eq!(o.visible(B).len(), 1);
        assert_eq!(o.visible(B)[0].id, other);
    }

    #[test]
    fn next_wake_is_the_earliest_runnable_time() {
        let mut o = Outbox::new();
        assert_eq!(o.next_wake_ms(), None);
        o.queue(msg(A), 0, 9_000);
        o.queue(msg(A), 0, 3_000);
        assert_eq!(o.next_wake_ms(), Some(3_000));
        o.set_offline(A, true);
        assert_eq!(o.next_wake_ms(), None);
    }

    #[test]
    fn the_per_account_takes_hand_out_only_that_accounts_mail() {
        let mut o = Outbox::new();
        let a = o.queue(msg(A), 0, 0);
        let b = o.queue(msg(B), 0, 0);
        assert_eq!(o.due_for(A, 0, 10), vec![a]);
        assert_eq!(o.due_for(B, 0, 10), vec![b]);
        let got: Vec<_> = o.take_due_for(A, 0, 10).iter().map(|e| e.id).collect();
        assert_eq!(got, vec![a]);
        assert_eq!(state(&o, a), OutboxState::Sending);
        assert_eq!(state(&o, b), OutboxState::Queued, "B was not touched");
        assert!(o.take_due_for(A, 0, 10).is_empty());
        let got: Vec<_> = o.take_due_for(B, 0, 10).iter().map(|e| e.id).collect();
        assert_eq!(got, vec![b]);

        // The same for the copies in Sent.
        assert!(o.submitted(a, 5));
        assert!(o.submitted(b, 5));
        assert_eq!(o.filing_due_for(A, 5, 10), vec![a]);
        let got: Vec<_> = o.take_filing_for(B, 5, 10).iter().map(|e| e.id).collect();
        assert_eq!(got, vec![b]);
        assert_eq!(state(&o, a), OutboxState::Submitted, "A was not touched");
        assert_eq!(state(&o, b), OutboxState::Filing);
        let got: Vec<_> = o.take_filing_for(A, 5, 10).iter().map(|e| e.id).collect();
        assert_eq!(got, vec![a]);
    }

    #[test]
    fn a_per_account_take_follows_that_accounts_offline_pause_and_hold_only() {
        let mut o = Outbox::new();
        let a = o.queue(msg(A), 0, 0);
        let b = o.queue(msg(B), 0, 0);
        o.set_offline(B, true);
        assert!(o.take_due_for(B, 0, 10).is_empty());
        let got: Vec<_> = o.take_due_for(A, 0, 10).iter().map(|e| e.id).collect();
        assert_eq!(got, vec![a], "B being offline does not hold A back");
        assert_eq!(state(&o, b), OutboxState::Queued);
        assert_eq!(o.get(b).unwrap().attempts, 0);
    }

    #[test]
    fn the_wake_time_of_an_account_follows_its_own_pause_offline_and_hold() {
        let mut o = Outbox::new();
        let a = o.queue(msg(A), 0, 1_000);
        o.queue(msg(B), 0, 3_000);
        assert_eq!(o.next_wake_ms_for(A), Some(1_000));
        assert_eq!(o.next_wake_ms_for(B), Some(3_000));
        assert_eq!(o.next_wake_ms_for(9), None);

        // A sign-in failure: back to Queued with an old time, but paused: no wake time at all (a
        // task that woke for it would find nothing to hand out, and wake again at once).
        o.take_due_for(A, 1_000, 1);
        o.failed(a, &BackendError::Auth("no".into()), 1_500);
        assert!(o.is_paused(A));
        assert_eq!(o.get(a).unwrap().send_at_ms, 1_000);
        assert_eq!(o.next_wake_ms_for(A), None);
        assert_eq!(o.next_wake_ms_for(B), Some(3_000), "B is not affected");
        o.resume(A);
        assert_eq!(o.next_wake_ms_for(A), Some(1_000));

        o.set_offline(A, true);
        assert_eq!(o.next_wake_ms_for(A), None);
        o.set_offline(A, false);

        // A back-off hold moves the wake time to the hold.
        o.take_due_for(A, 1_000, 1);
        assert_eq!(
            o.failed(a, &transient(), 2_000),
            Some(SendRejection::RetryAt(2_000 + backoff_ms(1)))
        );
        assert_eq!(o.next_wake_ms_for(A), Some(2_000 + backoff_ms(1)));
        // Never earlier than what take_due_for would hand out.
        let wake = o.next_wake_ms_for(A).unwrap();
        assert!(o.take_due_for(A, wake - 1, 1).is_empty());
        assert_eq!(o.take_due_for(A, wake, 1).len(), 1);
    }

    #[test]
    fn clear_hold_makes_tried_messages_due_but_never_shortens_an_undo_window() {
        let mut o = Outbox::new();
        let tried = o.queue(msg(A), 0, 0);
        let fresh = o.queue(msg(A), 0, 60_000);
        let other = o.queue(msg(B), 0, 0);
        o.take_due_for(A, 0, 10);
        o.take_due_for(B, 0, 10);
        o.failed(tried, &transient(), 1_000);
        o.failed(other, &transient(), 1_000);
        let at = 1_000 + backoff_ms(1);
        assert_eq!(o.get(tried).unwrap().send_at_ms, at);
        assert!(o.take_due_for(A, at - 1, 10).is_empty(), "held");

        o.clear_hold(A, 2_000);
        assert_eq!(o.get(tried).unwrap().send_at_ms, 2_000);
        assert_eq!(o.get(tried).unwrap().attempts, 1, "attempts stay used");
        assert_eq!(
            o.get(fresh).unwrap().send_at_ms,
            60_000,
            "the Undo window is never shortened"
        );
        assert_eq!(o.get(other).unwrap().send_at_ms, at, "B keeps its back-off");
        assert_eq!(o.next_wake_ms_for(A), Some(2_000));
        let got: Vec<_> = o.take_due_for(A, 2_000, 10).iter().map(|e| e.id).collect();
        assert_eq!(got, vec![tried]);
        assert!(o.undo(fresh, 2_000).is_ok(), "still inside its window");
    }

    #[test]
    fn clear_hold_also_ends_the_hold_on_saving_copies() {
        let mut o = Outbox::new();
        let id = o.queue(msg(A), 0, 0);
        o.take_due_for(A, 0, 1);
        o.submitted(id, 0);
        o.take_filing_for(A, 0, 1);
        assert!(matches!(
            o.filing_failed(id, &transient(), 100),
            Some(FilingRejection::RetryAt(_))
        ));
        assert!(o.take_filing_for(A, 200, 1).is_empty());
        o.clear_hold(A, 200);
        assert_eq!(o.take_filing_for(A, 200, 1).len(), 1);
    }

    #[test]
    fn unknown_ids_are_refused_quietly() {
        let mut o = Outbox::new();
        assert_eq!(o.failed(9, &transient(), 0), None);
        assert_eq!(o.filing_failed(9, &transient(), 0), None);
        assert!(!o.submitted(9, 0));
        assert!(!o.filed(9));
        assert_eq!(o.retry(9, 0), Err(ActionError::NotFound));
        assert_eq!(o.undo(9, 0), Err(UndoError::NotFound));
    }
}
