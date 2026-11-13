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
use crate::core::{now_ms, Core, Inner};
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
    /// The sender's address, which "Also delete the older messages" goes by (S31 shows it: the From
    /// address is not authenticated).
    pub sender_address: Option<String>,
    /// The email's subject and body (email), shown in S31 before anything is sent.
    pub subject: Option<String>,
    /// See `subject`.
    pub body: Option<String>,
    /// The older messages from that sender in the account, outside Trash, Spam, Sent and Drafts
    /// (S31's "Also delete the N older messages"). 0 when the From is the account's own address.
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
    /// The message changed since S31 was shown (a sync replaced it): nothing was done; show S31
    /// again with the new facts.
    Changed,
    /// "Also delete" was asked, but the account has no Trash folder: nothing was done.
    NoTrashFolder,
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
    /// Unsubscribing was done but moving the older mail to Trash failed (nothing else is undone).
    pub delete_failed: bool,
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
            // Never the account's own mail, whatever a message claims to come from.
            let sender_address = match sender_address {
                Some(a) if store::is_own_address(conn, m.account_id, &a)? => None,
                other => other,
            };
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

impl Core {
    /// What S31 shows for these facts; also what [`Core::unsubscribe`] compares against.
    fn info_of(&self, f: &Facts) -> Result<UnsubscribeInfo, CoreError> {
        let already = match &f.sender_address {
            Some(a) => self
                .inner
                .handle
                .read(|db| store::is_unsubscribed(db.connection(), f.account, a))?,
            None => false,
        };
        let (way, host, address, subject, body) = match &f.way {
            Way::OneClick(u) => (
                UnsubscribeWay::OneClick,
                rules::host_of(u),
                None,
                None,
                None,
            ),
            Way::Website(u) => (UnsubscribeWay::Website, rules::host_of(u), None, None, None),
            Way::Email(m) => (
                UnsubscribeWay::Email,
                None,
                Some(m.to.clone()),
                Some(m.subject.clone()),
                Some(m.body.clone()),
            ),
        };
        Ok(UnsubscribeInfo {
            way,
            host,
            address,
            sender: f.sender.clone(),
            sender_address: f.sender_address.clone(),
            subject,
            body,
            older_count: u32::try_from(f.older.len()).unwrap_or(u32::MAX),
            already,
        })
    }
}

/// An outcome that did nothing.
fn nothing(failure: UnsubscribeFailure) -> UnsubscribeOutcome {
    UnsubscribeOutcome {
        done: false,
        open_url: None,
        failure: Some(failure),
        deleted_ops: Vec::new(),
        outbox_id: None,
        delete_failed: false,
    }
}

#[uniffi::export]
impl Core {
    /// What S21's banner and S31 show for message `message_id`, or `None` when it has no usable
    /// `List-Unsubscribe`. Contacts no one.
    pub fn unsubscribe_info(&self, message_id: i64) -> Result<Option<UnsubscribeInfo>, CoreError> {
        match self.unsubscribe_facts(message_id)? {
            Some(f) => Ok(Some(self.info_of(&f)?)),
            None => Ok(None),
        }
    }

    /// Unsubscribes from the sender of `message_id` exactly as `shown` (the [`UnsubscribeInfo`]
    /// S31 showed) says, after the person confirmed; when the message no longer gives the same
    /// facts, nothing is done ([`UnsubscribeFailure::Changed`]). With `delete_older`, once
    /// unsubscribing is done, the older messages from that sender go to Trash (with Undo); a website
    /// is only handed back to the app, and deletes nothing. One attempt: a failure is reported,
    /// never retried. Runs on the core's runtime, never on the caller's thread.
    pub async fn unsubscribe(
        self: std::sync::Arc<Self>,
        message_id: i64,
        shown: UnsubscribeInfo,
        delete_older: bool,
    ) -> Result<UnsubscribeOutcome, CoreError> {
        let core = self.clone();
        self.spawn(
            async move { core.unsubscribe_now(message_id, shown, delete_older).await },
            false,
        )
        .await?
    }
}

impl Core {
    async fn unsubscribe_now(
        &self,
        message_id: i64,
        shown: UnsubscribeInfo,
        delete_older: bool,
    ) -> Result<UnsubscribeOutcome, CoreError> {
        let Some(f) = self.unsubscribe_facts(message_id)? else {
            return Ok(nothing(UnsubscribeFailure::Changed));
        };
        let mut now_shown = self.info_of(&f)?;
        // Being unsubscribed already is not a change of what is sent.
        now_shown.already = shown.already;
        if now_shown != shown {
            return Ok(nothing(UnsubscribeFailure::Changed));
        }
        let delete = delete_older && !f.older.is_empty() && !matches!(f.way, Way::Website(_));
        // Checked before the sender is contacted, so a missing Trash never follows a sent request.
        if delete
            && self
                .inner
                .handle
                .read(|db| Inner::special_folder(db, f.account, "trash", CoreError::NoTrashFolder))
                .is_err()
        {
            return Ok(nothing(UnsubscribeFailure::NoTrashFolder));
        }
        let mut out = UnsubscribeOutcome {
            done: false,
            open_url: None,
            failure: None,
            deleted_ops: Vec::new(),
            outbox_id: None,
            delete_failed: false,
        };
        match &f.way {
            Way::OneClick(url) => match fetch::post_unsubscribe(url).await {
                Ok(status) if (200..300).contains(&status) => out.done = true,
                Ok(_) | Err(FetchError::Redirect | FetchError::Refused | FetchError::BadUrl) => {
                    out.failure = Some(UnsubscribeFailure::Refused)
                }
                Err(FetchError::Unreachable) => out.failure = Some(UnsubscribeFailure::Unreachable),
            },
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
                match self.save_draft(f.account, None, content) {
                    Ok(d) => match self.send(f.account, d.id) {
                        Ok(s) => {
                            out.done = true;
                            out.outbox_id = Some(s.outbox_id);
                        }
                        Err(_) => {
                            // Nothing left behind in Drafts.
                            let _ = self.discard_draft(f.account, d.id);
                            out.failure = Some(UnsubscribeFailure::CannotSend);
                        }
                    },
                    Err(_) => out.failure = Some(UnsubscribeFailure::CannotSend),
                }
            }
            Way::Website(url) => out.open_url = Some(url.clone()),
        }
        if !out.done {
            return Ok(out);
        }
        // After the sender was contacted nothing may turn into an error: the app must learn it is
        // done (and get the Outbox id for Undo), or it would offer the same request again.
        if let Some(a) = &f.sender_address {
            let _ = self
                .inner
                .handle
                .read(|db| store::remember(db.connection(), f.account, a, now_ms()));
        }
        if delete {
            match self.trash(f.older.clone()) {
                Ok(ops) => out.deleted_ops = ops,
                Err(_) => out.delete_failed = true,
            }
        }
        Ok(out)
    }
}
