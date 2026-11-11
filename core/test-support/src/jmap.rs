// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! A small in-process JMAP server for fast tests of the JMAP adapter. **Test helper, not product
//! code**: it listens on `127.0.0.1` only and keeps everything in memory.
//!
//! What it speaks: HTTP/1.1 (one request per connection), optionally over TLS with a fresh
//! self-signed certificate. `GET /.well-known/jmap` (a redirect), `GET /jmap/session`,
//! `POST /jmap/` with `Mailbox/get`, `Email/query` (filters `inMailbox` and `header`), `Email/get`,
//! `Email/changes` with real state strings, `Email/set` (keyword and mailbox patches, destroy),
//! `Email/import`, `Identity/get` and `EmailSubmission/set` (with `onSuccessUpdateEmail`, which
//! produces the implied extra `Email/set` answer), and `POST /jmap/upload/{accountId}/`. One user,
//! checked with HTTP Basic (name and password) or Bearer (a token).
//!
//! What tests can do to it: add, change, move and destroy mail as "another client" would; make
//! `Email/changes` answer `cannotCalculateChanges`; make the `header` filter find nothing (as
//! Stalwart does for `Message-ID`); answer the API with a chosen HTTP status; make the session
//! advertise another host; redirect the session; hang; drop the answer to a method after doing its
//! work; and read the log of requests (with whether an `Authorization` header was present) and of
//! method calls.
//!
//! Only `Date` headers in `+0000` are turned into `sentAt`; this is a test double, not a parser.

use crate::tls_acceptor;
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::net::SocketAddr;
use std::sync::{Arc, Mutex, MutexGuard};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpListener;

/// How the server is started.
#[derive(Debug, Clone, Default)]
pub struct JmapOptions {
    /// Serve TLS with a self-signed certificate (`CN=localhost`, also valid for `127.0.0.1`).
    pub tls: bool,
}

/// One request the server saw.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestRecord {
    /// `GET` or `POST`.
    pub method: String,
    /// The path and query.
    pub path: String,
    /// Whether an `Authorization` header came with it.
    pub had_authorization: bool,
}

#[derive(Debug, Clone)]
struct Mailbox {
    id: String,
    name: String,
    role: Option<String>,
    parent: Option<String>,
}

#[derive(Debug, Clone)]
struct Email {
    mailboxes: BTreeSet<String>,
    keywords: BTreeSet<String>,
    raw: String,
    received: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Created,
    Updated,
    Destroyed,
}

/// A message accepted for delivery by `EmailSubmission/set`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Delivery {
    /// The envelope sender.
    pub from: String,
    /// The envelope recipients.
    pub recipients: Vec<String>,
    /// The message as uploaded.
    pub raw: String,
}

struct State {
    user: String,
    password: String,
    bearer: Option<String>,
    account: String,
    mailboxes: Vec<Mailbox>,
    emails: BTreeMap<String, Email>,
    blobs: HashMap<String, String>,
    next_id: u64,
    state: u64,
    /// (state after the change, email id, kind)
    log: Vec<(u64, String, Kind)>,
    oldest_state: u64,
    // knobs
    cannot_calculate: bool,
    header_search: bool,
    api_status: Option<u16>,
    session_api_url: Option<String>,
    session_redirect: Option<String>,
    well_known_redirect: bool,
    hang: bool,
    drop_answer_for: Option<String>,
    submission_error: Option<String>,
    // logs
    requests: Vec<RequestRecord>,
    calls: Vec<String>,
    deliveries: Vec<Delivery>,
}

type Shared = Arc<Mutex<State>>;

/// A running server. Dropping it stops accepting connections.
pub struct JmapServer {
    addr: SocketAddr,
    state: Shared,
    cert_der: Option<Vec<u8>>,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for JmapServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// The user name the server accepts.
pub const JMAP_USER: &str = "alice@example.org";
/// The password the server accepts.
pub const JMAP_PASSWORD: &str = "alice-test-password-1";

impl JmapServer {
    /// Starts a plaintext server with the standard mailboxes: Inbox (role `inbox`), Drafts, Sent
    /// Items, Deleted Items (`trash`), Junk Mail (`junk`), and Archive, all empty.
    pub async fn start() -> Self {
        Self::start_with(JmapOptions::default()).await
    }

