// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The JMAP implementation of [`MailBackend`]: one [`JmapBackend`] per account.
//!
//! See the module notes in [`super`] for how identity, folders and errors are mapped. This file
//! holds the operations. All of them take one lock on the adapter's state for their whole run, so
//! UIDs are assigned in a single order and the caches never see a half-finished update.

use super::api::{ApiError, Client, Responses};
use super::config::JmapConfig;
use super::error::{from_set_error, malformed};
use super::idmap::{IdStoreError, JmapIdStore};
use super::mapping::*;
use super::session::{Session, CAP_CORE, CAP_MAIL};
use crate::backend::{BackendFuture, MailBackend};
use crate::imap::Credential;
use crate::types::*;
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;
use tokio::sync::Mutex;

/// Ids asked of `Email/query` per page.
const QUERY_PAGE: usize = 1000;
/// More messages than this in one folder is refused rather than loaded.
const MAX_MESSAGES: usize = 1_000_000;
/// How many of the newest messages `find_by_message_id` looks at when the server's header search
/// finds nothing (some servers do not index `Message-ID`).
const SCAN_LIMIT: usize = 2000;
/// Most `Email/changes` pages followed in one sync.
const MAX_CHANGE_PAGES: usize = 1000;

const MAIL: &[&str] = &[CAP_CORE, CAP_MAIL];

/// One step of a permanent delete.
enum DeleteStep {
    /// The message is in other mailboxes too: only take it out of this one (id and patch).
    Leave(String, Value),
    /// The message is only here: destroy it.
    Destroy(String),
}

/// The outcome of [`JmapBackend::update_emails`]: what was done, and what stopped the rest.
#[derive(Default)]
struct Updated {
    /// Ids the server confirmed.
    done: Vec<String>,
    /// The first failure, if the run did not finish.
    error: Option<BackendError>,
}

/// What the adapter remembers about one mailbox between calls: the `Email` state its flags are
/// valid for, and every message in it. With this, `snapshot` can ask `Email/changes` and fetch
/// only what changed. It lives in memory; after a restart the first snapshot of each folder is a
/// full one.
struct MailboxCache {
    /// The mailbox's UIDVALIDITY when the cache was built. A cache is only good for the numbering
    /// it was built in: if the id map has been reset or lost since, its UIDs are stale.
    validity: u32,
    state: String,
    /// Email id to (UID, flags).
    messages: HashMap<String, (Uid, Flags)>,
}

#[derive(Default)]
struct State {
    session: Option<Arc<Session>>,
    mailboxes: Option<Mailboxes>,
    caches: HashMap<String, MailboxCache>,
}

/// A JMAP account: settings, the credential, and the cached session and mailbox list.
pub struct JmapBackend {
    config: JmapConfig,
    client: Client,
    ids: Arc<dyn JmapIdStore + Send + Sync>,
    state: Mutex<State>,
}

impl std::fmt::Debug for JmapBackend {
    /// Prints the settings (origin only) and never the credential.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("JmapBackend")
            .field("config", &self.config)
            .field("credential", &"***")
            .finish()
    }
}

impl JmapBackend {
    /// Creates the adapter. Nothing is sent yet: the first operation fetches the session. Fails
    /// with [`BackendError::Permanent`] when the settings are unusable, for example a session
    /// address that is not `https` (plain `http` is accepted for localhost only).
    ///
    /// `ids` is the account's persistent id map (see [`JmapIdStore`]); give the same one back after
    /// a restart so UIDs stay the same.
    pub fn new(
        config: JmapConfig,
        credential: Credential,
        ids: Arc<dyn JmapIdStore + Send + Sync>,
    ) -> Result<Self, BackendError> {
        let session_url = config.validate()?;
        let client = Client::new(&config, &credential, session_url)?;
        Ok(Self {
            config,
            client,
            ids,
            state: Mutex::new(State::default()),
        })
    }

    /// The settings this adapter was made with.
    pub fn config(&self) -> &JmapConfig {
        &self.config
    }

    /// Builds the adapter from what discovery found, when the account's domain offers JMAP: `None`
    /// when `settings.jmap` is not set. The address is used as discovery recorded it; the caller
    /// shows its host to the person at setup (see [`DiscoveredSettings::jmap`](crate::discovery::DiscoveredSettings::jmap)).
    pub fn from_discovered(
        settings: &crate::discovery::DiscoveredSettings,
        username: &str,
        credential: Credential,
        ids: Arc<dyn JmapIdStore + Send + Sync>,
    ) -> Option<Result<Self, BackendError>> {
        let url = settings.jmap.as_deref()?;
        Some(Self::new(JmapConfig::new(url, username), credential, ids))
    }

    // ------------------------------------------------------------------------------------
    // Plumbing
    // ------------------------------------------------------------------------------------

