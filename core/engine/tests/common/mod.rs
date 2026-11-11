// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Test helpers: an in-memory [`MailBackend`] with scriptable failures, and a rig that opens an
//! engine on a database file so a test can "kill" and restart it.

#![allow(dead_code, clippy::unwrap_used, clippy::expect_used)]

use std::collections::{BTreeMap, VecDeque};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use croft_engine::{AccountId, Change, Clock, Engine, EngineConfig, EngineHandle, ManualClock};
use croft_protocols::{
    BackendError, BackendFuture, Capabilities, FlagChange, Flags, FolderStatus, MailBackend,
    MoveOutcome, RemoteFolder, RemoteHeader, RemoteSnapshot, SpecialUse, Uid,
};
use croft_storage::accounts::{self, NewAccount, Protocol};
use croft_storage::messages::{self, Message};
use croft_storage::{folders, Db};
use tokio::sync::{broadcast, Notify};

/// Which backend call, for scripted failures and gates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Method {
    ListFolders,
    Status,
    Snapshot,
    Fetch,
    Find,
    SetFlag,
    Move,
    Delete,
}

/// One call the backend received, with the test clock's time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Call {
    ListFolders,
    Status(String),
    Snapshot(String),
    Fetch(String, Vec<Uid>),
    Find(String, String),
    SetFlag(String, Vec<Uid>, FlagChange),
    Move(String, Vec<Uid>, String),
    Delete(String, Vec<Uid>),
}

impl Call {
    fn method(&self) -> Method {
        match self {
            Call::ListFolders => Method::ListFolders,
            Call::Status(_) => Method::Status,
            Call::Snapshot(_) => Method::Snapshot,
            Call::Fetch(..) => Method::Fetch,
            Call::Find(..) => Method::Find,
            Call::SetFlag(..) => Method::SetFlag,
            Call::Move(..) => Method::Move,
            Call::Delete(..) => Method::Delete,
        }
    }

    /// True for calls that change something on the server.
    pub fn is_write(&self) -> bool {
        matches!(self, Call::SetFlag(..) | Call::Move(..) | Call::Delete(..))
    }
}

#[derive(Debug, Clone)]
pub struct FakeMessage {
    pub message_id: String,
    pub subject: String,
    pub flags: Flags,
    pub date_ms: i64,
}

#[derive(Debug, Clone)]
pub struct FakeFolder {
    pub special: SpecialUse,
    pub uid_validity: u32,
    pub uid_next: u32,
    pub messages: BTreeMap<Uid, FakeMessage>,
}

/// A gate: the call is recorded, then waits until the test opens the gate.
#[derive(Default)]
pub struct Gate {
    pub open: Notify,
}

pub struct FakeState {
    pub folders: BTreeMap<String, FakeFolder>,
    pub calls: Vec<(i64, Call)>,
    /// COPYUID in move answers.
    pub uidplus: bool,
    /// Every call fails with this (airplane mode, server down).
    pub fail_all: Option<BackendError>,
    /// The next call of a method fails with the given error.
    pub fail_next: VecDeque<(Method, BackendError)>,
    /// `find_by_message_id` answers "not found" this many times before it works (a server whose
    /// search index lags behind a move).
    pub search_misses: u32,
    pub gates: BTreeMap<Method, Arc<Gate>>,
}

/// An in-memory mail server.
pub struct FakeBackend {
    pub state: Mutex<FakeState>,
    clock: Arc<dyn Clock>,
}

impl FakeBackend {
    /// INBOX, Archive and Trash, empty, UIDVALIDITY 100.
    pub fn new(clock: Arc<dyn Clock>) -> Arc<Self> {
        let mut folders = BTreeMap::new();
        for (name, special) in [
            ("INBOX", SpecialUse::Inbox),
            ("Archive", SpecialUse::Archive),
            ("Trash", SpecialUse::Trash),
        ] {
            folders.insert(
                name.to_owned(),
                FakeFolder {
                    special,
                    uid_validity: 100,
                    uid_next: 1,
                    messages: BTreeMap::new(),
                },
            );
        }
        Arc::new(Self {
            state: Mutex::new(FakeState {
                folders,
                calls: Vec::new(),
                uidplus: true,
                fail_all: None,
                fail_next: VecDeque::new(),
                search_misses: 0,
                gates: BTreeMap::new(),
            }),
            clock,
        })
    }

