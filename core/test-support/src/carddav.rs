// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! An in-process CardDAV and CalDAV server for tests (T38a, T39a), plaintext on loopback:
//! `/.well-known/carddav` and `/.well-known/caldav` redirect to `/dav/`, the principal is
//! `/dav/principals/alice/`, its address book home `/dav/books/alice/` with one address book,
//! `default/`, and its calendar home `/dav/cal/alice/` with a calendar of events, `work/`, and one
//! of tasks only, `tasks/`. It answers `PROPFIND`, `REPORT` (`sync-collection` with tokens `tok-N`,
//! the multigets), conditional `PUT` and `DELETE`, and checks HTTP Basic. Every request is recorded
//! (method, path, headers without the credential).

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

/// The made-up user of the fake.
pub const DAV_USER: &str = "alice";
/// Its made-up password.
pub const DAV_PASSWORD: &str = "made-up-dav-password";
/// The address book's path.
pub const BOOK: &str = "/dav/books/alice/default/";
/// The calendar of events.
pub const CALENDAR: &str = "/dav/cal/alice/work/";
/// A calendar of tasks only (Croft leaves it out).
pub const TASKS: &str = "/dav/cal/alice/tasks/";

/// One request the server got.
#[derive(Debug, Clone)]
pub struct DavRequest {
    /// Method.
    pub method: String,
    /// Path.
    pub path: String,
    /// Header names (lower case) and values, without `Authorization`.
    pub headers: Vec<(String, String)>,
    /// The body.
    pub body: String,
    /// It carried an `Authorization` header.
    pub had_authorization: bool,
}

#[derive(Default)]
struct State {
    user: String,
    password: String,
    cards: BTreeMap<String, (String, String)>,
    /// (sequence, path, removed).
    log: Vec<(u64, String, bool)>,
    seq: u64,
    sync: bool,
    well_known: bool,
    principal_href: Option<String>,
    /// The scheme named in `WWW-Authenticate` on a 401 (`Basic`), or none.
    challenge: Option<String>,
    requests: Vec<DavRequest>,
}

type Shared = Arc<Mutex<State>>;

/// The fake server; it stops when dropped.
pub struct DavServer {
    state: Shared,
    port: u16,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for DavServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl DavServer {
    /// Starts the server with an empty address book and sync support.
    pub async fn start() -> Self {
        Self::start_with(DAV_USER, DAV_PASSWORD).await
    }

    /// The same, signing in `user` with `password`.
    pub async fn start_with(user: &str, password: &str) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let port = listener.local_addr().expect("addr").port();
        let state: Shared = Arc::new(Mutex::new(State {
            user: user.to_owned(),
            password: password.to_owned(),
            sync: true,
            well_known: true,
            challenge: Some("Basic".into()),
            ..State::default()
        }));
        let s = state.clone();
        let task = tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                let s = s.clone();
                tokio::spawn(async move { serve(stream, s).await });
            }
        });
        Self { state, port, task }
    }

    fn st(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().expect("dav state")
    }

    /// `http://127.0.0.1:<port>`.
    pub fn origin(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    /// Adds or replaces a card (as another client would); returns its etag.
    pub fn put_card(&self, name: &str, text: &str) -> String {
        let mut s = self.st();
        let path = format!("{BOOK}{name}");
        bump(&mut s, &path, Some(text.to_owned()))
    }

    /// Adds or replaces a calendar object in [`CALENDAR`] (as another client would); returns its etag.
    pub fn put_event(&self, name: &str, text: &str) -> String {
        let mut s = self.st();
        let path = format!("{CALENDAR}{name}");
        bump(&mut s, &path, Some(text.to_owned()))
    }

    /// Removes a calendar object from [`CALENDAR`] (as another client would).
    pub fn remove_event(&self, name: &str) {
        let mut s = self.st();
        let path = format!("{CALENDAR}{name}");
        bump(&mut s, &path, None);
    }

    /// Removes a card (as another client would).
    pub fn remove_card(&self, name: &str) {
        let mut s = self.st();
        let path = format!("{BOOK}{name}");
        bump(&mut s, &path, None);
    }

    /// Every object on the server (cards and calendar objects): path and text.
    pub fn cards(&self) -> Vec<(String, String)> {
        self.st()
            .cards
            .iter()
            .map(|(p, (_, t))| (p.clone(), t.clone()))
            .collect()
    }

    /// Turns `sync-collection` support on or off.
    pub fn set_sync(&self, on: bool) {
        self.st().sync = on;
    }

    /// Makes the principal's `href` point elsewhere (another origin, to test refusal).
    pub fn set_principal_href(&self, href: &str) {
        self.st().principal_href = Some(href.to_owned());
    }

    /// The scheme a 401 asks for (`None`: a 401 asks for nothing Croft has).
    pub fn set_challenge(&self, scheme: Option<&str>) {
        self.st().challenge = scheme.map(str::to_owned);
    }

    /// Every request so far.
    pub fn requests(&self) -> Vec<DavRequest> {
        self.st().requests.clone()
    }
}

