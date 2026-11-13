// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Inbox categories for the app (T35): the Inbox filtered by category, the unread count per
//! category (S10's chips), and moving a message to a category, with "Always do this" for its
//! sender. Categories are the device's own: nothing here reaches the server.

use croft_engine::Change;
use croft_mail_core::category::Category;
use croft_storage::{accounts, folders, messages};
use rusqlite::params;

use crate::core::{now_ms, Core};
use crate::error::CoreError;
use crate::list;
use crate::types::{MessagePage, MessageScope, RowCursor};

/// An Inbox category (S10's chips, S49).
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum InboxCategory {
    /// Mail from people.
    Primary,
    /// Notifications, receipts and mailing lists.
    Updates,
    /// Bulk and marketing mail.
    Promotions,
    /// Social networks.
    Social,
}

impl From<InboxCategory> for Category {
    fn from(c: InboxCategory) -> Self {
        match c {
            InboxCategory::Primary => Category::Primary,
            InboxCategory::Updates => Category::Updates,
            InboxCategory::Promotions => Category::Promotions,
            InboxCategory::Social => Category::Social,
        }
    }
}

impl From<Category> for InboxCategory {
    fn from(c: Category) -> Self {
        match c {
            Category::Primary => InboxCategory::Primary,
            Category::Updates => InboxCategory::Updates,
            Category::Promotions => InboxCategory::Promotions,
            Category::Social => InboxCategory::Social,
        }
    }
}

/// The unread messages of one category in the Inbox.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CategoryCount {
    /// The category.
    pub category: InboxCategory,
    /// Unread messages in it (snoozed and hidden ones left out, as in the list).
    pub unread: u64,
}

#[uniffi::export]
impl Core {
    /// [`Core::list_messages`] with only the messages of `category` (in the Inbox scopes: one
    /// account's Inbox folder, or the unified Inbox). `None` is the whole list. Starred ignores it.
    pub fn list_messages_in(
        &self,
        scope: MessageScope,
        category: Option<InboxCategory>,
        before: Option<RowCursor>,
        limit: u32,
    ) -> Result<MessagePage, CoreError> {
        let category = category.map(|c| Category::from(c).as_str());
        self.inner.handle.read(|db| {
            let conn = db.connection();
            let ids = match scope {
                MessageScope::UnifiedInbox | MessageScope::Starred { account: None } => {
                    accounts::list(conn)?.into_iter().map(|a| a.id).collect()
                }
                MessageScope::Folder { .. } | MessageScope::Starred { .. } => Vec::new(),
            };
            list::page(conn, &ids, scope, now_ms(), before, limit, category)
        })
    }

    /// The unread count per category in the Inbox of `account`, or of every account (`None`),
    /// in the chips' order. An unknown account gives zeros.
    pub fn inbox_category_counts(
        &self,
        account: Option<i64>,
    ) -> Result<Vec<CategoryCount>, CoreError> {
        self.inner.handle.read(|db| {
            let conn = db.connection();
            let wanted: Vec<i64> = match account {
                Some(a) => vec![a],
                None => accounts::list(conn)?.into_iter().map(|a| a.id).collect(),
            };
            let mut totals: Vec<(Category, u64)> = Category::ALL.iter().map(|c| (*c, 0)).collect();
            for a in wanted {
                for f in folders::list(conn, a)? {
                    if f.special_use != "inbox" {
                        continue;
                    }
                    for (c, n) in messages::unread_by_category(conn, a, f.id, now_ms())? {
                        if let Some(slot) = totals.iter_mut().find(|(k, _)| *k == c) {
                            slot.1 += n;
                        }
                    }
                }
            }
            Ok(totals
                .into_iter()
                .map(|(c, n)| CategoryCount {
                    category: c.into(),
                    unread: n,
                })
                .collect())
        })
    }

    /// Moves a message to `category` (S10's long-press "Move to"). With `always`, mail from its
    /// sender goes there from now on and the sender's stored mail is re-filed ("Always do this").
    pub fn set_message_category(
        &self,
        message_id: i64,
        category: InboxCategory,
        always: bool,
    ) -> Result<(), CoreError> {
        let m = self
            .inner
            .resolve(&[message_id])?
            .into_iter()
            .next()
            .ok_or(CoreError::NoSuchMessage)?;
        let category = Category::from(category);
        let sender = m
            .from_addr
            .as_deref()
            .map(|a| a.trim().to_lowercase())
            .filter(|a| a.contains('@') && a.len() <= 320);
        let changed = self.inner.handle.read(|db| {
            let conn = db.connection();
            if !messages::set_category(conn, m.account_id, m.id, category)? {
                return Err(CoreError::NoSuchMessage);
            }
            let mut folders = vec![m.folder_id];
            if let (true, Some(address)) = (always, sender.as_deref()) {
                messages::add_category_rule(conn, m.account_id, address, category)?;
                let mut stmt = conn.prepare(
                    "SELECT DISTINCT folder_id FROM message
                      WHERE account_id = ?1 AND lower(from_addr) = ?2",
                )?;
                let more = stmt
                    .query_map(params![m.account_id, address], |r| r.get::<_, i64>(0))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                folders.extend(more);
            }
            folders.sort_unstable();
            folders.dedup();
            Ok::<_, CoreError>(folders)
        })?;
        self.inner.hub.apply(Change::Messages {
            account: m.account_id,
            folders: changed,
        });
        Ok(())
    }
}
