// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The JMAP session resource (RFC 8620 section 2): what the server offers and where to send
//! requests.
//!
//! The addresses in the answer (`apiUrl`, `uploadUrl`) are used **for their path only**, on the
//! origin of the session URL the person set up. A server behind a reverse proxy often advertises
//! its internal name, and a hostile one could advertise somebody else's host; either way the
//! credential only ever goes to the origin it was given for. (A provider whose API really lives
//! on another host has to put that host in the session URL.)

use super::error::malformed;
use super::url::{percent_encode, Url};
use crate::types::BackendError;
use serde::Deserialize;
use std::collections::HashMap;

/// `urn:ietf:params:jmap:core`.
pub(crate) const CAP_CORE: &str = "urn:ietf:params:jmap:core";
/// `urn:ietf:params:jmap:mail`.
pub(crate) const CAP_MAIL: &str = "urn:ietf:params:jmap:mail";
/// `urn:ietf:params:jmap:submission`.
pub(crate) const CAP_SUBMISSION: &str = "urn:ietf:params:jmap:submission";

/// Largest `Email/get` or `Email/set` batch Croft sends when the server does not say.
const DEFAULT_OBJECTS: usize = 500;
/// Cap on batches whatever the server allows: keeps one answer a manageable size.
const MAX_OBJECTS: usize = 500;

#[derive(Deserialize)]
struct RawSession {
    #[serde(default)]
    capabilities: HashMap<String, serde_json::Value>,
    #[serde(default, rename = "primaryAccounts")]
    primary_accounts: HashMap<String, String>,
    #[serde(default)]
    accounts: HashMap<String, serde_json::Value>,
    #[serde(rename = "apiUrl")]
    api_url: String,
    #[serde(default, rename = "uploadUrl")]
    upload_url: Option<String>,
    #[serde(default)]
    state: Option<String>,
}

/// What the session says the server offers, reduced to what the adapter uses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Session {
    /// The account that holds the mail.
    pub account_id: String,
    /// Where requests go (on the session URL's origin).
    pub api_url: Url,
    /// The upload address with `{accountId}` still in it, on the session URL's origin.
    upload_template: Option<Url>,
    /// `maxObjectsInGet`, capped.
    pub max_get: usize,
    /// `maxObjectsInSet`, capped.
    pub max_set: usize,
    /// `maxCallsInRequest`.
    pub max_calls: usize,
    /// `maxSizeUpload`, when stated.
    pub max_upload: Option<u64>,
    /// Whether the server offers `urn:ietf:params:jmap:submission`.
    pub has_submission: bool,
    /// The session state, to notice when the session changed (a request's `sessionState`).
    pub state: Option<String>,
}

impl Session {
    /// Reads the session resource. `session_url` is the address it was fetched from (after
    /// same-origin redirects).
    pub fn parse(body: &[u8], session_url: &Url) -> Result<Self, BackendError> {
        let raw: RawSession = serde_json::from_slice(body).map_err(|_| malformed())?;
        if !raw.capabilities.contains_key(CAP_CORE) || !raw.capabilities.contains_key(CAP_MAIL) {
            return Err(BackendError::Permanent(
                "the server does not offer JMAP mail".into(),
            ));
        }
        let account_id = match raw.primary_accounts.get(CAP_MAIL) {
            Some(id) => id.clone(),
            None => {
                let mut with_mail = raw.accounts.iter().filter(|(_, a)| {
                    a.get("accountCapabilities")
                        .and_then(|c| c.get(CAP_MAIL))
                        .is_some()
                });
                match (with_mail.next(), with_mail.next()) {
                    (Some((id, _)), None) => id.clone(),
                    _ => {
                        return Err(BackendError::Permanent(
                            "the server names no mail account".into(),
                        ))
                    }
                }
            }
        };
        let api_url = session_url
            .resolve(&raw.api_url)
            .ok_or_else(malformed)?
            .on_origin_of(session_url);
        let upload_template = match raw.upload_url {
            Some(u) => {
                // `{` and `}` are not valid in a `Url`, so the template is parsed with the
                // account id already in it and the id is swapped back for each use.
                let with_id = u.replace("{accountId}", &percent_encode(&account_id));
                if with_id == u {
                    None
                } else {
                    Some(
                        session_url
                            .resolve(&with_id)
                            .ok_or_else(malformed)?
                            .on_origin_of(session_url),
                    )
                }
            }
            None => None,
        };
        let core = raw.capabilities.get(CAP_CORE);
        let limit = |name: &str, default: usize| {
            core.and_then(|c| c.get(name))
                .and_then(serde_json::Value::as_u64)
                .map_or(default, |n| n as usize)
                .max(1)
        };
        Ok(Self {
            account_id,
            api_url,
            upload_template,
            max_get: limit("maxObjectsInGet", DEFAULT_OBJECTS).min(MAX_OBJECTS),
            max_set: limit("maxObjectsInSet", DEFAULT_OBJECTS).min(MAX_OBJECTS),
            max_calls: limit("maxCallsInRequest", 1),
            max_upload: core
                .and_then(|c| c.get("maxSizeUpload"))
                .and_then(serde_json::Value::as_u64),
            has_submission: raw.capabilities.contains_key(CAP_SUBMISSION),
            state: raw.state,
        })
    }

