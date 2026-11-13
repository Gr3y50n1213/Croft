// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! One-tap unsubscribe (T37, S31). The way comes from the message's `List-Unsubscribe`
//! (`croft_mail_core::unsubscribe`): an RFC 8058 one-click `POST` to public addresses only, else an
//! email through the Outbox, else the sender's page, which only the app opens, in the browser,
//! after the person confirmed. Nothing is sent before the app calls [`Core::unsubscribe`], which it
//! does only from S31's confirmation. Nothing about the addresses or answers is logged.

use croft_mail_core::unsubscribe::{self as rules, Way};
use croft_protocols::oauth::fetch::{self, FetchError};
use croft_storage::unsubscribe as store;

use crate::compose::{DraftContent, FfiAddress};
use crate::core::{now_ms, Core};
use crate::error::CoreError;

/// The most older messages S31 offers to delete at once.
const MAX_OLDER: u32 = 500;

/// How S31 would unsubscribe.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum UnsubscribeWay {
    /// One request to the sender (RFC 8058), from this device.
    OneClick,
    /// An email to the sender's unsubscribe address, through the Outbox.
    Email,
    /// The sender's page, opened in the browser.
    Website,
}

/// What S21's banner and S31 show for a message that can be unsubscribed from.
#[derive(Clone, PartialEq, Eq, uniffi::Record)]
pub struct UnsubscribeInfo {
    /// How.
    pub way: UnsubscribeWay,
    /// The host the request goes to, or the page is on (one-click and website), for the dialog.
    pub host: Option<String>,
    /// The address the email goes to (email).
    pub address: Option<String>,
    /// The sender's name, else address, for "Unsubscribe from ...".
    pub sender: String,
    /// The older messages from that sender in the account, outside Trash and Spam (S31's "Also
    /// delete the N older messages").
    pub older_count: u32,
    /// The person already unsubscribed from this sender in this account: the banner says so.
    pub already: bool,
}

impl std::fmt::Debug for UnsubscribeInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UnsubscribeInfo")
            .field("way", &self.way)
            .field("older_count", &self.older_count)
            .field("already", &self.already)
            .finish_non_exhaustive()
    }
}

/// Why unsubscribing did not happen. Fixed categories.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum UnsubscribeFailure {
    /// The sender's server could not be reached, or did not answer in time: try again later.
    Unreachable,
    /// The sender's server refused, answered with something else than success, or redirected.
    Refused,
    /// The email could not be put in the Outbox (no SMTP server, an address the account refuses).
    CannotSend,
}

/// What [`Core::unsubscribe`] did.
#[derive(Clone, PartialEq, Eq, uniffi::Record)]
pub struct UnsubscribeOutcome {
    /// Done: the request was accepted, or the email is in the Outbox.
    pub done: bool,
    /// The page to open in the browser (the website way); the app opens it, never the core.
    pub open_url: Option<String>,
    /// Why it failed, when it did.
    pub failure: Option<UnsubscribeFailure>,
    /// The operation ids of the older messages moved to Trash, for Undo.
    pub deleted_ops: Vec<u64>,
    /// The Outbox id of the email, for Undo (the email way).
    pub outbox_id: Option<u64>,
}

impl std::fmt::Debug for UnsubscribeOutcome {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UnsubscribeOutcome")
            .field("done", &self.done)
            .field("failure", &self.failure)
            .field("deleted", &self.deleted_ops.len())
            .finish_non_exhaustive()
    }
}

/// A message's unsubscribe facts, read in one go.
struct Facts {
    account: i64,
    way: Way,
    sender_address: Option<String>,
    sender: String,
    older: Vec<i64>,
}

impl Core {
    fn unsubscribe_facts(&self, message_id: i64) -> Result<Option<Facts>, CoreError> {
        let m = self
            .inner
            .resolve(&[message_id])?
            .pop()
            .ok_or(CoreError::NoSuchMessage)?;
        self.inner.handle.read(|db| {
            let conn = db.connection();
            let Some(h) = store::headers(conn, m.account_id, m.id)? else {
                return Ok(None);
            };
            let Some(way) = h
                .unsubscribe
                .as_deref()
                .and_then(|v| rules::choose(&rules::parse(v), h.one_click))
            else {
                return Ok(None);
            };
            let sender_address = h.from_addr.filter(|a| !a.trim().is_empty());
            let older = match &sender_address {
                Some(a) => store::older_from(conn, m.account_id, a, h.date_ms, m.id, MAX_OLDER)?,
                None => Vec::new(),
            };
            let sender = h
                .from_name
                .filter(|n| !n.trim().is_empty())
                .or_else(|| sender_address.clone())
                .unwrap_or_default();
            Ok::<_, CoreError>(Some(Facts {
                account: m.account_id,
                way,
                sender_address,
                sender,
                older,
            }))
        })
    }
}

