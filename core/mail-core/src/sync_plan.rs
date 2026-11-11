//! Decides what a folder sync has to do. Pure function, no I/O.

use croft_protocols::{Flags, RemoteSnapshot, Uid};
use std::collections::{BTreeMap, BTreeSet};

/// What the local database knows about one folder.
#[derive(Debug, Clone, Default)]
pub struct LocalFolder {
    /// UIDVALIDITY seen at the last sync. `None` before the first sync.
    pub uid_validity: Option<u32>,
    /// Cached messages and their flags.
    pub messages: BTreeMap<Uid, Flags>,
    /// UIDs with a queued or running local change. The server's flags must not overwrite these,
    /// or a message the person just marked read would flicker back to unread until the queue
    /// catches up.
    pub pending: BTreeSet<Uid>,
}

/// The work a sync has to do for one folder.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SyncPlan {
    /// UIDVALIDITY changed: every cached UID is meaningless and was listed in `remove`. Queued
    /// operations for this folder refer to UIDs that no longer exist and must be re-resolved or
    /// failed by the caller.
    pub reset: bool,
    /// New messages to fetch headers for, highest UID (newest) first so the top of the inbox fills first.
    pub fetch: Vec<Uid>,
    /// Messages whose flags changed on the server.
    pub update_flags: Vec<(Uid, Flags)>,
    /// Messages gone from the server.
    pub remove: Vec<Uid>,
}

impl SyncPlan {
    /// True when nothing needs doing.
    pub fn is_noop(&self) -> bool {
        !self.reset
            && self.fetch.is_empty()
            && self.update_flags.is_empty()
            && self.remove.is_empty()
    }
}

/// Compares the server's folder with the local copy.
pub fn plan(local: &LocalFolder, remote: &RemoteSnapshot) -> SyncPlan {
    let validity_changed = local
        .uid_validity
        .is_some_and(|v| v != remote.status.uid_validity);

    if validity_changed {
        return SyncPlan {
            reset: true,
            fetch: remote.messages.keys().rev().copied().collect(),
            update_flags: Vec::new(),
            remove: local.messages.keys().copied().collect(),
        };
    }

    let fetch = remote
        .messages
        .keys()
        .rev()
        .filter(|uid| !local.messages.contains_key(uid))
        .copied()
        .collect();
    let remove = local
        .messages
        .keys()
        .filter(|uid| !remote.messages.contains_key(uid))
        .copied()
        .collect();
    let update_flags = remote
        .messages
        .iter()
        .filter(|(uid, _)| !local.pending.contains(uid))
        .filter(|(uid, flags)| local.messages.get(uid).is_some_and(|mine| mine != *flags))
        .map(|(uid, flags)| (*uid, flags.clone()))
        .collect();

    SyncPlan {
        reset: false,
        fetch,
        update_flags,
        remove,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use croft_protocols::FolderStatus;

    fn seen() -> Flags {
        Flags {
            seen: true,
            ..Flags::default()
        }
    }

    fn remote(validity: u32, msgs: &[(Uid, Flags)]) -> RemoteSnapshot {
        RemoteSnapshot {
            status: FolderStatus {
                uid_validity: validity,
                uid_next: 100,
                highest_modseq: None,
                exists: msgs.len() as u32,
            },
            messages: msgs.iter().cloned().collect(),
        }
    }

    fn local(validity: Option<u32>, msgs: &[(Uid, Flags)]) -> LocalFolder {
        LocalFolder {
            uid_validity: validity,
            messages: msgs.iter().cloned().collect(),
            pending: BTreeSet::new(),
        }
    }

    #[test]
    fn first_sync_fetches_everything_newest_first() {
        let p = plan(
            &local(None, &[]),
            &remote(
                7,
                &[(1, seen()), (2, Flags::default()), (3, Flags::default())],
            ),
        );
        assert!(!p.reset);
        assert_eq!(p.fetch, vec![3, 2, 1]);
        assert!(p.remove.is_empty() && p.update_flags.is_empty());
    }

    #[test]
    fn identical_state_is_a_noop() {
        let msgs = [(1, seen()), (2, Flags::default())];
        assert!(plan(&local(Some(7), &msgs), &remote(7, &msgs)).is_noop());
    }

    #[test]
    fn new_gone_and_changed_messages_are_found_together() {
        let l = local(
            Some(7),
            &[(1, Flags::default()), (2, Flags::default()), (3, seen())],
        );
        let r = remote(
            7,
            &[
                (2, seen()),
                (3, seen()),
                (4, Flags::default()),
                (5, Flags::default()),
            ],
        );
        let p = plan(&l, &r);
        assert_eq!(p.fetch, vec![5, 4]);
        assert_eq!(p.remove, vec![1]);
        assert_eq!(p.update_flags, vec![(2, seen())]);
        assert!(!p.reset);
    }

    #[test]
    fn changed_uid_validity_resets_the_folder() {
        let l = local(Some(7), &[(1, seen()), (2, seen())]);
        let r = remote(
            8,
            &[
                (1, Flags::default()),
                (2, Flags::default()),
                (3, Flags::default()),
            ],
        );
        let p = plan(&l, &r);
        assert!(p.reset);
        assert_eq!(p.remove, vec![1, 2]);
        assert_eq!(p.fetch, vec![3, 2, 1]);
        assert!(
            p.update_flags.is_empty(),
            "after a reset flags come with the re-fetched headers"
        );
    }

    #[test]
    fn local_changes_in_flight_are_not_overwritten() {
        let mut l = local(Some(7), &[(1, seen()), (2, Flags::default())]);
        l.pending.insert(1);
        // The server still says unread for 1 because our "mark read" has not been applied yet.
        let r = remote(7, &[(1, Flags::default()), (2, seen())]);
        let p = plan(&l, &r);
        assert_eq!(p.update_flags, vec![(2, seen())]);
    }

    #[test]
    fn a_pending_message_that_vanished_is_still_removed() {
        let mut l = local(Some(7), &[(1, seen())]);
        l.pending.insert(1);
        let p = plan(&l, &remote(7, &[]));
        assert_eq!(p.remove, vec![1]);
    }

    #[test]
    fn empty_remote_folder_removes_everything_without_reset() {
        let p = plan(
            &local(Some(7), &[(1, seen()), (2, seen())]),
            &remote(7, &[]),
        );
        assert!(!p.reset);
        assert_eq!(p.remove, vec![1, 2]);
    }
}
