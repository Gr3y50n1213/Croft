// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! A small CardDAV client (RFC 6352) for an account's address books (T38a), on the core's own
//! HTTPS client: the same certificate rules and pin as the account's mail, the process's address
//! policy, no redirects followed (except `/.well-known/carddav` once, on the same origin), no
//! cookies, bounded answers. The credential is the account's own (HTTP Basic with the password, or
//! the OAuth token) and is only ever sent to the origin the account's settings name.
//!
//! What it does: find the address books ([`CardDav::discover`]), list what changed
//! ([`CardDav::changes`]: `sync-collection` with a token, else every `getetag`), fetch cards
//! ([`CardDav::fetch`]: `addressbook-multiget`), and write ([`CardDav::put`], [`CardDav::delete`],
//! conditional on the etag). Nothing about a contact is logged.

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

/// The most hrefs asked for in one `addressbook-multiget`.
pub const MULTIGET_BATCH: usize = 50;
/// The largest answer read for a listing or a multiget.
const MAX_ANSWER: usize = 16 * 1024 * 1024;
/// The largest card sent.
const MAX_CARD: usize = 256 * 1024;

/// Where and how to reach an account's CardDAV service.
#[derive(Debug, Clone)]
pub struct CardDavConfig {
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

/// An address book.
#[derive(Clone, PartialEq, Eq)]
pub struct AddressBook {
    /// Its path on the server (`/dav/card/alice/default/`).
    pub path: String,
    /// Its name, as the server gives it (else the last part of the path).
    pub name: String,
    /// The `sync-token` (RFC 6578), when the server offers one.
    pub sync_token: Option<String>,
    /// The `getctag`, when the server offers one (it changes with any change in the book).
    pub ctag: Option<String>,
}

/// One card's address and version.
#[derive(Clone, PartialEq, Eq)]
pub struct CardRef {
    /// Its path on the server.
    pub path: String,
    /// Its etag.
    pub etag: String,
}

/// What changed in a book.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct Changes {
    /// Cards added or changed.
    pub changed: Vec<CardRef>,
    /// Paths of cards removed (empty for a full listing: the caller compares).
    pub removed: Vec<String>,
    /// The new sync token, when the server gave one.
    pub sync_token: Option<String>,
    /// True when this is a full listing (no token, or the server refused it): every card is in
    /// `changed`, and a stored card not in it is gone.
    pub full: bool,
}

/// A card with its address and version.
#[derive(Clone, PartialEq, Eq)]
pub struct FetchedCard {
    /// Its path.
    pub path: String,
    /// Its etag.
    pub etag: String,
    /// The vCard text.
    pub text: String,
}

impl std::fmt::Debug for FetchedCard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FetchedCard")
            .field("len", &self.text.len())
            .finish_non_exhaustive()
    }
}

/// What a conditional write found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WriteOutcome {
    /// Written. The new etag comes with the next listing.
    Done,
    /// The card changed on the server since its etag (412): nothing was written.
    Conflict,
    /// The card is not there any more (404 on an update or delete).
    Gone,
}

/// An account's CardDAV client.
pub struct CardDav {
    http: Http,
    origin: Url,
    start: Url,
    authorization: Zeroizing<String>,
    /// `basic` or `bearer`: the scheme the server must ask for before the credential is sent.
    scheme: &'static str,
    /// The server asked for the credential (a 401 naming [`CardDav::scheme`]): it goes with every
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
const PROPFIND_HOME: &str = r#"<?xml version="1.0" encoding="utf-8"?><d:propfind xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:carddav"><d:prop><c:addressbook-home-set/></d:prop></d:propfind>"#;
const PROPFIND_BOOKS: &str = r#"<?xml version="1.0" encoding="utf-8"?><d:propfind xmlns:d="DAV:" xmlns:cs="http://calendarserver.org/ns/"><d:prop><d:resourcetype/><d:displayname/><d:sync-token/><cs:getctag/></d:prop></d:propfind>"#;
const PROPFIND_ETAGS: &str = r#"<?xml version="1.0" encoding="utf-8"?><d:propfind xmlns:d="DAV:"><d:prop><d:getetag/><d:resourcetype/></d:prop></d:propfind>"#;

impl CardDav {
    /// A client for `config` with `credential`. The credential becomes the `Authorization` value
    /// once and is kept in no other form.
    pub fn new(config: &CardDavConfig, credential: &Credential) -> Result<Self, BackendError> {
        let bad = || BackendError::Permanent("the CardDAV address is not usable".into());
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
            401 => Err(BackendError::Auth(
                "the CardDAV server refused the sign-in".into(),
            )),
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
        multistatus(&answer)
    }

