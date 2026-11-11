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
        if unchanged(&stored, &status) {
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
    async fn apply_snapshot(
        &self,
        folder_id: i64,
        path: &str,
        snapshot: &RemoteSnapshot,
    ) -> Result<bool, EngineError> {
        let account = self.account();
        let reset = {
            let q = self.shared.lock();
            plan(&self.local_folder(&q, folder_id, path)?, snapshot).reset
        };
        if reset && !self.reset_folder(folder_id, path, snapshot).await? {
            // An action changed what points into the folder while we looked things up: the
            // folder still has its old UIDVALIDITY, so the next sync starts the reset again.
            return Ok(false);
        }

        let (fetch, changed) = {
            let mut q = self.shared.lock();
            let local = self.local_folder(&q, folder_id, path)?;
            let plan = plan(&local, snapshot);
            if plan.reset {
                return Ok(false);
            }
            let keep = local.pending;
            let changed = q.store_mut().db_mut().transaction(|tx| {
                // Design note section 6: a hidden row or a pending count with no unfinished
                // operation behind it is left over, and nothing else would ever clear it.
                let mut n = messages::release_local_state(tx, account, folder_id, &keep)?;
                for (uid, flags) in &plan.update_flags {
                    if !keep.contains(uid)
                        && messages::set_server_flags(tx, account, folder_id, *uid, flags)?
                    {
                        n += 1;
                    }
                }
                // Never a row that still has an unfinished operation: let that operation resolve.
                let gone: Vec<Uid> = plan
                    .remove
                    .iter()
                    .filter(|u| !keep.contains(u))
                    .copied()
                    .collect();
                n += messages::delete_uids(tx, account, folder_id, &gone)?;
                Ok::<_, StorageError>(n > 0)
            })?;
            (plan.fetch, changed)
        };
        if changed {
            self.emit_folder(folder_id);
        }

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
        let (targets, moved_out, moved_in) = {
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
            (targets, moved_out, moved_in)
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
            if !same {
                return Ok(false);
            }
            let reset = FolderReset {
                folder: path.to_owned(),
                status: snapshot.status,
                synced_at_ms: self.shared.now(),
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

    async fn fetch_one(&self, folder: &str, uid: Uid) -> Result<Option<RemoteHeader>, EngineError> {
        match self.backend().fetch_headers(folder, &[uid]).await {
            Ok(headers) => Ok(headers.into_iter().find(|h| h.uid == uid)),
            Err(BackendError::Permanent(_)) => Ok(None),
            Err(e) => Err(self.backend_error(e)),
        }
    }
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
/// UIDVALIDITY, UIDNEXT and HIGHESTMODSEQ. Without CONDSTORE a snapshot is always taken.
fn unchanged(stored: &Folder, status: &croft_protocols::FolderStatus) -> bool {
    let Some(modseq) = status.highest_modseq else {
        return false;
    };
    stored.uid_validity == Some(i64::from(status.uid_validity))
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
            exists: 0,
        }
    }

    #[test]
    fn a_snapshot_is_skipped_only_when_condstore_says_nothing_changed() {
        assert!(unchanged(&stored(7, 10, Some(5)), &status(7, 10, Some(5))));
        assert!(!unchanged(&stored(7, 10, Some(5)), &status(7, 10, Some(6))));
        assert!(!unchanged(&stored(7, 10, Some(5)), &status(7, 11, Some(5))));
        assert!(!unchanged(&stored(7, 10, Some(5)), &status(8, 10, Some(5))));
        assert!(!unchanged(&stored(7, 10, None), &status(7, 10, None)));
        assert!(!unchanged(&stored(7, 10, None), &status(7, 10, Some(5))));
    }
}
