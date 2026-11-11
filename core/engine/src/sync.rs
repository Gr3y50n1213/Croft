// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Folder sync: the folder list, one folder against the server's snapshot, and a UIDVALIDITY
//! reset (design note sections 4.3 and 6).
//!
//! Every network call happens with no lock held. The plan is worked out again under the lock right
//! before it is applied, with the queue's pending set as it is at that moment, so an action the
//! person takes while a snapshot is on its way is never overwritten by it.

use std::collections::{BTreeMap, BTreeSet};

use croft_mail_core::{plan, AwaitingUid, FolderReset, LocalFolder, MovedOut};
use croft_protocols::{BackendError, RemoteHeader, RemoteSnapshot, SpecialUse, Uid};
use croft_storage::folders::{self, Folder};
use croft_storage::{messages, StorageError};

use crate::engine::{Change, EngineError, Queue};
use crate::worker::{Worker, REASON_LOST_AFTER_RESET};

/// How many rows one lock hold may change (server flags, removals, a reset's deletions). Each
/// row removed fires the counter and full-text triggers, so a folder of thousands of rows in one
/// hold would keep the UI's own actions waiting. Between chunks the lock is let go and the thread
/// is given back to the runtime.
const LOCK_CHUNK: usize = 500;

impl Worker {
    /// Syncs the folder list, then every selectable folder (Inbox first), then resolves awaiting
    /// entries whose destination the server does not list. A folder the server refuses is
    /// skipped; any other error stops the sync.
    pub(crate) async fn sync_all(&self) -> Result<(), EngineError> {
        let account = self.account();
        let mut remote = self
            .backend()
            .list_folders()
            .await
            .map_err(|e| self.backend_error(e))?;
        let changed = {
            let mut q = self.shared.lock();
            q.store_mut().db_mut().transaction(|tx| {
                let before = folders::list(tx, account)?;
                for f in &remote {
                    folders::upsert(tx, account, f)?;
                }
                let after = folders::list(tx, account)?;
                Ok::<_, StorageError>(shape(&before) != shape(&after))
            })?
        };
        if changed {
            self.shared.emit(Change::Folders { account });
        }
        let listed: Vec<String> = remote.iter().map(|f| f.path.clone()).collect();
        remote.retain(|f| f.selectable);
        remote.sort_by_key(|f| f.special_use != SpecialUse::Inbox);
        for f in &remote {
            if self.stop_now() {
                return Ok(());
            }
            match self.sync_folder(&f.path).await {
                Ok(()) => {}
                Err(EngineError::Backend(BackendError::Permanent(_))) => {
                    log::warn!("account {account}: the server refused to sync one folder");
                }
                Err(e) => return Err(e),
            }
        }
        // Moves into a folder the server no longer lists are never looked for by a sync of it:
        // look now (the lookups fail and, after the third, the later operations are given up).
        self.resolve_awaiting_into(None, &listed).await
    }

    /// Syncs one folder: status, snapshot (skipped when CONDSTORE says nothing changed), plan,
    /// apply, fetch new headers newest first, store the status, then look for messages moved here
    /// whose new UID is not known yet.
    pub(crate) async fn sync_folder(&self, path: &str) -> Result<(), EngineError> {
        let account = self.account();
        let stored = {
            let q = self.shared.lock();
            folders::find_by_path(q.store().db().connection(), account, path)?
        };
        let Some(stored) = stored else {
            return Ok(());
        };
        let status = self
            .backend()
            .folder_status(path)
            .await
            .map_err(|e| self.backend_error(e))?;
        let local_rows = {
            let q = self.shared.lock();
            messages::count_in_folder(q.store().db().connection(), account, stored.id)?
        };
        if unchanged(&stored, &status, local_rows) {
            let q = self.shared.lock();
            folders::update_status(
                q.store().db().connection(),
                account,
                stored.id,
                &status,
                self.shared.now(),
            )?;
        } else {
            let snapshot = self
                .backend()
                .snapshot(path)
                .await
                .map_err(|e| self.backend_error(e))?;
            if !self.apply_snapshot(stored.id, path, &snapshot).await? {
                // A reset was put off, so the folder's rows are still in the old numbering: a
                // message found here now must not be copied over them.
                return Ok(());
            }
        }
        self.resolve_awaiting_into(Some(path), &[]).await
    }

