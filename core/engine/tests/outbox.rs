// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Sending, end to end: the engine's Outbox with the real SMTP sender and IMAP adapter against the
//! in-process SMTP and IMAP servers from `croft-test-support` (loopback, plaintext). Driven step by
//! step with a manual clock through `run_once`; the network is real, so these use real time only
//! for I/O. One test (`the_account_task_wakes_by_itself_when_the_undo_window_ends`) runs the
//! account task with the system clock.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::future::pending;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use common::create_account;
use croft_engine::{
    AccountId, Change, Clock, Engine, EngineConfig, EngineHandle, ManualClock, NewMessage,
    OutboxActionError, OutboxEntry, OutboxId, OutboxState, SendError, SystemClock,
};
use croft_protocols::smtp::MAYBE_SENT_REASON;
use croft_protocols::{
    BackendError, BackendFuture, Credential, ImapBackend, ImapConfig, MailBackend, MailSubmitter,
    Security, SmtpConfig, SmtpImapSubmitter, SmtpSecurity, SmtpSender,
};
use croft_storage::Db;
use croft_test_support::{sample_message, SmtpMisbehavior, SmtpServer, Stage, TestServer};
use tokio::sync::broadcast;
use tokio::task::JoinHandle;

const START_MS: i64 = 1_000_000;
/// The engine's default Undo window.
const WINDOW_MS: i64 = 5_000;
const ENVELOPE_FROM: &str = "alice@example.org";
const ENVELOPE_TO: &str = "bob@example.org";

fn imap_for(server: &TestServer) -> Arc<ImapBackend> {
    let config = ImapConfig::new(
        server.host(),
        server.port(),
        Security::PlaintextForTestsOnly,
        server.username(),
    )
    .with_timeout(Duration::from_secs(5));
    Arc::new(ImapBackend::new(config, Credential::password(server.password())).unwrap())
}

fn smtp_for(server: &SmtpServer) -> SmtpSender {
    let config = SmtpConfig::new(
        server.host(),
        server.port(),
        SmtpSecurity::PlaintextForTestsOnly,
        server.username(),
    )
    .with_timeout(Duration::from_secs(5))
    .with_deadline(Duration::from_secs(20));
    SmtpSender::new(config, Credential::password(server.password())).unwrap()
}

fn submitter(
    smtp: &SmtpServer,
    imap: &Arc<ImapBackend>,
    files_itself: bool,
) -> Arc<SmtpImapSubmitter> {
    Arc::new(SmtpImapSubmitter::new(
        smtp_for(smtp),
        imap.clone(),
        files_itself,
    ))
}

fn mime(id: &str) -> Vec<u8> {
    sample_message(id, &format!("Subject of {id}"), "Alice <alice@example.org>").into_bytes()
}

fn envelope(account: AccountId) -> NewMessage {
    NewMessage {
        account,
        identity: None,
        envelope_from: ENVELOPE_FROM.into(),
        recipients: vec![ENVELOPE_TO.into()],
    }
}

/// Polls `cond` for up to five seconds of real time.
async fn wait_until(what: &str, mut cond: impl FnMut() -> bool) {
    for _ in 0..500 {
        if cond() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("timed out waiting for: {what}");
}

/// Wraps the real submitter and stops ("the app dies") at a chosen point of the filing step.
struct StallsWhileFiling {
    inner: Arc<SmtpImapSubmitter>,
    /// True: the copy is appended and then the call never returns. False: it never starts.
    append_first: bool,
}

impl MailSubmitter for StallsWhileFiling {
    fn send<'a>(
        &'a self,
        envelope_from: &'a str,
        recipients: &'a [String],
        mime: &'a [u8],
    ) -> BackendFuture<'a, ()> {
        self.inner.send(envelope_from, recipients, mime)
    }

    fn files_sent_itself(&self) -> bool {
        false
    }

    fn file_in_sent<'a>(
        &'a self,
        message_id: Option<&'a str>,
        mime: &'a [u8],
    ) -> BackendFuture<'a, ()> {
        Box::pin(async move {
            if self.append_first {
                self.inner.file_in_sent(message_id, mime).await?;
            }
            pending::<()>().await;
            Ok(())
        })
    }
}

/// Wraps the real submitter, and refuses every copy for Sent for good.
struct RefusesCopy(Arc<SmtpImapSubmitter>);