    /// Starts a server with these options.
    pub async fn start_with(opts: JmapOptions) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("local addr");
        let (acceptor, cert_der) = if opts.tls {
            let (a, d) = tls_acceptor();
            (Some(a), Some(d))
        } else {
            (None, None)
        };
        let mb = |id: &str, name: &str, role: Option<&str>| Mailbox {
            id: id.into(),
            name: name.into(),
            role: role.map(Into::into),
            parent: None,
        };
        let state: Shared = Arc::new(Mutex::new(State {
            user: JMAP_USER.into(),
            password: JMAP_PASSWORD.into(),
            bearer: None,
            account: "A1".into(),
            mailboxes: vec![
                mb("mb-inbox", "Inbox", Some("inbox")),
                mb("mb-drafts", "Drafts", Some("drafts")),
                mb("mb-sent", "Sent Items", Some("sent")),
                mb("mb-trash", "Deleted Items", Some("trash")),
                mb("mb-junk", "Junk Mail", Some("junk")),
                mb("mb-archive", "Archive", Some("archive")),
            ],
            emails: BTreeMap::new(),
            blobs: HashMap::new(),
            next_id: 1,
            state: 1,
            log: Vec::new(),
            oldest_state: 1,
            cannot_calculate: false,
            header_search: true,
            api_status: None,
            session_api_url: None,
            session_redirect: None,
            well_known_redirect: true,
            hang: false,
            drop_answer_for: None,
            submission_error: None,
            requests: Vec::new(),
            calls: Vec::new(),
            deliveries: Vec::new(),
        }));
        let task = {
            let state = state.clone();
            tokio::spawn(async move {
                loop {
                    let Ok((tcp, _)) = listener.accept().await else {
                        break;
                    };
                    let state = state.clone();
                    let acceptor = acceptor.clone();
                    tokio::spawn(async move {
                        match acceptor {
                            Some(a) => {
                                if let Ok(tls) = a.accept(tcp).await {
                                    serve(tls, state, addr).await;
                                }
                            }
                            None => serve(tcp, state, addr).await,
                        }
                    });
                }
            })
        };
        Self {
            addr,
            state,
            cert_der,
            task,
        }
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().expect("server state")
    }

    /// The port.
    pub fn port(&self) -> u16 {
        self.addr.port()
    }

    /// The session URL for this server (`http`, or `https` when started with TLS).
    pub fn session_url(&self) -> String {
        let scheme = if self.cert_der.is_some() {
            "https"
        } else {
            "http"
        };
        let host = if self.cert_der.is_some() {
            "localhost"
        } else {
            "127.0.0.1"
        };
        format!("{scheme}://{host}:{}/jmap/session", self.addr.port())
    }

    /// The `/.well-known/jmap` URL, which redirects to the session.
    pub fn well_known_url(&self) -> String {
        self.session_url()
            .replace("/jmap/session", "/.well-known/jmap")
    }

    /// The certificate (DER) when started with TLS.
    pub fn certificate_der(&self) -> Option<Vec<u8>> {
        self.cert_der.clone()
    }

