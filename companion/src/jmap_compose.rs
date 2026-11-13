// This Source Code Form is subject to the terms of the GNU Affero General Public License,
// version 3 or later. See LICENSE-AGPL-3.0 at the repository root.

//! Composing and sending (T31b), a part of the JMAP API ([`super`]), under the capability
//! [`COMPOSE`]: a Croft extension, because the reply rules (who a reply goes to, `Re:` once, the
//! quote, the threading headers) and the Outbox with its undo window live in the core, and standard
//! JMAP leaves them to the client. Every method checks that the account is the caller's; the core
//! checks that a draft or an Outbox item belongs to that account.
//!
//! - `CroftDraft/get` `{accountId, ids}`: drafts (`d<id>`), in full, at most 32 at once.
//! - `CroftDraft/set` `{accountId, create?, update?, destroy?}`: save (create, or patch) and
//!   discard drafts; at most 1000 drafts per account.
//! - `CroftDraft/reply` `{accountId, emailId, kind: reply | replyAll | forward}`: the fields of a new
//!   reply or forward (not saved).
//! - `CroftDraft/check` `{accountId, id}`: why Send would be refused.
//! - `CroftOutbox/send` `{accountId, draftId}`: into the Outbox, sent when the undo window ends;
//!   counted against the user's sending budget (messages a minute, recipients an hour and a day).
//! - `CroftOutbox/undo` `{accountId, outboxId}`: back to its draft, while the window lasts.
//! - `CroftOutbox/get` `{accountId}`: the Outbox.

use croft_ffi::{
    ComposeError, DraftContent, DraftProblem, FfiAddress, FfiDraft, OutboxItem, OutboxItemState,
    OutboxReason,
};

use super::{error, iso, parse_id, Ctx, MAX_OBJECTS};
use serde_json::{json, Map, Value};

/// The capability of these methods.
pub const COMPOSE: &str = "urn:croft:params:jmap:compose";
/// Drafts one `CroftDraft/get` returns in full (T31b review: repeated or listed ids loaded and
/// answered many times over).
pub const MAX_DRAFTS_IN_GET: usize = 32;
/// Drafts an account may hold (T31b review: one request could make thousands).
pub const MAX_DRAFTS: usize = 1000;

fn address_json(a: &FfiAddress) -> Value {
    json!({ "name": a.name, "email": a.address })
}

fn content_json(c: &DraftContent) -> Value {
    json!({
        "identityId": c.identity.map(|i| format!("i{i}")),
        "to": c.to.iter().map(address_json).collect::<Vec<_>>(),
        "cc": c.cc.iter().map(address_json).collect::<Vec<_>>(),
        "bcc": c.bcc.iter().map(address_json).collect::<Vec<_>>(),
        "subject": c.subject,
        "body": c.body,
        "inReplyTo": c.in_reply_to,
        "references": c.references,
    })
}

fn draft_json(d: &FfiDraft) -> Value {
    let mut v = content_json(&d.content);
    if let Some(o) = v.as_object_mut() {
        o.insert("id".into(), json!(format!("d{}", d.id)));
        o.insert(
            "outboxId".into(),
            json!(d.outbox_id.map(|o| format!("o{o}"))),
        );
        o.insert("updatedAt".into(), json!(iso(d.updated_ms)));
    }
    v
}

/// The method error for a `sendAt` that is not a `UTCDate` or is more than a year and a day away.
fn bad_send_at() -> Value {
    json!({ "type": "invalidArguments", "description": "sendAt" })
}

/// A JMAP SetError for a property that is not right.
fn invalid(property: &str) -> Value {
    json!({ "type": "invalidProperties", "properties": [property] })
}

fn addresses(v: Option<&Value>, property: &str) -> Result<Vec<FfiAddress>, Value> {
    let Some(v) = v else {
        return Ok(Vec::new());
    };
    if v.is_null() {
        return Ok(Vec::new());
    }
    let list = v.as_array().ok_or_else(|| invalid(property))?;
    list.iter()
        .map(|a| {
            let email = a
                .get("email")
                .and_then(Value::as_str)
                .ok_or_else(|| invalid(property))?;
            let name = match a.get("name") {
                None | Some(Value::Null) => None,
                Some(Value::String(n)) => Some(n.clone()),
                Some(_) => return Err(invalid(property)),
            };
            Ok(FfiAddress {
                name,
                address: email.to_owned(),
            })
        })
        .collect()
}

fn text(v: Option<&Value>, property: &str) -> Result<String, Value> {
    match v {
        None | Some(Value::Null) => Ok(String::new()),
        Some(Value::String(s)) => Ok(s.clone()),
        Some(_) => Err(invalid(property)),
    }
}

