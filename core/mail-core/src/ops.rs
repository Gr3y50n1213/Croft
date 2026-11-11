//! The offline action queue.
//!
//! Every change a person makes (mark read, star, archive, delete, move) is applied to the local
//! database straight away, so the screen answers instantly, and recorded here until the server
//! confirms it. The UX plan (`docs/ux/server-contract.md`) depends on these behaviours, all
//! implemented below:
//!
//! * **Delay per kind.** Moves and deletes wait out the person's Undo window (3, 5 or 10 s), and
//!   Undo cancels them if they have not become due, so "Undo" never has to reverse something the
//!   server already did. Read and star changes show no snackbar; they wait only
//!   [`FLAG_COALESCE_MS`] so a quick second swipe can cancel the first.
//! * **Ordering.** Operations on the same message run in the order they were made, and a move
//!   re-points the later ones at the message's new UID ([`OpQueue::confirm_moved`]).
//! * **Retry.** A transient failure backs the whole account off (5 s, 30 s, 2 min, 10 min, 1 h,
//!   then hourly); permanent ones fail so the screen can put the row back and say what happened;
//!   a sign-in failure pauses the account. An offline account is not tried at all, so being
//!   offline never uses up an operation's attempts.
//! * **Coalescing.** Mark read then mark unread inside the delay cancel out and send nothing.
//!
//! Times are Unix milliseconds passed in by the caller, so tests never sleep.

use croft_protocols::{BackendError, Uid};
use std::collections::{BTreeMap, BTreeSet};

/// Identifier of a queued operation.
pub type OpId = u64;
/// Identifier of an account, matching `account.id` in the database.
pub type AccountId = i64;
/// Where a message is: folder path and UID.
pub type Location = (String, Uid);

/// A transient answer from a connected server is retried this many times before the operation is
/// reported as failed. Time spent offline does not count.
pub const MAX_ATTEMPTS: u32 = 8;

/// How long a read, unread, star or unstar change waits before it is sent. There is no snackbar
/// for these, so this only has to be long enough for a second swipe to cancel the first.
pub const FLAG_COALESCE_MS: i64 = 1_500;

/// What the person asked for. One operation acts on one message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OpKind {
    /// Mark read or unread.
    SetSeen {
        /// Folder the message is in on the server.
        folder: String,
        /// UID in that folder.
        uid: Uid,
        /// New state: true means read.
        value: bool,
    },
    /// Star or unstar.
    SetFlagged {
        /// Folder the message is in on the server.
        folder: String,
        /// UID in that folder.
        uid: Uid,
        /// New state: true means starred.
        value: bool,
    },
    /// Move to another folder (archive, trash, spam, Move to).
    MoveTo {
        /// Folder the message is in on the server.
        folder: String,
        /// UID in that folder.
        uid: Uid,
        /// Destination folder path.
        dest: String,
    },
    /// Delete permanently.
    DeleteForever {
        /// Folder the message is in on the server.
        folder: String,
        /// UID in that folder.
        uid: Uid,
    },
}

impl OpKind {
    /// The folder and UID the operation acts on.
    pub fn target(&self) -> (&str, Uid) {
        match self {
            Self::SetSeen { folder, uid, .. }
            | Self::SetFlagged { folder, uid, .. }
            | Self::MoveTo { folder, uid, .. }
            | Self::DeleteForever { folder, uid } => (folder, *uid),
        }
    }

    /// How long the operation waits before it may run. Flag changes use the short coalescing
    /// delay; moves and deletes use the person's Undo window.
    pub fn delay_ms(&self, undo_window_ms: i64) -> i64 {
        match self {
            Self::SetSeen { .. } | Self::SetFlagged { .. } => FLAG_COALESCE_MS,
            Self::MoveTo { .. } | Self::DeleteForever { .. } => undo_window_ms.max(0),
        }
    }

    /// Points the operation at the message's new location after a move.
    fn retarget(&mut self, new_folder: &str, new_uid: Uid) {
        match self {
            Self::SetSeen { folder, uid, .. }
            | Self::SetFlagged { folder, uid, .. }
            | Self::MoveTo { folder, uid, .. }
            | Self::DeleteForever { folder, uid } => {
                new_folder.clone_into(folder);
                *uid = new_uid;
            }
        }
    }

    /// Operations of the same slot on the same message can cancel each other out.
    fn slot(&self) -> Option<(u8, bool)> {
        match self {
            Self::SetSeen { value, .. } => Some((0, *value)),
            Self::SetFlagged { value, .. } => Some((1, *value)),
            Self::MoveTo { .. } | Self::DeleteForever { .. } => None,
        }
    }
}

/// Where an operation is in its life.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpState {
    /// Waiting for its time or its turn.
    Queued,
    /// Handed to the backend; the result has not come back.
    Applying,
    /// The server accepted it.
    Confirmed,
    /// Gave up. The screen restores the row and shows a message.
    Failed,
    /// Undone before it started.
    Cancelled,
}

impl OpState {
    /// True once nothing more will happen to the operation.
    pub fn is_finished(self) -> bool {
        matches!(self, Self::Confirmed | Self::Failed | Self::Cancelled)
    }
}

/// One queued operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Op {
    /// Queue id, increasing in creation order.
    pub id: OpId,
    /// Which account it belongs to.
    pub account: AccountId,
    /// What to do.
    pub kind: OpKind,
    /// Current state.
    pub state: OpState,
    /// When the person made the change.
    pub created_ms: i64,
    /// Earliest time it may run (end of its delay, or the retry time).
    pub not_before_ms: i64,
    /// Times it has been handed to the backend.
    pub attempts: u32,
    /// Last failure, for the Outbox-style detail view.
    pub last_error: Option<String>,
}