    /// The user name the server accepts.
    pub fn username(&self) -> &'static str {
        JMAP_USER
    }

    /// The password the server accepts.
    pub fn password(&self) -> &'static str {
        JMAP_PASSWORD
    }

    /// Also accepts `Authorization: Bearer <token>`.
    pub fn set_bearer_token(&self, token: &str) {
        self.lock().bearer = Some(token.to_string());
    }

    // ---- "another client" ----

    /// Adds a message (raw RFC 5322 text) to the mailbox with this name or role, with these
    /// keywords. Returns the JMAP id. Messages get increasing `receivedAt` times, one second apart.
    pub fn add_message(&self, mailbox: &str, raw: &str, keywords: &[&str]) -> String {
        let mut s = self.lock();
        add_email(&mut s, mailbox, raw, keywords)
    }

    /// Sets or clears a keyword on a message.
    pub fn set_keyword(&self, email_id: &str, keyword: &str, on: bool) {
        let mut s = self.lock();
        let changed = match s.emails.get_mut(email_id) {
            Some(e) if on => e.keywords.insert(keyword.to_string()),
            Some(e) => e.keywords.remove(keyword),
            None => false,
        };
        if changed {
            record(&mut s, email_id, Kind::Updated);
        }
    }

    /// Moves a message from one mailbox (name or role) to another.
    pub fn move_message(&self, email_id: &str, from: &str, to: &str) {
        let mut s = self.lock();
        let (from, to) = (mailbox_id(&s, from), mailbox_id(&s, to));
        if let Some(e) = s.emails.get_mut(email_id) {
            e.mailboxes.remove(&from);
            e.mailboxes.insert(to);
            record(&mut s, email_id, Kind::Updated);
        }
    }

    /// Files a message under another mailbox as well (a label).
    pub fn add_to_mailbox(&self, email_id: &str, mailbox: &str) {
        let mut s = self.lock();
        let mb = mailbox_id(&s, mailbox);
        if let Some(e) = s.emails.get_mut(email_id) {
            e.mailboxes.insert(mb);
            record(&mut s, email_id, Kind::Updated);
        }
    }

    /// Destroys a message.
    pub fn destroy_message(&self, email_id: &str) {
        let mut s = self.lock();
        if s.emails.remove(email_id).is_some() {
            record(&mut s, email_id, Kind::Destroyed);
        }
    }

    /// Adds a mailbox with this name under a parent (by name, or `None`).
    pub fn add_mailbox(&self, name: &str, parent: Option<&str>) -> String {
        let mut s = self.lock();
        let parent = parent.map(|p| mailbox_id(&s, p));
        let id = format!("mb-{}", s.next_id);
        s.next_id += 1;
        s.mailboxes.push(Mailbox {
            id: id.clone(),
            name: name.into(),
            role: None,
            parent,
        });
        id
    }

    /// Removes a mailbox (and the messages that are only in it).
    pub fn remove_mailbox(&self, name: &str) {
        let mut s = self.lock();
        let id = mailbox_id(&s, name);
        s.mailboxes.retain(|m| m.id != id);
        let gone: Vec<String> = s
            .emails
            .iter()
            .filter(|(_, e)| e.mailboxes.len() == 1 && e.mailboxes.contains(&id))
            .map(|(k, _)| k.clone())
            .collect();
        for g in gone {
            s.emails.remove(&g);
            record(&mut s, &g, Kind::Destroyed);
        }
    }

    // ---- misbehaviour ----

    /// Makes `Email/changes` answer `cannotCalculateChanges` (or stops doing so).
    pub fn set_cannot_calculate_changes(&self, on: bool) {
        self.lock().cannot_calculate = on;
    }

    /// Forgets the change log, so every earlier state is too old for `Email/changes`.
    pub fn forget_changes(&self) {
        let mut s = self.lock();
        s.log.clear();
        s.oldest_state = s.state;
    }

    /// Makes the `header` filter of `Email/query` find nothing (as Stalwart does for Message-ID).
    pub fn set_header_search(&self, works: bool) {
        self.lock().header_search = works;
    }

    /// Answers `POST /jmap/` with this HTTP status instead of doing anything.
    pub fn set_api_status(&self, status: Option<u16>) {
        self.lock().api_status = status;
    }

    /// Makes the session advertise this `apiUrl` and `uploadUrl` base (for example another host).
    pub fn set_session_api_base(&self, base: Option<&str>) {
        self.lock().session_api_url = base.map(str::to_string);
    }

    /// Makes `/jmap/session` redirect to this location.
    pub fn set_session_redirect(&self, location: Option<&str>) {
        self.lock().session_redirect = location.map(str::to_string);
    }

    /// Stops answering (the connection stays open).
    pub fn set_hang(&self, on: bool) {
        self.lock().hang = on;
    }

    /// Does the work for this method but closes the connection instead of answering.
    pub fn drop_answer_for(&self, method: Option<&str>) {
        self.lock().drop_answer_for = method.map(str::to_string);
    }

    /// Makes `EmailSubmission/set` refuse creation with this error type (`forbiddenFrom`, ...).
    pub fn set_submission_error(&self, kind: Option<&str>) {
        self.lock().submission_error = kind.map(str::to_string);
    }

    // ---- what the server saw ----

    /// Every request, oldest first.
    pub fn requests(&self) -> Vec<RequestRecord> {
        self.lock().requests.clone()
    }

    /// Every JMAP method called, oldest first.
    pub fn calls(&self) -> Vec<String> {
        self.lock().calls.clone()
    }

    /// Forgets the logs.
    pub fn clear_logs(&self) {
        let mut s = self.lock();
        s.requests.clear();
        s.calls.clear();
    }

    /// Messages accepted by `EmailSubmission/set`.
    pub fn deliveries(&self) -> Vec<Delivery> {
        self.lock().deliveries.clone()
    }

    /// The JMAP ids of the messages in a mailbox (name or role).
    pub fn ids_in(&self, mailbox: &str) -> Vec<String> {
        let s = self.lock();
        let id = mailbox_id(&s, mailbox);
        s.emails
            .iter()
            .filter(|(_, e)| e.mailboxes.contains(&id))
            .map(|(k, _)| k.clone())
            .collect()
    }

    /// The current `Email` state string.
    pub fn email_state(&self) -> String {
        format!("s{}", self.lock().state)
    }

    /// The keywords of a message.
    pub fn keywords_of(&self, email_id: &str) -> Vec<String> {
        self.lock()
            .emails
            .get(email_id)
            .map(|e| e.keywords.iter().cloned().collect())
            .unwrap_or_default()
    }
}

// ---------------------------------------------------------------------------------------------
// State changes
// ---------------------------------------------------------------------------------------------

fn mailbox_id(s: &State, name_or_role: &str) -> String {
    s.mailboxes
        .iter()
        .find(|m| {
            m.name == name_or_role
                || m.role.as_deref() == Some(name_or_role)
                || m.id == name_or_role
        })
        .map(|m| m.id.clone())
        .unwrap_or_else(|| name_or_role.to_string())
}

fn record(s: &mut State, id: &str, kind: Kind) {
    s.state += 1;
    let at = s.state;
    s.log.push((at, id.to_string(), kind));
}

fn add_email(s: &mut State, mailbox: &str, raw: &str, keywords: &[&str]) -> String {
    let mb = mailbox_id(s, mailbox);
    let id = format!("e{}", s.next_id);
    s.next_id += 1;
    let received = s.next_id;
    s.emails.insert(
        id.clone(),
        Email {
            mailboxes: BTreeSet::from([mb]),
            keywords: keywords.iter().map(|k| k.to_string()).collect(),
            raw: raw.to_string(),
            received,
        },
    );
    record(s, &id, Kind::Created);
    id
}

// ---------------------------------------------------------------------------------------------
// HTTP
// ---------------------------------------------------------------------------------------------

struct Reply {
    status: u16,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
    /// Close without answering.
    drop_connection: bool,
}

impl Reply {
    fn status(status: u16) -> Self {
        Self {
            status,
            headers: Vec::new(),
            body: Vec::new(),
            drop_connection: false,
        }
    }