    /// The local side of the plan: stored UIDVALIDITY, every row's flags, and the protected set.
    fn local_folder(
        &self,
        q: &Queue,
        folder_id: i64,
        path: &str,
    ) -> Result<LocalFolder, StorageError> {
        let account = self.account();
        let conn = q.store().db().connection();
        let uid_validity = folders::get(conn, account, folder_id)?
            .and_then(|f| f.uid_validity)
            .and_then(|v| u32::try_from(v).ok());
        Ok(LocalFolder {
            uid_validity,
            messages: messages::folder_flags(conn, account, folder_id)?,
            pending: self.protected(q, path),
        })
    }

    /// UIDs in the folder whose rows must keep their local state: an unfinished operation targets
    /// them, or a move from there is waiting for its new UID (its old row is kept until then).
    fn protected(&self, q: &Queue, path: &str) -> BTreeSet<Uid> {
        let account = self.account();
        let mut keep = q.queue().pending_uids(account, path);
        keep.extend(
            q.queue()
                .awaiting()
                .filter(|a| a.account == account && a.folder == path)
                .map(|a| a.uid),
        );
        keep
    }

    /// Applies a snapshot. Returns false when the folder's UIDVALIDITY changed and the reset was
    /// put off (the local rows are still in the old numbering).
    ///
    /// The folder's rows are read **once** (a plain SELECT) and the plan is worked out with no lock
    /// held. Server flags and removals are then written in chunks of [`LOCK_CHUNK`] rows, one lock
    /// hold per chunk, and the protected set (rows with an unfinished operation, awaiting
    /// entries) is read again under each chunk's lock: an action taken between two chunks is
    /// respected, and the UI is never kept waiting for the whole folder.
    async fn apply_snapshot(
        &self,
        folder_id: i64,
        path: &str,
        snapshot: &RemoteSnapshot,
    ) -> Result<bool, EngineError> {
        let account = self.account();
        let local = {
            let q = self.shared.lock();
            self.local_folder(&q, folder_id, path)?
        };
        let mut todo = plan(&local, snapshot);
        if todo.reset {
            if !self.reset_folder(folder_id, path, snapshot).await? {
                // An action changed what points into the folder while we looked things up: the
                // folder still has its old UIDVALIDITY, so the next sync starts the reset again.
                return Ok(false);
            }
            // The folder now holds only the rows the re-pointed operations needed.
            let local = {
                let q = self.shared.lock();
                self.local_folder(&q, folder_id, path)?
            };
            todo = plan(&local, snapshot);
            if todo.reset {
                return Ok(false);
            }
        }

        // Design note section 6: a hidden row or a pending count with no unfinished operation
        // behind it is left over, and nothing else would ever clear it.
        let released = {
            let mut q = self.shared.lock();
            let keep = self.protected(&q, path);
            q.store_mut()
                .db_mut()
                .transaction(|tx| messages::release_local_state(tx, account, folder_id, &keep))?
        };
        if released > 0 {
            self.emit_folder(folder_id);
        }
        for chunk in todo.update_flags.chunks(LOCK_CHUNK) {
            tokio::task::yield_now().await;
            if self.stop_now() {
                return Ok(true);
            }
            let changed = {
                let mut q = self.shared.lock();
                let keep = self.protected(&q, path);
                q.store_mut().db_mut().transaction(|tx| {
                    let mut n = 0;
                    for (uid, flags) in chunk {
                        if !keep.contains(uid)
                            && messages::set_server_flags(tx, account, folder_id, *uid, flags)?
                        {
                            n += 1;
                        }
                    }
                    Ok::<_, StorageError>(n)
                })?
            };
            if changed > 0 {
                self.emit_folder(folder_id);
            }
        }
        for chunk in todo.remove.chunks(LOCK_CHUNK) {
            tokio::task::yield_now().await;
            if self.stop_now() {
                return Ok(true);
            }
            let removed = {
                let mut q = self.shared.lock();
                // Never a row that still has an unfinished operation: let that operation resolve.
                // (Read under this chunk's lock: the person may have acted since the plan.)
                let keep = self.protected(&q, path);
                let gone: Vec<Uid> = chunk
                    .iter()
                    .filter(|u| !keep.contains(u))
                    .copied()
                    .collect();
                q.store_mut()
                    .db_mut()
                    .transaction(|tx| messages::delete_uids(tx, account, folder_id, &gone))?
            };
            if removed > 0 {
                self.emit_folder(folder_id);
            }
        }
        let fetch = todo.fetch;

        for chunk in fetch.chunks(self.shared.config.fetch_chunk.max(1)) {
            if self.stop_now() {
                return Ok(true);
            }
            let headers = self
                .backend()
                .fetch_headers(path, chunk)
                .await
                .map_err(|e| self.backend_error(e))?;
            {
                let mut q = self.shared.lock();
                q.store_mut().db_mut().transaction(|tx| {
                    for h in &headers {
                        messages::upsert(tx, account, folder_id, h)?;
                    }
                    Ok::<_, StorageError>(())
                })?;
            }
            self.emit_folder(folder_id);
        }

        let q = self.shared.lock();
        folders::update_status(
            q.store().db().connection(),
            account,
            folder_id,
            &snapshot.status,
            self.shared.now(),
        )?;
        Ok(true)
    }

