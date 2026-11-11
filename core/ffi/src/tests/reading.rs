// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Reading through the exported surface: the sample mailbox (no network) for conversations,
//! remote-image decisions, known senders and saving attachments; the in-process IMAP server for
//! the real fetch, offline reads and "never marks read".

use super::*;
use crate::reading::{BodyState, MessageBody, ReadError, RemoteImagesSetting};

fn sample_core() -> (TempDb, Arc<Core>, i64) {
    let db = TempDb::new();
    let core = db.open(&MemProvider::new());
    let account = block(core.add_demo_account()).unwrap().id;
    block(wait_for(&core, "sample inbox", || {
        inbox_rows(&core, account).len() >= 10
    }));
    (db, core, account)
}

fn inbox_rows(core: &Core, account: i64) -> Vec<MessageRow> {
    let Some(f) = core
        .list_folders(account)
        .unwrap()
        .into_iter()
        .find(|f| f.path == "INBOX")
    else {
        return Vec::new();
    };
    core.list_messages(
        MessageScope::Folder {
            account,
            folder_id: f.id,
        },
        None,
        500,
    )
    .unwrap()
    .rows
}

fn load(core: &Core, id: i64, setting: RemoteImagesSetting) -> MessageBody {
    block(core.load_body(id, setting)).unwrap()
}

#[test]
fn remote_images_follow_the_setting_the_message_and_the_known_senders() {
    let (_db, core, account) = sample_core();
    let rows = inbox_rows(&core, account);
    let (row, body) = rows
        .iter()
        .map(|r| (r.clone(), load(&core, r.id, RemoteImagesSetting::Ask)))
        .find(|(_, b)| b.blocked_remote_count > 0)
        .expect("the sample mailbox has a message with a remote image");
    let html = body.html_sanitised.clone().unwrap();
    assert!(!body.remote_allowed);
    assert!(!html.contains("src=\"https://"), "{html}");
    assert!(html.contains(croft_reader::BLOCKED_IMAGE_PLACEHOLDER));

    // Known senders: only under that setting, only once the sender is trusted.
    let sender = row.sender_address.clone().unwrap();
    assert!(!load(&core, row.id, RemoteImagesSetting::KnownSenders).remote_allowed);
    core.trust_sender(account, sender.to_uppercase()).unwrap();
    assert_eq!(
        core.trusted_senders(account).unwrap(),
        [sender.to_lowercase()]
    );
    let known = load(&core, row.id, RemoteImagesSetting::KnownSenders);
    assert!(known.remote_allowed && known.sender_known);
    assert_eq!(known.blocked_remote_count, 0);
    assert!(known.html_sanitised.unwrap().contains("src=\"https://"));
    assert!(!load(&core, row.id, RemoteImagesSetting::Ask).remote_allowed);
    assert!(core.untrust_sender(account, sender.clone()).unwrap());

    // "Show images" on the message.
    core.allow_remote_content(row.id).unwrap();
    let shown = load(&core, row.id, RemoteImagesSetting::Ask);
    assert!(shown.remote_allowed);
    assert!(shown.html_sanitised.unwrap().contains("src=\"https://"));
    // Never wins over everything.
    let never = load(&core, row.id, RemoteImagesSetting::Never);
    assert!(!never.remote_allowed && never.blocked_remote_count > 0);

    // Loading never marked anything read.
    let after = inbox_rows(&core, account);
    for r in &rows {
        let now = after.iter().find(|a| a.id == r.id).unwrap();
        assert_eq!(now.seen, r.seen);
    }
}

#[test]
fn a_conversation_lists_its_messages_and_their_body_state() {
    let (_db, core, account) = sample_core();
    let rows = inbox_rows(&core, account);
    let first = &rows[0];
    let c = core.open_conversation(first.id).unwrap();
    assert_eq!(c.account, account);
    assert_eq!(c.opened_id, first.id);
    assert!(c.messages.iter().any(|m| m.id == first.id));
    assert!(c.messages.windows(2).all(|w| w[0].date_ms <= w[1].date_ms));
    let state = |c: &crate::reading::Conversation| {
        c.messages
            .iter()
            .find(|m| m.id == first.id)
            .unwrap()
            .body_state
    };
    assert_eq!(state(&c), BodyState::NotLoaded);
    load(&core, first.id, RemoteImagesSetting::Ask);
    let c = core.open_conversation(first.id).unwrap();
    assert_eq!(state(&c), BodyState::Loaded);
    // A thread in the sample mailbox (a reply) brings its other messages along.
    let longest = rows
        .iter()
        .map(|r| core.open_conversation(r.id).unwrap().messages.len())
        .max()
        .unwrap();
    assert!(longest >= 2, "the sample mailbox has a conversation");
}