#[uniffi::export]
impl Core {
    /// What S21's banner and S31 show for message `message_id`, or `None` when it has no usable
    /// `List-Unsubscribe`. Contacts no one.
    pub fn unsubscribe_info(&self, message_id: i64) -> Result<Option<UnsubscribeInfo>, CoreError> {
        let Some(f) = self.unsubscribe_facts(message_id)? else {
            return Ok(None);
        };
        let already = match &f.sender_address {
            Some(a) => self
                .inner
                .handle
                .read(|db| store::is_unsubscribed(db.connection(), f.account, a))?,
            None => false,
        };
        let (way, host, address) = match &f.way {
            Way::OneClick(u) => (UnsubscribeWay::OneClick, rules::host_of(u), None),
            Way::Website(u) => (UnsubscribeWay::Website, rules::host_of(u), None),
            Way::Email(m) => (UnsubscribeWay::Email, None, Some(m.to.clone())),
        };
        Ok(Some(UnsubscribeInfo {
            way,
            host: host.map(str::to_owned),
            address,
            sender: f.sender,
            older_count: u32::try_from(f.older.len()).unwrap_or(u32::MAX),
            already,
        }))
    }

    /// Unsubscribes from the sender of `message_id` the way [`Core::unsubscribe_info`] said, after
    /// the person confirmed in S31; with `delete_older`, the older messages from that sender go to
    /// Trash (with Undo). One attempt: a failure is reported, never retried in the background.
    pub async fn unsubscribe(
        &self,
        message_id: i64,
        delete_older: bool,
    ) -> Result<UnsubscribeOutcome, CoreError> {
        let f = self
            .unsubscribe_facts(message_id)?
            .ok_or(CoreError::NoSuchMessage)?;
        let mut out = UnsubscribeOutcome {
            done: false,
            open_url: None,
            failure: None,
            deleted_ops: Vec::new(),
            outbox_id: None,
        };
        match &f.way {
            Way::OneClick(url) => {
                let url = url.clone();
                let answer = self
                    .spawn(async move { fetch::post_unsubscribe(&url).await }, false)
                    .await?;
                match answer {
                    Ok(status) if (200..300).contains(&status) => out.done = true,
                    Ok(_)
                    | Err(FetchError::Redirect | FetchError::Refused | FetchError::BadUrl) => {
                        out.failure = Some(UnsubscribeFailure::Refused)
                    }
                    Err(FetchError::Unreachable) => {
                        out.failure = Some(UnsubscribeFailure::Unreachable)
                    }
                }
            }
            Way::Email(m) => {
                let content = DraftContent {
                    identity: None,
                    to: vec![FfiAddress {
                        name: None,
                        address: m.to.clone(),
                    }],
                    cc: Vec::new(),
                    bcc: Vec::new(),
                    subject: m.subject.clone(),
                    body: m.body.clone(),
                    in_reply_to: None,
                    references: Vec::new(),
                };
                let sent = self
                    .save_draft(f.account, None, content)
                    .and_then(|d| self.send(f.account, d.id));
                match sent {
                    Ok(s) => {
                        out.done = true;
                        out.outbox_id = Some(s.outbox_id);
                    }
                    Err(_) => out.failure = Some(UnsubscribeFailure::CannotSend),
                }
            }
            Way::Website(url) => out.open_url = Some(url.clone()),
        }
        if out.done {
            if let Some(a) = &f.sender_address {
                self.inner
                    .handle
                    .read(|db| store::remember(db.connection(), f.account, a, now_ms()))?;
            }
        }
        if delete_older && out.failure.is_none() && !f.older.is_empty() {
            out.deleted_ops = self.trash(f.older)?;
        }
        Ok(out)
    }
}
