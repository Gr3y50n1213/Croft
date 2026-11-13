// This Source Code Form is subject to the terms of the GNU Affero General Public License,
// version 3 or later. See LICENSE-AGPL-3.0 at the repository root.

//! Search (T33b), a part of the JMAP API ([`super`]), under the capability [`SEARCH`]: a Croft
//! extension over the core's search, so the web app finds what the phone finds, with the same
//! query language (`from:`, `to:`, `subject:`, `in:`, `has:attachment`, `is:unread`, `before:`,
//! `after:`, words and phrases). The index is the core's own, inside the user's encrypted mail
//! database (DECISIONS: not tantivy). Queries are never logged or stored.
//!
//! - `CroftSearch/query` `{accountId, query, server?, limit?}`: the stored mail first (headers and
//!   downloaded bodies); with `server: true`, the account's mail server too (limited per user).
//!   Answers the email ids (`m<id>`), best first, and for each the matched parts of its subject and
//!   a body snippet (ranges in UTF-16 code units, as JavaScript counts), with what kept the search
//!   from being complete.

use std::time::Duration;

use croft_ffi::{SearchFailure, SearchHit, TextRange};
use serde_json::{json, Map, Value};

use super::{error, Ctx};

/// The capability of this method.
pub const SEARCH: &str = "urn:croft:params:jmap:search";

/// The longest query, in characters.
pub const MAX_QUERY_CHARS: usize = 512;
/// The most hits one call answers (and the default).
pub const MAX_HITS: u32 = 100;
const DEFAULT_HITS: u32 = 50;
/// How long the mail server is given.
const SERVER_TIMEOUT: Duration = Duration::from_secs(30);

fn bad(what: &str) -> Value {
    json!({ "type": "invalidArguments", "description": what })
}

fn ranges(r: &[TextRange]) -> Value {
    Value::Array(r.iter().map(|t| json!([t.start, t.end])).collect())
}

fn hit_json(h: &SearchHit) -> Value {
    json!({
        "subjectRanges": ranges(&h.subject_ranges),
        "snippet": h.snippet,
        "snippetRanges": ranges(&h.snippet_ranges),
    })
}

fn search_error(e: &SearchFailure) -> Value {
    match e {
        SearchFailure::BadDate => bad("query"),
        SearchFailure::NoSuchAccount => error("accountNotFound"),
        SearchFailure::Storage { .. } | SearchFailure::Internal => error("serverFail"),
    }
}

impl Ctx {
    /// `CroftSearch/query`
    pub(super) async fn search_query(&self, args: &Value) -> Result<Value, Value> {
        let account = self.account(args)?;
        let query = args
            .get("query")
            .and_then(Value::as_str)
            .ok_or_else(|| bad("query"))?
            .to_owned();
        if query.chars().count() > MAX_QUERY_CHARS {
            return Err(bad("query"));
        }
        let limit = match args.get("limit") {
            None | Some(Value::Null) => DEFAULT_HITS,
            Some(v) => v
                .as_u64()
                .filter(|n| (1..=u64::from(MAX_HITS)).contains(n))
                .and_then(|n| u32::try_from(n).ok())
                .ok_or_else(|| bad("limit"))?,
        };
        let server = match args.get("server") {
            None | Some(Value::Null) => false,
            Some(v) => v.as_bool().ok_or_else(|| bad("server"))?,
        };
        if server && !(self.server_search_allowed)() {
            return Err(error("rateLimit"));
        }

        let q = query.clone();
        let local = self
            .blocking(move |c| c.search_local(Some(account), q, limit))
            .await?
            .map_err(|e| search_error(&e))?;
        let mut hits: Vec<SearchHit> = local.hits;
        let mut out = Map::new();
        out.insert("accountId".into(), json!(format!("a{account}")));
        out.insert("bodiesPartial".into(), json!(local.bodies_partial));
        out.insert("serverSearched".into(), json!(server));
        let (mut unsupported, mut offline, mut sign_in, mut failed) = (false, false, false, false);
        if server {
            let core = self.core.clone();
            match tokio::time::timeout(SERVER_TIMEOUT, core.search_server(Some(account), query))
                .await
            {
                Ok(Ok(found)) => {
                    unsupported = found.unsupported;
                    offline = found.offline;
                    sign_in = found.sign_in_needed;
                    failed = found.failed;
                    for h in found.hits {
                        if !hits.iter().any(|x| x.row.id == h.row.id) {
                            hits.push(h);
                        }
                    }
                }
                Ok(Err(e)) => return Err(search_error(&e)),
                Err(_) => failed = true,
            }
        }
        hits.truncate(usize::try_from(limit).unwrap_or(usize::MAX));
        out.insert("unsupported".into(), json!(unsupported));
        out.insert("offline".into(), json!(offline));
        out.insert("signInNeeded".into(), json!(sign_in));
        out.insert("failed".into(), json!(failed));
        out.insert(
            "ids".into(),
            Value::Array(
                hits.iter()
                    .map(|h| json!(format!("m{}", h.row.id)))
                    .collect(),
            ),
        );
        out.insert(
            "hits".into(),
            Value::Object(
                hits.iter()
                    .map(|h| (format!("m{}", h.row.id), hit_json(h)))
                    .collect(),
            ),
        );
        Ok(Value::Object(out))
    }
}