    fn emit_folder(&self, folder_id: i64) {
        self.shared.emit(Change::Messages {
            account: self.account(),
            folders: vec![folder_id],
        });
    }

    /// The UIDs in the folder whose operations must be matched again after a reset (awaiting
    /// sources are handled as `moved_out`), with the Message-ID of their local row.
    fn reset_targets(
        &self,
        q: &Queue,
        folder_id: i64,
        path: &str,
    ) -> Result<BTreeMap<Uid, Option<String>>, StorageError> {
        let account = self.account();
        let sources = awaiting_from(q, account, path);
        let mut targets = BTreeMap::new();
        for uid in q.queue().pending_uids(account, path) {
            if sources.contains(&uid) {
                continue;
            }
            let id = messages::find_by_uid(q.store().db().connection(), account, folder_id, uid)?
                .and_then(|m| m.message_id_hdr);
            targets.insert(uid, id);
        }
        Ok(targets)
    }

    /// A Message-ID lookup that treats a refusal (folder gone) as "not found".
    async fn find(&self, folder: &str, message_id: &str) -> Result<Option<Uid>, EngineError> {
        match self.backend().find_by_message_id(folder, message_id).await {
            Ok(found) => Ok(found),
            Err(BackendError::Permanent(_)) => Ok(None),
            Err(e) => Err(self.backend_error(e)),
        }
    }

