// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! A small DAV client for an account's address books (CardDAV, RFC 6352; T38a) and calendars
//! (CalDAV, RFC 4791; T39a), on the core's own HTTPS client: the same certificate rules and pin as
//! the account's mail, the process's address policy, no redirects followed (except the
//! `/.well-known/` address once, on the same origin), no cookies, bounded answers. The credential
//! is the account's own (HTTP Basic with the password, or the OAuth token), is only ever sent to
//! the origin the account's settings name, and only after the server asked for it (a 401 naming
//! its scheme).
//!
//! What it does: find the collections ([`Dav::discover`]), list what changed ([`Dav::changes`]:
//! `sync-collection` with a token, else every `getetag`), fetch objects ([`Dav::fetch`]: the
//! kind's multiget), and write ([`Dav::put`], [`Dav::delete`], conditional on the etag). Nothing
//! about a contact or an event is logged.

pub(crate) mod xml;

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use zeroize::Zeroizing;

use crate::imap::{CertFingerprint, Credential};
use crate::jmap::http::{Http, HttpRequest, HttpResponse};
use crate::jmap::tls::Trust;
use crate::jmap::url::Url;
use crate::types::BackendError;
use xml::Node;

/// The longest collection name kept.
const MAX_NAME_CHARS: usize = 200;
/// The most hrefs asked for in one multiget.
pub const MULTIGET_BATCH: usize = 50;
/// The largest answer read for a listing or a multiget.
const MAX_ANSWER: usize = 16 * 1024 * 1024;

/// Which DAV service: address books or calendars.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DavKind {
    /// CardDAV.
    Card,
    /// CalDAV.
    Cal,
}

impl DavKind {
    fn label(self) -> &'static str {
        match self {
            DavKind::Card => "CardDAV",
            DavKind::Cal => "CalDAV",
        }
    }

    fn well_known(self) -> &'static str {
        match self {
            DavKind::Card => "/.well-known/carddav",
            DavKind::Cal => "/.well-known/caldav",
        }
    }

    fn home_set(self) -> &'static str {
        match self {
            DavKind::Card => "addressbook-home-set",
            DavKind::Cal => "calendar-home-set",
        }
    }

    fn collection(self) -> &'static str {
        match self {
            DavKind::Card => "addressbook",
            DavKind::Cal => "calendar",
        }
    }

    fn data(self) -> &'static str {
        match self {
            DavKind::Card => "address-data",
            DavKind::Cal => "calendar-data",
        }
    }

    fn content_type(self) -> &'static str {
        match self {
            DavKind::Card => "text/vcard; charset=utf-8",
            DavKind::Cal => "text/calendar; charset=utf-8",
        }
    }

    /// The largest object read or sent: a card, or a calendar object (an event with its overrides).
    pub fn max_object(self) -> usize {
        match self {
            DavKind::Card => 256 * 1024,
            DavKind::Cal => 1024 * 1024,
        }
    }

    fn propfind_home(self) -> String {
        format!(
            r#"<?xml version="1.0" encoding="utf-8"?><d:propfind xmlns:d="DAV:" xmlns:c="{}"><d:prop><c:{}/></d:prop></d:propfind>"#,
            self.namespace(),
            self.home_set()
        )
    }

    fn propfind_collections(self) -> &'static str {
        match self {
            DavKind::Card => PROPFIND_BOOKS,
            DavKind::Cal => PROPFIND_CALENDARS,
        }
    }

    fn namespace(self) -> &'static str {
        match self {
            DavKind::Card => "urn:ietf:params:xml:ns:carddav",
            DavKind::Cal => "urn:ietf:params:xml:ns:caldav",
        }
    }

    fn multiget(self, paths: &[String]) -> String {
        let (element, data) = match self {
            DavKind::Card => ("addressbook-multiget", "address-data"),
            DavKind::Cal => ("calendar-multiget", "calendar-data"),
        };
        let mut body = format!(
            r#"<?xml version="1.0" encoding="utf-8"?><c:{element} xmlns:d="DAV:" xmlns:c="{}"><d:prop><d:getetag/><c:{data}/></d:prop>"#,
            self.namespace()
        );
        for p in paths {
            body.push_str("<d:href>");
            body.push_str(&xml::escape(p));
            body.push_str("</d:href>");
        }
        body.push_str(&format!("</c:{element}>"));
        body
    }
}

