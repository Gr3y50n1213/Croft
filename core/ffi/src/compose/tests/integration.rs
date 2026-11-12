//! The compose flow against a real mail server (GreenMail in a container), through the exported
//! calls: a draft is appended to the server and replaced, a message sent with no network waits and
//! is delivered exactly once when the connection returns, and Undo inside the window sends
//! nothing. Only built with `--features integration`; needs GreenMail running:
//!
//! ```text
//! CROFT_GREENMAIL_SMTP_PORT=23025 CROFT_GREENMAIL_IMAP_PORT=23143 \
//!   CROFT_GREENMAIL_IMAPS_PORT=23993 CROFT_GREENMAIL_SMTPS_PORT=23465 \
//!   docker compose -p t18c -f test-servers/docker-compose.yml up -d --wait greenmail
//! CROFT_GREENMAIL_SMTP_PORT=23025 CROFT_GREENMAIL_IMAP_PORT=23143 \
//!   cargo test -p croft-ffi --features integration compose::tests::integration
//! ```
//!
//! The default ports are GreenMail's own (SMTP 3025, IMAP 3143). The users are `alice` (password
//! `alice`, address `alice@localhost`), who is the account here, and `test` (password `test`).

use croft_protocols::{
    Credential, ImapBackend, ImapConfig, MailBackend, Security, SpecialUse, Uid,
};

use super::*;

fn port(var: &str, default: u16) -> u16 {
    std::env::var(var)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

fn unique(prefix: &str) -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    format!("{prefix}{nanos}")
}

fn imap_as(user: &str, password: &str) -> ImapBackend {
    ImapBackend::new(
        ImapConfig::new(
            "127.0.0.1",
            port("CROFT_GREENMAIL_IMAP_PORT", 3143),
            Security::PlaintextForTestsOnly,
            user,
        )
        .with_timeout(Duration::from_secs(20)),
        Credential::password(password),
    )
    .unwrap()
}

/// How many messages in `folder` carry this Message-ID (a folder that does not exist has none).
fn count(imap: &ImapBackend, folder: &str, message_id: &str) -> usize {
    block(async {
        let Ok(snapshot) = imap.snapshot(folder).await else {
            return 0;
        };
        let uids: Vec<Uid> = snapshot.messages.keys().copied().collect();
        if uids.is_empty() {
            return 0;
        }
        imap.fetch_headers(folder, &uids)
            .await
            .unwrap()
            .iter()
            .filter(|h| h.message_id.as_deref() == Some(message_id))
            .count()
    })
}

/// The account `alice@localhost` on GreenMail, opened in a core.
struct Green {
    db: TempDb,
    core: Arc<Core>,
    account: i64,
}

impl Green {
    fn new() -> Self {
        let db = TempDb::new();
        let settings = StoredSettings {
            version: SETTINGS_VERSION,
            imap: server(
                "127.0.0.1",
                port("CROFT_GREENMAIL_IMAP_PORT", 3143),
                "alice",
            ),
            smtp: Some(server(
                "127.0.0.1",
                port("CROFT_GREENMAIL_SMTP_PORT", 3025),
                "alice",
            )),
            pinned_fingerprint: None,
        };
        let account = insert_account_with(&db.path, &settings, "alice@localhost");
        let provider = MemProvider::new();
        provider.store(account, password_cred("alice"));
        let core = db.open(&provider);
        core.set_undo_window_ms(0);
        settle(&core);
        Self { db, core, account }
    }

    fn draft(&self, subject: &str, body: &str) -> SavedDraft {
        self.core
            .save_draft(
                self.account,
                None,
                content(&["test@localhost"], subject, body),
            )
            .unwrap()
    }
}

#[test]
fn a_draft_is_appended_then_replaced_and_deleted_on_a_real_server() {
    let g = Green::new();
    let alice = imap_as("alice", "alice");
    let d = g.draft(&unique("Draft "), "first version");
    assert_eq!(
        block(g.core.sync_draft_to_server(g.account, d.id, false)).unwrap(),
        DraftSyncOutcome::Saved
    );
    // GreenMail has no Drafts folder at first: the core made one.
    let folder = block(alice.list_folders())
        .unwrap()
        .into_iter()
        .find(|f| f.special_use == SpecialUse::Drafts || f.path == "Drafts")
        .expect("a Drafts folder")
        .path;
    assert_eq!(count(&alice, &folder, &d.message_id), 1);

    g.core
        .save_draft(
            g.account,
            Some(d.id),
            content(&["test@localhost"], "Second", "second version"),
        )
        .unwrap();
    assert_eq!(
        block(g.core.sync_draft_to_server(g.account, d.id, true)).unwrap(),
        DraftSyncOutcome::Saved
    );
    assert_eq!(
        count(&alice, &folder, &d.message_id),
        1,
        "replaced, not added"
    );
    let uid = block(alice.find_by_message_id(&folder, &d.message_id))
        .unwrap()
        .unwrap();
    let flags = block(alice.snapshot(&folder)).unwrap().messages;
    assert!(flags[&uid].draft, "the copy is flagged \\Draft");

    g.core.discard_draft(g.account, d.id).unwrap();
    wait_until("the copy is deleted", || {
        count(&alice, &folder, &d.message_id) == 0
    });
}

#[test]
fn a_message_sent_with_no_network_waits_and_is_delivered_exactly_once() {
    let g = Green::new();
    let (alice, test) = (imap_as("alice", "alice"), imap_as("test", "test"));
    g.core.set_network_available(false).unwrap();
    let d = g.draft(&unique("Offline "), "Sent while the phone had no network");
    g.core.send(g.account, d.id).unwrap();
    std::thread::sleep(Duration::from_millis(1500));
    assert_eq!(count(&test, "INBOX", &d.message_id), 0);
    assert_eq!(
        g.core.list_outbox(g.account).unwrap()[0].state,
        OutboxItemState::Queued
    );

    g.core.set_network_available(true).unwrap();
    wait_until("delivered", || count(&test, "INBOX", &d.message_id) == 1);
    wait_until("filed in Sent", || {
        count(&alice, "Sent", &d.message_id) == 1
    });
    wait_until("outbox empty", || {
        g.core.list_outbox(g.account).unwrap().is_empty()
    });
    // A second connectivity signal and the passing of time change nothing.
    g.core.set_network_available(true).unwrap();
    std::thread::sleep(Duration::from_millis(1500));
    assert_eq!(count(&test, "INBOX", &d.message_id), 1);
    assert_eq!(count(&alice, "Sent", &d.message_id), 1);
    wait_until("draft finished", || {
        g.core.list_drafts(g.account).unwrap().is_empty()
    });
}

#[test]
fn undo_inside_the_window_sends_nothing_to_a_real_server() {
    let g = Green::new();
    let (alice, test) = (imap_as("alice", "alice"), imap_as("test", "test"));
    g.core.set_undo_window_ms(60_000);
    let d = g.draft(&unique("Undone "), "never sent");
    let started = g.core.send(g.account, d.id).unwrap();
    assert_eq!(
        g.core.undo_send(g.account, started.outbox_id).unwrap(),
        d.id
    );
    std::thread::sleep(Duration::from_millis(2000));
    assert_eq!(count(&test, "INBOX", &d.message_id), 0);
    assert_eq!(count(&alice, "Sent", &d.message_id), 0);
    assert!(g.core.list_outbox(g.account).unwrap().is_empty());
    assert_eq!(g.core.load_draft(g.account, d.id).unwrap().outbox_id, None);
    assert!(g.db.path.exists());
}