    /// The folder's UIDVALIDITY changed (design note section 4, case 3, and section 6). Every
    /// operation that points into the folder is matched again by Message-ID, then the folder's
    /// rows, its new status and the operations change in **one** transaction
    /// ([`croft_mail_core::PersistentQueue::reset_folder`]).
    ///
    /// All lookups happen first, with no lock and nothing changed: a temporary failure leaves the
    /// folder as it was, and the next sync starts again. Returns false when an action changed
    /// what points into the folder in the meantime (nothing was changed either).
    ///
    /// Awaiting entries whose **source** is this folder (moved out, new UID never learned): their
    /// old row went with the folder and the old address may now be another message, so they are
    /// resolved here and never left waiting: looked up in their destination, re-pointed if found,
    /// failed if not (design note section 6 left this case to T04). Entries whose
    /// **destination** is this folder and that are found are re-pointed here; the others stay
    /// waiting and are counted by the usual lookup after the sync.
    async fn reset_folder(
        &self,
        folder_id: i64,
        path: &str,
        snapshot: &RemoteSnapshot,
    ) -> Result<bool, EngineError> {
        let account = self.account();
        let (targets, moved_out, moved_in, danger) = {
            let q = self.shared.lock();
            let targets = self.reset_targets(&q, folder_id, path)?;
            let awaiting: Vec<AwaitingUid> = q
                .queue()
                .awaiting()
                .filter(|a| a.account == account)
                .cloned()
                .collect();
            let moved_out: Vec<AwaitingUid> = awaiting
                .iter()
                .filter(|a| a.folder == path)
                .cloned()
                .collect();
            let moved_in: Vec<AwaitingUid> = awaiting
                .into_iter()
                .filter(|a| a.dest_folder == path && a.folder != path)
                .collect();
            let danger = self.reset_danger(&q, folder_id, path, &targets, &moved_out, &moved_in)?;
            (targets, moved_out, moved_in, danger)
        };

        // Lookups, with no lock held.
        let mut matched = Vec::new();
        let mut unmatched = Vec::new();
        for (old, id) in &targets {
            let found = match id {
                Some(id) => self.find(path, id).await?,
                None => None,
            };
            match found {
                Some(new) => matched.push((*old, new)),
                None => unmatched.push(*old),
            }
        }
        let mut out = Vec::new();
        for entry in &moved_out {
            let found = match &entry.message_id {
                Some(id) => self.find(&entry.dest_folder, id).await?,
                None => None,
            };
            // A destructive operation waits on this message: if the destination holds another
            // copy of the Message-ID, the UID found may be the other one. Give up, do not guess.
            let found = match (found, &entry.message_id) {
                (Some(new), Some(id))
                    if danger.moved_out.contains(&entry.uid)
                        && self.other_copies(&entry.dest_folder, id, new)? =>
                {
                    None
                }
                (found, _) => found,
            };
            let found = match found {
                Some(new) => self
                    .fetch_one(&entry.dest_folder, new)
                    .await?
                    .map(|h| ((entry.dest_folder.clone(), new), h)),
                None => None,
            };
            out.push(MovedOut {
                old_uid: entry.uid,
                found,
            });
        }
        let mut moved_in_found = Vec::new();
        for entry in &moved_in {
            if let Some(id) = &entry.message_id {
                if let Some(new) = self.find(path, id).await? {
                    moved_in_found.push(((entry.folder.clone(), entry.uid), new));
                }
            }
        }
        // Two old UIDs that map to one new UID cannot be told apart (the server names one UID per
        // Message-ID). Operations that destroy something are not re-pointed on a guess: they fail.
        let mut mapped: BTreeMap<Uid, usize> = BTreeMap::new();
        for new in matched
            .iter()
            .map(|(_, new)| *new)
            .chain(moved_in_found.iter().map(|(_, new)| *new))
        {
            *mapped.entry(new).or_default() += 1;
        }
        let shared_new = |new: &Uid| mapped.get(new).is_some_and(|n| *n > 1);
        let (matched, guesses): (Vec<_>, Vec<_>) = matched.into_iter().partition(|(old, new)| {
            !(danger.targets.contains(old) && (danger.shared_id.contains(old) || shared_new(new)))
        });
        unmatched.extend(guesses.into_iter().map(|(old, _)| old));
        moved_in_found.retain(|(from, new)| !(danger.moved_in.contains(from) && shared_new(new)));
        let wanted: BTreeSet<Uid> = matched
            .iter()
            .map(|(_, new)| *new)
            .chain(moved_in_found.iter().map(|(_, new)| *new))
            .collect();
        let wanted: Vec<Uid> = wanted.into_iter().rev().collect();
        let headers = if wanted.is_empty() {
            Vec::new()
        } else {
            self.backend()
                .fetch_headers(path, &wanted)
                .await
                .map_err(|e| self.backend_error(e))?
        };
        // A message found by Message-ID but gone before its header was read has no row for its
        // operations to land on: treat it as not found.
        let have: BTreeSet<Uid> = headers.iter().map(|h| h.uid).collect();
        let (matched, lost): (Vec<_>, Vec<_>) =
            matched.into_iter().partition(|(_, new)| have.contains(new));
        unmatched.extend(lost.into_iter().map(|(old, _)| old));
        moved_in_found.retain(|(_, new)| have.contains(new));

        // The folder's rows are deleted in chunks, before the one batch that renumbers the
        // operations: that batch then clears only the few rows that are left (those with
        // operations) and stays short. Safe to be cut short: the UIDVALIDITY is stored only by
        // that batch, so the next sync sees the old one and starts the reset again.
        if !self.clear_rows_in_chunks(folder_id, path).await? {
            return Ok(false);
        }
        let failed = {
            let mut q = self.shared.lock();
            let still_targets: BTreeSet<Uid> = self
                .reset_targets(&q, folder_id, path)?
                .into_keys()
                .collect();
            let still_out = awaiting_from(&q, account, path);
            let still_in: BTreeSet<(String, Uid)> = q
                .queue()
                .awaiting()
                .filter(|a| a.account == account && a.dest_folder == path && a.folder != path)
                .map(|a| (a.folder.clone(), a.uid))
                .collect();
            let same = still_targets == targets.keys().copied().collect()
                && still_out == moved_out.iter().map(|a| a.uid).collect()
                && still_in == moved_in.iter().map(|a| (a.folder.clone(), a.uid)).collect();
            // The danger set was worked out before the lookups and the chunk yields: an action
            // since then (a pending flag change turned into a delete, say) may have changed it.
            // Do not go on with the old answer; the reset starts again at the next sync.
            if !same || !self.danger_holds(&q, path, &targets, &danger, &moved_out, &moved_in)? {
                return Ok(false);
            }
            let reset = FolderReset {
                folder: path.to_owned(),
                uid_validity: snapshot.status.uid_validity,
                headers,
                matched,
                unmatched,
                moved_out: out.clone(),
                moved_in: moved_in_found.clone(),
                reason: REASON_LOST_AFTER_RESET.to_owned(),
            };
            q.reset_folder(account, &reset)?
        };
        self.emit_failed(&failed, REASON_LOST_AFTER_RESET);
        let mut paths: Vec<&str> = vec![path];
        paths.extend(
            out.iter()
                .filter_map(|m| m.found.as_ref().map(|((f, _), _)| f.as_str())),
        );
        paths.extend(moved_in_found.iter().map(|((f, _), _)| f.as_str()));
        self.shared.emit_folders(account, &paths);
        Ok(true)
    }