/// Where and how to reach an account's DAV service. `Debug` leaves the user name out (often an
/// address).
#[derive(Clone)]
pub struct DavConfig {
    /// The service's origin, `https://host[:port]` (the account's mail host by default), or a
    /// full URL to start from.
    pub base_url: String,
    /// The user name for HTTP Basic.
    pub username: String,
    /// The account's pinned certificate, if it has one.
    pub pinned_cert: Option<CertFingerprint>,
    /// Each wait for the server.
    pub timeout: Duration,
    /// One whole request.
    pub deadline: Duration,
}

/// An address book or a calendar.
#[derive(Clone, PartialEq, Eq)]
pub struct Collection {
    /// Its path on the server (`/dav/card/alice/default/`).
    pub path: String,
    /// Its name, as the server gives it (else the last part of the path).
    pub name: String,
    /// The `sync-token` (RFC 6578), when the server offers one.
    pub sync_token: Option<String>,
    /// The `getctag`, when the server offers one (it changes with any change in the book).
    pub ctag: Option<String>,
    /// A calendar's colour (Apple's `calendar-color`, `#RRGGBB` or `#RRGGBBAA`), when given and valid.
    pub color: Option<String>,
    /// A calendar's place in the list (Apple's `calendar-order`), when given.
    pub order: Option<i32>,
}

/// One object's address and version.
#[derive(Clone, PartialEq, Eq)]
pub struct ItemRef {
    /// Its path on the server.
    pub path: String,
    /// Its etag.
    pub etag: String,
}

/// What changed in a book.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct Changes {
    /// Objects added or changed.
    pub changed: Vec<ItemRef>,
    /// Paths of cards removed (empty for a full listing: the caller compares).
    pub removed: Vec<String>,
    /// The new sync token, when the server gave one.
    pub sync_token: Option<String>,
    /// True when this is a full listing (no token, or the server refused it): every card is in
    /// `changed`, and a stored card not in it is gone.
    pub full: bool,
}

/// An object (a vCard or an iCalendar object) with its address and version.
#[derive(Clone, PartialEq, Eq)]
pub struct Fetched {
    /// Its path.
    pub path: String,
    /// Its etag.
    pub etag: String,
    /// The object's text.
    pub text: String,
}

impl std::fmt::Debug for Fetched {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Fetched")
            .field("len", &self.text.len())
            .finish_non_exhaustive()
    }
}

/// What a conditional write found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WriteOutcome {
    /// Written. The new etag comes with the next listing.
    Done,
    /// The object changed on the server since its etag (412): nothing was written.
    Conflict,
    /// The object is not there any more (404 on an update or delete).
    Gone,
}

/// An account's DAV client for one kind of collection.
pub struct Dav {
    kind: DavKind,
    http: Http,
    origin: Url,
    start: Url,
    authorization: Zeroizing<String>,
    /// `basic` or `bearer`: the scheme the server must ask for before the credential is sent.
    scheme: &'static str,
    /// The server asked for the credential (a 401 naming [`Dav::scheme`]): it goes with every
    /// request from then on. Until then requests carry none (T38a review: the mail password never
    /// goes to a CardDAV address that did not ask for it).
    challenged: AtomicBool,
}

/// The longest etag sent back in a condition.
const MAX_ETAG: usize = 256;

/// An etag that may go into a header: visible ASCII, bounded.
fn usable_etag(etag: &str) -> bool {
    !etag.is_empty() && etag.len() <= MAX_ETAG && etag.bytes().all(|b| (0x21..=0x7e).contains(&b))
}

