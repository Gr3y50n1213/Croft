// This Source Code Form is subject to the terms of the GNU Affero General Public License,
// version 3 or later. See LICENSE-AGPL-3.0 at the repository root.

//! The JMAP API for clients (T27; RFC 8620 and RFC 8621), a part of [`super`]: a signed-in user's
//! apps read and organise that user's mail through it, served from the user's own core (T26).
//!
//! One JMAP account per mail account (`a<id>`); mailboxes are the core's folders (`f<id>`), emails
//! its messages (`m<id>`), threads its conversations (`t<lowest email id>`), identities its sending
//! identities (`i<id>`). Every call runs on the caller's own core, and every object is checked to
//! belong to the account the call names. Changes go through the core's operation queue, as on the
//! phone. States are `<epoch>-<change number>` of the core (see `MailCores::core_and_epoch`).

use std::collections::HashMap;
use std::sync::Arc;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use croft_ffi::{Core, MessageScope, RemoteImagesSetting, RowCursor};
use serde_json::{json, Map, Value};

use super::{ApiError, ApiResult, Full};
use crate::http::App;

const CORE: &str = "urn:ietf:params:jmap:core";
const MAIL: &str = "urn:ietf:params:jmap:mail";
/// Most method calls in one request.
pub const MAX_CALLS: usize = 16;
/// Most objects in one `/get` or `/set`.
pub const MAX_OBJECTS: usize = 256;
/// Most results one `Email/query` gives.
const MAX_QUERY: usize = 256;
/// Most emails one `Email/get` returns (each opens its conversation).
const MAX_EMAILS_IN_GET: usize = 64;
/// Most emails one `Email/get` returns with bodies (each body can be fetched from the server).
const MAX_BODIES_IN_GET: usize = 16;
/// The furthest `Email/query` pages into a mailbox.
const MAX_POSITION: usize = 50_000;
/// Most result references in one call.
const MAX_REFERENCES: usize = 8;
/// Most bytes the arguments and answers of one request may add up to (review finding: result
/// references copied answers into arguments and back, multiplying a small request into gigabytes).
pub const MAX_RESPONSE_BYTES: usize = 4 * 1024 * 1024;
/// Requests a minute per user, and at once per user (the session's `maxConcurrentRequests`).
pub const JMAP_PER_MINUTE: u32 = 600;
/// See [`JMAP_PER_MINUTE`].
pub const JMAP_CONCURRENT: usize = 4;

/// The caller's core and its epoch.
async fn core(app: &App, user_id: i64) -> Result<(Arc<Core>, u64), ApiError> {
    app.user_core(user_id).await
}

fn state(core: &Core, epoch: u64) -> String {
    format!("{epoch}-{}", core.change_seq())
}

fn parse_id(id: &str, kind: char) -> Option<i64> {
    let rest = id.strip_prefix(kind)?;
    if rest.is_empty() || rest.len() > 18 || !rest.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    rest.parse().ok()
}

/// A JMAP `UTCDate` (RFC 3339, `YYYY-MM-DDTHH:MM:SSZ`, optionally with fractional seconds, always
/// `Z`) as Unix milliseconds; `None` for anything else, a date that does not exist included. The
/// inverse of [`iso`] (days from the civil date, Howard Hinnant's algorithm).
fn utc_date(s: &str) -> Option<i64> {
    let b = s.as_bytes();
    if b.len() < 20 || b.len() > 30 || b[b.len() - 1] != b'Z' {
        return None;
    }
    let num = |r: std::ops::Range<usize>| -> Option<i64> {
        let part = s.get(r)?;
        if part.is_empty() || !part.bytes().all(|c| c.is_ascii_digit()) {
            return None;
        }
        part.parse().ok()
    };
    if b[4] != b'-' || b[7] != b'-' || b[10] != b'T' || b[13] != b':' || b[16] != b':' {
        return None;
    }
    let (y, m, d) = (num(0..4)?, num(5..7)?, num(8..10)?);
    let (hh, mm, ss) = (num(11..13)?, num(14..16)?, num(17..19)?);
    let millis = match b.len() {
        20 => 0,
        n if b[19] == b'.' && n > 21 => {
            let frac = s.get(20..n - 1)?;
            if !frac.bytes().all(|c| c.is_ascii_digit()) {
                return None;
            }
            let ms3: String = frac.chars().chain(std::iter::repeat('0')).take(3).collect();
            ms3.parse::<i64>().ok()?
        }
        _ => return None,
    };
    let leap = (y % 4 == 0 && y % 100 != 0) || y % 400 == 0;
    let days_in = [
        31,
        if leap { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    if !(1..=12).contains(&m) || d < 1 || hh > 23 || mm > 59 || ss > 59 {
        return None;
    }
    if d > days_in[usize::try_from(m - 1).ok()?] {
        return None;
    }
    let y2 = if m <= 2 { y - 1 } else { y };
    let era = y2.div_euclid(400);
    let yoe = y2 - era * 400;
    let mp = if m > 2 { m - 3 } else { m + 9 };
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    Some(((days * 86_400 + hh * 3600 + mm * 60 + ss) * 1000) + millis)
}

pub(crate) fn iso(ms: i64) -> String {
    // RFC 3339 in UTC from Unix milliseconds (civil date from days, Howard Hinnant's algorithm).
    let secs = ms.div_euclid(1000);
    let days = secs.div_euclid(86_400);
    let tod = secs.rem_euclid(86_400);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        tod / 3600,
        (tod / 60) % 60,
        tod % 60
    )
}

/// A method-level error.
fn error(kind: &str) -> Value {
    json!({ "type": kind })
}

/// A request-level error (RFC 8620 3.6.1): a problem details object.
fn request_error(kind: &str, detail: &str) -> Response {
    (
        StatusCode::BAD_REQUEST,
        Json(json!({
            "type": format!("urn:ietf:params:jmap:error:{kind}"),
            "status": 400,
            "detail": detail,
        })),
    )
        .into_response()
}

fn mailbox_role(special_use: &str, path: &str) -> Option<&'static str> {
    let s = special_use.trim_start_matches('\\').to_ascii_lowercase();
    match s.as_str() {
        "inbox" => Some("inbox"),
        "sent" => Some("sent"),
        "drafts" => Some("drafts"),
        "trash" => Some("trash"),
        "junk" | "spam" => Some("junk"),
        "archive" => Some("archive"),
        "all" => Some("all"),
        "flagged" => Some("flagged"),
        _ if path.eq_ignore_ascii_case("INBOX") => Some("inbox"),
        _ => None,
    }
}