    /// The address blobs are uploaded to, if the server offers uploads.
    pub fn upload_url(&self) -> Option<&Url> {
        self.upload_template.as_ref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(url: &str) -> Url {
        Url::parse(url).unwrap()
    }

    #[test]
    fn reads_a_recorded_stalwart_session_and_keeps_to_the_session_origin() {
        let body = include_bytes!("fixtures/stalwart_session.json");
        // The server advertises https://localhost/..., the person set up http://127.0.0.1:18080.
        let origin = at("http://127.0.0.1:18080/jmap/session");
        let s = Session::parse(body, &origin).unwrap();
        assert_eq!(s.account_id, "c");
        assert!(s.api_url.same_origin(&origin));
        assert_eq!(s.api_url.path_and_query, "/jmap/");
        let up = s.upload_url().unwrap();
        assert!(up.same_origin(&origin));
        assert_eq!(up.path_and_query, "/jmap/upload/c/");
        assert_eq!((s.max_get, s.max_set, s.max_calls), (500, 500, 16));
        assert!(s.has_submission);
    }

    #[test]
    fn relative_addresses_and_a_foreign_host_are_resolved_against_the_session_url() {
        let body = br#"{"capabilities":{"urn:ietf:params:jmap:core":{"maxObjectsInGet":50},
            "urn:ietf:params:jmap:mail":{}},"primaryAccounts":{"urn:ietf:params:jmap:mail":"u1"},
            "apiUrl":"/jmap/api/","uploadUrl":"https://evil.example.net/up/{accountId}/","state":"s1"}"#;
        let origin = at("https://api.example.org/jmap/session");
        let s = Session::parse(body, &origin).unwrap();
        assert_eq!(s.api_url, at("https://api.example.org/jmap/api/"));
        assert_eq!(
            s.upload_url().unwrap(),
            &at("https://api.example.org/up/u1/")
        );
        assert_eq!(s.max_get, 50);
        assert!(!s.has_submission);
        assert_eq!(s.state.as_deref(), Some("s1"));
    }

    #[test]
    fn the_mail_account_is_found_without_primary_accounts_when_there_is_only_one() {
        let body =
            br#"{"capabilities":{"urn:ietf:params:jmap:core":{},"urn:ietf:params:jmap:mail":{}},
            "accounts":{"a1":{"accountCapabilities":{"urn:ietf:params:jmap:mail":{}}},
                        "a2":{"accountCapabilities":{}}},"apiUrl":"https://x.org/api"}"#;
        let s = Session::parse(body, &at("https://x.org/s")).unwrap();
        assert_eq!(s.account_id, "a1");
    }

    #[test]
    fn a_server_without_mail_or_with_junk_is_refused() {
        let origin = at("https://x.org/s");
        let no_mail = br#"{"capabilities":{"urn:ietf:params:jmap:core":{}},"apiUrl":"/a"}"#;
        assert!(matches!(
            Session::parse(no_mail, &origin),
            Err(BackendError::Permanent(_))
        ));
        for junk in [&b"not json"[..], b"{}", b"[]", b""] {
            assert!(matches!(
                Session::parse(junk, &origin),
                Err(BackendError::Permanent(_))
            ));
        }
    }
}