const PROPFIND_PRINCIPAL: &str = r#"<?xml version="1.0" encoding="utf-8"?><d:propfind xmlns:d="DAV:"><d:prop><d:current-user-principal/></d:prop></d:propfind>"#;
const PROPFIND_BOOKS: &str = r#"<?xml version="1.0" encoding="utf-8"?><d:propfind xmlns:d="DAV:" xmlns:cs="http://calendarserver.org/ns/"><d:prop><d:resourcetype/><d:displayname/><d:sync-token/><cs:getctag/></d:prop></d:propfind>"#;
const PROPFIND_CALENDARS: &str = r#"<?xml version="1.0" encoding="utf-8"?><d:propfind xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav" xmlns:cs="http://calendarserver.org/ns/" xmlns:a="http://apple.com/ns/ical/"><d:prop><d:resourcetype/><d:displayname/><d:sync-token/><cs:getctag/><c:supported-calendar-component-set/><a:calendar-color/><a:calendar-order/></d:prop></d:propfind>"#;
const PROPFIND_ETAGS: &str = r#"<?xml version="1.0" encoding="utf-8"?><d:propfind xmlns:d="DAV:"><d:prop><d:getetag/><d:resourcetype/></d:prop></d:propfind>"#;

impl Dav {
    /// A client of `kind` for `config` with `credential`. The credential becomes the
    /// `Authorization` value once and is kept in no other form.
    pub fn new(
        kind: DavKind,
        config: &DavConfig,
        credential: &Credential,
    ) -> Result<Self, BackendError> {
        let bad = || BackendError::Permanent(format!("the {} address is not usable", kind.label()));
        let start = Url::parse(config.base_url.trim()).ok_or_else(bad)?;
        if !start.is_allowed() {
            return Err(bad());
        }
        let scheme = match credential {
            Credential::Password(_) => "basic",
            Credential::OAuth2 { .. } => "bearer",
        };
        let authorization = match credential {
            Credential::Password(pw) => crate::jmap::api::basic_header(&config.username, pw)?,
            Credential::OAuth2 { access_token, .. } => {
                if access_token.contains(['\r', '\n']) {
                    return Err(BackendError::Permanent(
                        "the access token cannot be used".into(),
                    ));
                }
                let mut header = Zeroizing::new(String::with_capacity(7 + access_token.len()));
                header.push_str("Bearer ");
                header.push_str(access_token);
                header
            }
        };
        let trust = match config.pinned_cert {
            Some(pin) => Trust::Pinned(pin),
            None => Trust::Roots,
        };
        Ok(Self {
            kind,
            http: Http::new(trust, config.timeout, config.deadline),
            origin: start.clone(),
            start,
            authorization,
            scheme,
            challenged: AtomicBool::new(false),
        })
    }

    async fn request(
        &self,
        method: &'static str,
        path: &str,
        depth: Option<&str>,
        content_type: Option<&str>,
        body: &[u8],
        extra: &[(&str, &str)],
    ) -> Result<HttpResponse, BackendError> {
        let url = self.url(path)?;
        let mut headers: Vec<(&str, &str)> = Vec::with_capacity(extra.len() + 1);
        if let Some(d) = depth {
            headers.push(("Depth", d));
        }
        headers.extend_from_slice(extra);
        let send = |with_credential: bool| {
            let headers = &headers;
            let url = &url;
            async move {
                self.http
                    .send(&HttpRequest {
                        method,
                        url,
                        authorization: with_credential.then_some(self.authorization.as_str()),
                        content_type,
                        body,
                        max_response: MAX_ANSWER,
                        extra_time: Duration::ZERO,
                        error_body: false,
                        status_only: false,
                        extra_headers: headers,
                    })
                    .await
                    .map_err(|e| e.error)
            }
        };
        let challenged = self.challenged.load(Ordering::Relaxed);
        let mut answer = send(challenged).await?;
        if answer.status == 401
            && !challenged
            && answer.auth_schemes.iter().any(|s| s == self.scheme)
        {
            // The server asked for the credential, in the scheme it has: now it is sent.
            self.challenged.store(true, Ordering::Relaxed);
            answer = send(true).await?;
        }
        // Only 401 means the sign-in: a 403 is also how RFC 6578 refuses an old sync token.
        match answer.status {
            401 => Err(BackendError::Auth(format!(
                "the {} server refused the sign-in",
                self.kind.label()
            ))),
            _ => Ok(answer),
        }
    }

    /// A path on the account's own origin. A server-given address on another origin is refused:
    /// the credential never leaves the origin the settings name.
    fn url(&self, path: &str) -> Result<Url, BackendError> {
        let url = self.origin.resolve(path).ok_or_else(|| {
            BackendError::Permanent("the server named an unusable address".into())
        })?;
        if !url.same_origin(&self.origin) {
            return Err(BackendError::Permanent(
                "the server named an address on another server".into(),
            ));
        }
        Ok(url)
    }