    /// Finds the address books: `/.well-known/carddav` (one redirect on the same origin), the
    /// principal, its `addressbook-home-set`, and the address books in it. An empty list when the
    /// server offers no CardDAV.
    pub async fn discover(&self) -> Result<Vec<AddressBook>, BackendError> {
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
        let home = match self.propfind(&principal, "0", PROPFIND_HOME).await {
            Ok(root) => first_prop_href(&root, "addressbook-home-set"),
            Err(BackendError::Permanent(_)) => None,
            Err(e) => return Err(e),
        };
        let Some(home) = home else {
            return Ok(Vec::new());
        };
        let root = self.propfind(&home, "1", PROPFIND_BOOKS).await?;
        let mut books = Vec::new();
        for r in root.all("response") {
            let Some(path) = self.href_path(r) else {
                continue;
            };
            let Some(prop) = ok_prop(r) else { continue };
            let is_book = prop
                .child("resourcetype")
                .is_some_and(|t| t.child("addressbook").is_some());
            if !is_book {
                continue;
            }
            let name = prop
                .text_of("displayname")
                .filter(|n| !n.is_empty())
                .unwrap_or_else(|| last_segment(&path));
            books.push(AddressBook {
                sync_token: prop.text_of("sync-token").filter(|t| !t.is_empty()),
                ctag: prop.text_of("getctag").filter(|t| !t.is_empty()),
                path,
                name,
            });
        }
        Ok(books)
    }

    /// Where to start looking: the configured path when it is not the root, else
    /// `/.well-known/carddav` and its one redirect on the same origin, else `/`.
    async fn context_path(&self) -> Result<Option<String>, BackendError> {
        let configured = self.start.path_and_query.clone();
        if configured != "/" {
            return Ok(Some(configured));
        }
        let answer = self
            .request(
                "PROPFIND",
                "/.well-known/carddav",
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
            Ok(a) if a.status == 207 => Ok(Some("/.well-known/carddav".to_owned())),
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
                let root = multistatus(&answer)?;
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
                        out.changed.push(CardRef { path, etag });
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
                out.changed.push(CardRef { path, etag });
            }
        }
        Ok(out)
    }

