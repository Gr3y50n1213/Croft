// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Tests of the exported surface, called directly from Rust the way the generated bindings call
//! it, with an in-memory credential provider. `local` needs no network; `server` runs against the
//! in-process IMAP server from `croft-test-support` (TLS on 127.0.0.1 with a self-signed
//! certificate, trusted through the fingerprint exactly as the app does it).

mod demo;
mod encrypted;
mod local;
mod readd;
mod secrets;
mod server;

use std::collections::HashMap;
use std::future::Future;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use croft_protocols::CertFingerprint;
use croft_storage::accounts::{self, NewAccount, Protocol};
use croft_storage::Db;
use croft_test_support::{ServerOptions, TestServer, TlsMode};

use crate::settings::{StoredSecurity, StoredServer, StoredSettings, SETTINGS_VERSION};
use crate::*;

/// The fixture password. It must never show up in any error, any returned value or the database.
pub(crate) const PASSWORD: &str = "fixture-pw-Zk3-8841";

/// A runtime for the tests' own needs: it drives the futures the exported methods return and
/// hosts the in-process server (which must keep running between calls).
fn rt() -> &'static tokio::runtime::Runtime {
    static RT: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RT.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .unwrap()
    })
}

pub(crate) fn block<F: Future>(f: F) -> F::Output {
    rt().block_on(f)
}

/// Like [`block`], giving up after `limit` (`None`).
pub(crate) fn block_for<F: Future>(limit: Duration, f: F) -> Option<F::Output> {
    rt().block_on(async { tokio::time::timeout(limit, f).await.ok() })
}

/// The app's credential store, in memory. Like the Android one it answers only for ids the app
/// stored (`store`, which a test calls right after `add_account` returns, as the app does).
#[derive(Default)]
pub(crate) struct MemProvider {
    stored: Mutex<HashMap<i64, Arc<FfiCredential>>>,
    asked: Mutex<Vec<i64>>,
    sign_ins: Mutex<Vec<i64>>,
    /// How many of the next `credential` calls answer `NotReady` (the keystore is busy).
    not_ready: Mutex<usize>,
}

impl MemProvider {
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub(crate) fn store(&self, id: i64, c: Arc<FfiCredential>) {
        self.stored.lock().unwrap().insert(id, c);
    }

    /// The next `n` calls to `credential` fail with `NotReady`.
    pub(crate) fn not_ready_for(&self, n: usize) {
        *self.not_ready.lock().unwrap() = n;
    }

    pub(crate) fn sign_ins(&self) -> Vec<i64> {
        self.sign_ins.lock().unwrap().clone()
    }

    pub(crate) fn asked(&self) -> Vec<i64> {
        self.asked.lock().unwrap().clone()
    }
}

impl CredentialProvider for MemProvider {
    fn credential(&self, account_id: i64) -> Result<Option<Arc<FfiCredential>>, ProviderError> {
        self.asked.lock().unwrap().push(account_id);
        {
            let mut n = self.not_ready.lock().unwrap();
            if *n > 0 {
                *n -= 1;
                return Err(ProviderError::NotReady);
            }
        }
        Ok(self.stored.lock().unwrap().get(&account_id).cloned())
    }

    fn sign_in_needed(&self, account_id: i64) -> Result<(), ProviderError> {
        self.sign_ins.lock().unwrap().push(account_id);
        Ok(())
    }
}

pub(crate) fn password_cred(p: &str) -> Arc<FfiCredential> {
    FfiCredential::password(p.as_bytes().to_vec())
}

/// An in-process server with TLS, one user and the fixture password.
pub(crate) fn start_server() -> TestServer {
    block(TestServer::start(ServerOptions {
        username: "alice".into(),
        password: PASSWORD.into(),
        tls: TlsMode::Implicit,
        ..ServerOptions::default()
    }))
}

/// A raw message for the server.
pub(crate) fn raw(id: &str, subject: &str) -> String {
    croft_test_support::sample_message(id, subject, "Ann Example <ann@example.org>")
}

/// What `discover` would return for the test server (the app gets this from `discover`).
pub(crate) fn discovered_for(server: &TestServer) -> FfiDiscovered {
    FfiDiscovered {
        email: "alice@example.org".into(),
        imap: Some(ServerSettings {
            host: server.host().into(),
            port: server.port(),
            security: SecurityMode::Tls,
            username: server.username(),
        }),
        smtp: None,
        jmap: None,
        source: DiscoverySource::Autoconfig,
        provider: None,
        auth_hint: AuthKind::Password,
    }
}