/// The draft's fields from a client object: a whole draft, or with `base` a patch on it (only the
/// properties given change, as JMAP's `update`). Unknown properties are refused (a typo must not
/// be dropped silently); the core checks the sizes and counts.
fn content_from(v: &Value, base: Option<DraftContent>) -> Result<DraftContent, Value> {
    let o = v.as_object().ok_or_else(|| invalid("draft"))?;
    let base = base.unwrap_or(DraftContent {
        identity: None,
        to: Vec::new(),
        cc: Vec::new(),
        bcc: Vec::new(),
        subject: String::new(),
        body: String::new(),
        in_reply_to: None,
        references: Vec::new(),
    });
    const KNOWN: &[&str] = &[
        "identityId",
        "to",
        "cc",
        "bcc",
        "subject",
        "body",
        "inReplyTo",
        "references",
    ];
    if let Some(k) = o.keys().find(|k| !KNOWN.contains(&k.as_str())) {
        return Err(invalid(k));
    }
    let identity = match o.get("identityId") {
        None => base.identity,
        Some(Value::Null) => None,
        Some(Value::String(s)) => Some(parse_id(s, 'i').ok_or_else(|| invalid("identityId"))?),
        Some(_) => return Err(invalid("identityId")),
    };
    let in_reply_to = match o.get("inReplyTo") {
        None => base.in_reply_to,
        Some(Value::Null) => None,
        Some(Value::String(s)) => Some(s.clone()),
        Some(_) => return Err(invalid("inReplyTo")),
    };
    let references = match o.get("references") {
        None => base.references,
        Some(Value::Null) => Vec::new(),
        Some(Value::Array(a)) => a
            .iter()
            .map(|r| {
                r.as_str()
                    .map(str::to_owned)
                    .ok_or_else(|| invalid("references"))
            })
            .collect::<Result<_, _>>()?,
        Some(_) => return Err(invalid("references")),
    };
    let or = |key: &str| o.contains_key(key);
    Ok(DraftContent {
        identity,
        to: if or("to") {
            addresses(o.get("to"), "to")?
        } else {
            base.to
        },
        cc: if or("cc") {
            addresses(o.get("cc"), "cc")?
        } else {
            base.cc
        },
        bcc: if or("bcc") {
            addresses(o.get("bcc"), "bcc")?
        } else {
            base.bcc
        },
        subject: if or("subject") {
            text(o.get("subject"), "subject")?
        } else {
            base.subject
        },
        body: if or("body") {
            text(o.get("body"), "body")?
        } else {
            base.body
        },
        in_reply_to,
        references,
    })
}

/// A core error as a JMAP SetError (or method error) type; never the core's words.
fn compose_error(e: &ComposeError) -> Value {
    let kind = match e {
        ComposeError::NoSuchDraft
        | ComposeError::NoSuchOutboxItem
        | ComposeError::NoSuchMessage => "notFound",
        ComposeError::NoSuchAccount => "accountNotFound",
        ComposeError::NoSuchIdentity => return invalid("identityId"),
        ComposeError::InOutbox => "inOutbox",
        ComposeError::NoRecipient => "noRecipient",
        ComposeError::InvalidAddress => "invalidAddress",
        ComposeError::TooLate => "tooLate",
        ComposeError::WrongState => "wrongState",
        ComposeError::CannotSend => "cannotSend",
        ComposeError::DraftTooLarge => "tooLarge",
        ComposeError::BadSendTime => return bad_send_at(),
        ComposeError::Internal | ComposeError::Storage { .. } => "serverFail",
        _ => "invalidProperties",
    };
    json!({ "type": kind })
}

fn outbox_json(o: &OutboxItem) -> Value {
    json!({
        "id": format!("o{}", o.id),
        "state": match o.state {
            OutboxItemState::Queued => "queued",
            OutboxItemState::Sending => "sending",
            OutboxItemState::Failed => "failed",
        },
        "sendAt": iso(o.send_at_ms),
        "attempts": o.attempts,
        "reason": o.reason.map(reason_key),
        "mayHaveBeenSent": o.may_have_been_sent,
        "draftId": o.draft_id.map(|d| format!("d{d}")),
        "subject": o.subject,
        "recipientCount": o.recipient_count,
    })
}

fn reason_key(r: OutboxReason) -> String {
    croft_ffi::outbox_reason_key(r)
}

