// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Observing: turns the engine's broadcast of [`Change`]s into a numbered log the app can read
//! with `next_change(after)`, and keeps the account status chips.
//!
//! Why a log and not the broadcast itself: the app asks "what changed since N" from a coroutine
//! that may be busy or cancelled between two calls, and a broadcast receiver that is not polled
//! loses the changes in between. Here every change gets the next sequence number and goes in a
//! ring of the last [`RING`] entries. A caller that is behind gets everything since its number
//! merged into one batch (a folder listed once however many times it changed). A caller that is
//! behind by more than the ring holds (or lagged on the engine's own channel) gets
//! `everything = true`: re-run every query.

use std::collections::{BTreeSet, HashSet, VecDeque};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use croft_engine::{
    Change, REASON_AMBIGUOUS_COPY, REASON_LOST_AFTER_MOVE, REASON_LOST_AFTER_RESET,
};
use tokio::sync::{broadcast, watch};

use crate::credentials::{tell_sign_in_needed, CredentialProvider};
use crate::types::{AccountStatus, ChangeBatch, FailureReason, FolderRef, OpFailure};

/// How many recent changes are kept for callers that are behind.
pub(crate) const RING: usize = 512;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Entry {
    Folder(FolderRef),
    Account(i64),
    Failure(OpFailure),
    /// The engine's channel overflowed: some changes were lost.
    Reset,
}

#[derive(Default)]
struct State {
    seq: u64,
    ring: VecDeque<(u64, Entry)>,
    /// Accounts the server refused (or that have no credential).
    needs_sign_in: HashSet<i64>,
    /// Accounts whose stored settings cannot be used.
    broken: HashSet<i64>,
    offline: bool,
}

/// The change log and account states, shared by the listener task and the `Core` methods.
pub(crate) struct Hub {
    provider: Arc<dyn CredentialProvider>,
    state: Mutex<State>,
    seq: watch::Sender<u64>,
}