// ---- Session ----

/// `GET /jmap/session`
pub async fn session(State(app): State<App>, Full(c): Full) -> ApiResult {
    let (core, epoch) = core(&app, c.user_id).await?;
    let user_id = c.user_id;
    let user = super::with_store(&app, move |s| s.user_by_id(user_id))
        .await??
        .map(|u| u.name)
        .unwrap_or_default();
    let listed = core.clone();
    let accounts = tokio::task::spawn_blocking(move || listed.list_accounts())
        .await
        .map_err(|_| ApiError::Server)?
        .map_err(|_| ApiError::Server)?;
    let mut map = Map::new();
    for a in &accounts {
        map.insert(
            format!("a{}", a.id),
            json!({
                "name": a.email,
                "isPersonal": true,
                "isReadOnly": false,
                                "accountCapabilities": {
                    CORE: {},
                    jmap_compose::COMPOSE: {},
                    MAIL: {
                        "maxMailboxesPerEmail": 1,
                        "maxMailboxDepth": null,
                        "maxSizeMailboxName": 255,
                        "maxSizeAttachmentsPerEmail": 0,
                        "emailQuerySortOptions": ["receivedAt"],
                        "mayCreateTopLevelMailbox": false,
                    },
                },
            }),
        );
    }
    let primary = accounts.first().map(|a| format!("a{}", a.id));
    let base = app.auth.public_url();
    Ok(Json(json!({
        "capabilities": {
            CORE: {
                "maxSizeUpload": 0,
                "maxConcurrentUpload": 1,
                "maxSizeRequest": crate::http::MAX_BODY,
                "maxConcurrentRequests": 4,
                "maxCallsInRequest": MAX_CALLS,
                "maxObjectsInGet": MAX_OBJECTS,
                "maxObjectsInSet": MAX_OBJECTS,
                "collationAlgorithms": [],
            },
                        MAIL: {},
            jmap_compose::COMPOSE: {},
        },
        "accounts": map,
        "primaryAccounts": primary.map_or_else(|| json!({}), |p| json!({ CORE: p, MAIL: p })),
        "username": user,
        "apiUrl": format!("{base}/jmap/api"),
        "downloadUrl": format!("{base}/jmap/download/{{accountId}}/{{blobId}}/{{name}}?type={{type}}"),
        "uploadUrl": format!("{base}/jmap/upload/{{accountId}}"),
        "eventSourceUrl": format!("{base}/jmap/eventsource?types={{types}}&closeafter={{closeafter}}&ping={{ping}}"),
        "state": state(&core, epoch),
    }))
    .into_response())
}

// ---- The request ----