#[test]
fn attachments_are_saved_inside_the_given_folder_only() {
    let (_db, core, account) = sample_core();
    let rows = inbox_rows(&core, account);
    let (row, body) = rows
        .iter()
        .map(|r| (r.clone(), load(&core, r.id, RemoteImagesSetting::Ask)))
        .find(|(_, b)| b.attachments.iter().any(|a| !a.shown_inline))
        .expect("the sample mailbox has an attachment");
    let attachment = body.attachments.iter().find(|a| !a.shown_inline).unwrap();
    assert!(!attachment.blocked);
    let dir = tempfile::tempdir().unwrap();
    let dest = dir.path().display().to_string();
    let path = block(core.save_attachment(row.id, attachment.id, dest.clone())).unwrap();
    let path = std::path::PathBuf::from(path);
    assert_eq!(
        path.parent().unwrap(),
        std::fs::canonicalize(dir.path()).unwrap()
    );
    assert!(std::fs::metadata(&path).unwrap().len() > 0);
    // Saving again makes a second file, never overwriting the first.
    let again = block(core.save_attachment(row.id, attachment.id, dest.clone())).unwrap();
    assert_ne!(std::path::PathBuf::from(again), path);

    // Ids that do not belong together, and bad destinations.
    let other = rows.iter().find(|r| r.id != row.id).unwrap();
    assert_eq!(
        block(core.save_attachment(other.id, attachment.id, dest.clone())),
        Err(ReadError::NoSuchAttachment)
    );
    assert_eq!(
        block(core.save_attachment(row.id, 999_999, dest)),
        Err(ReadError::NoSuchAttachment)
    );
    assert_eq!(
        block(core.save_attachment(row.id, attachment.id, "relative/dir".into())),
        Err(ReadError::BadDestination)
    );
}

#[test]
fn unknown_ids_and_bad_input_are_refused() {
    let (_db, core, account) = sample_core();
    assert_eq!(
        block(core.load_body(999_999, RemoteImagesSetting::Ask)).unwrap_err(),
        ReadError::NoSuchMessage
    );
    assert_eq!(
        core.open_conversation(999_999).unwrap_err(),
        ReadError::NoSuchMessage
    );
    assert_eq!(
        core.allow_remote_content(999_999).unwrap_err(),
        ReadError::NoSuchMessage
    );
    assert_eq!(
        core.trust_sender(account + 1000, "ann@example.org".into()),
        Err(ReadError::NoSuchAccount)
    );
    assert_eq!(
        core.trusted_senders(account + 1000),
        Err(ReadError::NoSuchAccount)
    );
    assert_eq!(
        core.trust_sender(account, "not an address".into()),
        Err(ReadError::InvalidAddress)
    );
}

/// A multipart message with a PDF attachment and an HTML part with a remote image.
fn with_attachment(id: &str) -> String {
    format!(
        "From: Ann Example <ann@example.org>\r\nTo: alice@example.org\r\nSubject: Report\r\n\
         Date: Mon, 02 Mar 2026 09:30:00 +0000\r\nMessage-ID: <{id}>\r\nMIME-Version: 1.0\r\n\
         Content-Type: multipart/mixed; boundary=B\r\n\r\n--B\r\nContent-Type: text/html\r\n\r\n\
         <p>See the report <img src=\"https://img.example.org/chart.png\"></p>\r\n--B\r\n\
         Content-Type: application/pdf; name=report.pdf\r\nContent-Transfer-Encoding: base64\r\n\
         Content-Disposition: attachment; filename=report.pdf\r\n\r\nJVBERi0xLjQgdGVzdA==\r\n--B--\r\n"
    )
}

#[test]
fn over_imap_a_body_is_fetched_without_marking_it_read_and_then_read_offline() {
    let rig = Rig::new(|s| {
        s.add_message("INBOX", &with_attachment("r1@example.org"));
        s.add_message("INBOX", &with_attachment("r2@example.org"));
    });
    rig.wait_for_inbox(2);
    let rows = rig.core.list_messages(rig.inbox(), None, 10).unwrap().rows;
    let (first, second) = (&rows[0], &rows[1]);

    let body = load(&rig.core, first.id, RemoteImagesSetting::Ask);
    assert_eq!(body.blocked_remote_count, 1);
    assert_eq!(body.attachments.len(), 1);
    assert_eq!(body.attachments[0].file_name, "report.pdf");
    let commands = rig.server.commands().join("\n");
    assert!(commands.contains("BODY.PEEK[]"), "{commands}");
    assert!(
        rig.server
            .messages("INBOX")
            .iter()
            .all(|m| !m.flags.contains("\\Seen")),
        "nothing was marked read"
    );

    rig.core.set_network_available(false).unwrap();
    let offline = load(&rig.core, first.id, RemoteImagesSetting::Ask);
    assert_eq!(offline.attachments.len(), 1);
    let dir = tempfile::tempdir().unwrap();
    let saved = block(rig.core.save_attachment(
        first.id,
        offline.attachments[0].id,
        dir.path().display().to_string(),
    ))
    .unwrap();
    assert_eq!(std::fs::read(saved).unwrap(), b"%PDF-1.4 test");
    assert_eq!(
        block(rig.core.load_body(second.id, RemoteImagesSetting::Ask)).unwrap_err(),
        ReadError::Offline
    );
}