impl MailSubmitter for RefusesCopy {
    fn send<'a>(
        &'a self,
        envelope_from: &'a str,
        recipients: &'a [String],
        mime: &'a [u8],
    ) -> BackendFuture<'a, ()> {
        self.0.send(envelope_from, recipients, mime)
    }

    fn files_sent_itself(&self) -> bool {
        false
    }

    fn file_in_sent<'a>(&'a self, _: Option<&'a str>, _: &'a [u8]) -> BackendFuture<'a, ()> {
        Box::pin(async { Err(BackendError::Permanent("no copies".into())) })
    }
}

/// One account on a database file, with an SMTP and an IMAP server, a manual clock and an engine
/// that can be "killed" and opened again on the same file.
struct Rig {
    smtp: SmtpServer,
    imap: TestServer,
    backend: Arc<ImapBackend>,
    _dir: tempfile::TempDir,
    path: PathBuf,
    clock: ManualClock,
    engine: Option<Engine>,
    ui: EngineHandle,
    account: AccountId,
    changes: broadcast::Receiver<Change>,
}

impl Rig {
    async fn new() -> Self {
        Self::with(false).await
    }

    async fn with(files_itself: bool) -> Self {
        let smtp = SmtpServer::start_default().await;
        let imap = TestServer::start_default().await;
        let backend = imap_for(&imap);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("croft.db");
        let db = Db::open(&path, None).unwrap();
        let account = create_account(&db, "alice@example.org");
        drop(db);
        let clock = ManualClock::new(START_MS);
        let (engine, ui, changes) = Self::start(
            &path,
            &clock,
            account,
            &backend,
            submitter(&smtp, &backend, files_itself),
        );
        Self {
            smtp,
            imap,
            backend,
            _dir: dir,
            path,
            clock,
            engine: Some(engine),
            ui,
            account,
            changes,
        }
    }

    fn start(
        path: &Path,
        clock: &ManualClock,
        account: AccountId,
        backend: &Arc<ImapBackend>,
        submitter: Arc<dyn MailSubmitter>,
    ) -> (Engine, EngineHandle, broadcast::Receiver<Change>) {
        let engine = Engine::open_with_outbox(
            Db::open(path, None).unwrap(),
            Db::open(path, None).unwrap(),
            Arc::new(clock.clone()),
            EngineConfig::default(),
        )
        .unwrap();
        let ui = engine.handle();
        let changes = ui.subscribe();
        let backend: Arc<dyn MailBackend> = backend.clone();
        engine
            .attach_account_with_submitter(account, backend, submitter)
            .unwrap();
        (engine, ui, changes)
    }

    /// The app is killed and started again: the old engine is dropped (not shut down) and a new
    /// one is opened on the same database file, with this submitter.
    fn kill_and_restart(&mut self, submitter: Arc<dyn MailSubmitter>) {
        drop(self.engine.take());
        let (engine, ui, changes) = Self::start(
            &self.path,
            &self.clock,
            self.account,
            &self.backend,
            submitter,
        );
        self.engine = Some(engine);
        self.ui = ui;
        self.changes = changes;
    }

    fn real_submitter(&self) -> Arc<SmtpImapSubmitter> {
        submitter(&self.smtp, &self.backend, false)
    }

    fn send(&self, id: &str) -> OutboxId {
        self.ui.send(envelope(self.account), mime(id)).unwrap()
    }

    async fn run(&self) {
        self.ui.run_once(self.account, false).await.unwrap();
    }

    fn advance(&self, ms: i64) {
        self.clock.advance(ms);
    }

    fn outbox(&self) -> Vec<OutboxEntry> {
        self.ui.outbox(self.account)
    }

    fn accepted(&self) -> usize {
        self.smtp.accepted().len()
    }

    /// Message-IDs (as `<id>`) of what the IMAP server holds in Sent.
    fn sent_folder(&self) -> Vec<String> {
        self.imap
            .messages("Sent")
            .into_iter()
            .filter_map(|m| {
                m.raw
                    .lines()
                    .find_map(|l| l.strip_prefix("Message-ID: ").map(str::to_owned))
            })
            .collect()
    }

    /// Everything the engine announced so far.
    fn drain(&mut self) -> Vec<Change> {
        let mut out = Vec::new();
        while let Ok(change) = self.changes.try_recv() {
            out.push(change);
        }
        out
    }