fn bump(s: &mut State, path: &str, text: Option<String>) -> String {
    s.seq += 1;
    let etag = format!("\"e{}\"", s.seq);
    match text {
        Some(t) => {
            s.cards.insert(path.to_owned(), (etag.clone(), t));
            s.log.push((s.seq, path.to_owned(), false));
        }
        None => {
            s.cards.remove(path);
            s.log.push((s.seq, path.to_owned(), true));
        }
    }
    etag
}

async fn serve(mut stream: tokio::net::TcpStream, state: Shared) {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 8192];
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
    let mut parts = lines.next().unwrap_or("").split_whitespace();
    let (method, path) = (
        parts.next().unwrap_or("").to_owned(),
        parts.next().unwrap_or("").to_owned(),
    );
    let mut headers = Vec::new();
    let mut authorization = None;
    let mut length = 0usize;
    for l in lines {
        if let Some((k, v)) = l.split_once(':') {
            let k = k.trim().to_ascii_lowercase();
            let v = v.trim().to_owned();
            match k.as_str() {
                "authorization" => authorization = Some(v),
                "content-length" => {
                    length = v.parse().unwrap_or(0);
                    headers.push((k, v));
                }
                _ => headers.push((k, v)),
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
    let body = String::from_utf8_lossy(&body).into_owned();
    let (status, extra, out) = {
        let mut s = state.lock().expect("dav state");
        s.requests.push(DavRequest {
            method: method.clone(),
            path: path.clone(),
            headers: headers.clone(),
            body: body.clone(),
            had_authorization: authorization.is_some(),
        });
        if !authorized(&s, authorization.as_deref()) {
            let challenge = s
                .challenge
                .as_ref()
                .map(|c| {
                    vec![(
                        "WWW-Authenticate".to_owned(),
                        format!("{c} realm=\"croft-test\""),
                    )]
                })
                .unwrap_or_default();
            (401, challenge, String::new())
        } else {
            route(&mut s, &method, &path, &headers, &body)
        }
    };
    let mut reply = format!("HTTP/1.1 {status} X\r\n");
    for (k, v) in extra {
        reply.push_str(&format!("{k}: {v}\r\n"));
    }
    reply.push_str(&format!(
        "Content-Type: application/xml; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        out.len()
    ));
    let _ = stream.write_all(reply.as_bytes()).await;
    let _ = stream.write_all(out.as_bytes()).await;
    let _ = stream.shutdown().await;
}

fn authorized(s: &State, header: Option<&str>) -> bool {
    use base64::Engine as _;
    let Some(b64) = header.and_then(|h| h.strip_prefix("Basic ")) else {
        return false;
    };
    base64::engine::general_purpose::STANDARD
        .decode(b64.trim())
        .is_ok_and(|raw| raw == format!("{}:{}", s.user, s.password).into_bytes())
}

fn header<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(k, _)| k == name)
        .map(|(_, v)| v.as_str())
}