    pub fn st(&self) -> std::sync::MutexGuard<'_, FakeState> {
        self.state.lock().unwrap()
    }

    /// Adds a message (Message-ID `<id>`) and returns its UID.
    pub fn add(&self, folder: &str, id: &str, flags: Flags) -> Uid {
        let mut st = self.st();
        let f = st.folders.get_mut(folder).unwrap();
        let uid = f.uid_next;
        f.uid_next += 1;
        f.messages.insert(
            uid,
            FakeMessage {
                message_id: format!("<{id}>"),
                subject: format!("Subject {id}"),
                flags,
                date_ms: 1_000 + i64::from(uid),
            },
        );
        uid
    }

    pub fn flags(&self, folder: &str, uid: Uid) -> Option<Flags> {
        self.st()
            .folders
            .get(folder)?
            .messages
            .get(&uid)
            .map(|m| m.flags.clone())
    }

    /// The UID of the message with this Message-ID (`<id>`) in a folder.
    pub fn uid_of(&self, folder: &str, id: &str) -> Option<Uid> {
        let want = format!("<{id}>");
        self.st()
            .folders
            .get(folder)?
            .messages
            .iter()
            .find(|(_, m)| m.message_id == want)
            .map(|(u, _)| *u)
    }

    /// Changes flags as another client would.
    pub fn set_server_flags(&self, folder: &str, uid: Uid, change: impl FnOnce(&mut Flags)) {
        let mut st = self.st();
        let m = st
            .folders
            .get_mut(folder)
            .unwrap()
            .messages
            .get_mut(&uid)
            .unwrap();
        change(&mut m.flags);
    }

    /// The folder was deleted and recreated: new UIDVALIDITY, messages renumbered from 1 in
    /// reverse order (so old and new UIDs collide in both directions).
    pub fn change_uid_validity(&self, folder: &str, validity: u32) {
        let mut st = self.st();
        let f = st.folders.get_mut(folder).unwrap();
        f.uid_validity = validity;
        let old: Vec<FakeMessage> = f.messages.values().rev().cloned().collect();
        f.messages = old
            .into_iter()
            .enumerate()
            .map(|(i, m)| (u32::try_from(i + 1).unwrap(), m))
            .collect();
        f.uid_next = u32::try_from(f.messages.len() + 1).unwrap();
    }

    pub fn calls(&self) -> Vec<Call> {
        self.st().calls.iter().map(|(_, c)| c.clone()).collect()
    }

    pub fn timed_calls(&self) -> Vec<(i64, Call)> {
        self.st().calls.clone()
    }

    pub fn writes(&self) -> Vec<Call> {
        self.calls().into_iter().filter(Call::is_write).collect()
    }

    pub fn clear_calls(&self) {
        self.st().calls.clear();
    }

    /// Makes every call of `method` wait until the returned gate is opened (`gate.open.notify_one()`).
    pub fn gate(&self, method: Method) -> Arc<Gate> {
        let g = Arc::new(Gate::default());
        self.st().gates.insert(method, g.clone());
        g
    }

    pub fn ungate(&self, method: Method) {
        self.st().gates.remove(&method);
    }

    /// Records the call and decides whether it fails. Returns the gate to wait on, if any.
    fn enter(&self, call: Call) -> Result<Option<Arc<Gate>>, BackendError> {
        let now = self.clock.now_ms();
        let mut st = self.st();
        let method = call.method();
        st.calls.push((now, call));
        if let Some(e) = &st.fail_all {
            return Err(e.clone());
        }
        if let Some(i) = st.fail_next.iter().position(|(m, _)| *m == method) {
            if let Some((_, e)) = st.fail_next.remove(i) {
                return Err(e);
            }
        }
        Ok(st.gates.get(&method).cloned())
    }

    async fn begin(&self, call: Call) -> Result<(), BackendError> {
        if let Some(gate) = self.enter(call)? {
            gate.open.notified().await;
        }
        Ok(())
    }

    fn folder_missing() -> BackendError {
        BackendError::Permanent("no such folder".into())
    }

    fn status_of(f: &FakeFolder) -> FolderStatus {
        FolderStatus {
            uid_validity: f.uid_validity,
            uid_next: f.uid_next,
            highest_modseq: None,
            exists: u32::try_from(f.messages.len()).unwrap(),
        }
    }
}