    async fn session(&self, st: &mut State) -> Result<Arc<Session>, BackendError> {
        if let Some(s) = &st.session {
            return Ok(s.clone());
        }
        let s = Arc::new(self.client.fetch_session().await?);
        st.session = Some(s.clone());
        Ok(s)
    }

    /// One JMAP request. Drops the cached session when sign-in failed or the server says the
    /// session changed, so the next call fetches it again.
    async fn call(
        &self,
        st: &mut State,
        calls: Vec<(&'static str, Value)>,
    ) -> Result<Responses, BackendError> {
        let session = self.session(st).await?;
        match self.client.call(&session, MAIL, calls).await {
            Ok(r) => {
                if let (Some(now), Some(then)) = (&r.session_state, &session.state) {
                    if now != then {
                        st.session = None;
                    }
                }
                Ok(r)
            }
            Err(e) => {
                if matches!(e, BackendError::Auth(_)) {
                    st.session = None;
                }
                Err(e)
            }
        }
    }

    /// Runs the id map call `r`. A map that reports itself inconsistent or full is reset for this
    /// mailbox (new UIDVALIDITY, so the engine starts the folder over) and the call fails as
    /// retryable.
    fn store<T>(
        &self,
        st: &mut State,
        mailbox: &str,
        r: Result<T, IdStoreError>,
    ) -> Result<T, BackendError> {
        match r {
            Ok(v) => Ok(v),
            Err(IdStoreError::Unavailable) => Err(BackendError::Transient(
                IdStoreError::Unavailable.to_string(),
            )),
            Err(_) => {
                let _ = self.ids.reset_mailbox(mailbox);
                st.caches.remove(mailbox);
                Err(BackendError::Transient(
                    "the folder's numbering was reset; try again".into(),
                ))
            }
        }
    }

    async fn refresh_mailboxes(&self, st: &mut State) -> Result<(), BackendError> {
        let session = self.session(st).await?;
        let args = json!({"accountId": session.account_id,
            "properties": ["id", "name", "parentId", "role", "totalEmails", "myRights"]});
        let mut r = self.call(st, vec![("Mailbox/get", args)]).await?;
        let (boxes, _) =
            parse_mailboxes(&r.take(0, "Mailbox/get").map_err(ApiError::into_backend)?)?;
        // A mailbox that is gone takes its numbering with it.
        if let Some(old) = &st.mailboxes {
            let gone: Vec<String> = old
                .items
                .iter()
                .filter(|m| boxes.by_id(&m.id).is_none())
                .map(|m| m.id.clone())
                .collect();
            for id in gone {
                st.caches.remove(&id);
                self.ids
                    .forget_mailbox(&id)
                    .map_err(|e| BackendError::Transient(e.to_string()))?;
            }
        }
        st.mailboxes = Some(boxes);
        Ok(())
    }

    /// The mailbox with this path, refreshing the list once if it is not known.
    async fn mailbox(&self, st: &mut State, path: &str) -> Result<MailboxInfo, BackendError> {
        if let Some(m) = st.mailboxes.as_ref().and_then(|b| b.by_path(path)) {
            return Ok(m.clone());
        }
        self.refresh_mailboxes(st).await?;
        st.mailboxes
            .as_ref()
            .and_then(|b| b.by_path(path))
            .cloned()
            .ok_or_else(|| BackendError::Permanent("no such folder".into()))
    }

    /// `Email/get` for any number of ids, in batches the server allows.
    async fn get_emails(
        &self,
        st: &mut State,
        ids: &[String],
        properties: &[&str],
    ) -> Result<(Vec<RawEmail>, Vec<String>), BackendError> {
        let (found, missing, _) = self.get_emails_at(st, ids, properties).await?;
        Ok((found, missing))
    }

    /// Like [`get_emails`](Self::get_emails), and also the `Email` state of the first batch. A
    /// later `Email/set` with that state as `ifInState` fails if anything changed after the
    /// first read (the state is that of the earliest batch so that a change between batches is
    /// caught as well).
    async fn get_emails_at(
        &self,
        st: &mut State,
        ids: &[String],
        properties: &[&str],
    ) -> Result<(Vec<RawEmail>, Vec<String>, Option<String>), BackendError> {
        let session = self.session(st).await?;
        let mut found = Vec::with_capacity(ids.len());
        let mut missing = Vec::new();
        let mut first_state = None;
        for chunk in ids.chunks(session.max_get) {
            let args = json!({"accountId": session.account_id, "ids": chunk,
                "properties": properties});
            let mut r = self.call(st, vec![("Email/get", args)]).await?;
            let got = parse_email_get(&r.take(0, "Email/get").map_err(ApiError::into_backend)?)?;
            if first_state.is_none() {
                first_state = got.state;
            }
            found.extend(got.list);
            missing.extend(got.not_found);
        }
        Ok((found, missing, first_state))
    }

    /// The account's current `Email` state.
    async fn email_state(&self, st: &mut State) -> Result<String, BackendError> {
        let session = self.session(st).await?;
        let args = json!({"accountId": session.account_id, "ids": [], "properties": ["id"]});
        let mut r = self.call(st, vec![("Email/get", args)]).await?;
        parse_email_get(&r.take(0, "Email/get").map_err(ApiError::into_backend)?)?
            .state
            .ok_or_else(malformed)
    }

    /// `Email/set` with these updates (id to patch), in batches. Reports the ids that were updated
    /// and the first real failure, if any: the ids done before a failure are still reported, so
    /// the caller can record them. An id the server does not know is skipped silently.
    async fn update_emails(&self, st: &mut State, updates: Vec<(String, Value)>) -> Updated {
        let mut out = Updated::default();
        let session = match self.session(st).await {
            Ok(s) => s,
            Err(e) => {
                out.error = Some(e);
                return out;
            }
        };
        for chunk in updates.chunks(session.max_set) {
            let update: serde_json::Map<String, Value> = chunk.iter().cloned().collect();
            let args = json!({"accountId": session.account_id, "update": update});
            let set = match self.call(st, vec![("Email/set", args)]).await {
                Ok(mut r) => r
                    .take(0, "Email/set")
                    .map_err(ApiError::into_backend)
                    .map(|a| parse_set(&a)),
                Err(e) => Err(e),
            };
            match set {
                Ok(set) => {
                    out.done.extend(set.done);
                    if let Some(e) = set.failed.iter().find_map(|(_, kind)| from_set_error(kind)) {
                        out.error = Some(e);
                        return out;
                    }
                }
                Err(e) => {
                    out.error = Some(e);
                    return out;
                }
            }
        }
        out
    }

    // ------------------------------------------------------------------------------------
    // Operations
    // ------------------------------------------------------------------------------------

    async fn capabilities_inner(&self) -> Result<Capabilities, BackendError> {
        let mut st = self.state.lock().await;
        // Reading the session is what proves the sign-in works.
        self.session(&mut st).await?;
        Ok(Capabilities {
            move_cmd: true,
            uidplus: true,
            special_use: true,
            idle: false,
            // `highest_modseq` is a number derived from the `Email` state: it changes whenever
            // anything in the account changes. There are no per-message sequence numbers.
            condstore: true,
            qresync: false,
        })
    }

    async fn list_folders_inner(&self) -> Result<Vec<RemoteFolder>, BackendError> {
        let mut st = self.state.lock().await;
        self.refresh_mailboxes(&mut st).await?;
        Ok(st
            .mailboxes
            .iter()
            .flat_map(|b| b.items.iter())
            .map(|m| RemoteFolder {
                path: m.path.clone(),
                delimiter: Some('/'),
                special_use: m.special_use,
                selectable: m.selectable,
            })
            .collect())
    }

    async fn folder_status_inner(&self, folder: &str) -> Result<FolderStatus, BackendError> {
        let mut st = self.state.lock().await;
        let mb = self.mailbox(&mut st, folder).await?;
        let session = self.session(&mut st).await?;
        let calls = vec![
            (
                "Mailbox/get",
                json!({"accountId": session.account_id, "ids": [mb.id],
                    "properties": ["id", "totalEmails"]}),
            ),
            (
                "Email/get",
                json!({"accountId": session.account_id, "ids": [], "properties": ["id"]}),
            ),
        ];
        let mut r = self.call(&mut st, calls).await?;
        let (boxes, _) =
            parse_mailboxes(&r.take(0, "Mailbox/get").map_err(ApiError::into_backend)?)?;
        let state = parse_email_get(&r.take(1, "Email/get").map_err(ApiError::into_backend)?)?
            .state
            .ok_or_else(malformed)?;
        let Some(fresh) = boxes.by_id(&mb.id) else {
            // Gone since the list was read.
            st.mailboxes = None;
            return Err(BackendError::Permanent("no such folder".into()));
        };
        let uid_validity = self.store(&mut st, &mb.id, self.ids.uid_validity(&mb.id))?;
        let uid_next = self.store(&mut st, &mb.id, self.ids.next_uid(&mb.id))?;
        Ok(FolderStatus {
            uid_validity,
            uid_next,
            highest_modseq: Some(state_number(&state)),
            exists: fresh.total,
        })
    }

    async fn snapshot_inner(&self, folder: &str) -> Result<RemoteSnapshot, BackendError> {
        let mut st = self.state.lock().await;
        let mb = self.mailbox(&mut st, folder).await?;
        let uid_validity = self.store(&mut st, &mb.id, self.ids.uid_validity(&mb.id))?;
        let next_before = self.store(&mut st, &mb.id, self.ids.next_uid(&mb.id))?;
        // The cache only counts if the map is the one it was built from: same UIDVALIDITY, and
        // the map has not fallen behind the UIDs the cache already handed out (a map restored
        // from an older copy could even repeat a UIDVALIDITY).
        let cache = match st.caches.remove(&mb.id) {
            Some(cache)
                if cache.validity == uid_validity
                    && cache.messages.values().all(|(uid, _)| *uid < next_before) =>
            {
                self.apply_changes(&mut st, &mb, cache).await?
            }
            _ => None,
        };
        let cache = match cache {
            Some(c) => c,
            None => self.full_sync(&mut st, &mb).await?,
        };
        self.store(
            &mut st,
            &mb.id,
            self.ids.set_email_state(Some(&cache.state)),
        )?;
        let uid_next = self.store(&mut st, &mb.id, self.ids.next_uid(&mb.id))?;
        let messages: BTreeMap<Uid, Flags> = cache
            .messages
            .values()
            .map(|(uid, flags)| (*uid, flags.clone()))
            .collect();
        let status = FolderStatus {
            uid_validity,
            uid_next,
            highest_modseq: Some(state_number(&cache.state)),
            exists: u32::try_from(messages.len()).unwrap_or(u32::MAX),
        };
        st.caches.insert(mb.id.clone(), cache);
        Ok(RemoteSnapshot { status, messages })
    }

    /// Brings a cached folder up to date with `Email/changes`. `None` means the server cannot
    /// say what changed since the cached state (`cannotCalculateChanges`, or a state it no longer
    /// knows), and the caller does a full sync.
    async fn apply_changes(
        &self,
        st: &mut State,
        mb: &MailboxInfo,
        mut cache: MailboxCache,
    ) -> Result<Option<MailboxCache>, BackendError> {
        let session = self.session(st).await?;
        let mut since = cache.state.clone();
        let mut changed: Vec<String> = Vec::new();
        let mut destroyed: Vec<String> = Vec::new();
        for _ in 0..MAX_CHANGE_PAGES {
            let args = json!({"accountId": session.account_id, "sinceState": since,
                "maxChanges": session.max_get});
            let mut r = self.call(st, vec![("Email/changes", args)]).await?;
            let answer = match r.take(0, "Email/changes") {
                Ok(a) => a,
                // `cannotCalculateChanges` is the standard answer. Some servers (Stalwart) answer
                // `invalidArguments` for a state they do not recognise. A full sync is always right.
                Err(ApiError::Method(k))
                    if k == "cannotCalculateChanges" || k == "invalidArguments" =>
                {
                    return Ok(None)
                }
                Err(e) => return Err(e.into_backend()),
            };
            let c = parse_changes(&answer)?;
            changed.extend(c.created);
            changed.extend(c.updated);
            destroyed.extend(c.destroyed);
            let progressed = c.new_state != since;
            since = c.new_state;
            if !c.has_more {
                break;
            }
            if !progressed {
                return Err(malformed());
            }
        }

        let destroyed_set: HashSet<&String> = destroyed.iter().collect();
        let mut seen = HashSet::new();
        changed.retain(|id| !destroyed_set.contains(id) && seen.insert(id.clone()));
        let (emails, not_found) = self.get_emails(st, &changed, FLAG_PROPERTIES).await?;

        let mut leave: Vec<String> = destroyed;
        leave.extend(not_found);
        let mut newcomers: Vec<(i64, String, Flags)> = Vec::new();
        for e in emails {
            if e.mailboxes().contains(mb.id.as_str()) {
                let flags = e.flags();
                match cache.messages.get_mut(&e.id) {
                    Some(entry) => entry.1 = flags,
                    None => newcomers.push((e.received_ms(), e.id.clone(), flags)),
                }
            } else {
                // Moved to another mailbox (by someone else, or by us).
                leave.push(e.id.clone());
            }
        }
        for id in &leave {
            cache.messages.remove(id);
        }
        self.store(st, &mb.id, self.ids.remove(&mb.id, &leave))?;
        // Oldest first, so the UIDs of new mail follow the order it arrived in.
        newcomers.sort_by(|a, b| (a.0, &a.1).cmp(&(b.0, &b.1)));
        let new_ids: Vec<String> = newcomers.iter().map(|n| n.1.clone()).collect();
        let uids = self.store(st, &mb.id, self.ids.assign(&mb.id, &new_ids))?;
        for ((_, id, flags), uid) in newcomers.into_iter().zip(uids) {
            cache.messages.insert(id, (uid, flags));
        }
        cache.state = since;
        Ok(Some(cache))
    }

    /// Reads the whole folder: every id (oldest first) and its keywords, and reconciles the id
    /// map with it.
    async fn full_sync(
        &self,
        st: &mut State,
        mb: &MailboxInfo,
    ) -> Result<MailboxCache, BackendError> {
        let session = self.session(st).await?;
        // The state is read before the folder, so a change made while the folder is being read
        // shows up again in the next `Email/changes` instead of being missed.
        let state = self.email_state(st).await?;
        let mut ids: Vec<String> = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();
        // `position` counts what the server has returned so far, duplicates included.
        let mut position = 0usize;
        loop {
            let args = json!({"accountId": session.account_id,
                "filter": {"inMailbox": mb.id},
                "sort": [{"property": "receivedAt", "isAscending": true}],
                "position": position, "limit": QUERY_PAGE, "calculateTotal": true});
            let mut r = self.call(st, vec![("Email/query", args)]).await?;
            let page = parse_query(&r.take(0, "Email/query").map_err(ApiError::into_backend)?)?;
            if page.ids.is_empty() {
                break;
            }
            position += page.ids.len();
            ids.extend(page.ids.into_iter().filter(|id| seen.insert(id.clone())));
            if ids.len() > MAX_MESSAGES {
                return Err(BackendError::Permanent("the folder is too large".into()));
            }
            if page.total.is_some_and(|t| position as u64 >= t) {
                break;
            }
        }
        let (emails, _) = self.get_emails(st, &ids, &["id", "keywords"]).await?;
        let flags: HashMap<String, Flags> = emails
            .into_iter()
            .map(|e| (e.id.clone(), e.flags()))
            .collect();

        let validity = self.store(st, &mb.id, self.ids.uid_validity(&mb.id))?;
        let existing = self.store(st, &mb.id, self.ids.entries(&mb.id))?;
        let present: HashSet<&String> = ids.iter().collect();
        let gone: Vec<String> = existing
            .values()
            .filter(|e| !present.contains(e))
            .cloned()
            .collect();
        if !gone.is_empty() {
            self.store(st, &mb.id, self.ids.remove(&mb.id, &gone))?;
        }
        // New ids are numbered in the order given (oldest first); known ids keep their UID.
        let all = self.store(st, &mb.id, self.ids.assign(&mb.id, &ids))?;
        let messages = ids
            .iter()
            .zip(all)
            .filter_map(|(id, uid)| flags.get(id).map(|f| (id.clone(), (uid, f.clone()))))
            .collect();
        Ok(MailboxCache {
            validity,
            state,
            messages,
        })
    }

    async fn fetch_headers_inner(
        &self,
        folder: &str,
        uids: &[Uid],
    ) -> Result<Vec<RemoteHeader>, BackendError> {
        let mut st = self.state.lock().await;
        let mb = self.mailbox(&mut st, folder).await?;
        let by_uid = self.store(&mut st, &mb.id, self.ids.email_ids(&mb.id, uids))?;
        let email_ids: Vec<String> = by_uid.values().cloned().collect();
        let uid_of: HashMap<&String, Uid> = by_uid.iter().map(|(u, e)| (e, *u)).collect();
        let mut props: Vec<&str> = HEADER_PROPERTIES.to_vec();
        props.push("mailboxIds");
        let (emails, _) = self.get_emails(&mut st, &email_ids, &props).await?;
        let mut rows: Vec<RemoteHeader> = emails
            .into_iter()
            // Only what is still in this folder: a message someone else moved away has no UID here.
            .filter(|e| e.mailboxes().contains(mb.id.as_str()))
            .filter_map(|e| {
                let uid = *uid_of.get(&e.id)?;
                Some(e.into_header(uid))
            })
            .collect();
        rows.sort_by_key(|r| std::cmp::Reverse((r.date_ms, r.uid)));
        Ok(rows)
    }

    /// `Email/get` of the message's `blobId` and `size` (and `mailboxIds`, so a message someone
    /// moved away counts as gone from this folder), then a download of that blob. The state lock
    /// is let go before the download, so other calls on the account do not wait for a big message.
    async fn fetch_body_inner(&self, folder: &str, uid: Uid) -> Result<RawMessage, BackendError> {
        let (session, blob_id, size) = {
            let mut st = self.state.lock().await;
            let mb = self.mailbox(&mut st, folder).await?;
            let by_uid = self.store(&mut st, &mb.id, self.ids.email_ids(&mb.id, &[uid]))?;
            let Some(email_id) = by_uid.get(&uid).cloned() else {
                return Ok(RawMessage::Gone);
            };
            let (emails, _) = self
                .get_emails(
                    &mut st,
                    &[email_id],
                    &["id", "blobId", "size", "mailboxIds"],
                )
                .await?;
            let Some(email) = emails
                .into_iter()
                .find(|e| e.mailboxes().contains(mb.id.as_str()))
            else {
                return Ok(RawMessage::Gone);
            };
            let Some(blob_id) = email.blob_id().map(str::to_owned) else {
                return Err(malformed());
            };
            let size = email.size().unwrap_or(0);
            (self.session(&mut st).await?, blob_id, size)
        };
        if size > MAX_MESSAGE_BYTES {
            return Ok(RawMessage::TooLarge { size });
        }
        let max = usize::try_from(MAX_MESSAGE_BYTES).unwrap_or(usize::MAX);
        let body = self
            .client
            .download(
                &session,
                &blob_id,
                "message/rfc822",
                "message.eml",
                size,
                max,
            )
            .await?;
        Ok(match body {
            Some(b) => RawMessage::Found(b),
            None => RawMessage::Gone,
        })
    }

    async fn find_by_message_id_inner(
        &self,
        folder: &str,
        message_id: &str,
    ) -> Result<Option<Uid>, BackendError> {
        let mut st = self.state.lock().await;
        let mb = self.mailbox(&mut st, folder).await?;
        let session = self.session(&mut st).await?;
        let wanted = unbracket(message_id).to_string();
        if wanted.is_empty() {
            return Ok(None);
        }
        // First the server's own header search, which is quick where it works.
        let args = json!({"accountId": session.account_id,
            "filter": {"inMailbox": mb.id, "header": ["Message-ID", wanted]},
            "sort": [{"property": "receivedAt", "isAscending": false}], "limit": 20});
        let mut r = self.call(&mut st, vec![("Email/query", args)]).await?;
        let ids = match r.take(0, "Email/query") {
            Ok(a) => parse_query(&a)?.ids,
            // A server without the `header` filter still gets the scan below.
            Err(ApiError::Method(k)) if k == "unsupportedFilter" || k == "invalidArguments" => {
                Vec::new()
            }
            Err(e) => return Err(e.into_backend()),
        };
        let mut matches = self.with_message_id(&mut st, &mb, &ids, &wanted).await?;
        // Some servers do not index Message-ID. Then look through the newest messages, all the
        // way to the limit unless a second match turns up first: one hit is only trusted when the
        // rest of what was searched holds no other.
        if matches.is_empty() {
            let mut position = 0;
            while matches.len() < 2 && position < SCAN_LIMIT {
                let args = json!({"accountId": session.account_id, "filter": {"inMailbox": mb.id},
                    "sort": [{"property": "receivedAt", "isAscending": false}],
                    "position": position, "limit": 500});
                let mut r = self.call(&mut st, vec![("Email/query", args)]).await?;
                let page = parse_query(&r.take(0, "Email/query").map_err(ApiError::into_backend)?)?;
                if page.ids.is_empty() {
                    break;
                }
                position += page.ids.len();
                let found = self
                    .with_message_id(&mut st, &mb, &page.ids, &wanted)
                    .await?;
                for id in found {
                    if !matches.contains(&id) {
                        matches.push(id);
                    }
                }
            }
        }
        // Several different messages with this Message-ID (a duplicate delivered twice, a copy
        // filed again): guessing the "newest" could point a delete or move at the wrong one, so
        // the answer is "not found" and the engine stops.
        let [email_id] = matches.as_slice() else {
            return Ok(None);
        };
        let uids = self.store(
            &mut st,
            &mb.id,
            self.ids.assign(&mb.id, std::slice::from_ref(email_id)),
        )?;
        Ok(uids.first().copied())
    }

    /// Of these emails, those in the mailbox whose Message-ID is exactly `wanted`.
    async fn with_message_id(
        &self,
        st: &mut State,
        mb: &MailboxInfo,
        ids: &[String],
        wanted: &str,
    ) -> Result<Vec<String>, BackendError> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let (emails, _) = self
            .get_emails(st, ids, &["id", "messageId", "mailboxIds"])
            .await?;
        Ok(emails
            .into_iter()
            .filter(|e| e.mailboxes().contains(mb.id.as_str()))
            .filter(|e| e.message_id().as_deref().map(unbracket) == Some(wanted))
            .map(|e| e.id)
            .collect())
    }