    /// Deletes the folder's rows in chunks of [`LOCK_CHUNK`], one lock hold each, except rows that
    /// an unfinished operation or awaiting entry points at, and every other row with the same
    /// Message-ID as a row that has a destructive operation (see [`Worker::evidence_copies`]);
    /// both are re-read under every chunk's lock. The reset needs them, and the final batch
    /// removes them. Returns false when the engine is stopping.
    async fn clear_rows_in_chunks(&self, folder_id: i64, path: &str) -> Result<bool, EngineError> {
        let account = self.account();
        let uids = {
            let q = self.shared.lock();
            messages::uids_in_folder(q.store().db().connection(), account, folder_id)?
        };
        for chunk in uids.chunks(LOCK_CHUNK) {
            tokio::task::yield_now().await;
            if self.stop_now() {
                return Ok(false);
            }
            {
                let mut q = self.shared.lock();
                let mut keep = self.protected(&q, path);
                keep.extend(self.evidence_copies(&q, folder_id, path)?);
                let gone: Vec<Uid> = chunk
                    .iter()
                    .filter(|u| !keep.contains(u))
                    .copied()
                    .collect();
                q.store_mut()
                    .db_mut()
                    .transaction(|tx| messages::delete_uids(tx, account, folder_id, &gone))?;
            }
            self.emit_folder(folder_id);
        }
        Ok(true)
    }