/// Result of [`OpQueue::enqueue`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Enqueued {
    /// A new operation was queued. Keep the id for Undo.
    Queued(OpId),
    /// An identical change was already waiting; nothing new was added.
    Duplicate(OpId),
    /// The change undid one that had not started yet, so both were dropped and the server will
    /// see nothing.
    CancelledOut,
}

/// What [`OpQueue::reject`] decided.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rejection {
    /// Will be tried again at this time.
    RetryAt(i64),
    /// Gave up: restore the row and tell the person.
    Failed,
    /// Sign-in problem: the account's queue is paused until [`OpQueue::resume`].
    AccountNeedsAttention,
}

/// Why [`OpQueue::cancel`] refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CancelError {
    /// No such operation.
    NotFound,
    /// It already started, finished or its undo window has closed.
    TooLate,
}

/// Wait before retry number `attempts` (1 is the first failure): 5 s, 30 s, 2 min, 10 min, then 1 h.
pub fn backoff_ms(attempts: u32) -> i64 {
    const STEPS: [i64; 5] = [5_000, 30_000, 120_000, 600_000, 3_600_000];
    STEPS[(attempts.saturating_sub(1) as usize).min(STEPS.len() - 1)]
}

/// A message that was moved on the server but whose new UID is not known yet.
///
/// Later operations on the old address wait while this exists. The engine finds the message again
/// by `message_id` in `dest_folder` and calls [`OpQueue::repoint`], or gives up and calls
/// [`OpQueue::abandon`]. It is stored with the queue (see [`crate::persist`]), so a restart does
/// not let a later operation run against the old UID.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AwaitingUid {
    /// Owning account.
    pub account: AccountId,
    /// Old folder path (where the message was).
    pub folder: String,
    /// Old UID.
    pub uid: Uid,
    /// Folder the message was moved to. Empty when the operation was not a move.
    pub dest_folder: String,
    /// The message's `Message-ID` header, used to find it again. `None` means it cannot be found.
    pub message_id: Option<String>,
    /// How many times the engine has already looked for it.
    pub lookups: u32,
    /// When the move was confirmed (Unix milliseconds; 0 if the caller did not say).
    pub created_ms: i64,
}

/// The queue. Lives in memory; [`crate::persist::PersistentQueue`] keeps it in the `op_queue`
/// table so it survives a kill.
#[derive(Debug, Default, Clone)]
pub struct OpQueue {
    ops: BTreeMap<OpId, Op>,
    next_id: OpId,
    /// Accounts waiting for the person to sign in again.
    paused: BTreeSet<AccountId>,
    /// Accounts with no connection: nothing is tried, so no attempts are used up.
    offline: BTreeSet<AccountId>,
    /// Account-wide backoff after a transient failure: nothing for the account runs before then.
    holds: BTreeMap<AccountId, i64>,
    /// Messages that were moved but whose new UID is not known yet. Later operations on them wait.
    awaiting_uid: BTreeMap<(AccountId, String, Uid), AwaitingUid>,
}

impl OpQueue {
    /// An empty queue.
    pub fn new() -> Self {
        Self {
            next_id: 1,
            ..Self::default()
        }
    }

    /// Rebuilds a queue from stored operations and awaiting-UID entries (see
    /// [`crate::persist::OpStore::load`]). The next id is one above the largest of `max_id` (the
    /// largest id the store ever held, which may belong to a row that is gone) and the ids given.
    /// Offline flags, pauses and back-off are not stored: the engine sets them again.
    ///
    /// Operations still marked [`OpState::Applying`] are kept as they are; call
    /// [`OpQueue::recover_after_restart`] to put them back in the queue.
    pub fn from_parts(ops: Vec<Op>, awaiting: Vec<AwaitingUid>, max_id: OpId) -> Self {
        let next_id = ops
            .iter()
            .map(|op| op.id)
            .max()
            .unwrap_or(0)
            .max(max_id)
            .saturating_add(1);
        Self {
            next_id,
            ops: ops.into_iter().map(|op| (op.id, op)).collect(),
            awaiting_uid: awaiting
                .into_iter()
                .map(|a| ((a.account, a.folder.clone(), a.uid), a))
                .collect(),
            ..Self::default()
        }
    }

    /// Every operation, in id (creation) order.
    pub fn iter(&self) -> impl Iterator<Item = &Op> {
        self.ops.values()
    }

    /// Every message that is waiting for its new UID, in a stable order.
    pub fn awaiting(&self) -> impl Iterator<Item = &AwaitingUid> {
        self.awaiting_uid.values()
    }

    /// The awaiting-UID entry for the message that used to be at `from`, if any.
    pub fn awaiting_for(&self, account: AccountId, from: (&str, Uid)) -> Option<&AwaitingUid> {
        self.awaiting_uid.get(&(account, from.0.to_owned(), from.1))
    }

    /// Counts one more lookup by Message-ID for the message that used to be at `from`. Returns the
    /// new count, or `None` if nothing is waiting for that address.
    pub fn record_lookup(&mut self, account: AccountId, from: (&str, Uid)) -> Option<u32> {
        let entry = self
            .awaiting_uid
            .get_mut(&(account, from.0.to_owned(), from.1))?;
        entry.lookups = entry.lookups.saturating_add(1);
        Some(entry.lookups)
    }

