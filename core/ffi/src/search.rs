// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Search for S27 and S28: the device's index first ([`Core::search_local`], instant), then the
//! server ([`Core::search_server`], merged in by message id).
//!
//! The query is parsed once by the core (`croft_mail_core::search`); a malformed date is the only
//! error the person can cause. Results are ordinary list rows ([`MessageRow`]) with the matched
//! parts of the subject and of a body snippet given as ranges in UTF-16 code units (what Kotlin
//! and Swift strings count). Queries are never logged and go only to each account's own server;
//! the sample mailbox searches on the device only.

use croft_engine::SearchError as EngineSearchError;
use croft_mail_core::search::{parse, QueryError, SearchQuery, TextTerm};
use croft_protocols::ServerQuery;
use croft_storage::search::{self as local, MARK_END, MARK_START};
use croft_storage::{accounts, folders};

use crate::core::Core;
use crate::error::{CoreError, StorageReason};
use crate::list;
use crate::settings::is_demo;
use crate::types::MessageRow;

/// A matched part of a text: `[start, end)` in UTF-16 code units.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct TextRange {
    /// First code unit of the match.
    pub start: u32,
    /// Code unit just after the match.
    pub end: u32,
}

/// One search result: a list row and what matched in it.
#[derive(Clone, PartialEq, Eq, uniffi::Record)]
pub struct SearchHit {
    /// The row, as the message list shows it (actions go through the usual calls).
    pub row: MessageRow,
    /// The matched parts of `row.subject`.
    pub subject_ranges: Vec<TextRange>,
    /// A short piece of the downloaded body around a match, if the body matched.
    pub snippet: Option<String>,
    /// The matched parts of `snippet`.
    pub snippet_ranges: Vec<TextRange>,
}

/// What [`Core::search_local`] found.
#[derive(Clone, PartialEq, Eq, uniffi::Record)]
pub struct LocalResults {
    /// Best first.
    pub hits: Vec<SearchHit>,
    /// Some messages in the scope have no downloaded body, so their text was not searched: the
    /// screen shows "Searched headers and downloaded mail" and offers Search server.
    pub bodies_partial: bool,
}

/// What [`Core::search_server`] found, and what kept it from finding more. Several accounts may
/// each add a different note; the hits found are always kept.
#[derive(Clone, PartialEq, Eq, uniffi::Record)]
pub struct ServerResults {
    /// Newest first per account; merge them with the local hits by `row.id`.
    pub hits: Vec<SearchHit>,
    /// A server cannot run this search (or the query has characters it cannot be sent with).
    pub unsupported: bool,
    /// An account is offline: its server was not asked.
    pub offline: bool,
    /// An account needs the person to sign in again.
    pub sign_in_needed: bool,
    /// A server did not answer or refused. Try again.
    pub failed: bool,
}

/// Why a search call failed. Fixed categories: never the query.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Error)]
pub enum SearchFailure {
    /// `before:` or `after:` with something that is not a date (YYYY-MM-DD).
    BadDate,
    /// No account with this id.
    NoSuchAccount,
    /// The database failed.
    Storage {
        /// Fixed category.
        reason: StorageReason,
    },
    /// Something inside the core failed.
    Internal,
}

impl std::fmt::Display for SearchFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BadDate => f.write_str("a date in the search is not a date"),
            Self::NoSuchAccount => f.write_str("no such account"),
            Self::Storage { .. } => f.write_str("storage error"),
            Self::Internal => f.write_str("internal error"),
        }
    }
}

impl std::error::Error for SearchFailure {}

impl From<QueryError> for SearchFailure {
    fn from(e: QueryError) -> Self {
        match e {
            QueryError::BadDate => Self::BadDate,
        }
    }
}

impl From<CoreError> for SearchFailure {
    fn from(e: CoreError) -> Self {
        match e {
            CoreError::NoSuchAccount => Self::NoSuchAccount,
            CoreError::Storage { reason } => Self::Storage { reason },
            _ => Self::Internal,
        }
    }
}

impl From<croft_storage::StorageError> for SearchFailure {
    fn from(e: croft_storage::StorageError) -> Self {
        CoreError::from(e).into()
    }
}