    fn json(value: &Value) -> Self {
        Self {
            status: 200,
            headers: vec![("Content-Type".into(), "application/json".into())],
            body: value.to_string().into_bytes(),
            drop_connection: false,
        }
    }

    fn redirect(location: &str) -> Self {
        let mut r = Self::status(307);
        r.headers.push(("Location".into(), location.into()));
        r
    }
}

async fn serve<S: AsyncRead + AsyncWrite + Unpin>(mut stream: S, state: Shared, addr: SocketAddr) {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    let head_end = loop {
        if let Some(p) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break p;
        }
        match stream.read(&mut chunk).await {
            Ok(0) | Err(_) => return,
            Ok(n) => buf.extend_from_slice(&chunk[..n]),
        }
    };
    let head = String::from_utf8_lossy(&buf[..head_end]).to_string();
    let mut lines = head.split("\r\n");
    let request_line = lines.next().unwrap_or("");
    let mut parts = request_line.split_whitespace();
    let (method, path) = (
        parts.next().unwrap_or("").to_string(),
        parts.next().unwrap_or("").to_string(),
    );
    let mut authorization = None;
    let mut length = 0usize;
    for l in lines {
        if let Some((k, v)) = l.split_once(':') {
            match k.trim().to_ascii_lowercase().as_str() {
                "authorization" => authorization = Some(v.trim().to_string()),
                "content-length" => length = v.trim().parse().unwrap_or(0),
                _ => {}
            }
        }
    }
    let mut body = buf[head_end + 4..].to_vec();
    while body.len() < length {
        match stream.read(&mut chunk).await {
            Ok(0) | Err(_) => return,
            Ok(n) => body.extend_from_slice(&chunk[..n]),
        }
    }
    body.truncate(length);

    let hang = {
        let mut s = state.lock().expect("server state");
        s.requests.push(RequestRecord {
            method: method.clone(),
            path: path.clone(),
            had_authorization: authorization.is_some(),
        });
        s.hang
    };
    if hang {
        tokio::time::sleep(std::time::Duration::from_secs(3600)).await;
        return;
    }
    let reply = {
        let mut s = state.lock().expect("server state");
        route(
            &mut s,
            &method,
            &path,
            authorization.as_deref(),
            &body,
            addr,
        )
    };
    if reply.drop_connection {
        return;
    }
    let mut out = format!("HTTP/1.1 {} X\r\n", reply.status);
    for (k, v) in &reply.headers {
        out.push_str(&format!("{k}: {v}\r\n"));
    }
    out.push_str(&format!(
        "Content-Length: {}\r\nConnection: close\r\n\r\n",
        reply.body.len()
    ));
    let _ = stream.write_all(out.as_bytes()).await;
    let _ = stream.write_all(&reply.body).await;
    let _ = stream.shutdown().await;
}

fn authorized(s: &State, header: Option<&str>) -> bool {
    let Some(h) = header else { return false };
    if let Some(token) = h.strip_prefix("Bearer ") {
        return s.bearer.as_deref() == Some(token);
    }
    if let Some(b64) = h.strip_prefix("Basic ") {
        return decode_base64(b64.trim())
            .is_some_and(|raw| raw == format!("{}:{}", s.user, s.password).into_bytes());
    }
    false
}

fn decode_base64(text: &str) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    let (mut acc, mut bits) = (0u32, 0);
    for c in text.bytes().filter(|c| *c != b'=') {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return None,
        };
        acc = (acc << 6) | u32::from(v);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    Some(out)
}

fn route(
    s: &mut State,
    method: &str,
    path: &str,
    auth: Option<&str>,
    body: &[u8],
    addr: SocketAddr,
) -> Reply {
    if path == "/.well-known/jmap" {
        return if s.well_known_redirect {
            Reply::redirect("/jmap/session")
        } else {
            Reply::status(404)
        };
    }
    if !authorized(s, auth) {
        return Reply::status(401);
    }
    match (method, path) {
        ("GET", "/jmap/session") => {
            if let Some(to) = &s.session_redirect {
                return Reply::redirect(to);
            }
            Reply::json(&session_json(s, addr))
        }
        ("POST", "/jmap/") => {
            if let Some(status) = s.api_status {
                return Reply::status(status);
            }
            api(s, body)
        }
        ("POST", p) if p.starts_with("/jmap/upload/") => {
            let id = format!("blob{}", s.next_id);
            s.next_id += 1;
            let text = String::from_utf8_lossy(body).to_string();
            let size = body.len();
            s.blobs.insert(id.clone(), text);
            let mut r = Reply::json(
                &json!({"accountId": s.account, "blobId": id, "type": "message/rfc822", "size": size}),
            );
            r.status = 201;
            r
        }
        _ => Reply::status(404),
    }
}