    fn outbox_rows_on_disk(&self) -> i64 {
        self.ui.read(|db| {
            db.connection()
                .query_row("SELECT count(*) FROM outbox", [], |r| r.get(0))
                .unwrap()
        })
    }

    /// Starts `run_once` as its own task, so the test can "kill" it while the server holds the
    /// message.
    fn run_in_background(&self) -> JoinHandle<()> {
        let ui = self.ui.clone();
        let account = self.account;
        tokio::spawn(async move {
            let _ = ui.run_once(account, false).await;
        })
    }

    /// Waits until the SMTP server has the whole message (and is sitting on it).
    async fn wait_for_data_end(&self) {
        wait_until("the message to reach the SMTP server", || {
            self.smtp.commands().iter().any(|c| c == "DATA")
        })
        .await;
        // The body follows the DATA command; give it time to arrive in full.
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

#[tokio::test]
async fn offline_send_queues_and_goes_out_once_when_the_connection_is_back() {
    let rig = Rig::new().await;
    rig.ui.set_online(rig.account, false);
    let id = rig.send("offline-1@example.org");
    assert_eq!(rig.outbox().len(), 1);
    assert_eq!(rig.outbox()[0].id, id);
    assert_eq!(rig.outbox()[0].state, OutboxState::Queued);

    // Well past the Undo window, still offline: nothing reaches the server and no attempt is used.
    rig.advance(WINDOW_MS + 60_000);
    rig.run().await;
    assert_eq!(rig.accepted(), 0);
    assert_eq!(rig.smtp.connection_count(), 0);
    assert_eq!(rig.outbox()[0].attempts, 0);

    rig.ui.set_online(rig.account, true);
    rig.run().await;
    assert_eq!(rig.accepted(), 1);
    let accepted = &rig.smtp.accepted()[0];
    assert_eq!(accepted.mail_from, ENVELOPE_FROM);
    assert_eq!(accepted.rcpt_to, vec![ENVELOPE_TO.to_owned()]);
    assert!(String::from_utf8_lossy(&accepted.data).contains("offline-1@example.org"));
    assert!(rig.outbox().is_empty());
    assert_eq!(rig.outbox_rows_on_disk(), 0);

    // More time, more runs, more "online" callbacks: still the one message.
    rig.advance(3_600_000);
    rig.run().await;
    rig.ui.set_online(rig.account, true);
    rig.run().await;
    assert_eq!(rig.accepted(), 1);
}

#[tokio::test]
async fn nothing_is_sent_before_the_undo_window_ends() {
    let rig = Rig::new().await;
    rig.send("early@example.org");
    rig.advance(WINDOW_MS - 1);
    rig.run().await;
    assert_eq!(rig.accepted(), 0);
    rig.advance(1);
    rig.run().await;
    assert_eq!(rig.accepted(), 1);
}

#[tokio::test]
async fn undo_inside_the_window_sends_nothing_ever() {
    let rig = Rig::new().await;
    let id = rig.send("undo-1@example.org");
    rig.advance(1_000);
    let back = rig.ui.undo_send(rig.account, id).unwrap();
    assert_eq!(back.mime, mime("undo-1@example.org"));
    assert!(rig.outbox().is_empty());
    assert_eq!(rig.outbox_rows_on_disk(), 0);

    rig.advance(3_600_000);
    rig.run().await;
    rig.run().await;
    assert_eq!(rig.accepted(), 0);
    assert_eq!(rig.smtp.connection_count(), 0);
    assert!(rig.sent_folder().is_empty());
    // The id is not reusable: a second Undo finds nothing.
    assert!(matches!(
        rig.ui.undo_send(rig.account, id),
        Err(OutboxActionError::NoSuchMessage)
    ));
}

#[tokio::test]
async fn undo_after_the_window_is_too_late_and_the_message_is_sent() {
    let rig = Rig::new().await;
    let id = rig.send("late-undo@example.org");
    rig.advance(WINDOW_MS + 1);
    assert!(matches!(
        rig.ui.undo_send(rig.account, id),
        Err(OutboxActionError::TooLate)
    ));
    rig.run().await;
    assert_eq!(rig.accepted(), 1);
}

#[tokio::test]
async fn a_bcc_header_is_refused_before_anything_is_stored() {
    let mut rig = Rig::new().await;
    let with_bcc = format!(
        "Bcc: carol@example.org\r\n{}",
        String::from_utf8(mime("bcc@example.org")).unwrap()
    );
    let folded = String::from_utf8(mime("bcc2@example.org"))
        .unwrap()
        .replacen("\r\n\r\n", "\r\nbCC:\r\n carol@example.org\r\n\r\n", 1);
    for text in [with_bcc, folded] {
        assert!(matches!(
            rig.ui.send(envelope(rig.account), text.into_bytes()),
            Err(SendError::BccHeader)
        ));
    }
    assert!(rig.outbox().is_empty());
    assert_eq!(rig.outbox_rows_on_disk(), 0);
    assert!(rig.drain().is_empty());
    rig.advance(WINDOW_MS + 1);
    rig.run().await;
    assert_eq!(rig.accepted(), 0);
    // A Bcc recipient in the envelope is fine: it is not in the headers.
    let mut message = envelope(rig.account);
    message.recipients.push("carol@example.org".into());
    rig.ui.send(message, mime("bcc3@example.org")).unwrap();
    rig.advance(WINDOW_MS + 1);
    rig.run().await;
    let accepted = rig.smtp.accepted();
    assert_eq!(accepted.len(), 1);
    assert_eq!(accepted[0].rcpt_to.len(), 2);
    assert!(!String::from_utf8_lossy(&accepted[0].data).contains("carol"));
}

#[tokio::test]
async fn an_empty_envelope_and_an_account_that_cannot_send_are_refused() {
    let rig = Rig::new().await;
    let mut none = envelope(rig.account);
    none.recipients.clear();
    assert!(matches!(
        rig.ui.send(none, mime("x@example.org")),
        Err(SendError::NoRecipients)
    ));
    assert!(matches!(
        rig.ui.send(envelope(99), mime("x@example.org")),
        Err(SendError::UnknownAccount(99))
    ));

    // An account attached without a submitter, and an engine opened without an Outbox.
    let path = rig.path.clone();
    let db = Db::open(&path, None).unwrap();
    let second = create_account(&db, "second@example.org");
    drop(db);
    let backend: Arc<dyn MailBackend> = rig.backend.clone();
    rig.engine
        .as_ref()
        .unwrap()
        .attach_account(second, backend.clone())
        .unwrap();
    assert!(matches!(
        rig.ui.send(envelope(second), mime("y@example.org")),
        Err(SendError::NoSubmitter(a)) if a == second
    ));
    let bare = Engine::open(
        Db::open(&path, None).unwrap(),
        Arc::new(rig.clock.clone()),
        EngineConfig::default(),
    )
    .unwrap();
    bare.attach_account(rig.account, backend).unwrap();
    assert!(matches!(
        bare.handle()
            .send(envelope(rig.account), mime("z@example.org")),
        Err(SendError::NoSubmitter(_) | SendError::NoOutbox)
    ));
    assert!(bare.handle().outbox(rig.account).is_empty());
    assert!(matches!(
        bare.handle().undo_send(rig.account, 1),
        Err(OutboxActionError::NoOutbox)
    ));
}

#[tokio::test]
async fn after_a_kill_while_the_server_holds_the_message_it_is_failed_and_never_resent() {
    let mut rig = Rig::new().await;
    rig.smtp.set_misbehavior(SmtpMisbehavior::HangAfterData);
    let id = rig.send("killed@example.org");
    rig.advance(WINDOW_MS + 1);
    let in_flight = rig.run_in_background();
    rig.wait_for_data_end().await;
    assert_eq!(rig.outbox()[0].state, OutboxState::Sending);
    let accepted_before = rig.accepted();
    let connections_before = rig.smtp.connection_count();

    // The app dies here: the new engine opens the file the old one left behind.
    rig.kill_and_restart(rig.real_submitter());
    let rows = rig.outbox();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].id, id);
    assert_eq!(rows[0].state, OutboxState::Failed);
    assert_eq!(rows[0].last_error.as_deref(), Some(MAYBE_SENT_REASON));
    assert!(rows[0].may_have_been_sent());
    // Told once, when the account was added.
    let told: Vec<Change> = rig
        .drain()
        .into_iter()
        .filter(|c| matches!(c, Change::SendFailed { .. }))
        .collect();
    assert_eq!(
        told,
        vec![Change::SendFailed {
            account: rig.account,
            id
        }]
    );

