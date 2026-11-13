// Tests of the compose surface, called the way the generated bindings call it. `drafts` needs no
// server; `sending` runs against the in-process IMAP and SMTP servers of `croft-test-support`.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod drafts;
#[cfg(feature = "integration")]
mod integration;
mod reply;
mod sending;
mod unsubscribe;

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use croft_storage::accounts::{self, NewAccount, Protocol};
use croft_storage::Db;
use croft_test_support::{ServerOptions, SmtpOptions, SmtpServer, TestServer, TlsMode};

use crate::settings::{StoredSecurity, StoredServer, StoredSettings, SETTINGS_VERSION};
use crate::tests::{block, password_cred, MemProvider, TempDb, PASSWORD};
use crate::*;

pub(super) fn address(name: Option<&str>, a: &str) -> FfiAddress {
    FfiAddress {
        name: name.map(str::to_owned),
        address: a.to_owned(),
    }
}

/// A draft with one To recipient.
pub(super) fn content(to: &[&str], subject: &str, body: &str) -> DraftContent {
    DraftContent {
        identity: None,
        to: to.iter().map(|a| address(None, a)).collect(),
        cc: vec![],
        bcc: vec![],
        subject: subject.to_owned(),
        body: body.to_owned(),
        in_reply_to: None,
        references: vec![],
    }
}

/// Polls `cond` for up to 20 s of real time.
pub(super) fn wait_until(what: &str, mut cond: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while !cond() {
        assert!(Instant::now() < deadline, "timed out waiting for: {what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Stays quiet for `ms` and checks `cond` held the whole time.
pub(super) fn stays(ms: u64, mut cond: impl FnMut() -> bool) {
    let until = Instant::now() + Duration::from_millis(ms);
    while Instant::now() < until {
        assert!(cond());
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(cond());
}

pub(super) fn server(host: &str, port: u16, username: &str) -> StoredServer {
    StoredServer {
        host: host.into(),
        port,
        security: StoredSecurity::PlaintextForTestsOnly,
        username: username.into(),
    }
}

/// Inserts an account row (before `Core::open`) with these settings.
pub(super) fn insert_account_with(db: &Path, settings: &StoredSettings, email: &str) -> i64 {
    let d = Db::open(db, None).unwrap();
    accounts::insert(
        d.connection(),
        &NewAccount {
            label: "Alice Example".into(),
            email: email.into(),
            protocol: Protocol::Imap,
            provider: "generic".into(),
            config: settings.to_json().unwrap(),
            color: None,
            position: 0,
        },
        0,
    )
    .unwrap()
}

/// A real account on the in-process IMAP and SMTP servers (plaintext on loopback, which the
/// adapters allow for tests only), opened in a core.
pub(super) struct Mail {
    pub(super) imap: TestServer,
    pub(super) smtp: SmtpServer,
    pub(super) db: TempDb,
    pub(super) provider: Arc<MemProvider>,
    core: Option<Arc<Core>>,
    pub(super) account: i64,
}

impl Mail {
    pub(super) fn new() -> Self {
        Self::with(|_| {})
    }

    /// `change` edits the stored settings before the core opens.
    pub(super) fn with(change: impl FnOnce(&mut StoredSettings)) -> Self {
        Self::with_smtp(TlsMode::None, |settings, _| change(settings))
    }

    /// The SMTP server secured as `tls` says; `change` edits the stored settings, and can read the
    /// SMTP server's certificate.
    pub(super) fn with_smtp(
        tls: TlsMode,
        change: impl FnOnce(&mut StoredSettings, &SmtpServer),
    ) -> Self {
        let imap = block(TestServer::start(ServerOptions {
            username: "alice".into(),
            password: PASSWORD.into(),
            tls: TlsMode::None,
            ..ServerOptions::default()
        }));
        let smtp = block(SmtpServer::start(SmtpOptions {
            username: "alice".into(),
            password: PASSWORD.into(),
            tls,
            ..SmtpOptions::default()
        }));
        let db = TempDb::new();
        let mut settings = StoredSettings {
            version: SETTINGS_VERSION,
            imap: server(imap.host(), imap.port(), "alice"),
            smtp: Some(server(smtp.host(), smtp.port(), "alice")),
            pinned_fingerprint: None,
        };
        change(&mut settings, &smtp);
        let account = insert_account_with(&db.path, &settings, "alice@example.org");
        let provider = MemProvider::new();
        provider.store(account, password_cred(PASSWORD));
        let core = db.open(&provider);
        core.set_undo_window_ms(0);
        settle(&core);
        Self {
            imap,
            smtp,
            db,
            provider,
            core: Some(core),
            account,
        }
    }

    pub(super) fn core(&self) -> &Arc<Core> {
        self.core.as_ref().expect("the core is open")
    }

    /// Closes the core (its tasks stop; a send in flight is cancelled) and opens another on the
    /// same database: an app restart.
    pub(super) fn restart(&mut self) {
        drop(self.core.take());
        let fresh = self.db.open(&self.provider);
        fresh.set_undo_window_ms(0);
        settle(&fresh);
        self.core = Some(fresh);
    }

    /// Closes the core and leaves it closed.
    pub(super) fn kill(&mut self) {
        drop(self.core.take());
    }

    pub(super) fn reopen(&mut self) {
        assert!(self.core.is_none());
        self.restart();
    }

    pub(super) fn accepted(&self) -> usize {
        self.smtp.accepted().len()
    }

    /// Saves a draft to alice's friend and returns its id.
    pub(super) fn draft(&self, subject: &str) -> i64 {
        self.core()
            .save_draft(
                self.account,
                None,
                content(&["bob@example.org"], subject, "Hello Bob"),
            )
            .unwrap()
            .id
    }
}

/// Waits for the background work the core starts at open (the check for sent messages, the
/// clean-up of finished drafts, the copies due) so that a test starts from a quiet core.
pub(super) fn settle(core: &Core) {
    wait_until("the start-up work is done", || core.inner.compose.is_idle());
}

/// A core with no network at all: the sample mailbox.
pub(super) fn sample_core() -> (TempDb, Arc<Core>, i64) {
    let db = TempDb::new();
    let provider = MemProvider::new();
    let core = db.open(&provider);
    let account = block(core.add_demo_account()).unwrap().id;
    core.set_undo_window_ms(0);
    (db, core, account)
}