    async fn set_flag_inner(
        &self,
        folder: &str,
        uids: &[Uid],
        change: FlagChange,
    ) -> Result<(), BackendError> {
        let mut st = self.state.lock().await;
        let mb = self.mailbox(&mut st, folder).await?;
        let by_uid = self.store(&mut st, &mb.id, self.ids.email_ids(&mb.id, uids))?;
        if by_uid.is_empty() {
            return Ok(());
        }
        let (key, value) = keyword_patch(change);
        let updates: Vec<(String, Value)> = by_uid
            .values()
            .map(|id| (id.clone(), json!({ key: value })))
            .collect();
        let result = self.update_emails(&mut st, updates).await;
        if let Some(cache) = st.caches.get_mut(&mb.id) {
            for id in result.done {
                if let Some((_, flags)) = cache.messages.get_mut(&id) {
                    flags.apply(change);
                }
            }
        }
        result.error.map_or(Ok(()), Err)
    }

    async fn move_to_inner(
        &self,
        folder: &str,
        uids: &[Uid],
        dest: &str,
    ) -> Result<MoveOutcome, BackendError> {
        let mut st = self.state.lock().await;
        let src = self.mailbox(&mut st, folder).await?;
        let dst = self.mailbox(&mut st, dest).await?;
        let by_uid = self.store(&mut st, &src.id, self.ids.email_ids(&src.id, uids))?;
        let mut outcome = MoveOutcome::default();
        if by_uid.is_empty() {
            return Ok(outcome);
        }
        if src.id == dst.id {
            outcome.dest_uids = by_uid.keys().map(|u| (*u, *u)).collect();
            return Ok(outcome);
        }
        // Where each message is now. JMAP ids do not change on a move, so a message that is
        // already in the destination and no longer in the source was moved by an earlier attempt
        // whose answer was lost: it still needs its destination UID reported.
        let email_ids: Vec<String> = by_uid.values().cloned().collect();
        let (emails, _) = self
            .get_emails(&mut st, &email_ids, &["id", "mailboxIds"])
            .await?;
        let mut in_source: HashSet<String> = HashSet::new();
        let mut completed: HashSet<String> = HashSet::new();
        for e in emails {
            let boxes = e.mailboxes();
            if boxes.contains(src.id.as_str()) {
                in_source.insert(e.id);
            } else if boxes.contains(dst.id.as_str()) {
                completed.insert(e.id);
            }
        }
        let patch = json!({
            format!("mailboxIds/{}", pointer_escape(&src.id)): Value::Null,
            format!("mailboxIds/{}", pointer_escape(&dst.id)): true,
        });
        let updates: Vec<(String, Value)> = by_uid
            .values()
            .filter(|id| in_source.contains(*id))
            .map(|id| (id.clone(), patch.clone()))
            .collect();
        let result = if updates.is_empty() {
            Updated::default()
        } else {
            self.update_emails(&mut st, updates).await
        };
        completed.extend(result.done);
        // New UIDs in the destination, in the order of the old ones. This is recorded even when
        // part of the move failed: what was moved stays moved, and a retry reports it again.
        let moved_pairs: Vec<(Uid, String)> = by_uid
            .into_iter()
            .filter(|(_, id)| completed.contains(id))
            .collect();
        if !moved_pairs.is_empty() {
            let moved_ids: Vec<String> = moved_pairs.iter().map(|(_, id)| id.clone()).collect();
            let new_uids = self.store(&mut st, &dst.id, self.ids.assign(&dst.id, &moved_ids))?;
            for ((old, _), new) in moved_pairs.iter().zip(new_uids) {
                outcome.dest_uids.insert(*old, new);
            }
            // After a failure the caller gets an error and no `dest_uids`, and will try the same
            // UIDs again. Their entries in the source map stay (the next sync of the source drops
            // them, `fetch_headers` ignores them meanwhile), so the retry can find the messages in
            // the destination and report their new UIDs.
            if result.error.is_none() {
                self.store(&mut st, &src.id, self.ids.remove(&src.id, &moved_ids))?;
            }
            if let Some(cache) = st.caches.get_mut(&src.id) {
                for id in &moved_ids {
                    cache.messages.remove(id);
                }
            }
        }
        match result.error {
            Some(e) => Err(e),
            None => Ok(outcome),
        }
    }