fn ms(inner: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="utf-8"?><d:multistatus xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:carddav" xmlns:cal="urn:ietf:params:xml:ns:caldav" xmlns:cs="http://calendarserver.org/ns/" xmlns:a="http://apple.com/ns/ical/">{inner}</d:multistatus>"#
    )
}

fn ok(href: &str, props: &str) -> String {
    format!(
        "<d:response><d:href>{href}</d:href><d:propstat><d:prop>{props}</d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response>"
    )
}

fn xml_escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn between<'a>(text: &'a str, open: &str, close: &str) -> Vec<&'a str> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(i) = rest.find(open) {
        let after = &rest[i + open.len()..];
        let Some(j) = after.find(close) else { break };
        out.push(&after[..j]);
        rest = &after[j + close.len()..];
    }
    out
}

fn route(
    s: &mut State,
    method: &str,
    path: &str,
    headers: &[(String, String)],
    body: &str,
) -> (u16, Vec<(String, String)>, String) {
    let token = format!("tok-{}", s.seq);
    match (method, path) {
        ("PROPFIND", "/.well-known/carddav" | "/.well-known/caldav") if s.well_known => (
            301,
            vec![("Location".into(), "/dav/".into())],
            String::new(),
        ),
        ("PROPFIND", "/dav/") if body.contains("current-user-principal") => {
            let href = s
                .principal_href
                .clone()
                .unwrap_or_else(|| "/dav/principals/alice/".into());
            (
                207,
                Vec::new(),
                ms(&ok(
                    "/dav/",
                    &format!("<d:current-user-principal><d:href>{href}</d:href></d:current-user-principal>"),
                )),
            )
        }
        ("PROPFIND", "/dav/principals/alice/") if body.contains("addressbook-home-set") => (
            207,
            Vec::new(),
            ms(&ok(
                "/dav/principals/alice/",
                "<c:addressbook-home-set><d:href>/dav/books/alice/</d:href></c:addressbook-home-set>",
            )),
        ),
        ("PROPFIND", "/dav/principals/alice/") if body.contains("calendar-home-set") => (
            207,
            Vec::new(),
            ms(&ok(
                "/dav/principals/alice/",
                "<cal:calendar-home-set><d:href>/dav/cal/alice/</d:href></cal:calendar-home-set>",
            )),
        ),
        ("PROPFIND", "/dav/cal/alice/") => {
            let work = format!(
                "<d:resourcetype><d:collection/><cal:calendar/></d:resourcetype><d:displayname>Work</d:displayname>{}<cs:getctag>c{}</cs:getctag><cal:supported-calendar-component-set><cal:comp name=\"VEVENT\"/><cal:comp name=\"VTODO\"/></cal:supported-calendar-component-set><a:calendar-color>#3a7d44ff</a:calendar-color><a:calendar-order>2</a:calendar-order>",
                if s.sync { format!("<d:sync-token>{token}</d:sync-token>") } else { String::new() },
                s.seq
            );
            let tasks = "<d:resourcetype><d:collection/><cal:calendar/></d:resourcetype><d:displayname>Tasks</d:displayname><cal:supported-calendar-component-set><cal:comp name=\"VTODO\"/></cal:supported-calendar-component-set>";
            let inner = format!(
                "{}{}{}",
                ok("/dav/cal/alice/", "<d:resourcetype><d:collection/></d:resourcetype>"),
                ok(CALENDAR, &work),
                ok(TASKS, tasks)
            );
            (207, Vec::new(), ms(&inner))
        }
        ("PROPFIND", "/dav/books/alice/") => {
            let book_props = format!(
                "<d:resourcetype><d:collection/><c:addressbook/></d:resourcetype><d:displayname>Contacts</d:displayname>{}<cs:getctag>c{}</cs:getctag>",
                if s.sync { format!("<d:sync-token>{token}</d:sync-token>") } else { String::new() },
                s.seq
            );
            let inner = format!(
                "{}{}",
                ok("/dav/books/alice/", "<d:resourcetype><d:collection/></d:resourcetype>"),
                ok(BOOK, &book_props)
            );
            (207, Vec::new(), ms(&inner))
        }
        ("PROPFIND", c @ (BOOK | CALENDAR)) => {
            let mut inner = ok(c, "<d:resourcetype><d:collection/></d:resourcetype>");
            for (p, (etag, _)) in s.cards.iter().filter(|(p, _)| p.starts_with(c)) {
                inner.push_str(&ok(p, &format!("<d:getetag>{}</d:getetag><d:resourcetype/>", xml_escape(etag))));
            }
            (207, Vec::new(), ms(&inner))
        }
        ("REPORT", c @ (BOOK | CALENDAR)) if body.contains("sync-collection") => {
            if !s.sync {
                return (403, Vec::new(), String::new());
            }
            let asked = between(body, "<d:sync-token>", "</d:sync-token>")
                .first()
                .and_then(|t| t.strip_prefix("tok-"))
                .and_then(|n| n.parse::<u64>().ok());
            let Some(since) = asked.filter(|n| *n <= s.seq) else {
                return (403, Vec::new(), "<d:error xmlns:d=\"DAV:\"><d:valid-sync-token/></d:error>".into());
            };
            let mut latest: BTreeMap<&str, bool> = BTreeMap::new();
            for (n, p, removed) in &s.log {
                if *n > since && p.starts_with(c) {
                    latest.insert(p.as_str(), *removed);
                }
            }
            let mut inner = String::new();
            for (p, removed) in latest {
                if removed {
                    inner.push_str(&format!(
                        "<d:response><d:href>{p}</d:href><d:status>HTTP/1.1 404 Not Found</d:status></d:response>"
                    ));
                } else if let Some((etag, _)) = s.cards.get(p) {
                    inner.push_str(&ok(p, &format!("<d:getetag>{}</d:getetag>", xml_escape(etag))));
                }
            }
            inner.push_str(&format!("<d:sync-token>{token}</d:sync-token>"));
            (207, Vec::new(), ms(&inner))
        }
        ("REPORT", c @ (BOOK | CALENDAR)) if body.contains("addressbook-multiget") || body.contains("calendar-multiget") => {
            let data = if c == BOOK { "c:address-data" } else { "cal:calendar-data" };
            let mut inner = String::new();
            for href in between(body, "<d:href>", "</d:href>") {
                if let Some((etag, text)) = s.cards.get(href).filter(|_| href.starts_with(c)) {
                    inner.push_str(&ok(
                        href,
                        &format!(
                            "<d:getetag>{}</d:getetag><{data}>{}</{data}>",
                            xml_escape(etag),
                            xml_escape(text)
                        ),
                    ));
                }
            }
            (207, Vec::new(), ms(&inner))
        }
        ("PUT", p) if (p.starts_with(BOOK) && p.len() > BOOK.len()) || (p.starts_with(CALENDAR) && p.len() > CALENDAR.len()) => {
            let current = s.cards.get(p).map(|(e, _)| e.clone());
            match (header(headers, "if-match"), header(headers, "if-none-match"), current) {
                (Some(want), _, Some(have)) if want == have => {
                    bump(s, p, Some(body.to_owned()));
                    (204, Vec::new(), String::new())
                }
                (Some(_), _, Some(_)) => (412, Vec::new(), String::new()),
                (Some(_), _, None) => (404, Vec::new(), String::new()),
                (None, Some("*"), None) => {
                    bump(s, p, Some(body.to_owned()));
                    (201, Vec::new(), String::new())
                }
                (None, Some("*"), Some(_)) => (412, Vec::new(), String::new()),
                _ => (400, Vec::new(), String::new()),
            }
        }
        ("DELETE", p) => {
            let current = s.cards.get(p).map(|(e, _)| e.clone());
            match (header(headers, "if-match"), current) {
                (Some(want), Some(have)) if want == have => {
                    bump(s, p, None);
                    (204, Vec::new(), String::new())
                }
                (_, None) => (404, Vec::new(), String::new()),
                _ => (412, Vec::new(), String::new()),
            }
        }
        _ => (404, Vec::new(), String::new()),
    }
}