/// `POST /jmap/api`
pub async fn api(State(app): State<App>, Full(c): Full, body: axum::body::Bytes) -> ApiResult {
    let Ok(req) = serde_json::from_slice::<Value>(&body) else {
        return Ok(request_error("notJSON", "the request is not JSON"));
    };
    let (Some(using), Some(calls)) = (
        req.get("using").and_then(Value::as_array),
        req.get("methodCalls").and_then(Value::as_array),
    ) else {
        return Ok(request_error(
            "notRequest",
            "using and methodCalls are required",
        ));
    };
    let mut compose = false;
    for cap in using {
        match cap.as_str() {
            Some(CORE | MAIL) => {}
            Some(jmap_compose::COMPOSE) => compose = true,
            _ => {
                return Ok(request_error(
                    "unknownCapability",
                    "a capability is not supported",
                ))
            }
        }
    }
    if calls.len() > MAX_CALLS {
        return Ok((
            StatusCode::BAD_REQUEST,
            Json(json!({
                "type": "urn:ietf:params:jmap:error:limit",
                "status": 400,
                "limit": "maxCallsInRequest",
            })),
        )
            .into_response());
    }
    app.auth.jmap_limit(c.user_id)?;
    let Some(_running) = app.auth.jmap_slot(c.user_id) else {
        return Ok((
            StatusCode::TOO_MANY_REQUESTS,
            Json(json!({
                "type": "urn:ietf:params:jmap:error:limit",
                "status": 429,
                "limit": "maxConcurrentRequests",
            })),
        )
            .into_response());
    };
    let (core, epoch) = core(&app, c.user_id).await?;
    let accounts: Vec<i64> = {
        let listed = core.clone();
        tokio::task::spawn_blocking(move || listed.list_accounts())
            .await
            .map_err(|_| ApiError::Server)?
            .map_err(|_| ApiError::Server)?
            .iter()
            .map(|a| a.id)
            .collect()
    };
    let limits = app.auth.clone();
    let user_id = c.user_id;
    let ctx = Ctx {
        core,
        epoch,
        accounts,
        compose,
        send_allowed: Box::new(move |recipients| limits.send_budget(user_id, recipients)),
    };
    let mut responses: Vec<Value> = Vec::new();
    let mut spent = body.len();
    for call in calls {
        let Some([name, args, call_id]) = call
            .as_array()
            .map(Vec::as_slice)
            .and_then(|a| <&[Value; 3]>::try_from(a).ok())
        else {
            return Ok(request_error(
                "notRequest",
                "a method call is not [name, arguments, id]",
            ));
        };
        let (Some(name), Some(call_id)) = (name.as_str(), call_id.as_str()) else {
            return Ok(request_error(
                "notRequest",
                "a method call is not [name, arguments, id]",
            ));
        };
        let too_large = || vec![("error".to_owned(), json!({ "type": "requestTooLarge" }))];
        let answer = if spent > MAX_RESPONSE_BYTES {
            too_large()
        } else {
            match resolve_references(args, &responses) {
                Err(e) => vec![("error".to_owned(), e)],
                Ok(args) => {
                    spent += serde_json::to_vec(&args).map_or(MAX_RESPONSE_BYTES, |v| v.len());
                    if spent > MAX_RESPONSE_BYTES {
                        too_large()
                    } else {
                        ctx.call(name, &args).await
                    }
                }
            }
        };
        for (n, v) in answer {
            let item = json!([n, v, call_id]);
            spent += serde_json::to_vec(&item).map_or(MAX_RESPONSE_BYTES, |v| v.len());
            if spent > MAX_RESPONSE_BYTES {
                responses.push(json!(["error", { "type": "requestTooLarge" }, call_id]));
            } else {
                responses.push(item);
            }
        }
    }
    Ok(Json(json!({
        "methodResponses": responses,
        "createdIds": req.get("createdIds").cloned().unwrap_or_else(|| json!({})),
        "sessionState": state(&ctx.core, ctx.epoch),
    }))
    .into_response())
}

/// Replaces `#name` arguments by the value their result reference points at (RFC 8620 3.7).
fn resolve_references(args: &Value, responses: &[Value]) -> Result<Value, Value> {
    let Some(obj) = args.as_object() else {
        return Err(error("invalidArguments"));
    };
    if obj.keys().filter(|k| k.starts_with('#')).count() > MAX_REFERENCES {
        return Err(json!({ "type": "requestTooLarge" }));
    }
    let mut out = Map::new();
    for (k, v) in obj {
        if let Some(plain) = k.strip_prefix('#') {
            if obj.contains_key(plain) {
                return Err(error("invalidArguments"));
            }
            let (Some(result_of), Some(name), Some(path)) = (
                v.get("resultOf").and_then(Value::as_str),
                v.get("name").and_then(Value::as_str),
                v.get("path").and_then(Value::as_str),
            ) else {
                return Err(error("invalidResultReference"));
            };
            if name == "error" {
                return Err(error("invalidResultReference"));
            }
            let found = responses.iter().find(|r| {
                r.get(2).and_then(Value::as_str) == Some(result_of)
                    && r.get(0).and_then(Value::as_str) == Some(name)
            });
            let Some(found) = found.and_then(|r| r.get(1)) else {
                return Err(error("invalidResultReference"));
            };
            let value = pointer(found, path).ok_or_else(|| error("invalidResultReference"))?;
            out.insert(plain.to_owned(), value);
        } else {
            out.insert(k.clone(), v.clone());
        }
    }
    Ok(Value::Object(out))
}

/// A JSON pointer with JMAP's `*` (every item of an array, results flattened).
fn pointer(v: &Value, path: &str) -> Option<Value> {
    if path.is_empty() {
        return Some(v.clone());
    }
    let rest = path.strip_prefix('/')?;
    let (head, tail) = match rest.split_once('/') {
        Some((h, t)) => (h, format!("/{t}")),
        None => (rest, String::new()),
    };
    let head = head.replace("~1", "/").replace("~0", "~");
    if head == "*" {
        let items = v.as_array()?;
        let mut out = Vec::new();
        for item in items {
            match pointer(item, &tail)? {
                Value::Array(a) => out.extend(a),
                other => out.push(other),
            }
        }
        return Some(Value::Array(out));
    }
    let next = match v {
        Value::Object(o) => o.get(&head)?,
        Value::Array(a) => a.get(head.parse::<usize>().ok()?)?,
        _ => return None,
    };
    pointer(next, &tail)
}

// ---- The methods ----

struct Ctx {
    core: Arc<Core>,
    epoch: u64,
    accounts: Vec<i64>,
    /// The request's `using` names the compose capability (its methods are unknown otherwise).
    compose: bool,
    /// Counts a send to that many recipients against the user's sending budget; false when over.
    send_allowed: Box<dyn Fn(usize) -> bool + Send + Sync>,
}

type Answer = Vec<(String, Value)>;

fn one(name: &str, v: Result<Value, Value>) -> Answer {
    match v {
        Ok(v) => vec![(name.to_owned(), v)],
        Err(e) => vec![("error".to_owned(), e)],
    }
}

impl Ctx {
    fn state(&self) -> String {
        state(&self.core, self.epoch)
    }

