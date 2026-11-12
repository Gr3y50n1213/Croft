// This Source Code Form is subject to the terms of the GNU Affero General Public License,
// version 3 or later. See LICENSE-AGPL-3.0 at the repository root.

//! Composing and sending (T31b), a part of the JMAP API ([`super`]), under the capability
//! [`COMPOSE`]: a Croft extension, because the reply rules (who a reply goes to, `Re:` once, the
//! quote, the threading headers) and the Outbox with its undo window live in the core, and standard
//! JMAP leaves them to the client. Every method checks that the account is the caller's; the core
//! checks that a draft or an Outbox item belongs to that account.
//!
//! - `CroftDraft/get` `{accountId, ids?}`: drafts (`d<id>`), in full.
//! - `CroftDraft/set` `{accountId, create?, update?, destroy?}`: save (create or replace) and
//!   discard drafts.
//! - `CroftDraft/reply` `{accountId, emailId, kind: reply | replyAll | forward}`: the fields of a new
//!   reply or forward (not saved).
//! - `CroftDraft/check` `{accountId, id}`: why Send would be refused.
//! - `CroftOutbox/send` `{accountId, draftId}`: into the Outbox, sent when the undo window ends.
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

/// The draft's fields from a client object. Unknown properties are refused (a typo must not be
/// dropped silently); the core checks the sizes and counts.
fn content_from(v: &Value) -> Result<DraftContent, Value> {
    let o = v.as_object().ok_or_else(|| invalid("draft"))?;
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
        None | Some(Value::Null) => None,
        Some(Value::String(s)) => Some(parse_id(s, 'i').ok_or_else(|| invalid("identityId"))?),
        Some(_) => return Err(invalid("identityId")),
    };
    let in_reply_to = match o.get("inReplyTo") {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) => Some(s.clone()),
        Some(_) => return Err(invalid("inReplyTo")),
    };
    let references = match o.get("references") {
        None | Some(Value::Null) => Vec::new(),
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
    Ok(DraftContent {
        identity,
        to: addresses(o.get("to"), "to")?,
        cc: addresses(o.get("cc"), "cc")?,
        bcc: addresses(o.get("bcc"), "bcc")?,
        subject: text(o.get("subject"), "subject")?,
        body: text(o.get("body"), "body")?,
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
    /// `CroftDraft/get`
    pub(super) async fn draft_get(&self, args: &Value) -> Result<Value, Value> {
        let account = self.account(args)?;
        let ids = Self::ids_arg(args, 'd')?;
        let found = self
            .blocking(move |c| {
                let wanted: Vec<Option<i64>> = match ids {
                    Some(ids) => ids,
                    None => c
                        .list_drafts(account)
                        .map_err(|_| ())?
                        .iter()
                        .take(MAX_OBJECTS)
                        .map(|d| Some(d.id))
                        .collect(),
                };
                Ok::<_, ()>(
                    wanted
                        .into_iter()
                        .map(|id| (id, id.and_then(|i| c.load_draft(account, i).ok())))
                        .collect::<Vec<_>>(),
                )
            })
            .await?
            .map_err(|()| error("serverFail"))?;
        let mut list = Vec::new();
        let mut not_found = Vec::new();
        for (id, draft) in found {
            match draft {
                Some(d) => list.push(draft_json(&d)),
                None => not_found.push(json!(id.map_or_else(String::new, |i| format!("d{i}")))),
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
        let mut updated = Map::new();
        let mut not_updated = Map::new();
        let mut destroyed = Vec::new();
        let mut not_destroyed = Map::new();
        for (key, v) in create {
            let content = match content_from(&v) {
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
            let content = match content_from(&v) {
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
            let key = v.as_str().unwrap_or_default().to_owned();
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

    /// `CroftOutbox/send`: counts against the user's mail limit (a stolen session must not turn
    /// the account into a spam relay at full speed).
    pub(super) async fn outbox_send(&self, args: &Value) -> Result<Value, Value> {
        let account = self.account(args)?;
        let id = args
            .get("draftId")
            .and_then(Value::as_str)
            .and_then(|d| parse_id(d, 'd'))
            .ok_or_else(|| error("invalidArguments"))?;
        if !(self.send_allowed)() {
            return Err(error("rateLimit"));
        }
        let started = self
            .blocking(move |c| c.send(account, id))
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
