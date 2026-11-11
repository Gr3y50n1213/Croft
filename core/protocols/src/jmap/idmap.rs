// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The persistent map between JMAP ids and the `u32` UIDs the rest of Croft uses.
//!
//! `MailBackend` names a message by (folder, UID, folder UIDVALIDITY). JMAP ids are strings, stable
//! across moves, and one message can be in several mailboxes. The adapter keeps, per account and
//! per mailbox, a synthetic UID for each JMAP email id, and a UIDVALIDITY for the mailbox. The map
//! lives behind this trait, so `croft-protocols` needs no database; `croft-storage` implements it
//! over SQLite and [`MemoryIdStore`] is the in-memory version for tests.
//!
//! Rules every implementation keeps (the adapter relies on them):
//!
//! - UIDs of one mailbox are assigned in increasing order, starting at 1, and **never reused**:
//!   removing an entry does not lower the next UID. A UID is never given to two emails.
//! - A mailbox has one UIDVALIDITY, chosen at random (never 0) when the mailbox is first seen. It
//!   changes only through [`reset_mailbox`](JmapIdStore::reset_mailbox), which also forgets every
//!   entry, and the new value differs from the old one.
//! - An email that is in two mailboxes has an independent UID in each.
//!
//! All methods are synchronous and short. An implementation that cannot answer returns
//! [`IdStoreError::Unavailable`] (a retryable problem); one that finds its own data inconsistent
//! returns [`IdStoreError::Corrupt`], and the adapter then resets that mailbox, which makes the sync
//! engine drop and re-fetch its local copy of the folder.

use crate::types::Uid;
use std::collections::{BTreeMap, HashMap};
use std::sync::Mutex;

/// Why the id map could not answer. Fixed variants only: nothing here carries message data.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdStoreError {
    /// The map could not be read or written right now (a locked or failing database). Retry later.
    Unavailable,
    /// The map is inconsistent (an entry at or above the next UID, a missing counter). The adapter
    /// resets the mailbox.
    Corrupt,
    /// The mailbox has used all 4 294 967 295 UIDs. The adapter resets the mailbox.
    Exhausted,
}

impl std::fmt::Display for IdStoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Unavailable => "the local message map is not available",
            Self::Corrupt => "the local message map is inconsistent",
            Self::Exhausted => "the folder has used up its message numbers",
        })
    }
}

impl std::error::Error for IdStoreError {}

/// The id map of one account. See the module notes for the rules.
pub trait JmapIdStore: Send + Sync {
    /// The mailbox's UIDVALIDITY, chosen at random and stored if the mailbox is new.
    fn uid_validity(&self, mailbox: &str) -> Result<u32, IdStoreError>;

    /// Forgets every entry of the mailbox and gives it a new UIDVALIDITY (different from the old
    /// one), returned. The next UID starts again at 1.
    fn reset_mailbox(&self, mailbox: &str) -> Result<u32, IdStoreError>;

    /// Forgets the mailbox completely (it no longer exists on the server). A later
    /// [`uid_validity`](Self::uid_validity) makes a new one. The old UIDVALIDITY is gone, so the
    /// new random value repeats it with a chance of about 2^-32 (accepted).
    fn forget_mailbox(&self, mailbox: &str) -> Result<(), IdStoreError>;

    /// The UID the next new email in the mailbox will get.
    fn next_uid(&self, mailbox: &str) -> Result<Uid, IdStoreError>;

    /// The UID of each email id, in the same order. An id without one gets the next UID; new ids
    /// are numbered in the order given, so the caller sorts oldest first. Creates the mailbox's
    /// UIDVALIDITY if needed. Fails with [`IdStoreError::Exhausted`] without assigning anything
    /// if the UIDs would run out.
    fn assign(&self, mailbox: &str, email_ids: &[String]) -> Result<Vec<Uid>, IdStoreError>;

    /// The email id of each UID that has one. Unknown UIDs are absent from the answer.
    fn email_ids(&self, mailbox: &str, uids: &[Uid])
        -> Result<BTreeMap<Uid, String>, IdStoreError>;

    /// Every entry of the mailbox: UID to email id.
    fn entries(&self, mailbox: &str) -> Result<BTreeMap<Uid, String>, IdStoreError>;

    /// Removes the entries of these email ids from the mailbox (unknown ids are ignored). The next
    /// UID does not change.
    fn remove(&self, mailbox: &str, email_ids: &[String]) -> Result<(), IdStoreError>;

    /// The newest `Email` state string the map was reconciled with, if any.
    fn email_state(&self) -> Result<Option<String>, IdStoreError>;

    /// Records the `Email` state string (or clears it with `None`).
    fn set_email_state(&self, state: Option<&str>) -> Result<(), IdStoreError>;
}