    /// The account the call names, if it is the caller's.
    fn account(&self, args: &Value) -> Result<i64, Value> {
        args.get("accountId")
            .and_then(Value::as_str)
            .and_then(|a| parse_id(a, 'a'))
            .filter(|a| self.accounts.contains(a))
            .ok_or_else(|| error("accountNotFound"))
    }

    async fn call(&self, name: &str, args: &Value) -> Answer {
        match name {
            "Core/echo" => vec![(name.to_owned(), args.clone())],
            "Mailbox/get" => one(name, self.mailbox_get(args).await),
            "Mailbox/changes" | "Email/changes" | "Thread/changes" | "Identity/changes" => {
                one(name, self.changes(args))
            }
            "Email/query" => one(name, self.email_query(args).await),
            "Email/get" => one(name, self.email_get(args).await),
            "Email/set" => one(name, self.email_set(args).await),
            "Thread/get" => one(name, self.thread_get(args).await),
            "Identity/get" => one(name, self.identity_get(args).await),
            "CroftDraft/get" if self.compose => one(name, self.draft_get(args).await),
            "CroftDraft/set" if self.compose => one(name, self.draft_set(args).await),
            "CroftDraft/reply" if self.compose => one(name, self.draft_reply(args).await),
            "CroftDraft/check" if self.compose => one(name, self.draft_check(args).await),
            "CroftOutbox/send" if self.compose => one(name, self.outbox_send(args).await),
            "CroftOutbox/undo" if self.compose => one(name, self.outbox_undo(args).await),
            "CroftOutbox/get" if self.compose => one(name, self.outbox_get(args).await),
            _ => vec![("error".to_owned(), error("unknownMethod"))],
        }
    }