    // Not sent again, however long it waits and however often the engine runs.
    rig.smtp.set_misbehavior(SmtpMisbehavior::None);
    rig.advance(3_600_000);
    rig.run().await;
    rig.ui.set_online(rig.account, true);
    rig.run().await;
    assert_eq!(rig.accepted(), accepted_before);
    assert_eq!(rig.smtp.connection_count(), connections_before);
    assert_eq!(rig.outbox()[0].state, OutboxState::Failed);
    assert!(rig
        .drain()
        .iter()
        .all(|c| !matches!(c, Change::SendFailed { .. })));
    in_flight.abort();
}

#[tokio::test]
async fn a_send_that_is_cancelled_in_the_process_is_failed_as_maybe_sent_and_not_resent() {
    let mut rig = Rig::new().await;
    rig.smtp.set_misbehavior(SmtpMisbehavior::HangAfterData);
    let id = rig.send("cancelled@example.org");
    rig.advance(WINDOW_MS + 1);
    let in_flight = rig.run_in_background();
    rig.wait_for_data_end().await;
    in_flight.abort();
    let _ = in_flight.await;

    let rows = rig.outbox();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].state, OutboxState::Failed);
    assert!(rows[0].may_have_been_sent());
    let changes = rig.drain();
    assert!(changes.contains(&Change::SendFailed {
        account: rig.account,
        id
    }));

    rig.smtp.set_misbehavior(SmtpMisbehavior::None);
    rig.advance(3_600_000);
    rig.run().await;
    assert_eq!(rig.accepted(), 0);
    assert_eq!(rig.outbox()[0].state, OutboxState::Failed);
}