    /// Fetches the cards at `paths` in `book` (`addressbook-multiget`), at most [`MULTIGET_BATCH`]
    /// in one call, so the caller stores each batch before asking for the next. A card the server
    /// does not return is left out; one it was not asked for is ignored.
    pub async fn fetch(
        &self,
        book: &str,
        paths: &[String],
    ) -> Result<Vec<FetchedCard>, BackendError> {
        if paths.len() > MULTIGET_BATCH {
            return Err(BackendError::Permanent(
                "too many cards in one request".into(),
            ));
        }
        let mut out = Vec::new();
        for batch in paths.chunks(MULTIGET_BATCH) {
            let mut body = String::from(
                r#"<?xml version="1.0" encoding="utf-8"?><c:addressbook-multiget xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:carddav"><d:prop><d:getetag/><c:address-data/></d:prop>"#,
            );
            for p in batch {
                // Only paths on this origin are ever asked for.
                let url = self.url(p)?;
                body.push_str("<d:href>");
                body.push_str(&xml::escape(&url.path_and_query));
                body.push_str("</d:href>");
            }
            body.push_str("</c:addressbook-multiget>");
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
            let root = multistatus(&answer)?;
            for r in root.all("response") {
                let Some(path) = self.href_path(r) else {
                    continue;
                };
                // Only the cards asked for, each once.
                if !batch.iter().any(|p| path_eq(p, &path))
                    || out.iter().any(|c: &FetchedCard| c.path == path)
                {
                    continue;
                }
                let Some(prop) = ok_prop(r) else { continue };
                if let (Some(etag), Some(text)) = (
                    prop.text_of("getetag").filter(|e| usable_etag(e)),
                    prop.text_of("address-data").filter(|t| t.len() <= MAX_CARD),
                ) {
                    out.push(FetchedCard { path, etag, text });
                }
            }
        }
        Ok(out)
    }

    /// Writes `card` at `path`: an update when `etag` is given (`If-Match`), else a new card
    /// (`If-None-Match: *`).
    pub async fn put(
        &self,
        path: &str,
        card: &str,
        etag: Option<&str>,
    ) -> Result<WriteOutcome, BackendError> {
        if card.len() > MAX_CARD {
            return Err(BackendError::Permanent("the card is too large".into()));
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
                Some("text/vcard; charset=utf-8"),
                card.as_bytes(),
                &[condition],
            )
            .await?;
        write_outcome(&answer)
    }

    /// Deletes the card at `path` if it still has `etag`.
    pub async fn delete(&self, path: &str, etag: &str) -> Result<WriteOutcome, BackendError> {
        if !usable_etag(etag) {
            return Err(BackendError::Permanent("the etag cannot be used".into()));
        }
        let answer = self
            .request("DELETE", path, None, None, &[], &[("If-Match", etag)])
            .await?;
        write_outcome(&answer)
    }

    /// The path of a `response`'s `href`, on this origin only (`None` otherwise).
    fn href_path(&self, response: &Node) -> Option<String> {
        let href = response.text_of("href")?;
        self.url(&href).ok().map(|u| u.path_and_query)
    }
}

fn write_outcome(answer: &HttpResponse) -> Result<WriteOutcome, BackendError> {
    match answer.status {
        200..=299 => Ok(WriteOutcome::Done),
        412 => Ok(WriteOutcome::Conflict),
        404 | 410 => Ok(WriteOutcome::Gone),
        500..=599 => Err(BackendError::Transient("the CardDAV server failed".into())),
        _ => Err(BackendError::Permanent(
            "the CardDAV server refused the change".into(),
        )),
    }
}

/// The parsed 207 answer; anything else is a refusal.
fn multistatus(answer: &HttpResponse) -> Result<Node, BackendError> {
    if answer.status != 207 {
        return Err(match answer.status {
            500..=599 => BackendError::Transient("the CardDAV server failed".into()),
            _ => BackendError::Permanent("the server does not answer as CardDAV".into()),
        });
    }
    let text = std::str::from_utf8(&answer.body)
        .map_err(|_| BackendError::Permanent("the CardDAV answer is not text".into()))?;
    let root = xml::parse(text)
        .map_err(|_| BackendError::Permanent("the CardDAV answer is not readable".into()))?;
    if root.name != "multistatus" {
        return Err(BackendError::Permanent(
            "the CardDAV answer is not readable".into(),
        ));
    }
    Ok(root)
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

impl std::fmt::Debug for AddressBook {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Contact data stays out of logs and error output.
        f.debug_struct("AddressBook").finish_non_exhaustive()
    }
}

impl std::fmt::Debug for CardRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Contact data stays out of logs and error output.
        f.debug_struct("CardRef").finish_non_exhaustive()
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
        let dav = CardDav::new(
            &CardDavConfig {
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
}