    async fn propfind(&self, path: &str, depth: &str, body: &str) -> Result<Node, BackendError> {
        let answer = self
            .request(
                "PROPFIND",
                path,
                Some(depth),
                Some("application/xml; charset=utf-8"),
                body.as_bytes(),
                &[],
            )
            .await?;
        multistatus(&answer, self.kind)
    }

    /// Finds the collections: the kind's `/.well-known/` address (one redirect on the same
    /// origin), the principal, its home set, and the address books or calendars in it. An empty
    /// list when the server offers none. Calendars that cannot hold events are left out.
    pub async fn discover(&self) -> Result<Vec<Collection>, BackendError> {
        let context = self.context_path().await?;
        let Some(context) = context else {
            return Ok(Vec::new());
        };
        let principal = match self.propfind(&context, "0", PROPFIND_PRINCIPAL).await {
            Ok(root) => first_prop_href(&root, "current-user-principal"),
            Err(BackendError::Permanent(_)) => None,
            Err(e) => return Err(e),
        };
        let Some(principal) = principal else {
            return Ok(Vec::new());
        };
        let home = match self
            .propfind(&principal, "0", &self.kind.propfind_home())
            .await
        {
            Ok(root) => first_prop_href(&root, self.kind.home_set()),
            Err(BackendError::Permanent(_)) => None,
            Err(e) => return Err(e),
        };
        let Some(home) = home else {
            return Ok(Vec::new());
        };
        let root = self
            .propfind(&home, "1", self.kind.propfind_collections())
            .await?;
        let mut books = Vec::new();
        for r in root.all("response") {
            let Some(path) = self.href_path(r) else {
                continue;
            };
            let Some(prop) = ok_prop(r) else { continue };
            let is_collection = prop
                .child("resourcetype")
                .is_some_and(|t| t.child(self.kind.collection()).is_some());
            if !is_collection {
                continue;
            }
            // A calendar that names the components it holds must hold events.
            if let Some(set) = prop.child("supported-calendar-component-set") {
                let comps: Vec<&str> = set.all("comp").map(|c| c.name_attr.as_str()).collect();
                if !comps.is_empty() && !comps.iter().any(|c| c.eq_ignore_ascii_case("VEVENT")) {
                    continue;
                }
            }
            let name: String = prop
                .text_of("displayname")
                .filter(|n| !n.is_empty())
                .unwrap_or_else(|| last_segment(&path))
                .chars()
                .filter(|c| !c.is_control())
                .take(MAX_NAME_CHARS)
                .collect();
            books.push(Collection {
                sync_token: prop.text_of("sync-token").filter(|t| !t.is_empty()),
                ctag: prop.text_of("getctag").filter(|t| !t.is_empty()),
                color: prop.text_of("calendar-color").and_then(|c| color(&c)),
                order: prop
                    .text_of("calendar-order")
                    .and_then(|o| o.parse::<i32>().ok()),
                path,
                name,
            });
        }
        Ok(books)
    }

    /// Where to start looking: the configured path when it is not the root, else the kind's
    /// `/.well-known/` address and its one redirect on the same origin, else `/`.
    async fn context_path(&self) -> Result<Option<String>, BackendError> {
        let configured = self.start.path_and_query.clone();
        if configured != "/" {
            return Ok(Some(configured));
        }
        let answer = self
            .request(
                "PROPFIND",
                self.kind.well_known(),
                Some("0"),
                Some("application/xml; charset=utf-8"),
                PROPFIND_PRINCIPAL.as_bytes(),
                &[],
            )
            .await;
        match answer {
            Ok(a) if (300..400).contains(&a.status) => {
                let target = a
                    .location
                    .as_deref()
                    .and_then(|l| self.origin.resolve(l))
                    .filter(|u| u.same_origin(&self.origin));
                Ok(Some(
                    target.map_or_else(|| "/".to_owned(), |u| u.path_and_query),
                ))
            }
            Ok(a) if a.status == 207 => Ok(Some(self.kind.well_known().to_owned())),
            Ok(_) => Ok(Some("/".to_owned())),
            Err(BackendError::Permanent(_)) => Ok(Some("/".to_owned())),
            Err(e) => Err(e),
        }
    }