#[tokio::test]
async fn a_server_that_accepts_then_drops_the_connection_leaves_a_maybe_sent_row_that_is_not_resent(
) {
    let mut rig = Rig::new().await;
    rig.smtp.set_misbehavior(SmtpMisbehavior::AcceptThenDrop);
    let id = rig.send("accept-drop@example.org");
    rig.advance(WINDOW_MS + 1);
    rig.run().await;
    // The server kept it; the client could not know.
    assert_eq!(rig.accepted(), 1);
    let rows = rig.outbox();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].state, OutboxState::Failed);
    assert_eq!(rows[0].last_error.as_deref(), Some(MAYBE_SENT_REASON));
    assert!(rows[0].may_have_been_sent());
    assert!(rig.drain().contains(&Change::SendFailed {
        account: rig.account,
        id
    }));
    // No copy in Sent for a message that is not known to have gone out.
    assert!(rig.sent_folder().is_empty());

    rig.smtp.set_misbehavior(SmtpMisbehavior::None);
    rig.advance(3_600_000);
    rig.run().await;
    rig.run().await;
    assert_eq!(rig.accepted(), 1);

    // The person looked in Sent and chose Retry: now it is sent, once more, on purpose.
    rig.ui.retry_send(rig.account, id).unwrap();
    rig.run().await;
    assert_eq!(rig.accepted(), 2);
    assert!(rig.outbox().is_empty());
}

#[tokio::test]
async fn a_sent_message_is_in_the_sent_folder_exactly_once() {
    let rig = Rig::new().await;
    rig.send("filed-1@example.org");
    rig.advance(WINDOW_MS + 1);
    rig.run().await;
    assert_eq!(rig.accepted(), 1);
    assert_eq!(rig.sent_folder(), vec!["<filed-1@example.org>".to_owned()]);
    let stored = &rig.imap.messages("Sent")[0];
    assert!(stored.flags.contains("\\Seen"));
    assert!(rig.outbox().is_empty());
    assert_eq!(rig.outbox_rows_on_disk(), 0);

    rig.advance(3_600_000);
    rig.run().await;
    rig.run().await;
    assert_eq!(rig.sent_folder().len(), 1);
    assert_eq!(rig.accepted(), 1);
}

#[tokio::test]
async fn a_kill_after_the_append_but_before_it_was_recorded_does_not_file_a_second_copy() {
    let mut rig = Rig::new().await;
    rig.kill_and_restart(Arc::new(StallsWhileFiling {
        inner: rig.real_submitter(),
        append_first: true,
    }));
    rig.send("crash-after-append@example.org");
    rig.advance(WINDOW_MS + 1);
    let stuck = rig.run_in_background();
    wait_until("the copy to be appended", || rig.sent_folder().len() == 1).await;
    assert_eq!(rig.accepted(), 1);

    // Killed here: sent, copy saved, nothing recorded about it.
    rig.kill_and_restart(rig.real_submitter());
    rig.advance(1_000);
    rig.run().await;
    assert_eq!(
        rig.sent_folder().len(),
        1,
        "the lookup must prevent a duplicate"
    );
    assert_eq!(rig.accepted(), 1, "the message must not be sent again");
    assert!(rig.outbox().is_empty());
    assert_eq!(rig.outbox_rows_on_disk(), 0);
    stuck.abort();
}