impl MailBackend for FakeBackend {
    fn capabilities(&self) -> BackendFuture<'_, Capabilities> {
        Box::pin(async move {
            let uidplus = self.st().uidplus;
            Ok(Capabilities {
                move_cmd: true,
                uidplus,
                ..Capabilities::default()
            })
        })
    }

    fn list_folders(&self) -> BackendFuture<'_, Vec<RemoteFolder>> {
        Box::pin(async move {
            self.begin(Call::ListFolders).await?;
            Ok(self
                .st()
                .folders
                .iter()
                .map(|(path, f)| RemoteFolder {
                    path: path.clone(),
                    delimiter: Some('/'),
                    special_use: f.special,
                    selectable: true,
                })
                .collect())
        })
    }

    fn folder_status<'a>(&'a self, folder: &'a str) -> BackendFuture<'a, FolderStatus> {
        Box::pin(async move {
            self.begin(Call::Status(folder.into())).await?;
            let st = self.st();
            let f = st.folders.get(folder).ok_or_else(Self::folder_missing)?;
            Ok(Self::status_of(f))
        })
    }

    fn snapshot<'a>(&'a self, folder: &'a str) -> BackendFuture<'a, RemoteSnapshot> {
        Box::pin(async move {
            self.begin(Call::Snapshot(folder.into())).await?;
            let st = self.st();
            let f = st.folders.get(folder).ok_or_else(Self::folder_missing)?;
            Ok(RemoteSnapshot {
                status: Self::status_of(f),
                messages: f
                    .messages
                    .iter()
                    .map(|(u, m)| (*u, m.flags.clone()))
                    .collect(),
            })
        })
    }

    fn fetch_headers<'a>(
        &'a self,
        folder: &'a str,
        uids: &'a [Uid],
    ) -> BackendFuture<'a, Vec<RemoteHeader>> {
        Box::pin(async move {
            self.begin(Call::Fetch(folder.into(), uids.to_vec()))
                .await?;
            let st = self.st();
            let f = st.folders.get(folder).ok_or_else(Self::folder_missing)?;
            let mut out: Vec<RemoteHeader> = uids
                .iter()
                .filter_map(|u| {
                    f.messages.get(u).map(|m| RemoteHeader {
                        uid: *u,
                        message_id: Some(m.message_id.clone()),
                        references: Vec::new(),
                        subject: m.subject.clone(),
                        from_name: Some("Sam Rivera".into()),
                        from_addr: Some("sam@example.org".into()),
                        date_ms: m.date_ms,
                        size: 100,
                        flags: m.flags.clone(),
                    })
                })
                .collect();
            out.sort_by_key(|h| std::cmp::Reverse(h.uid));
            Ok(out)
        })
    }

    fn find_by_message_id<'a>(
        &'a self,
        folder: &'a str,
        message_id: &'a str,
    ) -> BackendFuture<'a, Option<Uid>> {
        Box::pin(async move {
            self.begin(Call::Find(folder.into(), message_id.into()))
                .await?;
            let mut st = self.st();
            if st.search_misses > 0 {
                st.search_misses -= 1;
                return Ok(None);
            }
            let f = st.folders.get(folder).ok_or_else(Self::folder_missing)?;
            Ok(f.messages
                .iter()
                .rev()
                .find(|(_, m)| m.message_id == message_id)
                .map(|(u, _)| *u))
        })
    }

    fn set_flag<'a>(
        &'a self,
        folder: &'a str,
        uids: &'a [Uid],
        change: FlagChange,
    ) -> BackendFuture<'a, ()> {
        Box::pin(async move {
            self.begin(Call::SetFlag(folder.into(), uids.to_vec(), change))
                .await?;
            let mut st = self.st();
            let f = st
                .folders
                .get_mut(folder)
                .ok_or_else(Self::folder_missing)?;
            for u in uids {
                if let Some(m) = f.messages.get_mut(u) {
                    m.flags.apply(change);
                }
            }
            Ok(())
        })
    }

    fn move_to<'a>(
        &'a self,
        folder: &'a str,
        uids: &'a [Uid],
        dest: &'a str,
    ) -> BackendFuture<'a, MoveOutcome> {
        Box::pin(async move {
            self.begin(Call::Move(folder.into(), uids.to_vec(), dest.into()))
                .await?;
            let mut st = self.st();
            if !st.folders.contains_key(dest) {
                return Err(Self::folder_missing());
            }
            let uidplus = st.uidplus;
            let mut moved = Vec::new();
            {
                let src = st
                    .folders
                    .get_mut(folder)
                    .ok_or_else(Self::folder_missing)?;
                for u in uids {
                    if let Some(m) = src.messages.remove(u) {
                        moved.push((*u, m));
                    }
                }
            }
            let mut out = MoveOutcome::default();
            let d = st.folders.get_mut(dest).ok_or_else(Self::folder_missing)?;
            for (old, m) in moved {
                let new = d.uid_next;
                d.uid_next += 1;
                d.messages.insert(new, m);
                if uidplus {
                    out.dest_uids.insert(old, new);
                }
            }
            Ok(out)
        })
    }

    fn delete_forever<'a>(&'a self, folder: &'a str, uids: &'a [Uid]) -> BackendFuture<'a, ()> {
        Box::pin(async move {
            self.begin(Call::Delete(folder.into(), uids.to_vec()))
                .await?;
            let mut st = self.st();
            let f = st
                .folders
                .get_mut(folder)
                .ok_or_else(Self::folder_missing)?;
            for u in uids {
                f.messages.remove(u);
            }
            Ok(())
        })
    }
}