/// The fingerprint the person would trust (as `verify` reports it).
pub(crate) fn fingerprint_of(server: &TestServer) -> String {
    CertFingerprint::of_der(&server.certificate_der().unwrap()).to_string()
}

/// A temporary folder with a database path in it.
pub(crate) struct TempDb {
    _dir: tempfile::TempDir,
    pub(crate) path: PathBuf,
}

impl TempDb {
    pub(crate) fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("croft.db");
        Self { _dir: dir, path }
    }

    pub(crate) fn open(&self, provider: &Arc<MemProvider>) -> Arc<Core> {
        Core::open(self.path.display().to_string(), None, provider.clone()).unwrap()
    }
}

/// Stored settings for an account that points at a port nobody listens on (for tests that must
/// not touch a network).
pub(crate) fn dead_settings() -> serde_json::Value {
    StoredSettings {
        version: SETTINGS_VERSION,
        imap: StoredServer {
            host: "127.0.0.1".into(),
            port: 1,
            security: StoredSecurity::Tls,
            username: "nobody".into(),
        },
        smtp: None,
        pinned_fingerprint: None,
    }
    .to_json()
    .unwrap()
}

/// Inserts an account row straight into the database file (before `Core::open`).
pub(crate) fn insert_account(db: &Db, n: i64) -> i64 {
    accounts::insert(
        db.connection(),
        &NewAccount {
            label: format!("Account {n}"),
            email: format!("user{n}@example.org"),
            protocol: Protocol::Imap,
            provider: "generic".to_owned(),
            config: dead_settings(),
            color: None,
            position: n,
        },
        0,
    )
    .unwrap()
}

/// Polls `cond`, waiting on `next_change` in between, until it holds or the time is up.
pub(crate) async fn wait_for(core: &Core, what: &str, mut cond: impl FnMut() -> bool) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    let mut seq = 0;
    loop {
        if cond() {
            return;
        }
        let left = deadline.saturating_duration_since(tokio::time::Instant::now());
        // Also wake every 300 ms: some conditions depend on the server, not on a change.
        let wait = left.min(Duration::from_millis(300));
        match tokio::time::timeout(wait, core.next_change(seq)).await {
            Ok(batch) => seq = batch.unwrap().seq,
            Err(_) if left.is_zero() => panic!("timed out waiting for: {what}"),
            Err(_) => {}
        }
    }
}

/// Opens a core on a fresh database, adds the test server's account and waits for its first sync.
pub(crate) struct Rig {
    pub(crate) server: TestServer,
    pub(crate) db: TempDb,
    pub(crate) provider: Arc<MemProvider>,
    pub(crate) core: Arc<Core>,
    pub(crate) account: i64,
    pub(crate) fingerprint: String,
}

impl Rig {
    /// `setup` puts messages on the server before the account is added.
    pub(crate) fn new(setup: impl FnOnce(&TestServer)) -> Self {
        let server = start_server();
        setup(&server);
        let db = TempDb::new();
        let provider = MemProvider::new();
        let core = db.open(&provider);
        let fingerprint = fingerprint_of(&server);
        let account = block(core.add_account(
            discovered_for(&server),
            "Alice".into(),
            Some(fingerprint.clone()),
            password_cred(PASSWORD),
        ))
        .unwrap();
        // What the app does once `add_account` has returned.
        provider.store(account, password_cred(PASSWORD));
        Self {
            server,
            db,
            provider,
            core,
            account,
            fingerprint,
        }
    }

    pub(crate) fn folder_id(&self, path: &str) -> Option<i64> {
        self.core
            .list_folders(self.account)
            .unwrap()
            .into_iter()
            .find(|f| f.path == path)
            .map(|f| f.id)
    }

    pub(crate) fn inbox(&self) -> MessageScope {
        MessageScope::Folder {
            account: self.account,
            folder_id: self.folder_id("INBOX").unwrap(),
        }
    }

    pub(crate) fn count(&self, scope: MessageScope) -> usize {
        self.core
            .list_messages(scope, None, 500)
            .unwrap()
            .rows
            .len()
    }

    /// Waits until the INBOX has `n` visible messages.
    pub(crate) fn wait_for_inbox(&self, n: usize) {
        block(wait_for(&self.core, "inbox rows", || {
            self.folder_id("INBOX").is_some_and(|id| {
                self.count(MessageScope::Folder {
                    account: self.account,
                    folder_id: id,
                }) == n
            })
        }));
    }
}