    async fn delete_forever_inner(&self, folder: &str, uids: &[Uid]) -> Result<(), BackendError> {
        let mut st = self.state.lock().await;
        let mb = self.mailbox(&mut st, folder).await?;
        let session = self.session(&mut st).await?;
        let by_uid = self.store(&mut st, &mb.id, self.ids.email_ids(&mb.id, uids))?;
        if by_uid.is_empty() {
            return Ok(());
        }
        let email_ids: Vec<String> = by_uid.values().cloned().collect();
        // Only what was asked for counts, whatever the server's answers contain.
        let requested: HashSet<&String> = email_ids.iter().collect();
        let (emails, not_found, read_at) = self
            .get_emails_at(&mut st, &email_ids, &["id", "mailboxIds"])
            .await?;
        // A message that is in other mailboxes too (a label) only leaves this one; the rest is
        // destroyed.
        let mut destroy: Vec<String> = Vec::new();
        let mut update: serde_json::Map<String, Value> = serde_json::Map::new();
        let mut leave: Vec<String> = not_found;
        for e in emails.iter().filter(|e| requested.contains(&e.id)) {
            let boxes = e.mailboxes();
            if !boxes.contains(mb.id.as_str()) {
                leave.push(e.id.clone());
            } else if boxes.len() == 1 {
                destroy.push(e.id.clone());
            } else {
                update.insert(
                    e.id.clone(),
                    json!({ format!("mailboxIds/{}", pointer_escape(&mb.id)): Value::Null }),
                );
            }
        }
        let mut work: Vec<DeleteStep> = update
            .into_iter()
            .map(|(id, patch)| DeleteStep::Leave(id, patch))
            .collect();
        work.extend(destroy.into_iter().map(DeleteStep::Destroy));
        let mut failure: Option<BackendError> = None;
        let mut gone: Vec<String> = leave;
        // `ifInState`: the decision above was made from the state read first. If anything changed
        // since (another client filed the message elsewhere, say), the server refuses with
        // `stateMismatch`, which is retryable, and the retry decides again. After each batch the
        // state it returned is the one the next batch must still find.
        let mut expected_state = read_at;
        for chunk in work.chunks(session.max_set) {
            let mut upd = serde_json::Map::new();
            let mut del: Vec<&String> = Vec::new();
            for step in chunk {
                match step {
                    DeleteStep::Leave(id, patch) => {
                        upd.insert(id.clone(), patch.clone());
                    }
                    DeleteStep::Destroy(id) => del.push(id),
                }
            }
            let mut args = json!({"accountId": session.account_id, "update": upd, "destroy": del});
            if let Some(state) = &expected_state {
                args["ifInState"] = json!(state);
            }
            let answer = match self.call(&mut st, vec![("Email/set", args)]).await {
                Ok(mut r) => r.take(0, "Email/set").map_err(ApiError::into_backend),
                Err(e) => Err(e),
            };
            let answer = match answer {
                Ok(a) => a,
                Err(e) => {
                    failure = Some(e);
                    break;
                }
            };
            expected_state = answer
                .get("newState")
                .and_then(Value::as_str)
                .map(str::to_string)
                .or(expected_state);
            let set = parse_set(&answer);
            gone.extend(set.done);
            for (id, kind) in set.failed {
                match from_set_error(&kind) {
                    None => gone.push(id), // already gone
                    Some(e) => failure = failure.or(Some(e)),
                }
            }
        }
        // Never act on an id that was not asked for, whatever the server said.
        gone.retain(|id| requested.contains(id));
        if !gone.is_empty() {
            self.store(&mut st, &mb.id, self.ids.remove(&mb.id, &gone))?;
        }
        if let Some(cache) = st.caches.get_mut(&mb.id) {
            for id in &gone {
                cache.messages.remove(id);
            }
        }
        match failure {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }
}

impl MailBackend for JmapBackend {
    fn capabilities(&self) -> BackendFuture<'_, Capabilities> {
        Box::pin(self.capabilities_inner())
    }

