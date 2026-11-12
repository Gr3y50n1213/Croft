// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Which new messages get a notification (S32, T20). Pure: the caller passes the Inbox's rows
//! and the stored marker, and posts what this returns.
//!
//! The marker is "notified up to this UID" for the account's Inbox, under the Inbox's
//! UIDVALIDITY. A message is notifiable when it is in the Inbox, unread, and its UID is above
//! the marker. Not notifiable:
//! - anything while there is no marker yet (the first sync, S09's backfill): a baseline is set
//!   at the newest UID instead, so the mail that was already there never notifies;
//! - anything after the Inbox was renumbered (another UIDVALIDITY): a new baseline, same reason;
//! - the sample mailbox (the caller passes `demo`);
//! - a message already read (on another device, or here) when it is looked at.
//!
//! The marker moves forward **only after** a notification was posted ([`advance`]), so a
//! notification that failed to post is offered again next time, and a duplicate delivery of the
//! same row is offered once.

use std::collections::BTreeSet;

/// The stored "notified up to" marker of one account's Inbox.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Marker {
    /// The Inbox's UIDVALIDITY when the marker was set.
    pub uid_validity: u32,
    /// The highest UID notified (or the baseline: the newest UID when the marker was set).
    pub up_to: u32,
}

/// One Inbox row as the decision needs it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InboxRow {
    /// The local row id (what the notification opens).
    pub id: i64,
    /// The server UID.
    pub uid: u32,
    /// Read.
    pub seen: bool,
}

/// What [`decide`] says.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Decision {
    /// No marker yet, or the Inbox was renumbered: store this baseline and notify nothing.
    Baseline(Marker),
    /// Notify these rows (oldest first, each once). The marker moves only once they are posted.
    Notify(Vec<InboxRow>),
}

/// Decides what to notify for an account's Inbox. `rows` is the Inbox after a sync (any order);
/// `uid_validity` is the Inbox's current one; `marker` the stored one, if any.
pub fn decide(
    rows: &[InboxRow],
    uid_validity: u32,
    marker: Option<Marker>,
    demo: bool,
) -> Decision {
    let newest = rows.iter().map(|r| r.uid).max().unwrap_or(0);
    let marker = match marker {
        Some(m) if m.uid_validity == uid_validity => m,
        // First sync, or renumbered: what is there now is the baseline.
        _ => {
            return Decision::Baseline(Marker {
                uid_validity,
                up_to: newest,
            })
        }
    };
    if demo {
        return Decision::Notify(Vec::new());
    }
    let mut seen_uids = BTreeSet::new();
    let mut out: Vec<InboxRow> = rows
        .iter()
        .filter(|r| r.uid > marker.up_to && !r.seen)
        .filter(|r| seen_uids.insert(r.uid))
        .copied()
        .collect();
    out.sort_by_key(|r| r.uid);
    Decision::Notify(out)
}

/// The marker after `posted` were notified: up to the highest posted UID (never backwards).
pub fn advance(marker: Marker, posted: &[u32]) -> Marker {
    let up_to = posted
        .iter()
        .copied()
        .max()
        .map_or(marker.up_to, |p| p.max(marker.up_to));
    Marker {
        uid_validity: marker.uid_validity,
        up_to,
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic)]
mod tests {
    use super::*;

    fn row(id: i64, uid: u32, seen: bool) -> InboxRow {
        InboxRow { id, uid, seen }
    }

    const V: u32 = 7;

    fn notify(d: Decision) -> Vec<u32> {
        match d {
            Decision::Notify(r) => r.into_iter().map(|r| r.uid).collect(),
            Decision::Baseline(m) => panic!("baseline {m:?}"),
        }
    }

    #[test]
    fn the_first_sync_never_notifies_and_sets_a_baseline() {
        let rows = [row(1, 10, false), row(2, 11, false)];
        assert_eq!(
            decide(&rows, V, None, false),
            Decision::Baseline(Marker {
                uid_validity: V,
                up_to: 11
            })
        );
        assert_eq!(
            decide(&[], V, None, false),
            Decision::Baseline(Marker {
                uid_validity: V,
                up_to: 0
            })
        );
    }

    #[test]
    fn only_unread_inbox_rows_above_the_marker_notify_oldest_first() {
        let m = Marker {
            uid_validity: V,
            up_to: 11,
        };
        let rows = [
            row(1, 10, false),
            row(2, 11, false),
            row(4, 14, false),
            row(3, 12, true),
            row(5, 13, false),
        ];
        assert_eq!(notify(decide(&rows, V, Some(m), false)), vec![13, 14]);
    }

    #[test]
    fn a_message_read_elsewhere_is_not_notified() {
        let m = Marker {
            uid_validity: V,
            up_to: 1,
        };
        assert!(notify(decide(&[row(1, 2, true)], V, Some(m), false)).is_empty());
    }

    #[test]
    fn a_renumbered_inbox_is_a_new_baseline() {
        let m = Marker {
            uid_validity: V,
            up_to: 500,
        };
        assert_eq!(
            decide(&[row(1, 3, false)], V + 1, Some(m), false),
            Decision::Baseline(Marker {
                uid_validity: V + 1,
                up_to: 3
            })
        );
    }

    #[test]
    fn the_sample_mailbox_never_notifies() {
        let m = Marker {
            uid_validity: V,
            up_to: 1,
        };
        assert!(notify(decide(&[row(1, 2, false)], V, Some(m), true)).is_empty());
    }

    #[test]
    fn the_marker_moves_only_after_posting_and_never_back() {
        let m = Marker {
            uid_validity: V,
            up_to: 11,
        };
        let rows = [row(1, 12, false), row(2, 13, false)];
        // Nothing posted (the notification failed): offered again.
        let m2 = advance(m, &[]);
        assert_eq!(notify(decide(&rows, V, Some(m2), false)), vec![12, 13]);
        // Posted: not offered again.
        let m3 = advance(m2, &[12, 13]);
        assert_eq!(m3.up_to, 13);
        assert!(notify(decide(&rows, V, Some(m3), false)).is_empty());
        assert_eq!(advance(m3, &[5]).up_to, 13, "never backwards");
    }

    #[test]
    fn a_duplicate_delivery_is_offered_once() {
        let m = Marker {
            uid_validity: V,
            up_to: 1,
        };
        let rows = [row(1, 5, false), row(1, 5, false), row(9, 5, false)];
        assert_eq!(notify(decide(&rows, V, Some(m), false)), vec![5]);
    }
}