#[tokio::test]
async fn a_kill_between_submitting_and_filing_files_the_copy_once_after_the_restart() {
    let mut rig = Rig::new().await;
    rig.kill_and_restart(Arc::new(StallsWhileFiling {
        inner: rig.real_submitter(),
        append_first: false,
    }));
    rig.send("crash-before-append@example.org");
    rig.advance(WINDOW_MS + 1);
    let stuck = rig.run_in_background();
    wait_until("the message to be accepted", || rig.accepted() == 1).await;
    // Give the engine time to record "submitted" and start the filing step.
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(rig.sent_folder().is_empty());

    rig.kill_and_restart(rig.real_submitter());
    // The row was not Failed: the message did go out, only its copy was missing.
    assert!(rig.outbox().is_empty());
    assert!(rig
        .drain()
        .iter()
        .all(|c| !matches!(c, Change::SendFailed { .. })));
    rig.advance(1_000);
    rig.run().await;
    assert_eq!(
        rig.sent_folder(),
        vec!["<crash-before-append@example.org>".to_owned()]
    );
    assert_eq!(rig.accepted(), 1);
    rig.run().await;
    assert_eq!(rig.sent_folder().len(), 1);
    assert_eq!(rig.outbox_rows_on_disk(), 0);
    stuck.abort();
}

#[tokio::test]
async fn when_the_server_files_sent_itself_nothing_is_appended() {
    let rig = Rig::with(true).await;
    rig.send("gmail-like@example.org");
    rig.advance(WINDOW_MS + 1);
    rig.run().await;
    assert_eq!(rig.accepted(), 1);
    assert!(rig.sent_folder().is_empty());
    assert!(!rig
        .imap
        .commands()
        .iter()
        .any(|c| c.to_ascii_uppercase().contains("APPEND")));
    assert!(rig.outbox().is_empty());
    assert_eq!(rig.outbox_rows_on_disk(), 0);
}

#[tokio::test]
async fn a_copy_that_is_not_utf8_is_refused_with_a_plain_reason() {
    let rig = Rig::new().await;
    let sub = rig.real_submitter();
    let err = sub
        .file_in_sent(Some("<x@example.org>"), &[0xff, 0xfe, b'\r', b'\n'])
        .await
        .unwrap_err();
    assert!(matches!(err, croft_protocols::BackendError::Permanent(_)));
    assert!(rig.sent_folder().is_empty());
}

#[tokio::test]
async fn a_refused_sign_in_stops_the_whole_account_and_signing_in_again_sends_the_message() {
    let mut rig = Rig::new().await;
    // A message on the IMAP side, synced, so that a queue operation can be tested too.
    rig.imap.add_message(
        "INBOX",
        &sample_message("inbox-1@example.org", "Hello", "Ann <ann@example.org>"),
    );
    rig.ui.run_once(rig.account, true).await.unwrap();
    let row = rig.ui.read(|db| {
        let f = croft_storage::folders::find_by_path(db.connection(), rig.account, "INBOX")
            .unwrap()
            .unwrap();
        croft_storage::messages::find_by_uid(db.connection(), rig.account, f.id, 1)
            .unwrap()
            .unwrap()
    });
    rig.drain();

    rig.smtp
        .set_reply(Stage::Auth, 535, "5.7.8 bad credentials");
    let id = rig.send("auth-1@example.org");
    rig.advance(WINDOW_MS + 1);
    rig.run().await;
    assert!(rig.drain().contains(&Change::SignInNeeded {
        account: rig.account
    }));
    assert_eq!(rig.accepted(), 0);
    // The message is back in the queue, the attempt not counted.
    let rows = rig.outbox();
    assert_eq!(
        (rows[0].id, rows[0].state, rows[0].attempts),
        (id, OutboxState::Queued, 0)
    );

    // The IMAP side is stopped too: a queued operation does not run, and nothing syncs.
    rig.ui.mark_seen(rig.account, row.id, true).unwrap();
    rig.advance(WINDOW_MS);
    let connections = rig.imap.connection_count();
    rig.ui.run_once(rig.account, true).await.unwrap();
    rig.run().await;
    assert_eq!(rig.imap.connection_count(), connections);
    assert!(!rig.imap.messages("INBOX")[0].flags.contains("\\Seen"));
    assert_eq!(rig.accepted(), 0);

    // Signed in again: the message goes out, once, and the operation runs.
    rig.smtp.clear_reply(Stage::Auth);
    rig.ui.signed_in(rig.account);
    rig.run().await;
    assert_eq!(rig.accepted(), 1);
    assert!(rig.imap.messages("INBOX")[0].flags.contains("\\Seen"));
    assert!(rig.outbox().is_empty());
    rig.run().await;
    assert_eq!(rig.accepted(), 1);
}