    /// The rows of the folder that show a destructive operation's message to have more than one
    /// copy: every row with the same Message-ID as a row that has a delete forever or a move to
    /// Trash or Junk waiting. A reset that is cut short leaves the old UIDVALIDITY stored, so the
    /// next one starts again and counts the copies once more; if an earlier chunk had deleted the
    /// other copy, the count would say "one" and the operation would be re-pointed to whichever
    /// message the server names. The final batch removes these rows with the rest.
    fn evidence_copies(
        &self,
        q: &Queue,
        folder_id: i64,
        path: &str,
    ) -> Result<BTreeSet<Uid>, StorageError> {
        let mut ids: Vec<String> = Vec::new();
        for (uid, id) in self.reset_targets(q, folder_id, path)? {
            if let Some(id) = id {
                if self.has_destructive_op(q, (path, uid))? {
                    ids.push(id);
                }
            }
        }
        if ids.is_empty() {
            return Ok(BTreeSet::new());
        }
        let ids: Vec<&str> = ids.iter().map(String::as_str).collect();
        messages::uids_with_message_ids(
            q.store().db().connection(),
            self.account(),
            folder_id,
            &ids,
        )
    }

    /// Which of the reset's operations must not be re-pointed on a guess (see [`Danger`]).
    fn reset_danger(
        &self,
        q: &Queue,
        folder_id: i64,
        path: &str,
        targets: &BTreeMap<Uid, Option<String>>,
        moved_out: &[AwaitingUid],
        moved_in: &[AwaitingUid],
    ) -> Result<Danger, StorageError> {
        let account = self.account();
        let conn = q.store().db().connection();
        let mut danger = Danger::default();
        for (uid, id) in targets {
            if !self.has_destructive_op(q, (path, *uid))? {
                continue;
            }
            danger.targets.insert(*uid);
            // The same Message-ID on another row of the folder: two old UIDs, one new UID.
            if let Some(id) = id {
                if messages::count_other_copies(conn, account, folder_id, id, *uid)? > 0 {
                    danger.shared_id.insert(*uid);
                }
            }
        }
        for entry in moved_out {
            if self.has_destructive_op(q, (path, entry.uid))? {
                danger.moved_out.insert(entry.uid);
            }
        }
        for entry in moved_in {
            if self.has_destructive_op(q, (entry.folder.as_str(), entry.uid))? {
                danger.moved_in.insert((entry.folder.clone(), entry.uid));
            }
        }
        Ok(danger)
    }

    /// True when every operation still answers "destructive or not" as [`Danger`] says it did.
    fn danger_holds(
        &self,
        q: &Queue,
        path: &str,
        targets: &BTreeMap<Uid, Option<String>>,
        danger: &Danger,
        moved_out: &[AwaitingUid],
        moved_in: &[AwaitingUid],
    ) -> Result<bool, StorageError> {
        for uid in targets.keys() {
            if self.has_destructive_op(q, (path, *uid))? != danger.targets.contains(uid) {
                return Ok(false);
            }
        }
        for entry in moved_out {
            let now = self.has_destructive_op(q, (path, entry.uid))?;
            if now != danger.moved_out.contains(&entry.uid) {
                return Ok(false);
            }
        }
        for entry in moved_in {
            let now = self.has_destructive_op(q, (entry.folder.as_str(), entry.uid))?;
            if now != danger.moved_in.contains(&(entry.folder.clone(), entry.uid)) {
                return Ok(false);
            }
        }
        Ok(true)
    }

    async fn fetch_one(&self, folder: &str, uid: Uid) -> Result<Option<RemoteHeader>, EngineError> {
        match self.backend().fetch_headers(folder, &[uid]).await {
            Ok(headers) => Ok(headers.into_iter().find(|h| h.uid == uid)),
            Err(BackendError::Permanent(_)) => Ok(None),
            Err(e) => Err(self.backend_error(e)),
        }
    }
}