/// A fresh random UIDVALIDITY: never 0 and never `avoid`. Randomness comes from the standard
/// library's per-process random hash seeds mixed with the clock; it only has to differ from the
/// previous value, not to be unpredictable.
pub fn fresh_uid_validity(avoid: Option<u32>) -> u32 {
    use std::hash::{BuildHasher, Hasher};
    loop {
        let mut h = std::collections::hash_map::RandomState::new().build_hasher();
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        h.write_u128(nanos);
        let v = (h.finish() >> 16) as u32;
        if v != 0 && Some(v) != avoid {
            return v;
        }
    }
}

#[derive(Debug, Default)]
struct MailboxMap {
    validity: u32,
    next_uid: u64,
    by_email: HashMap<String, Uid>,
    by_uid: BTreeMap<Uid, String>,
}

#[derive(Debug, Default)]
struct MemoryInner {
    mailboxes: HashMap<String, MailboxMap>,
    email_state: Option<String>,
    fail: Option<IdStoreError>,
    fail_once: Option<IdStoreError>,
}

/// An in-memory [`JmapIdStore`], for tests and for an account that is not kept on disk. It also
/// lets a test simulate a lost map ([`forget_all`](Self::forget_all)) and a failing one
/// ([`fail_with`](Self::fail_with)).
#[derive(Debug, Default)]
pub struct MemoryIdStore {
    inner: Mutex<MemoryInner>,
}

impl MemoryIdStore {
    /// An empty map.
    pub fn new() -> Self {
        Self::default()
    }

    /// Forgets everything, as if the database had been deleted.
    pub fn forget_all(&self) {
        if let Ok(mut g) = self.inner.lock() {
            *g = MemoryInner::default();
        }
    }

    /// Makes every call fail with `error` until called again with `None`.
    pub fn fail_with(&self, error: Option<IdStoreError>) {
        if let Ok(mut g) = self.inner.lock() {
            g.fail = error;
        }
    }

    /// Makes the next call, and only that one, fail with `error`.
    pub fn fail_next(&self, error: Option<IdStoreError>) {
        if let Ok(mut g) = self.inner.lock() {
            g.fail_once = error;
        }
    }

    fn with<T>(
        &self,
        f: impl FnOnce(&mut MemoryInner) -> Result<T, IdStoreError>,
    ) -> Result<T, IdStoreError> {
        let mut g = self.inner.lock().map_err(|_| IdStoreError::Unavailable)?;
        if let Some(e) = g.fail_once.take() {
            return Err(e);
        }
        if let Some(e) = g.fail {
            return Err(e);
        }
        f(&mut g)
    }
}

fn mailbox_mut<'a>(inner: &'a mut MemoryInner, mailbox: &str) -> &'a mut MailboxMap {
    inner
        .mailboxes
        .entry(mailbox.to_string())
        .or_insert_with(|| MailboxMap {
            validity: fresh_uid_validity(None),
            next_uid: 1,
            ..MailboxMap::default()
        })
}

impl JmapIdStore for MemoryIdStore {
    fn uid_validity(&self, mailbox: &str) -> Result<u32, IdStoreError> {
        self.with(|i| Ok(mailbox_mut(i, mailbox).validity))
    }

    fn reset_mailbox(&self, mailbox: &str) -> Result<u32, IdStoreError> {
        self.with(|i| {
            let old = i.mailboxes.get(mailbox).map(|m| m.validity);
            let fresh = MailboxMap {
                validity: fresh_uid_validity(old),
                next_uid: 1,
                ..MailboxMap::default()
            };
            let v = fresh.validity;
            i.mailboxes.insert(mailbox.to_string(), fresh);
            Ok(v)
        })
    }

    fn forget_mailbox(&self, mailbox: &str) -> Result<(), IdStoreError> {
        self.with(|i| {
            i.mailboxes.remove(mailbox);
            Ok(())
        })
    }

    fn next_uid(&self, mailbox: &str) -> Result<Uid, IdStoreError> {
        self.with(|i| {
            Uid::try_from(mailbox_mut(i, mailbox).next_uid).map_err(|_| IdStoreError::Exhausted)
        })
    }

    fn assign(&self, mailbox: &str, email_ids: &[String]) -> Result<Vec<Uid>, IdStoreError> {
        self.with(|i| {
            let m = mailbox_mut(i, mailbox);
            let mut fresh: Vec<&String> = Vec::new();
            let mut queued: std::collections::HashSet<&String> = std::collections::HashSet::new();
            for id in email_ids {
                if !m.by_email.contains_key(id) && queued.insert(id) {
                    fresh.push(id);
                }
            }
            if m.next_uid - 1 + fresh.len() as u64 > u64::from(u32::MAX) {
                return Err(IdStoreError::Exhausted);
            }
            for id in fresh {
                let uid = m.next_uid as Uid;
                m.next_uid += 1;
                m.by_email.insert(id.clone(), uid);
                m.by_uid.insert(uid, id.clone());
            }
            Ok(email_ids.iter().map(|id| m.by_email[id]).collect())
        })
    }