    /// Runs a blocking core call off the async threads.
    async fn blocking<T: Send + 'static>(
        &self,
        f: impl FnOnce(&Core) -> T + Send + 'static,
    ) -> Result<T, Value> {
        let core = self.core.clone();
        tokio::task::spawn_blocking(move || f(&core))
            .await
            .map_err(|_| error("serverFail"))
    }

    fn ids_arg(args: &Value, kind: char) -> Result<Option<Vec<Option<i64>>>, Value> {
        match args.get("ids") {
            None | Some(Value::Null) => Ok(None),
            Some(Value::Array(ids)) => {
                if ids.len() > MAX_OBJECTS {
                    return Err(json!({ "type": "requestTooLarge" }));
                }
                Ok(Some(
                    ids.iter()
                        .map(|v| v.as_str().and_then(|s| parse_id(s, kind)))
                        .collect(),
                ))
            }
            Some(_) => Err(error("invalidArguments")),
        }
    }

    fn changes(&self, args: &Value) -> Result<Value, Value> {
        let account = self.account(args)?;
        let since = args
            .get("sinceState")
            .and_then(Value::as_str)
            .ok_or_else(|| error("invalidArguments"))?;
        let now = self.state();
        if since != now {
            return Err(error("cannotCalculateChanges"));
        }
        Ok(json!({
            "accountId": format!("a{account}"),
            "oldState": since,
            "newState": now,
            "hasMoreChanges": false,
            "created": [], "updated": [], "destroyed": [],
        }))
    }

    async fn mailbox_get(&self, args: &Value) -> Result<Value, Value> {
        let account = self.account(args)?;
        let state = self.state();
        let wanted = Self::ids_arg(args, 'f')?;
        let folders = self
            .blocking(move |c| c.list_folders(account))
            .await?
            .map_err(|_| error("serverFail"))?;
        let mut list = Vec::new();
        let mut not_found = Vec::new();
        let by_id: HashMap<i64, _> = folders.iter().map(|f| (f.id, f)).collect();
        let pick: Vec<Option<i64>> = match wanted {
            Some(ids) => ids,
            None => folders.iter().map(|f| Some(f.id)).collect(),
        };
        let raw_ids: Vec<Value> = args
            .get("ids")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        for (i, id) in pick.iter().enumerate() {
            match id.and_then(|id| by_id.get(&id)) {
                Some(f) => list.push(json!({
                    "id": format!("f{}", f.id),
                    "name": f.path,
                    "parentId": null,
                    "role": mailbox_role(&f.special_use, &f.path),
                    "sortOrder": 0,
                    "totalEmails": f.total_count,
                    "unreadEmails": f.unread_count,
                    "totalThreads": f.total_count,
                    "unreadThreads": f.unread_count,
                    "myRights": {
                        "mayReadItems": true, "mayAddItems": true, "mayRemoveItems": true,
                        "maySetSeen": true, "maySetKeywords": true, "mayCreateChild": false,
                        "mayRename": false, "mayDelete": false, "maySubmit": false,
                    },
                    "isSubscribed": true,
                })),
                None => not_found.push(raw_ids.get(i).cloned().unwrap_or(Value::Null)),
            }
        }
        Ok(json!({
            "accountId": format!("a{account}"),
            "state": state,
            "list": list,
            "notFound": not_found,
        }))
    }

    async fn email_query(&self, args: &Value) -> Result<Value, Value> {
        let account = self.account(args)?;
        // The state before the data: a state older than the data is safe, a newer one is not.
        let query_state = self.state();
        let filter = args.get("filter").cloned().unwrap_or(Value::Null);
        let scope = match &filter {
            Value::Null => return Err(json!({ "type": "unsupportedFilter" })),
            Value::Object(f) => {
                if let Some(mb) = f.get("inMailbox") {
                    let folder = mb
                        .as_str()
                        .and_then(|m| parse_id(m, 'f'))
                        .ok_or_else(|| error("invalidArguments"))?;
                    if f.len() != 1 {
                        return Err(json!({ "type": "unsupportedFilter" }));
                    }
                    MessageScope::Folder {
                        account,
                        folder_id: folder,
                    }
                } else if f.get("hasKeyword").and_then(Value::as_str) == Some("$flagged")
                    && f.len() == 1
                {
                    MessageScope::Starred {
                        account: Some(account),
                    }
                } else {
                    return Err(json!({ "type": "unsupportedFilter" }));
                }
            }
            _ => return Err(error("invalidArguments")),
        };
        if let Some(sort) = args.get("sort").and_then(Value::as_array) {
            let ok = sort.iter().all(|s| {
                s.get("property").and_then(Value::as_str) == Some("receivedAt")
                    && s.get("isAscending").and_then(Value::as_bool) != Some(true)
            });
            if !ok {
                return Err(json!({ "type": "unsupportedSort" }));
            }
        }
        if args.get("collapseThreads").and_then(Value::as_bool) == Some(true)
            || args.get("anchor").is_some_and(|a| !a.is_null())
        {
            return Err(error("invalidArguments"));
        }
        let position = match args.get("position") {
            None | Some(Value::Null) => 0,
            Some(p) => p
                .as_u64()
                .and_then(|p| usize::try_from(p).ok())
                .ok_or_else(|| error("invalidArguments"))?,
        };
        let limit = args
            .get("limit")
            .and_then(Value::as_u64)
            .map_or(MAX_QUERY, |l| {
                usize::try_from(l).unwrap_or(MAX_QUERY).min(MAX_QUERY)
            });
        // The folder must be the account's; its count is the total.
        let mut folder_total = None;
        if let MessageScope::Folder { folder_id, .. } = &scope {
            let folder_id = *folder_id;
            let folders = self
                .blocking(move |c| c.list_folders(account))
                .await?
                .map_err(|_| error("serverFail"))?;
            let folder = folders
                .iter()
                .find(|f| f.id == folder_id)
                .ok_or_else(|| error("invalidArguments"))?;
            folder_total = Some(usize::try_from(folder.total_count).unwrap_or(usize::MAX));
        }
        // Never further than MAX_POSITION into a mailbox (a client pages, it does not dump).
        let wants_total = args.get("calculateTotal").and_then(Value::as_bool) == Some(true);
        let want = if wants_total && folder_total.is_none() {
            // Flagged across the account has no stored count: read to the end (bounded).
            MAX_POSITION
        } else {
            position.saturating_add(limit).min(MAX_POSITION)
        };
        let past_end = position >= MAX_POSITION || folder_total.is_some_and(|t| position >= t);
        let rows = if past_end {
            Vec::new()
        } else {
            self.blocking(move |c| {
                let mut out = Vec::new();
                let mut cursor: Option<RowCursor> = None;
                loop {
                    let page = c.list_messages(scope, cursor, 200)?;
                    out.extend(page.rows);
                    match page.next {
                        Some(n) if out.len() < want => cursor = Some(n),
                        _ => break,
                    }
                }
                Ok::<_, croft_ffi::CoreError>(out)
            })
            .await?
            .map_err(|_| error("serverFail"))?
        };
        let mine: Vec<_> = rows.iter().filter(|r| r.account == account).collect();
        let ids: Vec<String> = mine
            .iter()
            .skip(position)
            .take(limit)
            .map(|r| format!("m{}", r.id))
            .collect();
        let mut out = json!({
            "accountId": format!("a{account}"),
            "queryState": query_state,
            "canCalculateChanges": false,
            "position": position,
            "ids": ids,
            "limit": limit,
        });
        if wants_total {
            out["total"] = json!(folder_total.unwrap_or(mine.len()));
        }
        Ok(out)
    }

    async fn email_get(&self, args: &Value) -> Result<Value, Value> {
        let account = self.account(args)?;
        let state = self.state();
        let Some(ids) = Self::ids_arg(args, 'm')? else {
            return Err(json!({ "type": "requestTooLarge" }));
        };
        let raw_ids: Vec<Value> = args
            .get("ids")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let text = args.get("fetchTextBodyValues").and_then(Value::as_bool) == Some(true);
        let html = args.get("fetchHTMLBodyValues").and_then(Value::as_bool) == Some(true);
        // Each email opens its conversation, and each body may be fetched from the server.
        if ids.len() > MAX_EMAILS_IN_GET || ((text || html) && ids.len() > MAX_BODIES_IN_GET) {
            return Err(json!({ "type": "requestTooLarge" }));
        }
        let mut list = Vec::new();
        let mut not_found = Vec::new();
        for (i, id) in ids.iter().enumerate() {
            let raw = raw_ids.get(i).cloned().unwrap_or(Value::Null);
            let Some(id) = *id else {
                not_found.push(raw);
                continue;
            };
            let conv = self.blocking(move |c| c.open_conversation(id)).await?;
            let Ok(conv) = conv else {
                not_found.push(raw);
                continue;
            };
            let Some(m) = conv.messages.iter().find(|m| m.id == id) else {
                not_found.push(raw);
                continue;
            };
            if m.account != account {
                not_found.push(raw);
                continue;
            }
            let thread = conv.messages.iter().map(|m| m.id).min().unwrap_or(id);
            let addr = |list: &[croft_ffi::MailAddress]| -> Value {
                list.iter()
                    .map(|a| json!({ "name": a.name, "email": a.address }))
                    .collect::<Vec<_>>()
                    .into()
            };
            let mut keywords = Map::new();
            for (k, on) in [
                ("$seen", m.seen),
                ("$flagged", m.flagged),
                ("$answered", m.answered),
                ("$draft", m.draft),
            ] {
                if on {
                    keywords.insert(k.to_owned(), json!(true));
                }
            }
            let mut email = json!({
                "id": format!("m{}", m.id),
                "blobId": null,
                "threadId": format!("t{thread}"),
                "mailboxIds": { format!("f{}", m.folder): true },
                "keywords": keywords,
                "from": m.sender_address.as_ref().map(|a| json!([{ "name": m.sender_name, "email": a }])),
                "to": addr(&m.to),
                "cc": addr(&m.cc),
                "subject": m.subject,
                "receivedAt": iso(m.date_ms),
                "preview": m.snippet,
                "hasAttachment": m.has_attachments,
                "size": 0,
            });
            if text || html {
                let body = self
                    .core
                    .load_body(id, RemoteImagesSetting::Ask)
                    .await
                    .map_err(|_| error("serverFail"));
                if let Ok(body) = body {
                    let value = |v: &str| json!({ "value": v, "isEncodingProblem": false, "isTruncated": body.truncated });
                    let text_part = body
                        .text
                        .as_ref()
                        .map(|_| json!({ "partId": "text", "type": "text/plain" }));
                    let html_part = body
                        .html_sanitised
                        .as_ref()
                        .map(|_| json!({ "partId": "html", "type": "text/html" }));
                    // RFC 8621: each of textBody and htmlBody falls back to the other kind.
                    let text_body = text_part.clone().or_else(|| html_part.clone());
                    let html_body = html_part.or(text_part);
                    let mut values = Map::new();
                    for (wanted, part) in [(text, &text_body), (html, &html_body)] {
                        if !wanted {
                            continue;
                        }
                        match part.as_ref().and_then(|p| p["partId"].as_str()) {
                            Some("text") => {
                                if let Some(t) = &body.text {
                                    values.insert("text".into(), value(t));
                                }
                            }
                            Some("html") => {
                                if let Some(h) = &body.html_sanitised {
                                    values.insert("html".into(), value(h));
                                }
                            }
                            _ => {}
                        }
                    }
                    email["textBody"] = text_body.map_or_else(|| json!([]), |p| json!([p]));
                    email["htmlBody"] = html_body.map_or_else(|| json!([]), |p| json!([p]));
                    email["bodyValues"] = Value::Object(values);
                }
            }
            if let Some(props) = args.get("properties").and_then(Value::as_array) {
                let keep: Vec<&str> = props.iter().filter_map(Value::as_str).collect();
                if let Some(o) = email.as_object_mut() {
                    o.retain(|k, _| k == "id" || keep.contains(&k.as_str()) || k == "bodyValues");
                }
            }
            list.push(email);
        }
        Ok(json!({
            "accountId": format!("a{account}"),
            "state": state,
            "list": list,
            "notFound": not_found,
        }))
    }

    /// The folder (and its special use) of one of the account's emails, and whether a change of
    /// it is still waiting in the core's queue.
    async fn locate(&self, account: i64, id: i64) -> Option<(i64, String, bool)> {
        let conv = self
            .blocking(move |c| c.open_conversation(id))
            .await
            .ok()?
            .ok()?;
        conv.messages
            .iter()
            .find(|m| m.id == id && m.account == account)
            .map(|m| (m.folder, m.folder_special_use.clone(), m.pending))
    }

    async fn email_set(&self, args: &Value) -> Result<Value, Value> {
        let account = self.account(args)?;
        let old = self.state();
        let seq_before = self.core.change_seq();
        if let Some(if_in) = args.get("ifInState").and_then(Value::as_str) {
            if if_in != old {
                return Err(error("stateMismatch"));
            }
        }
        let mut not_created = Map::new();
        if let Some(create) = args.get("create").and_then(Value::as_object) {
            for k in create.keys() {
                not_created.insert(
                    k.clone(),
                    json!({ "type": "forbidden", "description": "creating emails is not supported yet" }),
                );
            }
        }
        let mut updated = Map::new();
        let mut not_updated = Map::new();
        let folders = self
            .blocking(move |c| c.list_folders(account))
            .await?
            .map_err(|_| error("serverFail"))?;
        if let Some(update) = args.get("update").and_then(Value::as_object) {
            if update.len() > MAX_OBJECTS {
                return Err(json!({ "type": "requestTooLarge" }));
            }
            for (key, patch) in update {
                let result = match parse_id(key, 'm') {
                    None => Err(json!({ "type": "notFound" })),
                    Some(id) => match self.locate(account, id).await {
                        None => Err(json!({ "type": "notFound" })),
                        Some((folder, _, _)) => self.apply_patch(id, folder, patch, &folders).await,
                    },
                };
                match result {
                    Ok(()) => {
                        updated.insert(key.clone(), Value::Null);
                    }
                    Err(e) => {
                        not_updated.insert(key.clone(), e);
                    }
                }
            }
        }
        let mut destroyed = Vec::new();
        let mut not_destroyed = Map::new();
        if let Some(destroy) = args.get("destroy").and_then(Value::as_array) {
            if destroy.len() > MAX_OBJECTS {
                return Err(json!({ "type": "requestTooLarge" }));
            }
            for raw in destroy {
                let key = raw.as_str().unwrap_or_default().to_owned();
                let located = match parse_id(&key, 'm') {
                    Some(id) => self.locate(account, id).await.map(|l| (id, l)),
                    None => None,
                };
                let Some((id, (_, special, pending))) = located else {
                    not_destroyed.insert(key, json!({ "type": "notFound" }));
                    continue;
                };
                let in_trash = mailbox_role(&special, "") == Some("trash");
                // Deleting for good is decided from where the email is; while a change of it
                // (a move out of Trash) is still queued, that is not known yet (review finding:
                // a restore followed by a destroy deleted the email for good).
                if in_trash && pending {
                    not_destroyed.insert(
                        key,
                        json!({ "type": "forbidden", "description": "a change of this email is still pending" }),
                    );
                    continue;
                }
                let done = self
                    .blocking(move |c| {
                        if in_trash {
                            c.delete_forever(vec![id]).map(|_| ())
                        } else {
                            c.trash(vec![id]).map(|_| ())
                        }
                    })
                    .await?;
                match done {
                    Ok(()) => destroyed.push(json!(key)),
                    Err(_) => {
                        not_destroyed.insert(key, json!({ "type": "forbidden" }));
                    }
                }
            }
        }
        // The core records a change a moment after it is made: wait (briefly) for it, so the new
        // state covers this call's changes (review finding: `newState` equalled `oldState`).
        if (!updated.is_empty() || !destroyed.is_empty()) && self.core.change_seq() == seq_before {
            let _ = tokio::time::timeout(
                std::time::Duration::from_secs(1),
                self.core.next_change(seq_before),
            )
            .await;
        }
        Ok(json!({
            "accountId": format!("a{account}"),
            "oldState": old,
            "newState": self.state(),
            "created": {},
            "updated": updated,
            "destroyed": destroyed,
            "notCreated": not_created,
            "notUpdated": not_updated,
            "notDestroyed": not_destroyed,
        }))
    }

    /// Applies an `Email/set` patch: `keywords/$seen`, `keywords/$flagged` (or a whole `keywords`
    /// object), and `mailboxIds` (a whole object, or `mailboxIds/<id>` entries) leaving the email in
    /// exactly one mailbox: a move. The patch is checked whole before anything is applied.
    async fn apply_patch(
        &self,
        id: i64,
        current_folder: i64,
        patch: &Value,
        folders: &[croft_ffi::FolderInfo],
    ) -> Result<(), Value> {
        let invalid_patch = || json!({ "type": "invalidPatch" });
        let Some(patch) = patch.as_object() else {
            return Err(invalid_patch());
        };
        let known = |f: &str| parse_id(f, 'f').filter(|f| folders.iter().any(|x| x.id == *f));
        let keyword_value = |v: &Value| match v {
            Value::Bool(true) => Ok(true),
            Value::Null => Ok(false),
            _ => Err(invalid_patch()),
        };
        let mut seen = None;
        let mut flagged = None;
        let mut mailboxes: Option<Vec<i64>> = None;
        let mut mailbox_changes: Vec<(i64, bool)> = Vec::new();
        for (path, value) in patch {
            match path.as_str() {
                "keywords/$seen" => seen = Some(keyword_value(value)?),
                "keywords/$flagged" => flagged = Some(keyword_value(value)?),
                "keywords" => {
                    let k = value.as_object().ok_or_else(invalid_patch)?;
                    for (name, v) in k {
                        if v != &Value::Bool(true) {
                            return Err(invalid_patch());
                        }
                        if !matches!(name.as_str(), "$seen" | "$flagged") {
                            return Err(
                                json!({ "type": "invalidProperties", "properties": ["keywords"] }),
                            );
                        }
                    }
                    seen = Some(k.contains_key("$seen"));
                    flagged = Some(k.contains_key("$flagged"));
                }
                "mailboxIds" => {
                    let m = value.as_object().ok_or_else(invalid_patch)?;
                    let mut ids = Vec::new();
                    for (mb, v) in m {
                        if v != &Value::Bool(true) {
                            return Err(invalid_patch());
                        }
                        ids.push(known(mb).ok_or_else(
                            || json!({ "type": "invalidProperties", "properties": ["mailboxIds"] }),
                        )?);
                    }
                    mailboxes = Some(ids);
                }
                other if other.starts_with("mailboxIds/") => {
                    let mb = known(&other["mailboxIds/".len()..]).ok_or_else(
                        || json!({ "type": "invalidProperties", "properties": [other] }),
                    )?;
                    mailbox_changes.push((mb, keyword_value(value)?));
                }
                _ => {
                    return Err(json!({ "type": "invalidProperties", "properties": [path] }));
                }
            }
        }
        if mailboxes.is_some() && !mailbox_changes.is_empty() {
            return Err(invalid_patch());
        }
        if !mailbox_changes.is_empty() {
            let mut set = vec![current_folder];
            for (mb, on) in &mailbox_changes {
                set.retain(|x| x != mb);
                if *on {
                    set.push(*mb);
                }
            }
            mailboxes = Some(set);
        }
        let move_to = match mailboxes {
            None => None,
            Some(m) if m.len() == 1 => Some(m[0]).filter(|f| *f != current_folder),
            Some(_) => return Err(json!({ "type": "tooManyMailboxes" })),
        };
        let done = self
            .blocking(move |c| {
                if let Some(s) = seen {
                    c.mark_seen(vec![id], s)?;
                }
                if let Some(f) = flagged {
                    c.set_flagged(vec![id], f)?;
                }
                if let Some(folder) = move_to {
                    c.move_to(vec![id], folder)?;
                }
                Ok::<_, croft_ffi::CoreError>(())
            })
            .await?;
        done.map_err(|_| json!({ "type": "forbidden" }))
    }

    async fn thread_get(&self, args: &Value) -> Result<Value, Value> {
        let account = self.account(args)?;
        let state = self.state();
        let Some(ids) = Self::ids_arg(args, 't')? else {
            return Err(json!({ "type": "requestTooLarge" }));
        };
        let raw_ids: Vec<Value> = args
            .get("ids")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let mut list = Vec::new();
        let mut not_found = Vec::new();
        for (i, id) in ids.iter().enumerate() {
            let raw = raw_ids.get(i).cloned().unwrap_or(Value::Null);
            let conv = match *id {
                Some(id) => self.blocking(move |c| c.open_conversation(id)).await?.ok(),
                None => None,
            };
            match conv.filter(|c| c.account == account) {
                Some(c) => {
                    let mut msgs: Vec<_> =
                        c.messages.iter().filter(|m| m.account == account).collect();
                    msgs.sort_by_key(|m| (m.date_ms, m.id));
                    let thread = msgs.iter().map(|m| m.id).min().unwrap_or(0);
                    list.push(json!({
                        "id": format!("t{thread}"),
                        "emailIds": msgs.iter().map(|m| format!("m{}", m.id)).collect::<Vec<_>>(),
                    }));
                }
                None => not_found.push(raw),
            }
        }
        Ok(json!({
            "accountId": format!("a{account}"),
            "state": state,
            "list": list,
            "notFound": not_found,
        }))
    }

    async fn identity_get(&self, args: &Value) -> Result<Value, Value> {
        let account = self.account(args)?;
        let state = self.state();
        let identities = self
            .blocking(move |c| c.identities(account))
            .await?
            .map_err(|_| error("serverFail"))?;
        let list: Vec<Value> = identities
            .iter()
            .map(|i| {
                json!({
                    "id": format!("i{}", i.id),
                    "name": i.display_name,
                    "email": i.address,
                    "replyTo": null, "bcc": null,
                    "textSignature": "", "htmlSignature": "",
                    "mayDelete": false,
                })
            })
            .collect();
        Ok(json!({
            "accountId": format!("a{account}"),
            "state": state,
            "list": list,
            "notFound": [],
        }))
    }
}