/// What a reset must not guess about: operations that destroy something (delete forever, a move
/// to Trash or Junk) on messages that are matched again by Message-ID.
#[derive(Debug, Default)]
struct Danger {
    /// Old UIDs in the reset folder with such an operation.
    targets: BTreeSet<Uid>,
    /// The subset of `targets` whose Message-ID is on another row of the folder, so two old UIDs
    /// name the same message.
    shared_id: BTreeSet<Uid>,
    /// Awaiting entries moved out of the folder (old UID) with such an operation.
    moved_out: BTreeSet<Uid>,
    /// Awaiting entries moved into the folder (old address) with such an operation.
    moved_in: BTreeSet<(String, Uid)>,
}

/// Old UIDs in `path` that have an awaiting entry (moves out of the folder waiting for their new
/// UID).
fn awaiting_from(q: &Queue, account: i64, path: &str) -> BTreeSet<Uid> {
    q.queue()
        .awaiting()
        .filter(|a| a.account == account && a.folder == path)
        .map(|a| a.uid)
        .collect()
}

/// What the folder list screen shows, to tell whether the list changed.
fn shape(list: &[Folder]) -> Vec<(&str, &str)> {
    list.iter()
        .map(|f| (f.path.as_str(), f.special_use.as_str()))
        .collect()
}

/// True when CONDSTORE says nothing in the folder changed since the stored status: same
/// UIDVALIDITY, UIDNEXT and HIGHESTMODSEQ, **and** the local row count is the server's message
/// count. The count is a second guard: a folder whose messages were never (or only partly)
/// stored, however that happened, is never taken for up to date. Without CONDSTORE a snapshot is
/// always taken.
fn unchanged(stored: &Folder, status: &croft_protocols::FolderStatus, local_rows: u64) -> bool {
    let Some(modseq) = status.highest_modseq else {
        return false;
    };
    local_rows == u64::from(status.exists)
        && stored.uid_validity == Some(i64::from(status.uid_validity))
        && stored.uid_next == Some(i64::from(status.uid_next))
        && stored
            .highest_modseq
            .is_some_and(|m| u64::try_from(m) == Ok(modseq))
}

#[cfg(test)]
mod tests {
    use super::*;
    use croft_protocols::FolderStatus;

    fn stored(validity: i64, next: i64, modseq: Option<i64>) -> Folder {
        Folder {
            id: 1,
            account_id: 1,
            path: "INBOX".into(),
            delimiter: None,
            special_use: "inbox".into(),
            uid_validity: Some(validity),
            uid_next: Some(next),
            highest_modseq: modseq,
            unread_count: 0,
            total_count: 0,
            last_synced_at: None,
        }
    }

    fn status(validity: u32, next: u32, modseq: Option<u64>) -> FolderStatus {
        FolderStatus {
            uid_validity: validity,
            uid_next: next,
            highest_modseq: modseq,
            exists: 3,
        }
    }

    #[test]
    fn a_snapshot_is_skipped_only_when_condstore_says_nothing_changed() {
        let same = |s: &Folder, st: &FolderStatus| unchanged(s, st, 3);
        assert!(same(&stored(7, 10, Some(5)), &status(7, 10, Some(5))));
        assert!(!same(&stored(7, 10, Some(5)), &status(7, 10, Some(6))));
        assert!(!same(&stored(7, 10, Some(5)), &status(7, 11, Some(5))));
        assert!(!same(&stored(7, 10, Some(5)), &status(8, 10, Some(5))));
        assert!(!same(&stored(7, 10, None), &status(7, 10, None)));
        assert!(!same(&stored(7, 10, None), &status(7, 10, Some(5))));
        // Only some of the folder's messages are stored (a sync was cut short): not up to date.
        assert!(!unchanged(
            &stored(7, 10, Some(5)),
            &status(7, 10, Some(5)),
            0
        ));
        assert!(!unchanged(
            &stored(7, 10, Some(5)),
            &status(7, 10, Some(5)),
            2
        ));
        assert!(!unchanged(
            &stored(7, 10, Some(5)),
            &status(7, 10, Some(5)),
            4
        ));
    }
}