    /// What changed in `book` since `token` (RFC 6578 `sync-collection`); with no token, or when
    /// the server refuses it, a full listing of every card's etag.
    pub async fn changes(&self, book: &str, token: Option<&str>) -> Result<Changes, BackendError> {
        if let Some(token) = token {
            let body = format!(
                r#"<?xml version="1.0" encoding="utf-8"?><d:sync-collection xmlns:d="DAV:"><d:sync-token>{}</d:sync-token><d:sync-level>1</d:sync-level><d:prop><d:getetag/></d:prop></d:sync-collection>"#,
                xml::escape(token)
            );
            let answer = self
                .request(
                    "REPORT",
                    book,
                    None,
                    Some("application/xml; charset=utf-8"),
                    body.as_bytes(),
                    &[],
                )
                .await?;
            if answer.status == 207 {
                let root = multistatus(&answer, self.kind)?;
                let mut out = Changes {
                    sync_token: root.text_of("sync-token").filter(|t| !t.is_empty()),
                    ..Changes::default()
                };
                for r in root.all("response") {
                    let Some(path) = self.href_path(r) else {
                        continue;
                    };
                    if path_eq(&path, book) {
                        continue;
                    }
                    let gone = r
                        .text_of("status")
                        .is_some_and(|s| status_code(&s) == Some(404));
                    if gone {
                        out.removed.push(path);
                    } else if let Some(etag) = ok_prop(r)
                        .and_then(|p| p.text_of("getetag"))
                        .filter(|e| usable_etag(e))
                    {
                        out.changed.push(ItemRef { path, etag });
                    }
                }
                return Ok(out);
            }
            // 403, 409 or 412 (`valid-sync-token`), or no sync support: a full listing.
        }
        let root = self.propfind(book, "1", PROPFIND_ETAGS).await?;
        let mut out = Changes {
            full: true,
            ..Changes::default()
        };
        for r in root.all("response") {
            let Some(path) = self.href_path(r) else {
                continue;
            };
            if path_eq(&path, book) {
                continue;
            }
            let Some(prop) = ok_prop(r) else { continue };
            if prop
                .child("resourcetype")
                .is_some_and(|t| t.child("collection").is_some())
            {
                continue;
            }
            if let Some(etag) = prop.text_of("getetag").filter(|e| usable_etag(e)) {
                out.changed.push(ItemRef { path, etag });
            }
        }
        Ok(out)
    }

    /// Fetches the objects at `paths` in `collection` (the kind's multiget), at most
    /// [`MULTIGET_BATCH`] in one call, so the caller stores each batch before asking for the next.
    /// An object the server does not return is left out; one it was not asked for is ignored.
    pub async fn fetch(&self, book: &str, paths: &[String]) -> Result<Vec<Fetched>, BackendError> {
        if paths.len() > MULTIGET_BATCH {
            return Err(BackendError::Permanent(
                "too many objects in one request".into(),
            ));
        }
        let mut out = Vec::new();
        for batch in paths.chunks(MULTIGET_BATCH) {
            // Only paths on this origin are ever asked for.
            let asked: Vec<String> = batch
                .iter()
                .map(|p| self.url(p).map(|u| u.path_and_query))
                .collect::<Result<_, _>>()?;
            let body = self.kind.multiget(&asked);
            let answer = self
                .request(
                    "REPORT",
                    book,
                    Some("1"),
                    Some("application/xml; charset=utf-8"),
                    body.as_bytes(),
                    &[],
                )
                .await?;
            let root = multistatus(&answer, self.kind)?;
            for r in root.all("response") {
                let Some(path) = self.href_path(r) else {
                    continue;
                };
                // Only the objects asked for, each once.
                if !batch.iter().any(|p| path_eq(p, &path))
                    || out.iter().any(|c: &Fetched| c.path == path)
                {
                    continue;
                }
                let Some(prop) = ok_prop(r) else { continue };
                if let (Some(etag), Some(text)) = (
                    prop.text_of("getetag").filter(|e| usable_etag(e)),
                    prop.text_of(self.kind.data())
                        .filter(|t| t.len() <= self.kind.max_object()),
                ) {
                    out.push(Fetched { path, etag, text });
                }
            }
        }
        Ok(out)
    }