pub fn seen() -> Flags {
    Flags {
        seen: true,
        ..Flags::default()
    }
}

/// Creates an account row and returns its id.
pub fn create_account(db: &Db, email: &str) -> AccountId {
    accounts::insert(
        db.connection(),
        &NewAccount {
            label: email.into(),
            email: email.into(),
            protocol: Protocol::Imap,
            provider: "generic".into(),
            config: serde_json::json!({}),
            color: None,
            position: 0,
        },
        0,
    )
    .unwrap()
}

/// An engine on a database file, with a manual clock and one account on a fake backend, attached
/// without a task (the test drives it with `run_once`).
pub struct Rig {
    pub dir: tempfile::TempDir,
    pub path: PathBuf,
    pub clock: ManualClock,
    pub backend: Arc<FakeBackend>,
    pub engine: Option<Engine>,
    pub ui: EngineHandle,
    pub account: AccountId,
    pub changes: broadcast::Receiver<Change>,
}

pub const START_MS: i64 = 1_000_000;

impl Rig {
    pub fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("croft.db");
        let db = Db::open(&path, None).unwrap();
        let account = create_account(&db, "one@example.org");
        drop(db);
        let clock = ManualClock::new(START_MS);
        let backend = FakeBackend::new(Arc::new(clock.clone()));
        let (engine, ui, changes) = Self::start(&path, &clock, &backend, account);
        Self {
            dir,
            path,
            clock,
            backend,
            engine: Some(engine),
            ui,
            account,
            changes,
        }
    }

    fn start(
        path: &std::path::Path,
        clock: &ManualClock,
        backend: &Arc<FakeBackend>,
        account: AccountId,
    ) -> (Engine, EngineHandle, broadcast::Receiver<Change>) {
        let db = Db::open(path, None).unwrap();
        let engine = Engine::open(db, Arc::new(clock.clone()), EngineConfig::default()).unwrap();
        engine.attach_account(account, backend.clone()).unwrap();
        let ui = engine.handle();
        let changes = ui.subscribe();
        (engine, ui, changes)
    }

    /// Simulates the app being killed: the engine and every handle are dropped without a
    /// shutdown, the database is closed, and a new engine is opened on the same file (same server).
    pub fn kill_and_restart(&mut self) {
        let engine = self.engine.take();
        drop(engine);
        // The handle holds the database too; replace it before reopening.
        let (engine, ui, changes) =
            Self::start(&self.path, &self.clock, &self.backend, self.account);
        self.ui = ui;
        self.changes = changes;
        self.engine = Some(engine);
    }

    pub async fn run(&self) {
        self.ui.run_once(self.account, false).await.unwrap();
    }

    pub async fn sync(&self) {
        self.ui.run_once(self.account, true).await.unwrap();
    }

    pub fn advance(&self, ms: i64) {
        self.clock.advance(ms);
    }

    pub fn clock_now(&self) -> i64 {
        self.clock.now_ms()
    }

    /// The local row at (folder, uid), hidden or not.
    pub fn row(&self, folder: &str, uid: Uid) -> Option<Message> {
        let account = self.account;
        self.ui.read(|db| {
            let f = folders::find_by_path(db.connection(), account, folder).unwrap()?;
            messages::find_by_uid(db.connection(), account, f.id, uid).unwrap()
        })
    }

    /// The local row with this Message-ID (`<id>`) in a folder.
    pub fn row_by_id(&self, folder: &str, id: &str) -> Option<Message> {
        let account = self.account;
        let want = format!("<{id}>");
        self.ui.read(|db| {
            let f = folders::find_by_path(db.connection(), account, folder).unwrap()?;
            let flags = messages::folder_flags(db.connection(), account, f.id).unwrap();
            flags.keys().find_map(|uid| {
                messages::find_by_uid(db.connection(), account, f.id, *uid)
                    .unwrap()
                    .filter(|m| m.message_id_hdr.as_deref() == Some(want.as_str()))
            })
        })
    }

    /// Visible rows (the list query) of a folder.
    pub fn listed(&self, folder: &str) -> Vec<Message> {
        let account = self.account;
        let now = self.clock.now_ms();
        self.ui.read(|db| {
            let Some(f) = folders::find_by_path(db.connection(), account, folder).unwrap() else {
                return Vec::new();
            };
            messages::list_by_folder(db.connection(), account, f.id, now, 1_000, 0).unwrap()
        })
    }

    /// Every change received so far.
    pub fn drain_changes(&mut self) -> Vec<Change> {
        let mut out = Vec::new();
        while let Ok(c) = self.changes.try_recv() {
            out.push(c);
        }
        out
    }
}