    fn list_folders(&self) -> BackendFuture<'_, Vec<RemoteFolder>> {
        Box::pin(self.list_folders_inner())
    }

    fn folder_status<'a>(&'a self, folder: &'a str) -> BackendFuture<'a, FolderStatus> {
        Box::pin(self.folder_status_inner(folder))
    }

    fn snapshot<'a>(&'a self, folder: &'a str) -> BackendFuture<'a, RemoteSnapshot> {
        Box::pin(self.snapshot_inner(folder))
    }

    fn fetch_headers<'a>(
        &'a self,
        folder: &'a str,
        uids: &'a [Uid],
    ) -> BackendFuture<'a, Vec<RemoteHeader>> {
        Box::pin(self.fetch_headers_inner(folder, uids))
    }

    /// Differs from the IMAP adapter in one case: when more than one different message in the
    /// folder has this Message-ID, the answer is `None` and not the newest of them, so the engine
    /// stops a destructive action instead of guessing which copy was meant.
    fn find_by_message_id<'a>(
        &'a self,
        folder: &'a str,
        message_id: &'a str,
    ) -> BackendFuture<'a, Option<Uid>> {
        Box::pin(self.find_by_message_id_inner(folder, message_id))
    }

    fn set_flag<'a>(
        &'a self,
        folder: &'a str,
        uids: &'a [Uid],
        change: FlagChange,
    ) -> BackendFuture<'a, ()> {
        Box::pin(self.set_flag_inner(folder, uids, change))
    }

    fn move_to<'a>(
        &'a self,
        folder: &'a str,
        uids: &'a [Uid],
        dest: &'a str,
    ) -> BackendFuture<'a, MoveOutcome> {
        Box::pin(self.move_to_inner(folder, uids, dest))
    }

    fn delete_forever<'a>(&'a self, folder: &'a str, uids: &'a [Uid]) -> BackendFuture<'a, ()> {
        Box::pin(self.delete_forever_inner(folder, uids))
    }

    /// Downloads the message's blob (`Email/get` of `blobId` and `size` first). Downloading never
    /// changes `$seen`.
    fn fetch_body<'a>(&'a self, folder: &'a str, uid: Uid) -> BackendFuture<'a, RawMessage> {
        Box::pin(self.fetch_body_inner(folder, uid))
    }
}