fn session_json(s: &State, addr: SocketAddr) -> Value {
    let base = s
        .session_api_url
        .clone()
        .unwrap_or_else(|| format!("http://127.0.0.1:{}", addr.port()));
    json!({
        "capabilities": {
            "urn:ietf:params:jmap:core": {"maxObjectsInGet": 50, "maxObjectsInSet": 50, "maxCallsInRequest": 16,
                "maxSizeUpload": 10_000_000},
            "urn:ietf:params:jmap:mail": {},
            "urn:ietf:params:jmap:submission": {},
        },
        "accounts": {s.account.clone(): {"name": s.user, "isPersonal": true, "isReadOnly": false,
            "accountCapabilities": {"urn:ietf:params:jmap:mail": {}, "urn:ietf:params:jmap:submission": {}}}},
        "primaryAccounts": {"urn:ietf:params:jmap:mail": s.account, "urn:ietf:params:jmap:submission": s.account},
        "username": s.user,
        "apiUrl": format!("{base}/jmap/"),
        "uploadUrl": format!("{base}/jmap/upload/{{accountId}}/"),
        "state": "session-1",
    })
}

// ---------------------------------------------------------------------------------------------
// JMAP methods
// ---------------------------------------------------------------------------------------------

fn api(s: &mut State, body: &[u8]) -> Reply {
    let Ok(req) = serde_json::from_slice::<Value>(body) else {
        return Reply::status(400);
    };
    let Some(calls) = req["methodCalls"].as_array() else {
        return Reply::status(400);
    };
    let mut responses: Vec<Value> = Vec::new();
    let mut created_ids: HashMap<String, String> = HashMap::new();
    let mut drop_connection = false;
    for call in calls {
        let name = call[0].as_str().unwrap_or("").to_string();
        let args = call[1].clone();
        let id = call[2].clone();
        s.calls.push(name.clone());
        let mut extra: Vec<Value> = Vec::new();
        let result = match name.as_str() {
            "Mailbox/get" => mailbox_get(s, &args),
            "Email/query" => email_query(s, &args),
            "Email/get" => email_get(s, &args),
            "Email/changes" => email_changes(s, &args),
            "Email/set" => email_set(s, &args),
            "Email/import" => email_import(s, &args, &mut created_ids),
            "Identity/get" => Ok(
                json!({"accountId": s.account, "state": "i1", "notFound": [],
                "list": [{"id": "I1", "name": s.user, "email": s.user}]}),
            ),
            "EmailSubmission/set" => submission_set(s, &args, &mut created_ids, &id, &mut extra),
            _ => Err(json!({"type": "unknownMethod"})),
        };
        match result {
            Ok(v) => responses.push(json!([name, v, id])),
            Err(e) => responses.push(json!(["error", e, id])),
        }
        responses.extend(extra);
        if s.drop_answer_for.as_deref() == Some(name.as_str()) {
            drop_connection = true;
        }
    }
    let mut reply =
        Reply::json(&json!({"methodResponses": responses, "sessionState": "session-1"}));
    reply.drop_connection = drop_connection;
    reply
}

fn mailbox_get(s: &State, args: &Value) -> Result<Value, Value> {
    let wanted: Option<Vec<String>> = args["ids"].as_array().map(|a| {
        a.iter()
            .filter_map(|v| v.as_str().map(str::to_string))
            .collect()
    });
    let mut list = Vec::new();
    let mut not_found = Vec::new();
    for m in &s.mailboxes {
        if wanted.as_ref().is_some_and(|w| !w.contains(&m.id)) {
            continue;
        }
        let total = s
            .emails
            .values()
            .filter(|e| e.mailboxes.contains(&m.id))
            .count();
        let unread = s
            .emails
            .values()
            .filter(|e| e.mailboxes.contains(&m.id) && !e.keywords.contains("$seen"))
            .count();
        list.push(
            json!({"id": m.id, "name": m.name, "parentId": m.parent, "role": m.role,
            "totalEmails": total, "unreadEmails": unread,
            "myRights": {"mayReadItems": true, "mayAddItems": true, "mayRemoveItems": true}}),
        );
    }
    if let Some(w) = wanted {
        not_found = w
            .into_iter()
            .filter(|id| !s.mailboxes.iter().any(|m| &m.id == id))
            .collect();
    }
    Ok(json!({"accountId": s.account, "state": "m1", "list": list, "notFound": not_found}))
}

fn header_value(raw: &str, name: &str) -> Option<String> {
    let head = raw.split("\r\n\r\n").next().unwrap_or(raw);
    let mut value: Option<String> = None;
    for line in head.split("\r\n") {
        if line.starts_with(' ') || line.starts_with('\t') {
            if let Some(v) = value.as_mut() {
                v.push(' ');
                v.push_str(line.trim());
            }
        } else if value.is_some() {
            break;
        } else if let Some((k, v)) = line.split_once(':') {
            if k.eq_ignore_ascii_case(name) {
                value = Some(v.trim().to_string());
            }
        }
    }
    value
}

fn angle_ids(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(a) = rest.find('<') {
        let Some(b) = rest[a..].find('>') else { break };
        out.push(rest[a + 1..a + b].to_string());
        rest = &rest[a + b + 1..];
    }
    out
}