    /// Records a change. `undo_window_ms` is the person's Undo setting (5 000 by default); it
    /// delays moves and deletes. Read and star changes wait [`FLAG_COALESCE_MS`] instead.
    pub fn enqueue(
        &mut self,
        account: AccountId,
        kind: OpKind,
        now_ms: i64,
        undo_window_ms: i64,
    ) -> Enqueued {
        if let Some((slot, value)) = kind.slot() {
            let (folder, uid) = kind.target();
            let earlier = self.ops.values().find(|op| {
                op.account == account
                    && op.state == OpState::Queued
                    && op.kind.target() == (folder, uid)
                    && op.kind.slot().is_some_and(|(s, _)| s == slot)
            });
            if let Some(op) = earlier {
                let id = op.id;
                let same = op.kind.slot().is_some_and(|(_, v)| v == value);
                if same {
                    return Enqueued::Duplicate(id);
                }
                if let Some(op) = self.ops.get_mut(&id) {
                    op.state = OpState::Cancelled;
                }
                return Enqueued::CancelledOut;
            }
        }

        let id = self.next_id.max(1);
        self.next_id = id + 1;
        let not_before_ms = now_ms + kind.delay_ms(undo_window_ms);
        self.ops.insert(
            id,
            Op {
                id,
                account,
                kind,
                state: OpState::Queued,
                created_ms: now_ms,
                not_before_ms,
                attempts: 0,
                last_error: None,
            },
        );
        Enqueued::Queued(id)
    }

    /// Undo. Only works while the operation is still queued and its window is open.
    pub fn cancel(&mut self, id: OpId, now_ms: i64) -> Result<(), CancelError> {
        let op = self.ops.get_mut(&id).ok_or(CancelError::NotFound)?;
        if op.state != OpState::Queued || now_ms >= op.not_before_ms {
            return Err(CancelError::TooLate);
        }
        op.state = OpState::Cancelled;
        Ok(())
    }

    /// Hands out operations that are due, oldest first, marking them as applying.
    ///
    /// An operation is held back while an earlier one on the same message is unfinished, so a
    /// "mark read" never overtakes the "move" before it.
    pub fn take_due(&mut self, now_ms: i64, limit: usize) -> Vec<Op> {
        self.take_due_matching(now_ms, limit, None)
    }

    /// Like [`OpQueue::take_due`], for one account only. The sync engine runs one task per account,
    /// and each task must only hand out its own account's operations.
    pub fn take_due_for(&mut self, account: AccountId, now_ms: i64, limit: usize) -> Vec<Op> {
        self.take_due_matching(now_ms, limit, Some(account))
    }

    fn take_due_matching(&mut self, now_ms: i64, limit: usize, only: Option<AccountId>) -> Vec<Op> {
        let mut chosen = Vec::new();
        let ids: Vec<OpId> = self.ops.keys().copied().collect();
        for id in ids {
            if chosen.len() >= limit {
                break;
            }
            let Some(op) = self.ops.get(&id) else {
                continue;
            };
            if only.is_some_and(|a| a != op.account) {
                continue;
            }
            if op.state != OpState::Queued
                || self.earliest_ms(op) > now_ms
                || !self.account_may_run(op.account)
            {
                continue;
            }
            if self.blocked_by_earlier(op) || self.awaits_new_uid(op) {
                continue;
            }
            chosen.push(id);
            // Mark immediately so a later op on the same message in this loop sees it as unfinished.
            if let Some(op) = self.ops.get_mut(&id) {
                op.state = OpState::Applying;
                op.attempts += 1;
            }
        }
        chosen
            .iter()
            .filter_map(|id| self.ops.get(id).cloned())
            .collect()
    }

    fn blocked_by_earlier(&self, op: &Op) -> bool {
        let target = op.kind.target();
        self.ops.range(..op.id).any(|(_, other)| {
            other.account == op.account
                && !other.state.is_finished()
                && other.kind.target() == target
        })
    }

    /// True when the account is neither waiting for a sign-in nor offline.
    fn account_may_run(&self, account: AccountId) -> bool {
        !self.paused.contains(&account) && !self.offline.contains(&account)
    }

    /// The earliest time the operation may run: its own time, or the end of the account's backoff.
    fn earliest_ms(&self, op: &Op) -> i64 {
        let hold = self.holds.get(&op.account).copied().unwrap_or(i64::MIN);
        op.not_before_ms.max(hold)
    }

    fn awaits_new_uid(&self, op: &Op) -> bool {
        let (folder, uid) = op.kind.target();
        !self.awaiting_uid.is_empty()
            && self
                .awaiting_uid
                .contains_key(&(op.account, folder.to_owned(), uid))
    }

    /// The server accepted the operation. Any backoff on the account ends.
    pub fn confirm(&mut self, id: OpId) {
        if let Some(op) = self
            .ops
            .get_mut(&id)
            .filter(|op| op.state == OpState::Applying)
        {
            op.state = OpState::Confirmed;
            op.last_error = None;
            self.holds.remove(&op.account);
        }
    }

    /// The server accepted a move (or any operation after which the message has a new address).
    ///
    /// With `new_location` (from the COPYUID or MOVE response) the message's later operations are
    /// re-pointed at it at once. Without it (a server that has no UIDPLUS) they wait until the
    /// engine has found the message by Message-ID and called [`OpQueue::repoint`], or given up and
    /// called [`OpQueue::abandon`].
    pub fn confirm_moved(&mut self, id: OpId, new_location: Option<(String, Uid)>) {
        self.confirm_moved_with(id, new_location, None, 0);
    }