    /// Writes `card` (a vCard or an iCalendar object) at `path`: an update when `etag` is given
    /// (`If-Match`), else a new object (`If-None-Match: *`).
    pub async fn put(
        &self,
        path: &str,
        card: &str,
        etag: Option<&str>,
    ) -> Result<WriteOutcome, BackendError> {
        self.put_tagged(path, card, etag).await.map(|w| w.outcome)
    }

    /// [`Dav::put`], with the etag of the version written when the server gave one, so the next
    /// change is conditional on exactly that version (T39a review: never on a newer one it did not see).
    pub async fn put_tagged(
        &self,
        path: &str,
        card: &str,
        etag: Option<&str>,
    ) -> Result<Written, BackendError> {
        if card.len() > self.kind.max_object() {
            return Err(BackendError::Permanent("the object is too large".into()));
        }
        let condition = match etag {
            Some(e) => ("If-Match", e),
            None => ("If-None-Match", "*"),
        };
        if etag.is_some_and(|e| !usable_etag(e)) {
            return Err(BackendError::Permanent("the etag cannot be used".into()));
        }
        let answer = self
            .request(
                "PUT",
                path,
                None,
                Some(self.kind.content_type()),
                card.as_bytes(),
                &[condition],
            )
            .await?;
        let outcome = write_outcome(&answer, self.kind)?;
        let etag = (outcome == WriteOutcome::Done)
            .then(|| answer.etag.clone())
            .flatten();
        Ok(Written { outcome, etag })
    }

    /// Deletes the object at `path` if it still has `etag`.
    pub async fn delete(&self, path: &str, etag: &str) -> Result<WriteOutcome, BackendError> {
        if !usable_etag(etag) {
            return Err(BackendError::Permanent("the etag cannot be used".into()));
        }
        let answer = self
            .request("DELETE", path, None, None, &[], &[("If-Match", etag)])
            .await?;
        write_outcome(&answer, self.kind)
    }

    /// The path of a `response`'s `href`, on this origin only (`None` otherwise).
    fn href_path(&self, response: &Node) -> Option<String> {
        let href = response.text_of("href")?;
        self.url(&href).ok().map(|u| u.path_and_query)
    }
}

fn write_outcome(answer: &HttpResponse, kind: DavKind) -> Result<WriteOutcome, BackendError> {
    let label = kind.label();
    match answer.status {
        200..=299 => Ok(WriteOutcome::Done),
        412 => Ok(WriteOutcome::Conflict),
        404 | 410 => Ok(WriteOutcome::Gone),
        500..=599 => Err(BackendError::Transient(format!(
            "the {label} server failed"
        ))),
        _ => Err(BackendError::Permanent(format!(
            "the {label} server refused the change"
        ))),
    }
}

/// The parsed 207 answer; anything else is a refusal.
fn multistatus(answer: &HttpResponse, kind: DavKind) -> Result<Node, BackendError> {
    let label = kind.label();
    if answer.status != 207 {
        return Err(match answer.status {
            500..=599 => BackendError::Transient(format!("the {label} server failed")),
            _ => BackendError::Permanent(format!("the server does not answer as {label}")),
        });
    }
    let unreadable = || BackendError::Permanent(format!("the {label} answer is not readable"));
    let text = std::str::from_utf8(&answer.body).map_err(|_| unreadable())?;
    let root = xml::parse(text).map_err(|_| unreadable())?;
    if root.name != "multistatus" {
        return Err(unreadable());
    }
    Ok(root)
}

/// A calendar colour as `#RRGGBB` (an alpha part is dropped); `None` for anything else.
fn color(text: &str) -> Option<String> {
    let t = text.trim();
    let hex = t.strip_prefix('#')?;
    if !(hex.len() == 6 || hex.len() == 8) || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    Some(format!("#{}", hex.get(..6)?.to_ascii_uppercase()))
}