impl Hub {
    pub(crate) fn new(provider: Arc<dyn CredentialProvider>) -> Arc<Self> {
        let (seq, _) = watch::channel(0);
        Arc::new(Self {
            provider,
            state: Mutex::new(State::default()),
            seq,
        })
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn push(&self, entries: impl IntoIterator<Item = Entry>) {
        let seq = {
            let mut st = self.lock();
            for e in entries {
                st.seq += 1;
                let n = st.seq;
                st.ring.push_back((n, e));
            }
            while st.ring.len() > RING {
                st.ring.pop_front();
            }
            st.seq
        };
        self.seq.send_replace(seq);
    }

    /// A subscription to the latest sequence number.
    pub(crate) fn subscribe(&self) -> watch::Receiver<u64> {
        self.seq.subscribe()
    }

    /// Notes that these accounts' entries, statuses or folder lists changed.
    pub(crate) fn accounts_changed(&self, accounts: impl IntoIterator<Item = i64>) {
        self.push(accounts.into_iter().map(Entry::Account));
    }

    /// Takes one change from the engine.
    pub(crate) fn apply(&self, change: Change) {
        match change {
            Change::Messages { account, folders } => self.push(folders.into_iter().map(|f| {
                Entry::Folder(FolderRef {
                    account,
                    folder_id: f,
                })
            })),
            Change::Folders { account } => self.push([Entry::Account(account)]),
            Change::OperationFailed {
                account,
                op,
                reason,
            } => self.push([Entry::Failure(OpFailure {
                account,
                op_id: op,
                reason: failure_reason(&reason),
            })]),
            Change::SignInNeeded { account } => {
                self.sign_in_needed(account);
            }
            // Sending is not on this surface yet (it comes with its own version bump, see
            // `docs/tasks/T10-ffi-mail-surface.md`): an outbox change only tells the app that
            // something about the account changed.
            Change::Outbox { account }
            | Change::SendFailed { account, .. }
            | Change::NotFiled { account, .. } => self.push([Entry::Account(account)]),
        }
    }

    /// The account needs a sign-in. Tells the provider once per entry into the state.
    pub(crate) fn sign_in_needed(&self, account: i64) {
        let newly = self.lock().needs_sign_in.insert(account);
        if newly {
            let provider = self.provider.clone();
            // The app's callback may take a moment (keystore, UI hand-off): not on a runtime
            // worker, and not while any lock is held.
            tokio::task::spawn_blocking(move || tell_sign_in_needed(&*provider, account));
            self.push([Entry::Account(account)]);
        }
    }

    /// The person signed in again: the account leaves the needs-sign-in state.
    pub(crate) fn clear_sign_in(&self, account: i64) {
        let was = self.lock().needs_sign_in.remove(&account);
        if was {
            self.push([Entry::Account(account)]);
        }
    }

    /// Marks an account whose stored settings are unusable.
    pub(crate) fn set_broken(&self, account: i64, broken: bool) {
        let mut st = self.lock();
        if broken {
            st.broken.insert(account);
        } else {
            st.broken.remove(&account);
        }
    }

    /// Forgets an account's state (it was removed).
    pub(crate) fn forget(&self, account: i64) {
        let mut st = self.lock();
        st.needs_sign_in.remove(&account);
        st.broken.remove(&account);
    }

    /// Records the network state the platform reported. True when it changed.
    pub(crate) fn set_offline(&self, offline: bool) -> bool {
        std::mem::replace(&mut self.lock().offline, offline) != offline
    }

    /// Whether the platform reported no network.
    pub(crate) fn is_offline(&self) -> bool {
        self.lock().offline
    }

    /// The status chip of an account. The engine does not report "syncing", so `Syncing` is not
    /// produced yet.
    pub(crate) fn status(&self, account: i64) -> AccountStatus {
        let st = self.lock();
        if st.needs_sign_in.contains(&account) {
            AccountStatus::NeedsSignIn
        } else if st.broken.contains(&account) {
            AccountStatus::Error
        } else if st.offline {
            AccountStatus::Offline
        } else {
            AccountStatus::Ok
        }
    }

    /// The engine's channel overflowed: tell callers to re-run everything.
    fn reset(&self) {
        self.push([Entry::Reset]);
    }

    /// Everything that changed after `after`, merged; `None` when nothing did.
    pub(crate) fn batch_after(&self, after: u64) -> Option<ChangeBatch> {
        let st = self.lock();
        if after > st.seq {
            // A number from another run (the app kept it across a new `Core`): nothing can be
            // said exactly.
            return Some(ChangeBatch {
                seq: st.seq,
                everything: true,
                folders: vec![],
                accounts: vec![],
                failures: vec![],
            });
        }
        if after == st.seq {
            return None;
        }
        let oldest = st.ring.front().map_or(st.seq + 1, |(n, _)| *n);
        let mut everything = after + 1 < oldest;
        let mut folders = BTreeSet::new();
        let mut accounts = BTreeSet::new();
        let mut failures = Vec::new();
        for (n, e) in &st.ring {
            if *n <= after {
                continue;
            }
            match e {
                Entry::Folder(f) => {
                    folders.insert(*f);
                }
                Entry::Account(a) => {
                    accounts.insert(*a);
                }
                Entry::Failure(f) => failures.push(*f),
                Entry::Reset => everything = true,
            }
        }
        Some(ChangeBatch {
            seq: st.seq,
            everything,
            folders: folders.into_iter().collect(),
            accounts: accounts.into_iter().collect(),
            failures,
        })
    }
}

fn failure_reason(reason: &str) -> FailureReason {
    if reason == REASON_LOST_AFTER_MOVE {
        FailureReason::LostAfterMove
    } else if reason == REASON_LOST_AFTER_RESET {
        FailureReason::LostAfterReset
    } else if reason == REASON_AMBIGUOUS_COPY {
        FailureReason::AmbiguousCopy
    } else {
        FailureReason::Rejected
    }
}

/// Feeds the engine's changes into the hub until the engine is gone.
pub(crate) async fn listen(hub: Arc<Hub>, mut changes: broadcast::Receiver<Change>) {
    loop {
        match changes.recv().await {
            Ok(change) => hub.apply(change),
            Err(broadcast::error::RecvError::Lagged(_)) => hub.reset(),
            Err(broadcast::error::RecvError::Closed) => break,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Nobody;
    impl CredentialProvider for Nobody {
        fn credential(
            &self,
            _: i64,
        ) -> Result<Option<std::sync::Arc<crate::FfiCredential>>, crate::ProviderError> {
            Ok(None)
        }
        fn sign_in_needed(&self, _: i64) -> Result<(), crate::ProviderError> {
            Ok(())
        }
    }

    fn hub() -> Arc<Hub> {
        Hub::new(Arc::new(Nobody))
    }

    fn messages(account: i64, folders: &[i64]) -> Change {
        Change::Messages {
            account,
            folders: folders.to_vec(),
        }
    }

    #[test]
    fn nothing_new_is_none_and_changes_merge() {
        let h = hub();
        assert!(h.batch_after(0).is_none());
        h.apply(messages(1, &[10]));
        h.apply(messages(1, &[10, 11]));
        h.apply(messages(2, &[20]));
        h.apply(Change::Folders { account: 1 });
        let b = h.batch_after(0).unwrap();
        assert_eq!(b.seq, 5);
        assert!(!b.everything);
        assert_eq!(
            b.folders,
            vec![
                FolderRef {
                    account: 1,
                    folder_id: 10
                },
                FolderRef {
                    account: 1,
                    folder_id: 11
                },
                FolderRef {
                    account: 2,
                    folder_id: 20
                },
            ]
        );
        assert_eq!(b.accounts, vec![1]);
        // Asking again from the returned number: nothing; from the middle: only the tail.
        assert!(h.batch_after(b.seq).is_none());
        let tail = h.batch_after(3).unwrap();
        assert_eq!(
            tail.folders,
            vec![FolderRef {
                account: 2,
                folder_id: 20
            }]
        );
        assert_eq!(tail.accounts, vec![1]);
    }

    #[test]
    fn a_caller_that_fell_off_the_ring_is_told_to_re_run_everything() {
        let h = hub();
        for i in 0..(RING as i64 + 10) {
            h.apply(messages(1, &[i]));
        }
        let behind = h.batch_after(0).unwrap();
        assert!(behind.everything);
        let seq = behind.seq;
        // Just inside the ring: exact.
        let recent = h.batch_after(seq - 5).unwrap();
        assert!(!recent.everything);
        assert_eq!(recent.folders.len(), 5);
    }

    #[test]
    fn a_number_from_another_run_means_everything() {
        let h = hub();
        h.apply(messages(1, &[1]));
        let b = h.batch_after(99).unwrap();
        assert!(b.everything);
        assert_eq!(b.seq, 1);
    }

    #[test]
    fn an_engine_overflow_means_everything() {
        let h = hub();
        h.reset();
        assert!(h.batch_after(0).unwrap().everything);
    }

    #[test]
    fn failures_carry_a_fixed_reason() {
        let h = hub();
        h.apply(Change::OperationFailed {
            account: 1,
            op: 7,
            reason: REASON_AMBIGUOUS_COPY.to_owned(),
        });
        h.apply(Change::OperationFailed {
            account: 1,
            op: 8,
            reason: "anything the server said".to_owned(),
        });
        let b = h.batch_after(0).unwrap();
        assert_eq!(
            b.failures,
            vec![
                OpFailure {
                    account: 1,
                    op_id: 7,
                    reason: FailureReason::AmbiguousCopy
                },
                OpFailure {
                    account: 1,
                    op_id: 8,
                    reason: FailureReason::Rejected
                },
            ]
        );
    }
}