impl Ctx {
    /// `CroftDraft/get`: the drafts named (`ids` is required; at most [`MAX_DRAFTS_IN_GET`]
    /// different ones).
    pub(super) async fn draft_get(&self, args: &Value) -> Result<Value, Value> {
        let account = self.account(args)?;
        let Some(asked) = args.get("ids").and_then(Value::as_array) else {
            return Err(error("invalidArguments"));
        };
        let mut seen = std::collections::HashSet::new();
        let asked: Vec<Value> = asked
            .iter()
            .filter(|v| seen.insert(v.to_string()))
            .cloned()
            .collect();
        if asked.len() > MAX_DRAFTS_IN_GET {
            return Err(error("requestTooLarge"));
        }
        let ids: Vec<(Value, Option<i64>)> = asked
            .into_iter()
            .map(|v| {
                let id = v.as_str().and_then(|s| parse_id(s, 'd'));
                (v, id)
            })
            .collect();
        let wanted: Vec<Option<i64>> = ids.iter().map(|(_, id)| *id).collect();
        let found = self
            .blocking(move |c| {
                wanted
                    .into_iter()
                    .map(|id| id.and_then(|i| c.load_draft(account, i).ok()))
                    .collect::<Vec<_>>()
            })
            .await?;
        let mut list = Vec::new();
        let mut not_found = Vec::new();
        for ((asked, _), draft) in ids.into_iter().zip(found) {
            match draft {
                Some(d) => list.push(draft_json(&d)),
                None => not_found.push(asked),
            }
        }
        Ok(json!({ "accountId": format!("a{account}"), "list": list, "notFound": not_found }))
    }

    /// `CroftDraft/set`
    pub(super) async fn draft_set(&self, args: &Value) -> Result<Value, Value> {
        let account = self.account(args)?;
        let object = |k: &str| {
            args.get(k)
                .and_then(Value::as_object)
                .cloned()
                .unwrap_or_default()
        };
        let (create, update) = (object("create"), object("update"));
        let destroy: Vec<Value> = args
            .get("destroy")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        if create.len() + update.len() + destroy.len() > MAX_OBJECTS {
            return Err(error("requestTooLarge"));
        }
        let mut created = Map::new();
        let mut not_created = Map::new();
        // The account's drafts may not grow past MAX_DRAFTS.
        let mut room = if create.is_empty() {
            0
        } else {
            let held = self
                .blocking(move |c| c.list_drafts(account).map(|d| d.len()))
                .await?
                .map_err(|e| compose_error(&e))?;
            MAX_DRAFTS.saturating_sub(held)
        };
        let mut updated = Map::new();
        let mut not_updated = Map::new();
        let mut destroyed = Vec::new();
        let mut not_destroyed = Map::new();
        for (key, v) in create {
            if room == 0 {
                not_created.insert(key, json!({ "type": "overQuota" }));
                continue;
            }
            let content = match content_from(&v, None) {
                Ok(c) => c,
                Err(e) => {
                    not_created.insert(key, e);
                    continue;
                }
            };
            match self
                .blocking(move |c| c.save_draft(account, None, content))
                .await?
            {
                Ok(saved) => {
                    room -= 1;
                    created.insert(key, json!({ "id": format!("d{}", saved.id) }));
                }
                Err(e) => {
                    not_created.insert(key, compose_error(&e));
                }
            }
        }
        for (key, v) in update {
            let Some(id) = parse_id(&key, 'd') else {
                not_updated.insert(key, json!({ "type": "notFound" }));
                continue;
            };
            // A patch on the stored draft: only the properties given change.
            let Ok(stored) = self.blocking(move |c| c.load_draft(account, id)).await? else {
                not_updated.insert(key, json!({ "type": "notFound" }));
                continue;
            };
            let content = match content_from(&v, Some(stored.content)) {
                Ok(c) => c,
                Err(e) => {
                    not_updated.insert(key, e);
                    continue;
                }
            };
            match self
                .blocking(move |c| c.save_draft(account, Some(id), content))
                .await?
            {
                Ok(_) => {
                    updated.insert(key, Value::Null);
                }
                Err(e) => {
                    not_updated.insert(key, compose_error(&e));
                }
            }
        }
        for v in destroy {
            // An id that is not one is answered as given (a non-string by its JSON text).
            let key = v.as_str().map_or_else(|| v.to_string(), str::to_owned);
            let Some(id) = parse_id(&key, 'd') else {
                not_destroyed.insert(key, json!({ "type": "notFound" }));
                continue;
            };
            match self.blocking(move |c| c.discard_draft(account, id)).await? {
                Ok(()) => destroyed.push(json!(key)),
                Err(e) => {
                    not_destroyed.insert(key, compose_error(&e));
                }
            }
        }
        Ok(json!({
            "accountId": format!("a{account}"),
            "created": created, "notCreated": not_created,
            "updated": updated, "notUpdated": not_updated,
            "destroyed": destroyed, "notDestroyed": not_destroyed,
        }))
    }

