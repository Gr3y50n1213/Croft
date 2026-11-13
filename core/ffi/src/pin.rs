// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Pin and follow-up (T36b): the keywords `$Pinned` and `$FollowUp`, set through the queue like a
//! star, so they reach the server and every client that reads them. A folder that cannot keep the
//! keyword makes the operation fail (`FailureReason::Rejected`) and the change is undone locally.

use croft_protocols::Keyword;
use croft_storage::accounts;

use crate::core::{now_ms, Core};
use crate::error::CoreError;
use crate::list::{self, Mark};
use crate::types::{MessagePage, MessageRow, RowCursor};

impl Core {
    /// Sets or clears `keyword` on each message; returns the operation ids for Undo (a message that
    /// already had the state gives none).
    fn set_keyword(
        &self,
        ids: &[i64],
        keyword: Keyword,
        value: bool,
    ) -> Result<Vec<u64>, CoreError> {
        let mut ops = Vec::new();
        for m in self.inner.resolve(ids)? {
            if let Some(op) = self
                .inner
                .handle
                .set_keyword(m.account_id, m.id, keyword, value)?
            {
                ops.push(op);
            }
        }
        Ok(ops)
    }
}

#[uniffi::export]
impl Core {
    /// Pins or unpins messages. A pinned message in an Inbox is shown in its Pinned section
    /// (`list_pinned`) instead of its list. Returns the operation ids for Undo.
    pub fn set_pinned(&self, ids: Vec<i64>, value: bool) -> Result<Vec<u64>, CoreError> {
        self.set_keyword(&ids, Keyword::Pinned, value)
    }

    /// Marks messages to follow up, or clears the mark ("Done"). Returns the operation ids for
    /// Undo.
    pub fn set_follow_up(&self, ids: Vec<i64>, value: bool) -> Result<Vec<u64>, CoreError> {
        self.set_keyword(&ids, Keyword::FollowUp, value)
    }

    /// The pinned messages in the Inbox of `account`, or of every account (`None`), newest first,
    /// at most 20 (the Inbox's Pinned section).
    pub fn list_pinned(&self, account: Option<i64>) -> Result<Vec<MessageRow>, CoreError> {
        self.inner.handle.read(|db| {
            let conn = db.connection();
            let mut out = Vec::new();
            for a in wanted(conn, account)? {
                out.extend(list::pinned_rows(conn, a, now_ms())?);
            }
            out.sort_by_key(|r| std::cmp::Reverse((r.date_ms, r.id)));
            out.truncate(list::MAX_PINNED as usize);
            Ok(out)
        })
    }

    /// One page of the Follow up mailbox of `account`, or of every account (`None`): the marked
    /// messages of every folder but Trash and Junk, newest first, paged like `list_messages`.
    pub fn list_follow_up(
        &self,
        account: Option<i64>,
        before: Option<RowCursor>,
        limit: u32,
    ) -> Result<MessagePage, CoreError> {
        let limit = limit.min(list::MAX_PAGE);
        self.inner.handle.read(|db| {
            let conn = db.connection();
            let want = limit + 1;
            let mut rows = Vec::new();
            for a in wanted(conn, account)? {
                rows.extend(list::marked_rows(
                    conn,
                    a,
                    Mark::FollowUp,
                    now_ms(),
                    before,
                    want,
                )?);
            }
            rows.sort_by_key(|r| std::cmp::Reverse((r.date_ms, r.id)));
            let more = rows.len() > limit as usize;
            rows.truncate(limit as usize);
            let next = if more {
                rows.last().map(|r| RowCursor {
                    date_ms: r.date_ms,
                    id: r.id,
                })
            } else {
                None
            };
            Ok(MessagePage { rows, next })
        })
    }
}

/// The accounts a call covers: the named one (an unknown id then has no rows), or every account.
fn wanted(conn: &rusqlite::Connection, account: Option<i64>) -> Result<Vec<i64>, CoreError> {
    Ok(match account {
        Some(a) => vec![a],
        None => accounts::list(conn)?.into_iter().map(|a| a.id).collect(),
    })
}