/// The text without the match markers, and the marked parts as UTF-16 ranges. A stray marker
/// (one the text itself carried) is dropped.
pub(crate) fn unmark(marked: &str) -> (String, Vec<TextRange>) {
    let mut out = String::with_capacity(marked.len());
    let mut ranges = Vec::new();
    let mut pos: u32 = 0;
    let mut open: Option<u32> = None;
    for c in marked.chars() {
        if c == MARK_START {
            open = Some(pos);
        } else if c == MARK_END {
            if let Some(start) = open.take() {
                if pos > start {
                    ranges.push(TextRange { start, end: pos });
                }
            }
        } else {
            out.push(c);
            pos = pos.saturating_add(u32::try_from(c.len_utf16()).unwrap_or(1));
        }
    }
    (out, ranges)
}

/// The places in `text` where one of `words` occurs (ignoring case), as UTF-16 ranges: the
/// highlighting of a server hit, which has no index marks.
pub(crate) fn find_words(text: &str, words: &[String]) -> Vec<TextRange> {
    let lower: Vec<char> = text.chars().flat_map(char::to_lowercase).collect();
    // Lower-casing can change the number of chars; then only exact-length matches are marked.
    let chars: Vec<char> = text.chars().collect();
    if lower.len() != chars.len() {
        return Vec::new();
    }
    let mut utf16_at = Vec::with_capacity(chars.len() + 1);
    let mut p = 0u32;
    for c in &chars {
        utf16_at.push(p);
        p = p.saturating_add(u32::try_from(c.len_utf16()).unwrap_or(1));
    }
    utf16_at.push(p);
    let mut ranges: Vec<TextRange> = Vec::new();
    for w in words {
        let needle: Vec<char> = w.chars().flat_map(char::to_lowercase).collect();
        if needle.is_empty() || needle.len() > lower.len() {
            continue;
        }
        let mut i = 0;
        while i + needle.len() <= lower.len() {
            if lower[i..i + needle.len()] == needle[..] {
                ranges.push(TextRange {
                    start: utf16_at[i],
                    end: utf16_at[i + needle.len()],
                });
                i += needle.len();
            } else {
                i += 1;
            }
        }
    }
    ranges.sort_by_key(|r| r.start);
    // Overlapping ranges merge into one.
    let mut merged: Vec<TextRange> = Vec::new();
    for r in ranges {
        match merged.last_mut() {
            Some(last) if r.start <= last.end => last.end = last.end.max(r.end),
            _ => merged.push(r),
        }
    }
    merged
}

fn term_text(t: &TextTerm) -> String {
    match t {
        TextTerm::Word(s) | TextTerm::Phrase(s) => s.clone(),
    }
}

/// The server form of a query: its text, sender, subject, recipient, flag and date conditions.
/// `has:attachment` and `in:` are not sent (the folders searched carry `in:`).
pub(crate) fn server_query(q: &SearchQuery) -> ServerQuery {
    ServerQuery {
        text: q.text.iter().map(term_text).collect(),
        from: q.from.iter().map(term_text).collect(),
        to: q.to.clone(),
        subject: q.subject.iter().map(term_text).collect(),
        unread: q.unread,
        starred: q.starred,
        before_ms: q.before_ms,
        since_ms: q.after_ms,
    }
}

/// The words a server hit's subject is highlighted with.
fn highlight_words(q: &SearchQuery) -> Vec<String> {
    q.text
        .iter()
        .chain(q.subject.iter())
        .map(term_text)
        .collect()
}

impl Core {
    /// The accounts a search covers: one (which must exist), or all.
    fn search_scope(&self, account: Option<i64>) -> Result<Vec<accounts::Account>, SearchFailure> {
        self.inner.handle.read(|db| {
            let all = accounts::list(db.connection())?;
            match account {
                None => Ok(all),
                Some(id) => {
                    let one: Vec<_> = all.into_iter().filter(|a| a.id == id).collect();
                    if one.is_empty() {
                        Err(SearchFailure::NoSuchAccount)
                    } else {
                        Ok(one)
                    }
                }
            }
        })
    }
}