    /// `CroftDraft/reply`
    pub(super) async fn draft_reply(&self, args: &Value) -> Result<Value, Value> {
        let account = self.account(args)?;
        let email = args
            .get("emailId")
            .and_then(Value::as_str)
            .and_then(|e| parse_id(e, 'm'))
            .ok_or_else(|| error("invalidArguments"))?;
        let kind = args
            .get("kind")
            .and_then(Value::as_str)
            .unwrap_or("reply")
            .to_owned();
        let reply = self
            .blocking(move |c| match kind.as_str() {
                "reply" => Ok(c.reply_draft(account, email)),
                "replyAll" => Ok(c.reply_all_draft(account, email)),
                "forward" => Ok(c.forward_draft(account, email)),
                _ => Err(()),
            })
            .await?
            .map_err(|()| error("invalidArguments"))?
            .map_err(|e| compose_error(&e))?;
        Ok(json!({
            "accountId": format!("a{account}"),
            "draft": content_json(&reply.content),
            "quoteIncluded": reply.quote_included,
        }))
    }

    /// `CroftDraft/check`
    pub(super) async fn draft_check(&self, args: &Value) -> Result<Value, Value> {
        let account = self.account(args)?;
        let id = args
            .get("id")
            .and_then(Value::as_str)
            .and_then(|d| parse_id(d, 'd'))
            .ok_or_else(|| error("invalidArguments"))?;
        let check = self
            .blocking(move |c| c.check_draft(account, id))
            .await?
            .map_err(|e| compose_error(&e))?;
        Ok(json!({
            "accountId": format!("a{account}"),
            "problems": check.problems.iter().map(|p| match p {
                DraftProblem::NoRecipient => "noRecipient",
                DraftProblem::InvalidRecipient => "invalidRecipient",
                DraftProblem::AttachmentsTooLarge => "attachmentsTooLarge",
            }).collect::<Vec<_>>(),
            "invalidRecipients": check.invalid_recipients,
            "subjectEmpty": check.subject_empty,
            "bodyEmpty": check.body_empty,
        }))
    }

    /// `CroftOutbox/send`: counts against the user's sending budget (a stolen session must not
    /// turn the account into a spam relay at full speed); a scheduled message counts when it is
    /// scheduled. With `sendAt` (a `UTCDate`) the message waits until then (S25, T33): the user's
    /// core runs in the Companion, so it goes out with no device online.
    pub(super) async fn outbox_send(&self, args: &Value) -> Result<Value, Value> {
        let account = self.account(args)?;
        let id = args
            .get("draftId")
            .and_then(Value::as_str)
            .and_then(|d| parse_id(d, 'd'))
            .ok_or_else(|| error("invalidArguments"))?;
        let send_at = match args.get("sendAt") {
            None | Some(Value::Null) => None,
            Some(v) => Some(
                v.as_str()
                    .and_then(super::utc_date)
                    .ok_or_else(bad_send_at)?,
            ),
        };
        // The budget counts recipients: the draft as it will be sent.
        let draft = self
            .blocking(move |c| c.load_draft(account, id))
            .await?
            .map_err(|e| compose_error(&e))?;
        let recipients = draft.content.to.len() + draft.content.cc.len() + draft.content.bcc.len();
        if !(self.send_allowed)(recipients) {
            return Err(error("rateLimit"));
        }
        let started = self
            .blocking(move |c| match send_at {
                Some(at) => c.schedule_send(account, id, at),
                None => c.send(account, id),
            })
            .await?
            .map_err(|e| compose_error(&e))?;
        Ok(json!({
            "accountId": format!("a{account}"),
            "outboxId": format!("o{}", started.outbox_id),
            "sendAt": iso(started.send_at_ms),
        }))
    }

    /// `CroftOutbox/undo`
    pub(super) async fn outbox_undo(&self, args: &Value) -> Result<Value, Value> {
        let account = self.account(args)?;
        let id = args
            .get("outboxId")
            .and_then(Value::as_str)
            .and_then(|o| parse_id(o, 'o'))
            .and_then(|o| u64::try_from(o).ok())
            .ok_or_else(|| error("invalidArguments"))?;
        let draft = self
            .blocking(move |c| c.undo_send(account, id))
            .await?
            .map_err(|e| compose_error(&e))?;
        Ok(json!({ "accountId": format!("a{account}"), "draftId": format!("d{draft}") }))
    }

    /// `CroftOutbox/get`
    pub(super) async fn outbox_get(&self, args: &Value) -> Result<Value, Value> {
        let account = self.account(args)?;
        let items = self
            .blocking(move |c| c.list_outbox(account))
            .await?
            .map_err(|e| compose_error(&e))?;
        Ok(json!({
            "accountId": format!("a{account}"),
            "list": items.iter().take(MAX_OBJECTS).map(outbox_json).collect::<Vec<_>>(),
        }))
    }
}
