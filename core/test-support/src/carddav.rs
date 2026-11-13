// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! An in-process CardDAV server for tests (T38a), plaintext on loopback: `/.well-known/carddav`
//! redirects to `/dav/`, the principal is `/dav/principals/alice/`, its home `/dav/books/alice/`
//! with one address book, `default/`. It answers `PROPFIND`, `REPORT` (`sync-collection` with
//! tokens `tok-N`, `addressbook-multiget`), conditional `PUT` and `DELETE`, and checks HTTP Basic.
//! Every request is recorded (method, path, headers without the credential).

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

    /// Removes a card (as another client would).
    pub fn remove_card(&self, name: &str) {
        let mut s = self.st();
        let path = format!("{BOOK}{name}");
        bump(&mut s, &path, None);
    }

    /// The cards on the server: path and text.
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
        });
        if !authorized(&s, authorization.as_deref()) {
            (401, Vec::new(), String::new())
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
        r#"<?xml version="1.0" encoding="utf-8"?><d:multistatus xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:carddav" xmlns:cs="http://calendarserver.org/ns/">{inner}</d:multistatus>"#
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
        ("PROPFIND", "/.well-known/carddav") if s.well_known => (
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
        ("PROPFIND", BOOK) => {
            let mut inner = ok(BOOK, "<d:resourcetype><d:collection/><c:addressbook/></d:resourcetype>");
            for (p, (etag, _)) in &s.cards {
                inner.push_str(&ok(p, &format!("<d:getetag>{}</d:getetag><d:resourcetype/>", xml_escape(etag))));
            }
            (207, Vec::new(), ms(&inner))
        }
        ("REPORT", BOOK) if body.contains("sync-collection") => {
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
                if *n > since {
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
        ("REPORT", BOOK) if body.contains("addressbook-multiget") => {
            let mut inner = String::new();
            for href in between(body, "<d:href>", "</d:href>") {
                if let Some((etag, text)) = s.cards.get(href) {
                    inner.push_str(&ok(
                        href,
                        &format!(
                            "<d:getetag>{}</d:getetag><c:address-data>{}</c:address-data>",
                            xml_escape(etag),
                            xml_escape(text)
                        ),
                    ));
                }
            }
            (207, Vec::new(), ms(&inner))
        }
        ("PUT", p) if p.starts_with(BOOK) && p.len() > BOOK.len() => {
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