    /// Like [`OpQueue::confirm_moved`], and when the new UID is not known, also keeps the
    /// `Message-ID` header (from the local row; `None` if it has none) and the confirmation time,
    /// so the engine can find the message again even after a restart. The destination folder is
    /// taken from the move itself.
    pub fn confirm_moved_with(
        &mut self,
        id: OpId,
        new_location: Option<(String, Uid)>,
        message_id: Option<String>,
        now_ms: i64,
    ) {
        let Some((account, from_folder, from_uid, dest)) = self
            .ops
            .get(&id)
            .filter(|op| op.state == OpState::Applying)
            .map(|op| {
                let (folder, uid) = op.kind.target();
                let dest = match &op.kind {
                    OpKind::MoveTo { dest, .. } => dest.clone(),
                    _ => String::new(),
                };
                (op.account, folder.to_owned(), uid, dest)
            })
        else {
            return;
        };
        self.confirm(id);
        match new_location {
            Some(to) => {
                self.repoint(account, (&from_folder, from_uid), to);
            }
            None => {
                self.awaiting_uid.insert(
                    (account, from_folder.clone(), from_uid),
                    AwaitingUid {
                        account,
                        folder: from_folder,
                        uid: from_uid,
                        dest_folder: dest,
                        message_id,
                        lookups: 0,
                        created_ms: now_ms,
                    },
                );
            }
        }
    }

    /// Points every unfinished operation on the message at `from` to its new location `to`, and
    /// lets them run. Returns how many were changed.
    pub fn repoint(&mut self, account: AccountId, from: (&str, Uid), to: (String, Uid)) -> usize {
        self.awaiting_uid
            .remove(&(account, from.0.to_owned(), from.1));
        let mut changed = 0;
        for op in self.ops.values_mut().filter(|op| {
            op.account == account && !op.state.is_finished() && op.kind.target() == from
        }) {
            op.kind.retarget(&to.0, to.1);
            changed += 1;
        }
        changed
    }

    /// Re-points several messages at once: every unfinished operation of `account` whose target is
    /// the first address of a pair moves to the second. Each operation moves at most once, even
    /// when one message's new address is another message's old address (a UIDVALIDITY change
    /// renumbers a whole folder, so this is common there). Calling [`OpQueue::repoint`] once per
    /// pair would move such operations twice. Awaiting-UID entries at the old addresses are
    /// resolved. Returns how many operations changed.
    pub fn repoint_many(&mut self, account: AccountId, moves: &[(Location, Location)]) -> usize {
        let map: BTreeMap<(&str, Uid), &Location> = moves
            .iter()
            .map(|(from, to)| ((from.0.as_str(), from.1), to))
            .collect();
        for (from, _) in moves {
            self.awaiting_uid.remove(&(account, from.0.clone(), from.1));
        }
        let mut changed = 0;
        for op in self
            .ops
            .values_mut()
            .filter(|op| op.account == account && !op.state.is_finished())
        {
            if let Some(to) = map.get(&op.kind.target()) {
                op.kind.retarget(&to.0, to.1);
                changed += 1;
            }
        }
        changed
    }

    /// Gives up on the unfinished operations on a message whose new location could not be found
    /// (or whose folder was reset by a UIDVALIDITY change and could not be matched). They fail, so
    /// the screen can say so. Returns their ids.
    pub fn abandon(&mut self, account: AccountId, from: (&str, Uid), reason: &str) -> Vec<OpId> {
        self.awaiting_uid
            .remove(&(account, from.0.to_owned(), from.1));
        let mut failed = Vec::new();
        for op in self.ops.values_mut().filter(|op| {
            op.account == account && !op.state.is_finished() && op.kind.target() == from
        }) {
            op.state = OpState::Failed;
            op.last_error = Some(reason.to_owned());
            failed.push(op.id);
        }
        failed
    }

    /// Records whether the account has a connection. While offline nothing is handed out for it,
    /// so no operation uses up an attempt just because the network is down.
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

    /// The backend call failed. Decides between retrying, giving up and pausing the account.
    pub fn reject(&mut self, id: OpId, error: &BackendError, now_ms: i64) -> Option<Rejection> {
        let op = self
            .ops
            .get_mut(&id)
            .filter(|op| op.state == OpState::Applying)?;
        op.last_error = Some(error.to_string());
        match error {
            BackendError::Auth(_) => {
                // Not the operation's fault and not counted against it: it waits for a fresh sign-in.
                op.state = OpState::Queued;
                op.attempts = op.attempts.saturating_sub(1);
                self.paused.insert(op.account);
                Some(Rejection::AccountNeedsAttention)
            }
            BackendError::Transient(_) if op.attempts < MAX_ATTEMPTS => {
                let at = now_ms + backoff_ms(op.attempts);
                op.state = OpState::Queued;
                op.not_before_ms = at;
                // The server is struggling: the whole account waits, not just this operation.
                let hold = self.holds.entry(op.account).or_insert(at);
                *hold = (*hold).max(at);
                Some(Rejection::RetryAt(at))
            }
            BackendError::Transient(_) | BackendError::Permanent(_) => {
                op.state = OpState::Failed;
                Some(Rejection::Failed)
            }
        }
    }

    /// Lets an account's queue run again after the person signed in.
    pub fn resume(&mut self, account: AccountId) {
        self.paused.remove(&account);
    }

    /// True while the account's queue is waiting for the person.
    pub fn is_paused(&self, account: AccountId) -> bool {
        self.paused.contains(&account)
    }

    /// After a crash or kill, anything that was mid-flight goes back to the queue. The server may
    /// or may not have applied it; every operation here is idempotent, so running it again is safe.
    pub fn recover_after_restart(&mut self) {
        for op in self
            .ops
            .values_mut()
            .filter(|op| op.state == OpState::Applying)
        {
            op.state = OpState::Queued;
        }
    }

    /// When the engine should next wake up to drain the queue, or `None` if nothing is waiting.
    ///
    /// An operation held back behind an earlier unfinished one on the same message does not count:
    /// it can only run once that one has finished, and the earlier one gives its own time (or is
    /// running). Counting it would give a time in the past that [`OpQueue::take_due`] then refuses,
    /// and a caller that sleeps until the next wake would spin.
    pub fn next_wake_ms(&self) -> Option<i64> {
        self.next_wake_matching(None)
    }