#[tokio::test]
async fn a_temporary_failure_is_retried_later_and_the_message_goes_out_once() {
    let mut rig = Rig::new().await;
    rig.smtp.set_reply(Stage::MailFrom, 451, "4.3.0 try later");
    rig.send("retry-1@example.org");
    rig.advance(WINDOW_MS + 1);
    rig.run().await;
    assert_eq!(rig.accepted(), 0);
    let rows = rig.outbox();
    assert_eq!((rows[0].state, rows[0].attempts), (OutboxState::Queued, 1));
    assert!(rows[0].send_at_ms > rig.clock.now_ms());
    assert!(!rig
        .drain()
        .iter()
        .any(|c| matches!(c, Change::SendFailed { .. })));

    // Still backing off: no new connection.
    let connections = rig.smtp.connection_count();
    rig.run().await;
    assert_eq!(rig.smtp.connection_count(), connections);

    rig.smtp.clear_reply(Stage::MailFrom);
    rig.advance(60_000);
    rig.run().await;
    assert_eq!(rig.accepted(), 1);
    assert!(rig.outbox().is_empty());
}

#[tokio::test]
async fn a_refused_message_fails_and_can_be_edited_retried_or_discarded() {
    let mut rig = Rig::new().await;
    rig.smtp.set_reply(Stage::MailFrom, 550, "5.7.1 no");
    let first = rig.send("refused-1@example.org");
    rig.advance(WINDOW_MS + 1);
    rig.run().await;
    let rows = rig.outbox();
    assert_eq!((rows[0].id, rows[0].state), (first, OutboxState::Failed));
    assert!(!rows[0].may_have_been_sent());
    assert!(rows[0].last_error.is_some());
    let changes = rig.drain();
    assert!(changes.contains(&Change::SendFailed {
        account: rig.account,
        id: first
    }));
    assert!(changes.contains(&Change::Outbox {
        account: rig.account
    }));

    // Edit gives the bytes back for the composer.
    let back = rig.ui.edit_back_to_draft(rig.account, first).unwrap();
    assert_eq!(back.mime, mime("refused-1@example.org"));
    assert!(rig.outbox().is_empty());

    // Retry after the server's mind is changed.
    let second = rig.send("refused-2@example.org");
    rig.advance(WINDOW_MS + 1);
    rig.run().await;
    assert_eq!(rig.outbox()[0].state, OutboxState::Failed);
    rig.smtp.clear_reply(Stage::MailFrom);
    rig.ui.retry_send(rig.account, second).unwrap();
    rig.run().await;
    assert_eq!(rig.accepted(), 1);

    // Discard.
    rig.smtp.set_reply(Stage::MailFrom, 550, "5.7.1 no");
    let third = rig.send("refused-3@example.org");
    rig.advance(WINDOW_MS + 1);
    rig.run().await;
    rig.ui.discard_send(rig.account, third).unwrap();
    assert!(rig.outbox().is_empty());
    assert_eq!(rig.outbox_rows_on_disk(), 0);
    assert!(matches!(
        rig.ui.discard_send(rig.account, third),
        Err(OutboxActionError::NoSuchMessage)
    ));
    assert!(matches!(
        rig.ui.retry_send(rig.account, 12345),
        Err(OutboxActionError::NoSuchMessage)
    ));
}