/// RFC 2047 encoded words (UTF-8, `Q` and `B`).
fn decode_words(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(start) = rest.find("=?") {
        out.push_str(&rest[..start]);
        let tail = &rest[start + 2..];
        let parts: Vec<&str> = tail.splitn(4, '?').collect();
        if parts.len() == 4 && parts[3].starts_with('=') {
            let data = parts[2];
            let after = &parts[3][1..];
            let bytes = if parts[1].eq_ignore_ascii_case("q") {
                let mut b = Vec::new();
                let raw = data.as_bytes();
                let mut i = 0;
                while i < raw.len() {
                    match raw[i] {
                        b'_' => b.push(b' '),
                        b'=' if i + 3 <= raw.len() => {
                            let h = std::str::from_utf8(&raw[i + 1..i + 3]).unwrap_or("00");
                            b.push(u8::from_str_radix(h, 16).unwrap_or(b'?'));
                            i += 2;
                        }
                        c => b.push(c),
                    }
                    i += 1;
                }
                b
            } else {
                decode_base64(data).unwrap_or_default()
            };
            out.push_str(&String::from_utf8_lossy(&bytes));
            rest = after.trim_start_matches(' ');
            if rest.starts_with("=?") {
                continue;
            }
        } else {
            out.push_str("=?");
            rest = tail;
        }
    }
    out.push_str(rest);
    out
}

fn sent_at(raw: &str) -> Value {
    // "Mon, 02 Mar 2026 09:30:00 +0000"
    let Some(date) = header_value(raw, "Date") else {
        return Value::Null;
    };
    let t: Vec<&str> = date.split_whitespace().collect();
    if t.len() < 6 || t[5] != "+0000" {
        return Value::Null;
    }
    let months = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let Some(m) = months.iter().position(|x| *x == t[2]) else {
        return Value::Null;
    };
    let day: u32 = t[1].parse().unwrap_or(1);
    json!(format!("{}-{:02}-{:02}T{}Z", t[3], m + 1, day, t[4]))
}

fn received_at(n: u64) -> String {
    let secs = n % 60;
    let mins = (n / 60) % 60;
    let hours = (n / 3600) % 24;
    format!("2026-11-01T{hours:02}:{mins:02}:{secs:02}Z")
}

fn email_json(id: &str, e: &Email, props: &[String]) -> Value {
    let mut o = Map::new();
    for p in props {
        let v = match p.as_str() {
            "id" => json!(id),
            "mailboxIds" => Value::Object(e.mailboxes.iter().map(|m| (m.clone(), json!(true))).collect()),
            "keywords" => Value::Object(e.keywords.iter().map(|k| (k.clone(), json!(true))).collect()),
            "receivedAt" => json!(received_at(e.received)),
            "sentAt" => sent_at(&e.raw),
            "size" => json!(e.raw.len()),
            "subject" => header_value(&e.raw, "Subject").map_or(Value::Null, |v| json!(decode_words(&v))),
            "messageId" => header_value(&e.raw, "Message-ID").map_or(Value::Null, |v| json!(angle_ids(&v))),
            "inReplyTo" => header_value(&e.raw, "In-Reply-To").map_or(Value::Null, |v| json!(angle_ids(&v))),
            "references" => header_value(&e.raw, "References").map_or(Value::Null, |v| json!(angle_ids(&v))),
            "from" => header_value(&e.raw, "From").map_or(Value::Null, |v| {
                let (name, addr) = match v.split_once('<') {
                    Some((n, a)) => (n.trim().trim_matches('"').to_string(), a.trim_end_matches('>').to_string()),
                    None => (String::new(), v.clone()),
                };
                json!([{"name": if name.is_empty() { Value::Null } else { json!(decode_words(&name)) }, "email": addr}])
            }),
            _ => continue,
        };
        o.insert(p.clone(), v);
    }
    Value::Object(o)
}