    /// Like [`OpQueue::next_wake_ms`], for one account only.
    pub fn next_wake_ms_for(&self, account: AccountId) -> Option<i64> {
        self.next_wake_matching(Some(account))
    }

    fn next_wake_matching(&self, only: Option<AccountId>) -> Option<i64> {
        self.ops
            .values()
            .filter(|op| {
                only.is_none_or(|a| a == op.account)
                    && op.state == OpState::Queued
                    && self.account_may_run(op.account)
                    && !self.awaits_new_uid(op)
                    && !self.blocked_by_earlier(op)
            })
            .map(|op| self.earliest_ms(op))
            .min()
    }

    /// UIDs in a folder with unfinished local changes. The sync planner uses this so server
    /// flags never overwrite a change that is still on its way.
    pub fn pending_uids(&self, account: AccountId, folder: &str) -> BTreeSet<Uid> {
        self.ops
            .values()
            .filter(|op| op.account == account && !op.state.is_finished())
            .filter_map(|op| {
                let (f, uid) = op.kind.target();
                (f == folder).then_some(uid)
            })
            .collect()
    }

    /// Looks one operation up.
    pub fn get(&self, id: OpId) -> Option<&Op> {
        self.ops.get(&id)
    }

    /// Number of operations that are not finished.
    pub fn unfinished(&self) -> usize {
        self.ops
            .values()
            .filter(|op| !op.state.is_finished())
            .count()
    }