#[tokio::test]
async fn a_copy_that_can_never_be_saved_is_given_up_with_one_notice() {
    let mut rig = Rig::new().await;
    // The message goes out, but the copy is refused for good: it is given up at once, the message
    // is not failed, and the person is told once.
    rig.kill_and_restart(Arc::new(RefusesCopy(rig.real_submitter())));
    let id = rig.send("not-filed@example.org");
    rig.advance(WINDOW_MS + 1);
    rig.run().await;
    assert_eq!(rig.accepted(), 1);
    assert!(rig.outbox().is_empty());
    let told: Vec<Change> = rig
        .drain()
        .into_iter()
        .filter(|c| matches!(c, Change::NotFiled { .. }))
        .collect();
    assert_eq!(
        told,
        vec![Change::NotFiled {
            account: rig.account,
            id
        }]
    );
    rig.advance(3_600_000);
    rig.run().await;
    assert_eq!(rig.accepted(), 1);
    assert!(rig
        .drain()
        .iter()
        .all(|c| !matches!(c, Change::NotFiled { .. })));
    assert_eq!(rig.outbox_rows_on_disk(), 0);
}

#[tokio::test]
async fn one_accounts_run_never_sends_another_accounts_mail() {
    let mut rig = Rig::new().await;
    // A second account on the same file, with its own servers.
    let smtp2 = SmtpServer::start_default().await;
    let imap2 = TestServer::start_default().await;
    let backend2 = imap_for(&imap2);
    let db = Db::open(&rig.path, None).unwrap();
    let second = create_account(&db, "second@example.org");
    drop(db);
    let backend: Arc<dyn MailBackend> = backend2.clone();
    rig.engine
        .as_ref()
        .unwrap()
        .attach_account_with_submitter(second, backend, submitter(&smtp2, &backend2, false))
        .unwrap();

    rig.send("first@example.org");
    rig.ui
        .send(envelope(second), mime("second@example.org"))
        .unwrap();
    rig.advance(WINDOW_MS + 1);

    rig.run().await;
    assert_eq!(rig.accepted(), 1);
    assert_eq!(smtp2.accepted().len(), 0);
    assert_eq!(rig.ui.outbox(second).len(), 1);
    assert!(rig
        .drain()
        .iter()
        .all(|c| !matches!(c, Change::SendFailed { .. })));

    rig.ui.run_once(second, false).await.unwrap();
    assert_eq!(rig.accepted(), 1);
    assert_eq!(smtp2.accepted().len(), 1);
    assert!(rig.ui.outbox(second).is_empty());
    assert_eq!(imap2.messages("Sent").len(), 1);
    assert_eq!(rig.imap.messages("Sent").len(), 1);

    // An account that is offline holds back only its own mail.
    rig.send("first-b@example.org");
    rig.ui
        .send(envelope(second), mime("second-b@example.org"))
        .unwrap();
    rig.ui.set_online(second, false);
    rig.advance(WINDOW_MS + 1);
    rig.run().await;
    rig.ui.run_once(second, false).await.unwrap();
    assert_eq!(rig.accepted(), 2);
    assert_eq!(smtp2.accepted().len(), 1);
    rig.ui.set_online(second, true);
    rig.ui.run_once(second, false).await.unwrap();
    assert_eq!(smtp2.accepted().len(), 2);
}

#[tokio::test]
async fn the_account_task_wakes_by_itself_when_the_undo_window_ends() {
    let smtp = SmtpServer::start_default().await;
    let imap = TestServer::start_default().await;
    let backend = imap_for(&imap);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("croft.db");
    let db = Db::open(&path, None).unwrap();
    let account = create_account(&db, "alice@example.org");
    drop(db);
    let engine = Engine::open_with_outbox(
        Db::open(&path, None).unwrap(),
        Db::open(&path, None).unwrap(),
        Arc::new(SystemClock),
        EngineConfig {
            undo_window_ms: 400,
            ..EngineConfig::default()
        },
    )
    .unwrap();
    let ui = engine.handle();
    let imap_backend: Arc<dyn MailBackend> = backend.clone();
    engine
        .add_account_with_submitter(account, imap_backend, submitter(&smtp, &backend, false))
        .unwrap();
    // Let the first cycle (a full sync) finish, so the task is asleep when the message arrives.
    tokio::time::sleep(Duration::from_millis(300)).await;

    ui.send(envelope(account), mime("task-1@example.org"))
        .unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(smtp.accepted().len(), 0, "still inside the window");
    wait_until("the task to send by itself", || smtp.accepted().len() == 1).await;
    wait_until("the copy to be filed", || imap.messages("Sent").len() == 1).await;
    wait_until("the outbox to empty", || ui.outbox(account).is_empty()).await;
    engine.shutdown().await;
    assert_eq!(smtp.accepted().len(), 1);
}