    fn email_ids(
        &self,
        mailbox: &str,
        uids: &[Uid],
    ) -> Result<BTreeMap<Uid, String>, IdStoreError> {
        self.with(|i| {
            let m = mailbox_mut(i, mailbox);
            Ok(uids
                .iter()
                .filter_map(|u| m.by_uid.get(u).map(|e| (*u, e.clone())))
                .collect())
        })
    }

    fn entries(&self, mailbox: &str) -> Result<BTreeMap<Uid, String>, IdStoreError> {
        self.with(|i| Ok(mailbox_mut(i, mailbox).by_uid.clone()))
    }

    fn remove(&self, mailbox: &str, email_ids: &[String]) -> Result<(), IdStoreError> {
        self.with(|i| {
            let m = mailbox_mut(i, mailbox);
            for id in email_ids {
                if let Some(uid) = m.by_email.remove(id) {
                    m.by_uid.remove(&uid);
                }
            }
            Ok(())
        })
    }

    fn email_state(&self) -> Result<Option<String>, IdStoreError> {
        self.with(|i| Ok(i.email_state.clone()))
    }

    fn set_email_state(&self, state: Option<&str>) -> Result<(), IdStoreError> {
        self.with(|i| {
            i.email_state = state.map(str::to_string);
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn uids_increase_and_are_never_reused() {
        let s = MemoryIdStore::new();
        assert_eq!(
            s.assign("m", &ids(&["a", "b", "c"])).unwrap(),
            vec![1, 2, 3]
        );
        s.remove("m", &ids(&["c", "b"])).unwrap();
        assert_eq!(s.next_uid("m").unwrap(), 4);
        assert_eq!(s.assign("m", &ids(&["d", "a"])).unwrap(), vec![4, 1]);
        // An id that was removed and comes back is a new message as far as UIDs go.
        assert_eq!(s.assign("m", &ids(&["c"])).unwrap(), vec![5]);
        assert_eq!(
            s.entries("m").unwrap().into_iter().collect::<Vec<_>>(),
            vec![(1, "a".into()), (4, "d".into()), (5, "c".into())]
        );
    }

    #[test]
    fn duplicates_in_one_call_get_one_uid() {
        let s = MemoryIdStore::new();
        assert_eq!(
            s.assign("m", &ids(&["a", "a", "b"])).unwrap(),
            vec![1, 1, 2]
        );
    }

    #[test]
    fn mailboxes_number_independently_and_have_their_own_validity() {
        let s = MemoryIdStore::new();
        assert_eq!(s.assign("m1", &ids(&["a"])).unwrap(), vec![1]);
        assert_eq!(s.assign("m2", &ids(&["a"])).unwrap(), vec![1]);
        let (v1, v2) = (s.uid_validity("m1").unwrap(), s.uid_validity("m2").unwrap());
        assert!(v1 != 0 && v2 != 0);
        assert_eq!(s.uid_validity("m1").unwrap(), v1);
    }

    #[test]
    fn reset_gives_a_new_validity_and_restarts_numbering() {
        let s = MemoryIdStore::new();
        s.assign("m", &ids(&["a", "b"])).unwrap();
        let before = s.uid_validity("m").unwrap();
        let after = s.reset_mailbox("m").unwrap();
        assert_ne!(before, after);
        assert_eq!(s.uid_validity("m").unwrap(), after);
        assert!(s.entries("m").unwrap().is_empty());
        assert_eq!(s.next_uid("m").unwrap(), 1);
    }

    #[test]
    fn a_lost_map_comes_back_with_another_validity() {
        let s = MemoryIdStore::new();
        let before = s.uid_validity("m").unwrap();
        s.forget_all();
        assert_ne!(s.uid_validity("m").unwrap(), before);
    }

    #[test]
    fn email_ids_leave_out_unknown_uids_and_state_round_trips() {
        let s = MemoryIdStore::new();
        s.assign("m", &ids(&["a", "b"])).unwrap();
        let got = s.email_ids("m", &[2, 9]).unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[&2], "b");
        assert_eq!(s.email_state().unwrap(), None);
        s.set_email_state(Some("s1")).unwrap();
        assert_eq!(s.email_state().unwrap().as_deref(), Some("s1"));
        s.set_email_state(None).unwrap();
        assert_eq!(s.email_state().unwrap(), None);
    }

    #[test]
    fn fresh_validity_is_never_zero_or_the_old_one() {
        for _ in 0..200 {
            let v = fresh_uid_validity(Some(7));
            assert!(v != 0 && v != 7);
        }
    }

    #[test]
    fn injected_failures_are_reported() {
        let s = MemoryIdStore::new();
        s.fail_with(Some(IdStoreError::Unavailable));
        assert_eq!(s.next_uid("m"), Err(IdStoreError::Unavailable));
        s.fail_with(None);
        assert!(s.next_uid("m").is_ok());
    }
}
