// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Server search for the search screen (S28): the account's backend searches the given folders;
//! hits the device does not have yet get their headers fetched and stored like a sync would store
//! them, so every hit is an ordinary row (it opens, archives and deletes like any other). The
//! answer is the hits' row ids, so the screen merges them into its local results by identity.
//!
//! The query goes only to this account's own server and is never logged.

use std::fmt;
use std::sync::atomic::Ordering;

use croft_mail_core::AccountId;
use croft_protocols::{BackendError, SearchOutcome, ServerQuery, Uid};
use croft_storage::{folders, messages, StorageError};

use crate::engine::Change;
use crate::handle::EngineHandle;

/// Why [`EngineHandle::search_server`] found nothing to show. Fixed categories: never the query.
#[derive(Debug)]
pub enum SearchError {
    /// No backend was added for this account.
    UnknownAccount(AccountId),
    /// The device is offline for this account.
    Offline,
    /// The server refused the sign-in (the account now waits for `EngineHandle::signed_in`).
    NeedsSignIn,
    /// The server cannot run this search (none of the folders could be searched).
    Unsupported,
    /// The server failed: `Transient` (try again) or `Permanent`.
    Server(BackendError),
    /// The database failed.
    Storage(StorageError),
}

impl fmt::Display for SearchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownAccount(a) => write!(f, "no backend for account {a}"),
            Self::Offline => f.write_str("offline"),
            Self::NeedsSignIn => f.write_str("sign-in needed"),
            Self::Unsupported => f.write_str("the server cannot search"),
            Self::Server(e) => e.fmt(f),
            Self::Storage(e) => write!(f, "database error: {e}"),
        }
    }
}

impl std::error::Error for SearchError {}

impl From<StorageError> for SearchError {
    fn from(e: StorageError) -> Self {
        Self::Storage(e)
    }
}

impl EngineHandle {
    /// Searches `folder_ids` of `account` on its server and returns the hits' row ids, folder by
    /// folder, newest first within each. A hit not stored yet is fetched (headers only) and
    /// stored first. A folder the server cannot search is skipped; when none can be,
    /// [`SearchError::Unsupported`]. Ids of folders not in this account are ignored. **Waits for
    /// the network**: call it from a task. Cancelling the call is safe (stored rows stay).
    pub async fn search_server(
        &self,
        account: AccountId,
        folder_ids: &[i64],
        query: &ServerQuery,
    ) -> Result<Vec<i64>, SearchError> {
        let shared = self.shared().clone();
        if query.is_empty() {
            return Ok(Vec::new());
        }
        let paths = {
            let q = shared.lock();
            if q.queue().is_offline(account) {
                return Err(SearchError::Offline);
            }
            let conn = q.store().db().connection();
            let mut paths = Vec::new();
            for &id in folder_ids {
                if let Some(f) = folders::get(conn, account, id)? {
                    paths.push((f.id, f.path));
                }
            }
            paths
        };
        let slot = shared
            .slot(account)
            .ok_or(SearchError::UnknownAccount(account))?;
        if slot.needs_sign_in() {
            return Err(SearchError::NeedsSignIn);
        }
        let auth = |e: BackendError| {
            if matches!(e, BackendError::Auth(_)) {
                if !slot.needs_sign_in.swap(true, Ordering::SeqCst) {
                    shared.emit(Change::SignInNeeded { account });
                }
                SearchError::NeedsSignIn
            } else {
                SearchError::Server(e)
            }
        };
        let mut hits = Vec::new();
        let mut searched_any = false;
        let mut changed = Vec::new();
        for (folder_id, path) in paths {
            let uids = match slot.backend.search(&path, query).await.map_err(auth)? {
                SearchOutcome::Found(u) => u,
                SearchOutcome::Unsupported => continue,
            };
            searched_any = true;
            let (mut known, missing): (Vec<(Uid, i64)>, Vec<Uid>) = {
                let q = shared.lock();
                let conn = q.store().db().connection();
                let mut known = Vec::new();
                let mut missing = Vec::new();
                for &uid in &uids {
                    match messages::find_by_uid(conn, account, folder_id, uid)? {
                        Some(m) if !m.local_hidden => known.push((uid, m.id)),
                        // Hidden while a move or delete waits: not a result.
                        Some(_) => {}
                        None => missing.push(uid),
                    }
                }
                (known, missing)
            };
            if !missing.is_empty() {
                let headers = slot
                    .backend
                    .fetch_headers(&path, &missing)
                    .await
                    .map_err(auth)?;
                let q = shared.lock();
                let conn = q.store().db().connection();
                for h in &headers {
                    // A folder removed meanwhile: `upsert` stores nothing and fails; skip it.
                    if let Ok(id) = messages::upsert(conn, account, folder_id, h) {
                        known.push((h.uid, id));
                    }
                }
                if !headers.is_empty() {
                    changed.push(folder_id);
                }
            }
            // Newest first, as the server's UIDs grow with arrival.
            known.sort_by_key(|k| std::cmp::Reverse(k.0));
            hits.extend(known.into_iter().map(|(_, id)| id));
        }
        drop(slot);
        if !changed.is_empty() {
            shared.emit(Change::Messages {
                account,
                folders: changed,
            });
        }
        if !searched_any {
            return Err(SearchError::Unsupported);
        }
        Ok(hits)
    }
}