#[path = "jmap_compose.rs"]
mod jmap_compose;

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod unit {
    use super::*;

    #[test]
    fn utc_dates_are_read_strictly_and_round_trip() {
        for ms in [0, 1_700_000_000_000, 951_782_400_000, 4_102_444_799_000] {
            assert_eq!(utc_date(&iso(ms)), Some(ms), "{}", iso(ms));
        }
        assert_eq!(utc_date("2026-11-14T08:00:00Z"), Some(1_794_643_200_000));
        assert_eq!(utc_date("2026-11-14T08:00:00.5Z"), Some(1_794_643_200_500));
        assert!(utc_date("2024-02-29T00:00:00Z").is_some());
        for bad in [
            "",
            "2026-11-14",
            "2026-11-14T08:00:00",
            "2026-11-14T08:00:00+01:00",
            "2026-11-14 08:00:00Z",
            "2026-13-01T00:00:00Z",
            "2026-02-29T00:00:00Z",
            "2026-11-31T00:00:00Z",
            "2026-11-14T24:00:00Z",
            "2026-11-14T08:60:00Z",
            "2026-11-14T08:00:00.Z",
            "2026-11-14T08:00:00.12a4Z",
            "+026-11-14T08:00:00Z",
            "2026-1a-14T08:00:00Z",
            "2026-11-14T08:00:00.1234567890Z",
        ] {
            assert_eq!(utc_date(bad), None, "{bad}");
        }
    }

    #[test]
    fn iso_dates_and_ids() {
        assert_eq!(iso(0), "1970-01-01T00:00:00Z");
        assert_eq!(iso(1_700_000_000_000), "2023-11-14T22:13:20Z");
        assert_eq!(iso(951_782_400_000), "2000-02-29T00:00:00Z");
        assert_eq!(parse_id("m42", 'm'), Some(42));
        for bad in ["m", "42", "f42", "m-1", "m1x", "m99999999999999999999"] {
            assert_eq!(parse_id(bad, 'm'), None, "{bad}");
        }
    }

    #[test]
    fn result_references_follow_json_pointers_with_stars() {
        let responses = vec![
            json!(["Email/query", {"ids": ["m1", "m2"]}, "0"]),
            json!(["Thread/get", {"list": [{"emailIds": ["m1"]}, {"emailIds": ["m2", "m3"]}]}, "1"]),
        ];
        let args = json!({"accountId": "a1", "#ids": {"resultOf": "0", "name": "Email/query", "path": "/ids"}});
        assert_eq!(
            resolve_references(&args, &responses).unwrap()["ids"],
            json!(["m1", "m2"])
        );
        let args =
            json!({"#ids": {"resultOf": "1", "name": "Thread/get", "path": "/list/*/emailIds"}});
        assert_eq!(
            resolve_references(&args, &responses).unwrap()["ids"],
            json!(["m1", "m2", "m3"])
        );
        for bad in [
            json!({"#ids": {"resultOf": "9", "name": "Email/query", "path": "/ids"}}),
            json!({"#ids": {"resultOf": "0", "name": "Email/get", "path": "/ids"}}),
            json!({"#ids": {"resultOf": "0", "name": "Email/query", "path": "/nothing"}}),
            json!({"ids": [], "#ids": {"resultOf": "0", "name": "Email/query", "path": "/ids"}}),
        ] {
            assert!(resolve_references(&bad, &responses).is_err(), "{bad}");
        }
    }
}