    /// Drops finished operations and returns how many.
    pub fn purge_finished(&mut self) -> usize {
        let before = self.ops.len();
        self.ops.retain(|_, op| !op.state.is_finished());
        before - self.ops.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: AccountId = 1;
    const WINDOW: i64 = 5_000;

    fn seen(uid: Uid, value: bool) -> OpKind {
        OpKind::SetSeen {
            folder: "INBOX".into(),
            uid,
            value,
        }
    }

    fn mv(uid: Uid, dest: &str) -> OpKind {
        OpKind::MoveTo {
            folder: "INBOX".into(),
            uid,
            dest: dest.into(),
        }
    }

    fn queued(q: &mut OpQueue, kind: OpKind, now: i64) -> OpId {
        match q.enqueue(A, kind, now, WINDOW) {
            Enqueued::Queued(id) => id,
            other => panic!("expected a new operation, got {other:?}"),
        }
    }

    #[test]
    fn nothing_runs_inside_the_undo_window() {
        let mut q = OpQueue::new();
        queued(&mut q, mv(1, "Trash"), 1_000);
        assert!(q.take_due(5_999, 10).is_empty());
        let due = q.take_due(6_000, 10);
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].state, OpState::Applying);
        assert_eq!(due[0].attempts, 1);
    }

    #[test]
    fn read_and_star_wait_only_the_short_coalescing_delay() {
        let mut q = OpQueue::new();
        queued(&mut q, seen(1, true), 1_000);
        assert!(q.take_due(2_499, 10).is_empty());
        assert_eq!(q.take_due(2_500, 10).len(), 1);
        // A move still waits for the whole Undo window.
        queued(&mut q, mv(2, "Trash"), 1_000);
        assert!(q.take_due(5_999, 10).is_empty());
        assert_eq!(q.take_due(6_000, 10).len(), 1);
    }

    #[test]
    fn a_longer_undo_setting_delays_moves_but_not_flags() {
        let mut q = OpQueue::new();
        assert!(matches!(
            q.enqueue(A, mv(1, "Trash"), 0, 10_000),
            Enqueued::Queued(_)
        ));
        assert!(matches!(
            q.enqueue(A, seen(2, true), 0, 10_000),
            Enqueued::Queued(_)
        ));
        assert_eq!(
            q.take_due(1_500, 10).len(),
            1,
            "only the flag change is due"
        );
        assert_eq!(q.take_due(10_000, 10).len(), 1);
    }

    #[test]
    fn undo_cancels_before_the_window_closes() {
        let mut q = OpQueue::new();
        let id = queued(&mut q, mv(1, "Trash"), 0);
        assert_eq!(q.cancel(id, 4_999), Ok(()));
        assert_eq!(q.get(id).map(|o| o.state), Some(OpState::Cancelled));
        assert!(q.take_due(60_000, 10).is_empty());
    }

    #[test]
    fn undo_is_refused_once_the_window_closed_or_work_started() {
        let mut q = OpQueue::new();
        let id = queued(&mut q, mv(1, "Trash"), 0);
        assert_eq!(q.cancel(id, 5_000), Err(CancelError::TooLate));
        q.take_due(5_000, 10);
        assert_eq!(q.cancel(id, 5_001), Err(CancelError::TooLate));
        assert_eq!(q.cancel(999, 0), Err(CancelError::NotFound));
    }

    #[test]
    fn a_second_snackbar_does_not_cancel_the_first_operation() {
        // The UI shows one Undo at a time. Replacing it must leave the earlier operation queued.
        let mut q = OpQueue::new();
        let first = queued(&mut q, mv(1, "Trash"), 0);
        let _second = queued(&mut q, mv(2, "Trash"), 1_000);
        assert_eq!(q.get(first).map(|o| o.state), Some(OpState::Queued));
        assert_eq!(
            q.take_due(5_500, 10).len(),
            1,
            "only the first window has closed"
        );
    }

    #[test]
    fn read_then_unread_inside_the_window_cancels_out() {
        let mut q = OpQueue::new();
        let id = queued(&mut q, seen(1, true), 0);
        assert_eq!(
            q.enqueue(A, seen(1, false), 500, WINDOW),
            Enqueued::CancelledOut
        );
        assert_eq!(q.get(id).map(|o| o.state), Some(OpState::Cancelled));
        assert!(q.take_due(60_000, 10).is_empty());
    }

    #[test]
    fn repeating_the_same_change_adds_nothing() {
        let mut q = OpQueue::new();
        let id = queued(&mut q, seen(1, true), 0);
        assert_eq!(
            q.enqueue(A, seen(1, true), 100, WINDOW),
            Enqueued::Duplicate(id)
        );
        assert_eq!(q.unfinished(), 1);
    }

    #[test]
    fn changes_to_different_messages_or_flags_do_not_coalesce() {
        let mut q = OpQueue::new();
        queued(&mut q, seen(1, true), 0);
        queued(&mut q, seen(2, false), 0);
        queued(
            &mut q,
            OpKind::SetFlagged {
                folder: "INBOX".into(),
                uid: 1,
                value: true,
            },
            0,
        );
        assert_eq!(q.unfinished(), 3);
    }

    #[test]
    fn a_started_operation_is_not_coalesced_away() {
        let mut q = OpQueue::new();
        queued(&mut q, seen(1, true), 0);
        q.take_due(5_000, 10);
        // Already sent to the server: the reverse change has to be a new operation.
        assert!(matches!(
            q.enqueue(A, seen(1, false), 5_100, WINDOW),
            Enqueued::Queued(_)
        ));
    }

    #[test]
    fn operations_on_one_message_keep_their_order() {
        let mut q = OpQueue::new();
        let first = queued(&mut q, seen(1, true), 0);
        let second = queued(&mut q, mv(1, "Archive"), 0);
        let batch = q.take_due(5_000, 10);
        assert_eq!(
            batch.iter().map(|o| o.id).collect::<Vec<_>>(),
            vec![first],
            "the move waits for the flag change"
        );
        q.confirm(first);
        let batch = q.take_due(5_000, 10);
        assert_eq!(batch.iter().map(|o| o.id).collect::<Vec<_>>(), vec![second]);
    }

    #[test]
    fn different_messages_run_in_parallel_up_to_the_limit() {
        let mut q = OpQueue::new();
        for uid in 1..=5 {
            queued(&mut q, seen(uid, true), 0);
        }
        assert_eq!(q.take_due(5_000, 3).len(), 3);
        assert_eq!(q.take_due(5_000, 3).len(), 2);
    }

    #[test]
    fn confirm_finishes_an_operation() {
        let mut q = OpQueue::new();
        let id = queued(&mut q, seen(1, true), 0);
        q.take_due(5_000, 10);
        q.confirm(id);
        assert_eq!(q.get(id).map(|o| o.state), Some(OpState::Confirmed));
        assert_eq!(q.unfinished(), 0);
    }

    #[test]
    fn transient_failures_back_off_then_give_up() {
        let mut q = OpQueue::new();
        let id = queued(&mut q, seen(1, true), 0);
        let mut now = 5_000;
        let err = BackendError::Transient("timeout".into());
        let mut waits = Vec::new();
        for attempt in 1..=MAX_ATTEMPTS {
            assert_eq!(
                q.take_due(now, 1).len(),
                1,
                "attempt {attempt} should be due"
            );
            match q.reject(id, &err, now) {
                Some(Rejection::RetryAt(at)) => {
                    waits.push(at - now);
                    now = at;
                }
                Some(Rejection::Failed) => {
                    assert_eq!(attempt, MAX_ATTEMPTS);
                }
                other => panic!("unexpected {other:?}"),
            }
        }
        assert_eq!(
            waits,
            vec![5_000, 30_000, 120_000, 600_000, 3_600_000, 3_600_000, 3_600_000]
        );
        assert_eq!(q.get(id).map(|o| o.state), Some(OpState::Failed));
        assert!(q.get(id).and_then(|o| o.last_error.clone()).is_some());
    }

    #[test]
    fn permanent_failure_fails_at_once() {
        let mut q = OpQueue::new();
        let id = queued(&mut q, mv(1, "Nope"), 0);
        q.take_due(5_000, 1);
        assert_eq!(
            q.reject(id, &BackendError::Permanent("no such folder".into()), 5_000),
            Some(Rejection::Failed)
        );
        assert_eq!(q.get(id).map(|o| o.state), Some(OpState::Failed));
    }

    #[test]
    fn sign_in_failure_pauses_the_account_without_burning_attempts() {
        let mut q = OpQueue::new();
        let id = queued(&mut q, seen(1, true), 0);
        let other = match q.enqueue(2, seen(1, true), 0, WINDOW) {
            Enqueued::Queued(id) => id,
            e => panic!("{e:?}"),
        };
        q.take_due(5_000, 1);
        assert_eq!(
            q.reject(id, &BackendError::Auth("token expired".into()), 5_000),
            Some(Rejection::AccountNeedsAttention)
        );
        assert!(q.is_paused(A));
        assert_eq!(q.get(id).map(|o| o.attempts), Some(0));
        let due: Vec<_> = q.take_due(60_000, 10).iter().map(|o| o.id).collect();
        assert_eq!(due, vec![other], "the other account keeps syncing");
        q.resume(A);
        assert_eq!(
            q.take_due(60_000, 10)
                .iter()
                .map(|o| o.id)
                .collect::<Vec<_>>(),
            vec![id]
        );
    }

    #[test]
    fn restart_requeues_anything_in_flight() {
        let mut q = OpQueue::new();
        let id = queued(&mut q, seen(1, true), 0);
        q.take_due(5_000, 1);
        q.recover_after_restart();
        assert_eq!(q.get(id).map(|o| o.state), Some(OpState::Queued));
        assert_eq!(q.take_due(5_000, 1).len(), 1);
    }

    #[test]
    fn next_wake_is_the_earliest_waiting_operation() {
        let mut q = OpQueue::new();
        assert_eq!(q.next_wake_ms(), None);
        queued(&mut q, seen(1, true), 10_000);
        queued(&mut q, seen(2, true), 2_000);
        assert_eq!(q.next_wake_ms(), Some(3_500));
    }

    #[test]
    fn an_operation_held_behind_an_earlier_one_does_not_set_the_wake_time() {
        let mut q = OpQueue::new();
        // A star (1.5 s) queued behind an archive (5 s) of the same message.
        queued(&mut q, mv(1, "Archive"), 0);
        queued(&mut q, seen(1, true), 0);
        assert_eq!(q.next_wake_ms(), Some(5_000));
        assert_eq!(q.next_wake_ms_for(A), Some(5_000));
        assert!(q.take_due(1_500, 10).is_empty());
    }

    #[test]
    fn a_transient_failure_backs_off_the_whole_account() {
        let mut q = OpQueue::new();
        let first = queued(&mut q, seen(1, true), 0);
        queued(&mut q, seen(2, true), 0);
        let due = q.take_due(5_000, 1);
        assert_eq!(due.iter().map(|o| o.id).collect::<Vec<_>>(), vec![first]);
        let err = BackendError::Transient("timeout".into());
        assert_eq!(
            q.reject(first, &err, 5_000),
            Some(Rejection::RetryAt(10_000))
        );
        assert!(
            q.take_due(9_999, 10).is_empty(),
            "the other message waits too"
        );
        assert_eq!(q.next_wake_ms(), Some(10_000));
        assert_eq!(q.take_due(10_000, 10).len(), 2);
    }

    #[test]
    fn another_account_is_not_held_back() {
        let mut q = OpQueue::new();
        let failing = queued(&mut q, seen(1, true), 0);
        let other = match q.enqueue(2, seen(1, true), 0, WINDOW) {
            Enqueued::Queued(id) => id,
            e => panic!("{e:?}"),
        };
        q.take_due(5_000, 1);
        let err = BackendError::Transient("timeout".into());
        q.reject(failing, &err, 5_000);
        let due: Vec<_> = q.take_due(6_000, 10).iter().map(|o| o.id).collect();
        assert_eq!(due, vec![other]);
    }

    #[test]
    fn a_confirmed_operation_ends_the_backoff() {
        let mut q = OpQueue::new();
        let a = queued(&mut q, seen(1, true), 0);
        let b = queued(&mut q, seen(2, true), 0);
        q.take_due(5_000, 2);
        q.reject(a, &BackendError::Transient("blip".into()), 5_000);
        q.confirm(b);
        let c = queued(&mut q, seen(3, true), 6_000);
        let due: Vec<_> = q.take_due(8_000, 10).iter().map(|o| o.id).collect();
        assert_eq!(
            due,
            vec![c],
            "the account recovered, so no one waits for 10 s"
        );
    }

    #[test]
    fn an_offline_account_is_not_tried_and_burns_no_attempts() {
        let mut q = OpQueue::new();
        let id = queued(&mut q, seen(1, true), 0);
        q.set_offline(A, true);
        assert!(q.is_offline(A));
        assert!(q.take_due(1_000_000, 10).is_empty());
        assert_eq!(q.next_wake_ms(), None);
        assert_eq!(q.get(id).map(|o| o.attempts), Some(0));
        q.set_offline(A, false);
        assert_eq!(q.take_due(1_000_000, 10).len(), 1);
    }

    #[test]
    fn a_confirmed_move_re_points_later_operations_at_the_new_uid() {
        let mut q = OpQueue::new();
        let moved = queued(&mut q, mv(1, "Archive"), 0);
        let later = queued(&mut q, seen(1, true), 0);
        let first = q.take_due(5_000, 10);
        assert_eq!(first.iter().map(|o| o.id).collect::<Vec<_>>(), vec![moved]);
        q.confirm_moved(moved, Some(("Archive".into(), 77)));
        let due = q.take_due(5_000, 10);
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].id, later);
        assert_eq!(due[0].kind.target(), ("Archive", 77));
    }

    #[test]
    fn a_move_without_a_known_uid_holds_later_operations_until_resolved() {
        let mut q = OpQueue::new();
        let moved = queued(&mut q, mv(1, "Archive"), 0);
        queued(&mut q, seen(1, true), 0);
        q.take_due(5_000, 10);
        q.confirm_moved(moved, None);
        assert!(q.take_due(60_000, 10).is_empty());
        assert_eq!(q.next_wake_ms(), None);
        assert_eq!(q.repoint(A, ("INBOX", 1), ("Archive".into(), 9)), 1);
        let due = q.take_due(60_000, 10);
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].kind.target(), ("Archive", 9));
    }

    #[test]
    fn operations_that_cannot_be_matched_fail_with_a_reason() {
        let mut q = OpQueue::new();
        let moved = queued(&mut q, mv(1, "Archive"), 0);
        let later = queued(&mut q, seen(1, true), 0);
        q.take_due(5_000, 10);
        q.confirm_moved(moved, None);
        assert_eq!(
            q.abandon(A, ("INBOX", 1), "message not found after move"),
            vec![later]
        );
        let op = q.get(later);
        assert_eq!(op.map(|o| o.state), Some(OpState::Failed));
        assert!(op.and_then(|o| o.last_error.clone()).is_some());
        assert_eq!(q.unfinished(), 0);
    }

    #[test]
    fn re_pointing_leaves_other_messages_alone() {
        let mut q = OpQueue::new();
        let other = queued(&mut q, seen(2, true), 0);
        assert_eq!(q.repoint(A, ("INBOX", 1), ("Archive".into(), 9)), 0);
        assert_eq!(
            q.get(other).map(|o| o.kind.target().1),
            Some(2),
            "untouched"
        );
    }

    #[test]
    fn confirm_moved_ignores_an_operation_that_is_not_in_flight() {
        let mut q = OpQueue::new();
        let moved = queued(&mut q, mv(1, "Archive"), 0);
        q.confirm_moved(moved, None);
        assert_eq!(q.get(moved).map(|o| o.state), Some(OpState::Queued));
        assert_eq!(q.take_due(5_000, 10).len(), 1, "nothing is held back");
    }

    #[test]
    fn pending_uids_feed_the_sync_planner() {
        let mut q = OpQueue::new();
        queued(&mut q, seen(4, true), 0);
        let id = queued(&mut q, seen(9, true), 0);
        q.take_due(5_000, 10);
        assert_eq!(
            q.pending_uids(A, "INBOX").into_iter().collect::<Vec<_>>(),
            vec![4, 9]
        );
        q.confirm(id);
        assert_eq!(
            q.pending_uids(A, "INBOX").into_iter().collect::<Vec<_>>(),
            vec![4]
        );
        assert!(q.pending_uids(A, "Archive").is_empty());
        assert!(q.pending_uids(2, "INBOX").is_empty());
    }

    #[test]
    fn purge_removes_only_finished_operations() {
        let mut q = OpQueue::new();
        let done = queued(&mut q, seen(1, true), 0);
        queued(&mut q, seen(2, true), 0);
        q.take_due(5_000, 1);
        q.confirm(done);
        assert_eq!(q.purge_finished(), 1);
        assert_eq!(q.unfinished(), 1);
    }

    #[test]
    fn take_due_for_hands_out_only_that_accounts_operations() {
        let mut q = OpQueue::new();
        let mine = queued(&mut q, seen(1, true), 0);
        let theirs = match q.enqueue(2, seen(1, true), 0, WINDOW) {
            Enqueued::Queued(id) => id,
            other => panic!("expected a new operation, got {other:?}"),
        };
        assert_eq!(q.next_wake_ms_for(A), Some(1_500));
        assert_eq!(q.next_wake_ms_for(3), None);
        let due = q.take_due_for(A, 5_000, 10);
        assert_eq!(due.iter().map(|o| o.id).collect::<Vec<_>>(), vec![mine]);
        assert_eq!(q.get(theirs).map(|o| o.state), Some(OpState::Queued));
        assert_eq!(q.next_wake_ms_for(A), None, "mine is applying");
        q.set_offline(2, true);
        assert_eq!(
            q.next_wake_ms_for(2),
            None,
            "an offline account never wakes"
        );
        assert!(q.take_due_for(2, 5_000, 10).is_empty());
    }

    #[test]
    fn repoint_many_moves_each_operation_once_even_when_addresses_swap() {
        let mut q = OpQueue::new();
        let a = queued(&mut q, seen(1, true), 0);
        let b = queued(&mut q, seen(2, true), 0);
        let c = queued(&mut q, seen(3, true), 0);
        let moves = vec![
            (("INBOX".to_owned(), 1), ("INBOX".to_owned(), 2)),
            (("INBOX".to_owned(), 2), ("INBOX".to_owned(), 1)),
        ];
        assert_eq!(q.repoint_many(A, &moves), 2);
        assert_eq!(q.get(a).map(|o| o.kind.target().1), Some(2));
        assert_eq!(q.get(b).map(|o| o.kind.target().1), Some(1));
        assert_eq!(q.get(c).map(|o| o.kind.target().1), Some(3), "untouched");
        assert_eq!(
            q.repoint_many(2, &moves),
            0,
            "other accounts are left alone"
        );
    }

    #[test]
    fn backoff_schedule_is_capped() {
        assert_eq!(backoff_ms(0), 5_000);
        assert_eq!(backoff_ms(1), 5_000);
        assert_eq!(backoff_ms(4), 600_000);
        assert_eq!(backoff_ms(5), 3_600_000);
        assert_eq!(backoff_ms(50), 3_600_000);
    }

    #[test]
    fn a_move_without_a_uid_keeps_where_it_went_and_how_to_find_it() {
        let mut q = OpQueue::new();
        let moved = queued(&mut q, mv(1, "Archive"), 0);
        q.take_due(5_000, 10);
        q.confirm_moved_with(moved, None, Some("<a@example.org>".into()), 5_100);
        let entry = q.awaiting_for(A, ("INBOX", 1)).cloned();
        assert_eq!(
            entry,
            Some(AwaitingUid {
                account: A,
                folder: "INBOX".into(),
                uid: 1,
                dest_folder: "Archive".into(),
                message_id: Some("<a@example.org>".into()),
                lookups: 0,
                created_ms: 5_100,
            })
        );
        assert_eq!(q.record_lookup(A, ("INBOX", 1)), Some(1));
        assert_eq!(q.record_lookup(A, ("INBOX", 2)), None);
        q.repoint(A, ("INBOX", 1), ("Archive".into(), 3));
        assert_eq!(q.awaiting().count(), 0);
    }

    #[test]
    fn a_rebuilt_queue_has_the_same_operations_and_hands_out_fresh_ids() {
        let mut q = OpQueue::new();
        let a = queued(&mut q, seen(1, true), 0);
        let b = queued(&mut q, mv(2, "Trash"), 0);
        let ops: Vec<Op> = q.iter().cloned().collect();
        let mut rebuilt = OpQueue::from_parts(ops.clone(), Vec::new(), 7);
        assert_eq!(rebuilt.iter().cloned().collect::<Vec<_>>(), ops);
        assert_eq!((a, b), (1, 2));
        assert_eq!(queued(&mut rebuilt, seen(3, true), 0), 8, "above max_id");
        let mut small = OpQueue::from_parts(ops, Vec::new(), 0);
        assert_eq!(
            queued(&mut small, seen(3, true), 0),
            3,
            "above the ids held"
        );
    }
}