fn props_of(args: &Value, default: &[&str]) -> Vec<String> {
    args["properties"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_else(|| default.iter().map(|s| s.to_string()).collect())
}

fn email_get(s: &State, args: &Value) -> Result<Value, Value> {
    let props = props_of(
        args,
        &[
            "id",
            "mailboxIds",
            "keywords",
            "receivedAt",
            "size",
            "subject",
        ],
    );
    let ids: Vec<String> = match args["ids"].as_array() {
        Some(a) => a
            .iter()
            .filter_map(|v| v.as_str().map(str::to_string))
            .collect(),
        None => s.emails.keys().cloned().collect(),
    };
    let max = 50;
    if ids.len() > max {
        return Err(json!({"type": "requestTooLarge"}));
    }
    let mut list = Vec::new();
    let mut not_found = Vec::new();
    for id in ids {
        match s.emails.get(&id) {
            Some(e) => list.push(email_json(&id, e, &props)),
            None => not_found.push(id),
        }
    }
    Ok(
        json!({"accountId": s.account, "state": format!("s{}", s.state), "list": list, "notFound": not_found}),
    )
}

fn email_query(s: &State, args: &Value) -> Result<Value, Value> {
    let filter = &args["filter"];
    let mut matches: Vec<(&String, &Email)> = s.emails.iter().collect();
    if let Some(obj) = filter.as_object() {
        for (k, v) in obj {
            match k.as_str() {
                "inMailbox" => {
                    let want = v.as_str().unwrap_or("");
                    matches.retain(|(_, e)| e.mailboxes.contains(want));
                }
                "header" => {
                    let name = v[0].as_str().unwrap_or("");
                    let text = v.get(1).and_then(Value::as_str);
                    matches.retain(|(_, e)| {
                        s.header_search
                            && header_value(&e.raw, name).is_some_and(|hv| {
                                text.is_none_or(|t| hv.to_lowercase().contains(&t.to_lowercase()))
                            })
                    });
                }
                _ => return Err(json!({"type": "unsupportedFilter"})),
            }
        }
    }
    let ascending = args["sort"][0]["isAscending"].as_bool().unwrap_or(false);
    matches.sort_by(|a, b| (a.1.received, a.0).cmp(&(b.1.received, b.0)));
    if !ascending {
        matches.reverse();
    }
    let total = matches.len();
    let position = args["position"].as_u64().unwrap_or(0) as usize;
    let limit = args["limit"].as_u64().map_or(100, |l| l as usize).min(100);
    let ids: Vec<&String> = matches
        .iter()
        .skip(position)
        .take(limit)
        .map(|(k, _)| *k)
        .collect();
    let mut out = json!({"accountId": s.account, "queryState": format!("s{}", s.state),
        "canCalculateChanges": true, "position": position, "ids": ids});
    if args["calculateTotal"].as_bool() == Some(true) {
        out["total"] = json!(total);
    }
    Ok(out)
}

fn email_changes(s: &State, args: &Value) -> Result<Value, Value> {
    let since = args["sinceState"].as_str().unwrap_or("");
    let Some(n) = since.strip_prefix('s').and_then(|n| n.parse::<u64>().ok()) else {
        return Err(json!({"type": "invalidArguments", "description": "invalid JMAP State"}));
    };
    if s.cannot_calculate || n < s.oldest_state || n > s.state {
        return Err(json!({"type": "cannotCalculateChanges"}));
    }
    let max = args["maxChanges"]
        .as_u64()
        .map_or(50, |m| m as usize)
        .max(1);
    let (mut created, mut updated, mut destroyed): (Vec<String>, Vec<String>, Vec<String>) =
        Default::default();
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut new_state = n;
    let mut has_more = false;
    for (at, id, _) in s.log.iter().filter(|(at, _, _)| *at > n) {
        if !seen.contains(id) && seen.len() >= max {
            has_more = true;
            break;
        }
        seen.insert(id.clone());
        new_state = *at;
    }
    for id in &seen {
        let kinds: Vec<Kind> = s
            .log
            .iter()
            .filter(|(at, i, _)| *at > n && *at <= new_state && i == id)
            .map(|(_, _, k)| *k)
            .collect();
        if kinds.contains(&Kind::Destroyed) {
            if !kinds.contains(&Kind::Created) {
                destroyed.push(id.clone());
            }
        } else if kinds.contains(&Kind::Created) {
            created.push(id.clone());
        } else {
            updated.push(id.clone());
        }
    }
    if !has_more {
        new_state = s.state;
    }
    Ok(
        json!({"accountId": s.account, "oldState": since, "newState": format!("s{new_state}"),
        "hasMoreChanges": has_more, "created": created, "updated": updated, "destroyed": destroyed}),
    )
}

fn apply_patch(s: &State, email: &mut Email, patch: &Value) -> Result<(), Value> {
    let bad = || json!({"type": "invalidProperties"});
    let Some(obj) = patch.as_object() else {
        return Err(bad());
    };
    for (k, v) in obj {
        if let Some(kw) = k.strip_prefix("keywords/") {
            if v.is_null() {
                email.keywords.remove(kw);
            } else if v == &json!(true) {
                email.keywords.insert(kw.to_string());
            } else {
                return Err(bad());
            }
        } else if let Some(mb) = k.strip_prefix("mailboxIds/") {
            let mb = mb.replace("~1", "/").replace("~0", "~");
            if v.is_null() {
                email.mailboxes.remove(&mb);
            } else if v == &json!(true) && s.mailboxes.iter().any(|m| m.id == mb) {
                email.mailboxes.insert(mb);
            } else {
                return Err(bad());
            }
        } else {
            return Err(bad());
        }
    }
    if email.mailboxes.is_empty() {
        return Err(bad());
    }
    Ok(())
}

fn email_set(s: &mut State, args: &Value) -> Result<Value, Value> {
    let mut updated = Map::new();
    let mut not_updated = Map::new();
    if let Some(u) = args["update"].as_object() {
        for (id, patch) in u {
            let Some(current) = s.emails.get(id).cloned() else {
                not_updated.insert(id.clone(), json!({"type": "notFound"}));
                continue;
            };
            let mut next = current.clone();
            match apply_patch(s, &mut next, patch) {
                Ok(()) => {
                    let changed =
                        next.keywords != current.keywords || next.mailboxes != current.mailboxes;
                    s.emails.insert(id.clone(), next);
                    if changed {
                        record(s, id, Kind::Updated);
                    }
                    updated.insert(id.clone(), Value::Null);
                }
                Err(e) => {
                    not_updated.insert(id.clone(), e);
                }
            }
        }
    }
    let mut destroyed = Vec::new();
    let mut not_destroyed = Map::new();
    if let Some(d) = args["destroy"].as_array() {
        for id in d.iter().filter_map(Value::as_str) {
            if s.emails.remove(id).is_some() {
                record(s, id, Kind::Destroyed);
                destroyed.push(id.to_string());
            } else {
                not_destroyed.insert(id.to_string(), json!({"type": "notFound"}));
            }
        }
    }
    let mut out = json!({"accountId": s.account, "newState": format!("s{}", s.state)});
    if !updated.is_empty() {
        out["updated"] = Value::Object(updated);
    }
    if !not_updated.is_empty() {
        out["notUpdated"] = Value::Object(not_updated);
    }
    if !destroyed.is_empty() {
        out["destroyed"] = json!(destroyed);
    }
    if !not_destroyed.is_empty() {
        out["notDestroyed"] = Value::Object(not_destroyed);
    }
    Ok(out)
}

fn email_import(
    s: &mut State,
    args: &Value,
    created_ids: &mut HashMap<String, String>,
) -> Result<Value, Value> {
    let mut created = Map::new();
    let mut not_created = Map::new();
    if let Some(emails) = args["emails"].as_object() {
        for (cid, spec) in emails {
            let blob = spec["blobId"].as_str().unwrap_or("");
            let Some(raw) = s.blobs.get(blob).cloned() else {
                not_created.insert(cid.clone(), json!({"type": "blobNotFound"}));
                continue;
            };
            let Some(mb) = spec["mailboxIds"]
                .as_object()
                .and_then(|m| m.keys().next().cloned())
            else {
                not_created.insert(cid.clone(), json!({"type": "invalidProperties"}));
                continue;
            };
            let keywords: Vec<String> = spec["keywords"]
                .as_object()
                .map(|k| k.keys().cloned().collect())
                .unwrap_or_default();
            let kw: Vec<&str> = keywords.iter().map(String::as_str).collect();
            let size = raw.len();
            let id = add_email(s, &mb, &raw, &kw);
            created_ids.insert(cid.clone(), id.clone());
            created.insert(
                cid.clone(),
                json!({"id": id, "threadId": "T1", "blobId": blob, "size": size}),
            );
        }
    }
    Ok(
        json!({"accountId": s.account, "newState": format!("s{}", s.state), "created": created, "notCreated": not_created}),
    )
}

fn submission_set(
    s: &mut State,
    args: &Value,
    created_ids: &mut HashMap<String, String>,
    call_id: &Value,
    extra: &mut Vec<Value>,
) -> Result<Value, Value> {
    let mut created = Map::new();
    let mut not_created = Map::new();
    let mut accepted: Vec<(String, String)> = Vec::new(); // (creation id, email id)
    if let Some(items) = args["create"].as_object() {
        for (cid, spec) in items {
            if let Some(kind) = s.submission_error.clone() {
                not_created.insert(
                    cid.clone(),
                    json!({"type": kind, "description": "alice@example.org"}),
                );
                continue;
            }
            let email_ref = spec["emailId"].as_str().unwrap_or("");
            let email_id = match email_ref.strip_prefix('#') {
                Some(r) => created_ids.get(r).cloned().unwrap_or_default(),
                None => email_ref.to_string(),
            };
            let Some(email) = s.emails.get(&email_id).cloned() else {
                not_created.insert(cid.clone(), json!({"type": "invalidProperties"}));
                continue;
            };
            if spec["identityId"] != json!("I1") {
                not_created.insert(cid.clone(), json!({"type": "invalidProperties"}));
                continue;
            }
            let from = spec["envelope"]["mailFrom"]["email"]
                .as_str()
                .unwrap_or("")
                .to_string();
            let recipients: Vec<String> = spec["envelope"]["rcptTo"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|r| r["email"].as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default();
            if recipients.is_empty() {
                not_created.insert(cid.clone(), json!({"type": "noRecipients"}));
                continue;
            }
            s.deliveries.push(Delivery {
                from,
                recipients,
                raw: email.raw,
            });
            created.insert(
                cid.clone(),
                json!({"id": format!("sub{}", s.deliveries.len()), "undoStatus": "pending"}),
            );
            accepted.push((cid.clone(), email_id));
        }
    }
    // onSuccessUpdateEmail: the implied Email/set, answered under the same call id.
    if let Some(updates) = args["onSuccessUpdateEmail"].as_object() {
        let mut patches = Map::new();
        for (key, patch) in updates {
            if let Some((_, email_id)) = key
                .strip_prefix('#')
                .and_then(|c| accepted.iter().find(|(cid, _)| cid == c))
            {
                patches.insert(email_id.clone(), patch.clone());
            }
        }
        if !patches.is_empty() {
            let r = email_set(s, &json!({"update": patches})).unwrap_or(Value::Null);
            extra.push(json!(["Email/set", r, call_id]));
        }
    }
    if let Some(ids) = args["onSuccessDestroyEmail"].as_array() {
        let targets: Vec<String> = ids
            .iter()
            .filter_map(Value::as_str)
            .filter_map(|r| r.strip_prefix('#'))
            .filter_map(|c| {
                accepted
                    .iter()
                    .find(|(cid, _)| cid == c)
                    .map(|(_, e)| e.clone())
            })
            .collect();
        if !targets.is_empty() {
            let r = email_set(s, &json!({"destroy": targets})).unwrap_or(Value::Null);
            extra.push(json!(["Email/set", r, call_id]));
        }
    }
    Ok(
        json!({"accountId": s.account, "newState": "sub1", "created": created, "notCreated": not_created}),
    )
}