#[uniffi::export]
impl Core {
    /// Searches the mail stored on the device: one account (`account`) or all (`None`). Instant
    /// (database only), works offline. Up to `limit` hits (at most 200), best first. An empty
    /// query finds nothing. The only error the query can cause is [`SearchFailure::BadDate`].
    pub fn search_local(
        &self,
        account: Option<i64>,
        query: String,
        limit: u32,
    ) -> Result<LocalResults, SearchFailure> {
        let q = parse(&query)?;
        let scope: Vec<i64> = self.search_scope(account)?.iter().map(|a| a.id).collect();
        self.inner.handle.read(|db| {
            let conn = db.connection();
            let found = local::search(conn, &scope, &q, limit)?;
            let ids: Vec<i64> = found.iter().map(|h| h.message_id).collect();
            let rows = list::rows_for(conn, &scope, &ids)?;
            let hits = rows
                .into_iter()
                .map(|mut row| {
                    let h = found.iter().find(|h| h.message_id == row.id);
                    // The ranges are counted on the subject with the marks removed; the row shows that same text, so a
                    // subject that itself carried a mark character cannot shift the highlights.
                    let subject_ranges =
                        match h.and_then(|h| h.subject_marked.as_deref()).map(unmark) {
                            Some((text, ranges)) => {
                                row.subject = text;
                                ranges
                            }
                            None => Vec::new(),
                        };
                    let (snippet, snippet_ranges) =
                        match h.and_then(|h| h.snippet_marked.as_deref()).map(unmark) {
                            Some((s, r)) => (Some(s), r),
                            None => (None, Vec::new()),
                        };
                    SearchHit {
                        row,
                        subject_ranges,
                        snippet,
                        snippet_ranges,
                    }
                })
                .collect();
            let bodies_partial = q.has_text() && local::bodies_missing(conn, &scope)?;
            Ok(LocalResults {
                hits,
                bodies_partial,
            })
        })
    }

    /// Asks the servers of the scope (one account or all; the sample mailbox is never asked) and
    /// returns their hits as rows: hits the device did not have are stored first, so they open
    /// and act like any row. Searched: the folder named by `in:`, else the Inbox, Archive, All
    /// Mail and Sent folders. Async; cancelling it (the person typed again) stops the requests,
    /// and rows already stored stay. Problems with one account are flags in the answer, so the
    /// hits of the others are kept.
    pub async fn search_server(
        &self,
        account: Option<i64>,
        query: String,
    ) -> Result<ServerResults, SearchFailure> {
        let q = parse(&query)?;
        let scope = self.search_scope(account)?;
        let sq = server_query(&q);
        let words = highlight_words(&q);
        let inner = self.inner.clone();
        let task = self.spawn(
            async move {
                let mut out = ServerResults {
                    hits: Vec::new(),
                    unsupported: false,
                    offline: false,
                    sign_in_needed: false,
                    failed: false,
                };
                if sq.is_empty() {
                    return Ok(out);
                }
                for a in scope {
                    if is_demo(&a.config, &a.email) {
                        continue;
                    }
                    let folder_ids = inner.handle.read(|db| {
                        let conn = db.connection();
                        match &q.folder {
                            Some(name) => local::named_folders(conn, a.id, name),
                            None => Ok(folders::list(conn, a.id)?
                                .into_iter()
                                .filter(|f| {
                                    matches!(
                                        f.special_use.as_str(),
                                        "inbox" | "archive" | "all" | "sent"
                                    )
                                })
                                .map(|f| f.id)
                                .collect()),
                        }
                    })?;
                    if folder_ids.is_empty() {
                        continue;
                    }
                    match inner.handle.search_server(a.id, &folder_ids, &sq).await {
                        Ok(ids) => {
                            let rows = inner
                                .handle
                                .read(|db| list::rows_for(db.connection(), &[a.id], &ids))?;
                            out.hits.extend(rows.into_iter().map(|row| SearchHit {
                                subject_ranges: find_words(&row.subject, &words),
                                row,
                                snippet: None,
                                snippet_ranges: Vec::new(),
                            }));
                        }
                        Err(EngineSearchError::Unsupported) => out.unsupported = true,
                        Err(EngineSearchError::Offline) => out.offline = true,
                        Err(EngineSearchError::NeedsSignIn) => out.sign_in_needed = true,
                        Err(EngineSearchError::Storage(e)) => return Err(e.into()),
                        Err(
                            EngineSearchError::Server(_) | EngineSearchError::UnknownAccount(_),
                        ) => {
                            out.failed = true;
                        }
                    }
                }
                Ok(out)
            },
            true,
        );
        task.await.map_err(|_| SearchFailure::Internal)?
    }
}