/// The `prop` of a `response`'s `propstat` with a 2xx status.
fn ok_prop(response: &Node) -> Option<&Node> {
    response
        .all("propstat")
        .find(|ps| {
            ps.text_of("status")
                .and_then(|s| status_code(&s))
                .is_some_and(|c| (200..300).contains(&c))
        })
        .and_then(|ps| ps.child("prop"))
}

/// The `href` inside the property `name` of the first OK `response`.
fn first_prop_href(root: &Node, name: &str) -> Option<String> {
    root.all("response")
        .filter_map(ok_prop)
        .filter_map(|p| p.child(name))
        .find_map(|n| n.text_of("href"))
        .filter(|h| !h.is_empty())
}

/// `HTTP/1.1 404 Not Found` → 404.
fn status_code(line: &str) -> Option<u16> {
    line.split_whitespace().nth(1)?.parse().ok()
}

/// The last non-empty path segment.
fn last_segment(path: &str) -> String {
    path.trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or_default()
        .to_owned()
}

/// The same collection, with or without a trailing slash.
fn path_eq(a: &str, b: &str) -> bool {
    a.trim_end_matches('/') == b.trim_end_matches('/')
}

impl std::fmt::Debug for DavConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DavConfig")
            .field("pinned", &self.pinned_cert.is_some())
            .finish_non_exhaustive()
    }
}

/// What a conditional write found, and the version it made when the server said (its `ETag`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Written {
    /// The outcome.
    pub outcome: WriteOutcome,
    /// The new etag, when the server gave one with a successful write.
    pub etag: Option<String>,
}

impl std::fmt::Debug for Collection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Contact and calendar data stay out of logs and error output.
        f.debug_struct("Collection").finish_non_exhaustive()
    }
}

impl std::fmt::Debug for ItemRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Contact and calendar data stay out of logs and error output.
        f.debug_struct("ItemRef").finish_non_exhaustive()
    }
}

impl std::fmt::Debug for Changes {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Contact data stays out of logs and error output.
        f.debug_struct("Changes").finish_non_exhaustive()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn status_lines_paths_and_segments() {
        assert_eq!(status_code("HTTP/1.1 404 Not Found"), Some(404));
        assert_eq!(status_code("garbage"), None);
        assert_eq!(last_segment("/dav/card/alice/default/"), "default");
        assert!(path_eq("/a/b/", "/a/b"));
    }

    #[test]
    fn only_the_settings_origin_gets_requests() {
        let dav = Dav::new(
            DavKind::Card,
            &DavConfig {
                base_url: "https://mail.example.org".into(),
                username: "alice".into(),
                pinned_cert: None,
                timeout: Duration::from_secs(5),
                deadline: Duration::from_secs(10),
            },
            &Credential::Password(Zeroizing::new("made-up".into())),
        )
        .unwrap();
        assert_eq!(dav.url("/dav/card/").unwrap().path_and_query, "/dav/card/");
        assert!(dav.url("https://mail.example.org/x").is_ok());
        assert!(dav.url("https://evil.example/x").is_err());
        assert!(dav.url("http://mail.example.org/x").is_err());
        assert!(dav.url("https://mail.example.org:8443/x").is_err());
        let r = xml::parse(
            r#"<multistatus xmlns="DAV:"><response><href>https://evil.example/c.vcf</href></response></multistatus>"#,
        )
        .unwrap();
        assert_eq!(dav.href_path(r.child("response").unwrap()), None);
    }

    #[test]
    fn calendar_colours_are_read_or_dropped() {
        assert_eq!(color("#ff8800"), Some("#FF8800".into()));
        assert_eq!(color(" #FF8800FF "), Some("#FF8800".into()));
        assert_eq!(color("red"), None);
        assert_eq!(color("#ff88"), None);
        assert_eq!(color("#gg8800"), None);
    }

    #[test]
    fn the_multiget_names_the_kind_and_escapes_paths() {
        let card = DavKind::Card.multiget(&["/a&b.vcf".into()]);
        assert!(
            card.contains("addressbook-multiget")
                && card.contains("address-data")
                && card.contains("/a&amp;b.vcf")
        );
        let cal = DavKind::Cal.multiget(&["/c.ics".into()]);
        assert!(
            cal.contains("calendar-multiget")
                && cal.contains("calendar-data")
                && cal.contains("urn:ietf:params:xml:ns:caldav")
        );
    }
}
