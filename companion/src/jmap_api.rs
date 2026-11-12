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

/// The caller's core and its epoch.
async fn core(app: &App, user_id: i64) -> Result<(Arc<Core>, u64), ApiError> {
    let mail = app.mail.clone().ok_or(ApiError::NotFound)?;
    let store = app.store.clone();
    let auth = app.auth.clone();
    tokio::task::spawn_blocking(move || mail.core_and_epoch(&store, auth.master(), user_id))
        .await
        .map_err(|_| ApiError::Server)?
        .map_err(|_| ApiError::Server)
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

fn iso(ms: i64) -> String {
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
    for cap in using {
        match cap.as_str() {
            Some(CORE | MAIL) => {}
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
    app.auth.mail_limit(c.user_id)?;
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
    let ctx = Ctx {
        core,
        epoch,
        accounts,
    };
    let mut responses: Vec<Value> = Vec::new();
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
        let answer = match resolve_references(args, &responses) {
            Err(e) => vec![("error".to_owned(), e)],
            Ok(args) => ctx.call(name, &args).await,
        };
        for (n, v) in answer {
            responses.push(json!([n, v, call_id]));
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
            "state": self.state(),
            "list": list,
            "notFound": not_found,
        }))
    }

    async fn email_query(&self, args: &Value) -> Result<Value, Value> {
        let account = self.account(args)?;
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
        if args.get("collapseThreads").and_then(Value::as_bool) == Some(true) {
            return Err(error("invalidArguments"));
        }
        let position = args.get("position").and_then(Value::as_u64).unwrap_or(0) as usize;
        let limit = args
            .get("limit")
            .and_then(Value::as_u64)
            .map_or(MAX_QUERY, |l| (l as usize).min(MAX_QUERY));
        // The folder must be the account's (an id from another account names nothing here).
        if let MessageScope::Folder { folder_id, .. } = &scope {
            let folder_id = *folder_id;
            let folders = self
                .blocking(move |c| c.list_folders(account))
                .await?
                .map_err(|_| error("serverFail"))?;
            if !folders.iter().any(|f| f.id == folder_id) {
                return Err(error("invalidArguments"));
            }
        }
        let want = position + limit;
        let rows = self
            .blocking(move |c| {
                let mut out = Vec::new();
                let mut cursor: Option<RowCursor> = None;
                loop {
                    let page = c.list_messages(scope, cursor, 200)?;
                    out.extend(page.rows);
                    match page.next {
                        Some(n) if out.len() < want + 1 => cursor = Some(n),
                        _ => break,
                    }
                }
                Ok::<_, croft_ffi::CoreError>(out)
            })
            .await?
            .map_err(|_| error("serverFail"))?;
        let ids: Vec<String> = rows
            .iter()
            .filter(|r| r.account == account)
            .skip(position)
            .take(limit)
            .map(|r| format!("m{}", r.id))
            .collect();
        let mut out = json!({
            "accountId": format!("a{account}"),
            "queryState": self.state(),
            "canCalculateChanges": false,
            "position": position,
            "ids": ids,
        });
        if args.get("calculateTotal").and_then(Value::as_bool) == Some(true) {
            out["total"] = json!(rows.iter().filter(|r| r.account == account).count());
        }
        Ok(out)
    }

    async fn email_get(&self, args: &Value) -> Result<Value, Value> {
        let account = self.account(args)?;
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
                    let mut values = Map::new();
                    if text {
                        if let Some(t) = &body.text {
                            values.insert(
                                "text".into(),
                                json!({ "value": t, "isEncodingProblem": false, "isTruncated": body.truncated }),
                            );
                        }
                    }
                    if html {
                        if let Some(h) = &body.html_sanitised {
                            values.insert(
                                "html".into(),
                                json!({ "value": h, "isEncodingProblem": false, "isTruncated": body.truncated }),
                            );
                        }
                    }
                    email["textBody"] = if body.text.is_some() {
                        json!([{ "partId": "text", "type": "text/plain" }])
                    } else {
                        json!([])
                    };
                    email["htmlBody"] = if body.html_sanitised.is_some() {
                        json!([{ "partId": "html", "type": "text/html" }])
                    } else {
                        email["textBody"].clone()
                    };
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
            "state": self.state(),
            "list": list,
            "notFound": not_found,
        }))
    }

    /// The account and folder of one of the account's emails.
    async fn locate(&self, account: i64, id: i64) -> Option<(i64, String)> {
        let conv = self
            .blocking(move |c| c.open_conversation(id))
            .await
            .ok()?
            .ok()?;
        conv.messages
            .iter()
            .find(|m| m.id == id && m.account == account)
            .map(|m| (m.folder, m.folder_special_use.clone()))
    }

    async fn email_set(&self, args: &Value) -> Result<Value, Value> {
        let account = self.account(args)?;
        if let Some(if_in) = args.get("ifInState").and_then(Value::as_str) {
            if if_in != self.state() {
                return Err(error("stateMismatch"));
            }
        }
        let old = self.state();
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
                        Some(_) => self.apply_patch(id, patch, &folders).await,
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
                let Some((id, (_, special))) = located else {
                    not_destroyed.insert(key, json!({ "type": "notFound" }));
                    continue;
                };
                let in_trash = mailbox_role(&special, "") == Some("trash");
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

    /// Applies an `Email/set` patch: `keywords/$seen`, `keywords/$flagged` (or a whole
    /// `keywords`), and `mailboxIds` (one mailbox: a move).
    async fn apply_patch(
        &self,
        id: i64,
        patch: &Value,
        folders: &[croft_ffi::FolderInfo],
    ) -> Result<(), Value> {
        let Some(patch) = patch.as_object() else {
            return Err(json!({ "type": "invalidPatch" }));
        };
        let mut seen = None;
        let mut flagged = None;
        let mut move_to = None;
        for (path, value) in patch {
            match path.as_str() {
                "keywords/$seen" => seen = Some(value.as_bool() == Some(true)),
                "keywords/$flagged" => flagged = Some(value.as_bool() == Some(true)),
                "keywords" => {
                    let k = value
                        .as_object()
                        .ok_or_else(|| json!({ "type": "invalidPatch" }))?;
                    if k.keys().any(|k| {
                        k != "$seen" && k != "$flagged" && k != "$answered" && k != "$draft"
                    }) {
                        return Err(
                            json!({ "type": "invalidProperties", "properties": ["keywords"] }),
                        );
                    }
                    seen = Some(k.contains_key("$seen"));
                    flagged = Some(k.contains_key("$flagged"));
                }
                "mailboxIds" => {
                    let m = value
                        .as_object()
                        .ok_or_else(|| json!({ "type": "invalidPatch" }))?;
                    if m.len() != 1 {
                        return Err(json!({ "type": "tooManyMailboxes" }));
                    }
                    let target = m
                        .keys()
                        .next()
                        .and_then(|k| parse_id(k, 'f'))
                        .filter(|f| folders.iter().any(|x| x.id == *f))
                        .ok_or_else(
                            || json!({ "type": "invalidProperties", "properties": ["mailboxIds"] }),
                        )?;
                    move_to = Some(target);
                }
                _ => {
                    return Err(json!({ "type": "invalidProperties", "properties": [path] }));
                }
            }
        }
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
            "state": self.state(),
            "list": list,
            "notFound": not_found,
        }))
    }

    async fn identity_get(&self, args: &Value) -> Result<Value, Value> {
        let account = self.account(args)?;
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
            "state": self.state(),
            "list": list,
            "notFound": [],
        }))
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod unit {
    use super::*;

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
