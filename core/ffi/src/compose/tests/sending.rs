//! Sending for a real account, end to end, against the in-process IMAP and SMTP servers: the
//! message goes out once, is filed in Sent, its draft is finished; offline, Undo, failures, and
//! the crash points of "never twice". Also the draft's copy on the server.

use croft_test_support::{SmtpMisbehavior, Stage};

use crate::core::now_ms;

use super::*;

fn first_outbox(m: &Mail) -> OutboxItem {
    m.core().list_outbox(m.account).unwrap().remove(0)
}

fn drafts_on_server(m: &Mail) -> Vec<croft_test_support::StoredMessage> {
    m.imap.messages("Drafts")
}

#[test]
fn a_message_is_delivered_once_filed_in_sent_and_its_draft_is_finished() {
    let m = Mail::new();
    let mut c = content(&["bob@example.org"], "Lunch", "Hello Bob");
    c.cc = vec![address(Some("Cy"), "cy@example.org")];
    c.bcc = vec![address(None, "hidden@example.org")];
    let id = m.core().save_draft(m.account, None, c).unwrap();

    // The copy on the server has the Bcc recipient (it is the person's own mailbox).
    assert_eq!(
        block(m.core().sync_draft_to_server(m.account, id.id, false)).unwrap(),
        DraftSyncOutcome::Saved
    );
    let on_server = drafts_on_server(&m);
    assert_eq!(on_server.len(), 1);
    assert!(on_server[0].raw.contains("Bcc: hidden@example.org"));
    assert!(on_server[0].flags.contains("\\Draft"));

    let started = m.core().send(m.account, id.id).unwrap();
    assert!(started.send_at_ms > 0);
    wait_until("the server has the message", || m.accepted() == 1);
    let got = &m.smtp.accepted()[0];
    assert_eq!(got.mail_from, "alice@example.org");
    assert_eq!(
        got.rcpt_to,
        ["bob@example.org", "cy@example.org", "hidden@example.org"]
    );
    let text = String::from_utf8(got.data.clone()).unwrap();
    assert!(
        text.contains(&format!("Message-ID: {}", id.message_id)),
        "{text}"
    );
    assert!(text.contains("Subject: Lunch\r\n"), "{text}");
    assert!(
        text.contains("From: Alice Example <alice@example.org>\r\n"),
        "{text}"
    );
    // No Bcc header, and the hidden recipient appears nowhere in the message. (Not a search for the
    // letters "bcc": the random hex of the Message-ID can contain them.)
    let head = text.split("\r\n\r\n").next().unwrap();
    assert!(
        head.split("\r\n")
            .all(|l| !l.to_ascii_lowercase().starts_with("bcc:")),
        "{text}"
    );
    assert!(!text.contains("hidden@example.org"), "{text}");

    // The copy in Sent, once.
    wait_until("the copy in Sent", || m.imap.messages("Sent").len() == 1);
    assert!(m.imap.messages("Sent")[0].flags.contains("\\Seen"));
    // The draft is finished and its copy on the server deleted only now that the send succeeded.
    wait_until("the draft is finished", || {
        m.core().list_drafts(m.account).unwrap().is_empty() && drafts_on_server(&m).is_empty()
    });
    assert!(m.core().list_outbox(m.account).unwrap().is_empty());
    assert_eq!(
        m.core().load_draft(m.account, id.id).unwrap_err(),
        ComposeError::NoSuchDraft
    );
    // The recipients are remembered, for the suggestions.
    let found = m
        .core()
        .suggest_recipients(m.account, "cy".into(), 10)
        .unwrap();
    assert_eq!(found, vec![address(Some("Cy"), "cy@example.org")]);
    assert_eq!(
        m.core()
            .suggest_recipients(m.account, "hid".into(), 10)
            .unwrap()
            .len(),
        1
    );
    // And nothing more goes out, however long we wait.
    stays(600, || {
        m.accepted() == 1 && m.imap.messages("Sent").len() == 1
    });
}

#[test]
fn the_server_draft_stays_until_the_send_has_succeeded() {
    let m = Mail::new();
    m.core().set_undo_window_ms(60_000);
    let id = m.draft("Held");
    block(m.core().sync_draft_to_server(m.account, id, true)).unwrap();
    m.core().send(m.account, id).unwrap();
    // Inside the Undo window: queued, nothing sent, the draft copy still on the server.
    stays(400, || m.accepted() == 0 && drafts_on_server(&m).len() == 1);
    assert_eq!(first_outbox(&m).state, OutboxItemState::Queued);
}

#[test]
fn offline_a_message_is_queued_and_sent_exactly_once_when_the_connection_returns() {
    let m = Mail::new();
    m.core().set_network_available(false).unwrap();
    let id = m.draft("Offline");
    m.core().send(m.account, id).unwrap();
    stays(500, || m.accepted() == 0);
    let item = first_outbox(&m);
    assert_eq!(item.state, OutboxItemState::Queued);
    assert_eq!(item.subject, "Offline");
    assert_eq!(
        (item.recipient_count, item.to_preview.as_slice()),
        (1, ["bob@example.org".to_owned()].as_slice())
    );
    // A draft written offline is kept locally, and the copy is declined for now.
    let other = m.draft("Written offline");
    assert_eq!(
        block(m.core().sync_draft_to_server(m.account, other, true)).unwrap(),
        DraftSyncOutcome::Offline
    );
    assert!(drafts_on_server(&m).is_empty());

    m.core().set_network_available(true).unwrap();
    wait_until("sent after reconnect", || m.accepted() == 1);
    // The draft written offline was copied by itself.
    wait_until("the offline draft is on the server", || {
        drafts_on_server(&m).len() == 1
    });
    // A second "online" signal and the passing of time send nothing more.
    m.core().set_network_available(true).unwrap();
    stays(700, || m.accepted() == 1);
    wait_until("outbox empty", || {
        m.core().list_outbox(m.account).unwrap().is_empty()
    });
}

#[test]
fn undo_inside_the_window_sends_nothing_and_returns_the_draft() {
    let m = Mail::new();
    m.core().set_undo_window_ms(60_000);
    let id = m.draft("Oops");
    let before = m.core().load_draft(m.account, id).unwrap().message_id;
    let started = m.core().send(m.account, id).unwrap();
    let back = m.core().undo_send(m.account, started.outbox_id).unwrap();
    assert_eq!(back, id);
    // Nothing reached the server, so the draft keeps its Message-ID.
    assert_eq!(
        m.core().load_draft(m.account, id).unwrap().message_id,
        before
    );
    assert!(m.core().list_outbox(m.account).unwrap().is_empty());
    let d = m.core().load_draft(m.account, id).unwrap();
    assert_eq!(d.outbox_id, None);
    assert_eq!(d.content.body, "Hello Bob");
    // The same message id: it is still the same draft.
    stays(600, || m.accepted() == 0);
    // Undo twice is "nothing to undo", not a crash.
    assert_eq!(
        m.core()
            .undo_send(m.account, started.outbox_id)
            .unwrap_err(),
        ComposeError::NoSuchOutboxItem
    );
    // The person edits and sends again: one message, with the edit.
    m.core().set_undo_window_ms(0);
    m.core()
        .save_draft(
            m.account,
            Some(id),
            content(&["bob@example.org"], "Oops", "Corrected text"),
        )
        .unwrap();
    m.core().send(m.account, id).unwrap();
    wait_until("sent", || m.accepted() == 1);
    assert!(String::from_utf8_lossy(&m.smtp.accepted()[0].data).contains("Corrected text"));
    stays(500, || m.accepted() == 1);
}

#[test]
fn undo_after_the_window_is_too_late_and_the_message_still_goes_out_once() {
    let m = Mail::new();
    m.core().set_undo_window_ms(150);
    let id = m.draft("Late");
    let started = m.core().send(m.account, id).unwrap();
    wait_until("sent", || m.accepted() == 1);
    let err = m
        .core()
        .undo_send(m.account, started.outbox_id)
        .unwrap_err();
    assert!(
        matches!(err, ComposeError::TooLate | ComposeError::NoSuchOutboxItem),
        "{err:?}"
    );
    stays(400, || m.accepted() == 1);
    // The draft was not brought back by the refused Undo.
    wait_until("draft finished", || {
        m.core().list_drafts(m.account).unwrap().is_empty()
    });
}

#[test]
fn a_second_send_of_the_same_draft_makes_no_second_message() {
    let m = Mail::new();
    m.core().set_undo_window_ms(60_000);
    let id = m.draft("Once");
    m.core().send(m.account, id).unwrap();
    assert_eq!(
        m.core().send(m.account, id).unwrap_err(),
        ComposeError::InOutbox
    );
    assert_eq!(m.core().list_outbox(m.account).unwrap().len(), 1);
    // Two threads tapping Send together: one wins.
    let id2 = m.draft("Twice at once");
    let results: Vec<_> = (0..4)
        .map(|_| {
            let core = m.core().clone();
            let account = m.account;
            std::thread::spawn(move || core.send(account, id2))
        })
        .collect::<Vec<_>>()
        .into_iter()
        .map(|h| h.join().unwrap())
        .collect();
    assert_eq!(
        results.iter().filter(|r| r.is_ok()).count(),
        1,
        "{results:?}"
    );
    assert_eq!(m.core().list_outbox(m.account).unwrap().len(), 2);
}

#[test]
fn a_refused_message_fails_with_a_reason_and_can_be_retried_edited_or_discarded() {
    let m = Mail::new();
    m.smtp
        .set_reply(Stage::RcptTo, 550, "no such user bob@example.org");
    let id = m.draft("Refused");
    m.core().send(m.account, id).unwrap();
    wait_until("failed", || {
        m.core()
            .list_outbox(m.account)
            .unwrap()
            .first()
            .map(|i| i.state)
            == Some(OutboxItemState::Failed)
    });
    let item = first_outbox(&m);
    assert_eq!(item.reason, Some(OutboxReason::RecipientRefused));
    assert!(!item.may_have_been_sent);
    assert_eq!(m.accepted(), 0);
    // The reason carries no server text or address.
    assert_eq!(
        outbox_reason_key(OutboxReason::RecipientRefused),
        "outbox.reason.recipient_refused"
    );
    assert!(!outbox_reason_default_message(OutboxReason::RecipientRefused).contains('@'));
    // Retry once the server is fine: sent once.
    m.smtp.clear_reply(Stage::RcptTo);
    m.core()
        .retry_outbox_item(m.account, item.id, false)
        .unwrap();
    wait_until("sent after retry", || m.accepted() == 1);
    stays(400, || m.accepted() == 1);
    wait_until("outbox empty", || {
        m.core().list_outbox(m.account).unwrap().is_empty()
    });

    // Edit: back to the draft, nothing sent.
    m.smtp.set_reply(Stage::RcptTo, 550, "no");
    let id = m.draft("Edit me");
    m.core().send(m.account, id).unwrap();
    wait_until("failed again", || {
        m.core()
            .list_outbox(m.account)
            .unwrap()
            .first()
            .map(|i| i.state)
            == Some(OutboxItemState::Failed)
    });
    let item = first_outbox(&m);
    assert_eq!(m.core().edit_outbox_item(m.account, item.id).unwrap(), id);
    assert!(m.core().list_outbox(m.account).unwrap().is_empty());
    assert_eq!(m.core().load_draft(m.account, id).unwrap().outbox_id, None);
    assert_eq!(m.accepted(), 1);

    // Discard: message and draft gone.
    m.core().send(m.account, id).unwrap();
    wait_until("failed a third time", || {
        m.core()
            .list_outbox(m.account)
            .unwrap()
            .first()
            .map(|i| i.state)
            == Some(OutboxItemState::Failed)
    });
    let item = first_outbox(&m);
    m.core().discard_outbox_item(m.account, item.id).unwrap();
    assert!(m.core().list_outbox(m.account).unwrap().is_empty());
    assert_eq!(
        m.core().load_draft(m.account, id).unwrap_err(),
        ComposeError::NoSuchDraft
    );
    assert_eq!(m.accepted(), 1);
}

#[test]
fn a_message_the_server_may_have_taken_is_never_sent_again_by_itself() {
    let mut m = Mail::new();
    m.smtp.set_misbehavior(SmtpMisbehavior::AcceptThenDrop);
    let id = m.draft("Maybe");
    m.core().send(m.account, id).unwrap();
    wait_until("failed with the may-not-have-been-sent reason", || {
        m.core()
            .list_outbox(m.account)
            .unwrap()
            .first()
            .is_some_and(|i| i.may_have_been_sent)
    });
    assert_eq!(
        first_outbox(&m).reason,
        Some(OutboxReason::MayNotHaveBeenSent)
    );
    assert_eq!(m.accepted(), 1, "the server did take it");
    stays(700, || m.accepted() == 1);
    // The draft stays locked: Send again is refused, so the person cannot send it twice by mistake.
    assert_eq!(
        m.core().send(m.account, id).unwrap_err(),
        ComposeError::InOutbox
    );
    // A restart changes nothing: still one message at the server, still Failed.
    m.smtp.set_misbehavior(SmtpMisbehavior::None);
    m.restart();
    stays(900, || m.accepted() == 1);
    let item = first_outbox(&m);
    assert!(item.may_have_been_sent && item.state == OutboxItemState::Failed);
    // Retry is refused until the person has confirmed that the message may already have been sent.
    assert_eq!(
        m.core()
            .retry_outbox_item(m.account, item.id, false)
            .unwrap_err(),
        ComposeError::NeedsConfirmation
    );
    stays(300, || m.accepted() == 1);
    assert_eq!(first_outbox(&m).state, OutboxItemState::Failed);
    // Only the person's own Retry anyway sends it again, once.
    m.core()
        .retry_outbox_item(m.account, item.id, true)
        .unwrap();
    wait_until("the retry", || m.accepted() == 2);
    stays(500, || m.accepted() == 2);
}

#[test]
fn killing_the_app_while_a_message_is_going_out_never_sends_it_again() {
    let mut m = Mail::new();
    m.smtp.set_misbehavior(SmtpMisbehavior::HangAfterData);
    let id = m.draft("Killed");
    m.core().send(m.account, id).unwrap();
    wait_until("the server is holding the message", || {
        m.smtp.connection_count() == 1
    });
    std::thread::sleep(Duration::from_millis(400));
    m.kill();
    m.smtp.set_misbehavior(SmtpMisbehavior::None);
    m.reopen();
    stays(900, || m.smtp.connection_count() == 1 && m.accepted() == 0);
    let item = first_outbox(&m);
    assert_eq!(item.state, OutboxItemState::Failed);
    assert!(item.may_have_been_sent);
    // The draft is still locked: nothing offers a second send.
    assert_eq!(
        m.core().send(m.account, id).unwrap_err(),
        ComposeError::InOutbox
    );
    assert_eq!(
        m.core().load_draft(m.account, id).unwrap().outbox_id,
        Some(item.id)
    );
}

#[test]
fn a_queued_message_survives_a_restart_and_is_sent_once() {
    let mut m = Mail::new();
    m.core().set_undo_window_ms(1_500);
    let id = m.draft("Restart");
    m.core().send(m.account, id).unwrap();
    m.restart();
    assert_eq!(first_outbox(&m).state, OutboxItemState::Queued);
    wait_until("sent", || m.accepted() == 1);
    stays(600, || m.accepted() == 1);
    wait_until("draft finished", || {
        m.core().list_drafts(m.account).unwrap().is_empty()
    });
}

#[test]
fn a_crash_between_the_outbox_and_the_draft_leaves_the_draft_locked_not_sendable_twice() {
    let m = Mail::new();
    m.core().set_undo_window_ms(60_000);
    let id = m.draft("Crash window");
    // What `send` does, stopped after the Outbox write and before the draft is marked.
    let inner = &m.core().inner;
    let queued = inner
        .handle
        .send(
            croft_engine::NewMessage {
                account: m.account,
                identity: None,
                draft: Some(id),
                envelope_from: "alice@example.org".into(),
                recipients: vec!["bob@example.org".into()],
            },
            b"Message-ID: <x@example.org>\r\n\r\nbody".to_vec(),
        )
        .unwrap();
    assert_eq!(
        m.core()
            .save_draft(
                m.account,
                Some(id),
                content(&["bob@example.org"], "changed", "x")
            )
            .unwrap_err(),
        ComposeError::InOutbox
    );
    assert_eq!(
        m.core().send(m.account, id).unwrap_err(),
        ComposeError::InOutbox
    );
    assert_eq!(m.core().list_outbox(m.account).unwrap().len(), 1);
    // Undo finds the draft through the Outbox row.
    assert_eq!(m.core().undo_send(m.account, queued).unwrap(), id);
    assert!(m
        .core()
        .save_draft(
            m.account,
            Some(id),
            content(&["bob@example.org"], "changed", "x")
        )
        .is_ok());
}

#[test]
fn a_crash_in_the_middle_of_taking_a_message_back_never_reads_as_sent() {
    let mut m = Mail::new();
    m.core().set_undo_window_ms(60_000);
    let id = m.draft("Mid undo");
    let started = m.core().send(m.account, id).unwrap();
    // `take_back` cleared the draft's link and the app died before the Outbox was asked.
    {
        let d = Db::open(&m.db.path, None).unwrap();
        assert!(croft_storage::drafts::unlink_outbox(d.connection(), m.account, id).unwrap());
    }
    m.restart();
    // The message is still in the Outbox and the draft knows it again.
    wait_until("relinked", || {
        m.core().load_draft(m.account, id).map(|d| d.outbox_id) == Ok(Some(started.outbox_id))
    });
    assert_eq!(m.core().list_outbox(m.account).unwrap().len(), 1);
    // Or the Outbox acted and the app died before the draft was unlocked: it is a draft again,
    // never deleted as if sent.
    let mut m2 = Mail::new();
    m2.core().set_undo_window_ms(60_000);
    let id2 = m2.draft("Mid undo 2");
    let started = m2.core().send(m2.account, id2).unwrap();
    {
        // The steps of `take_back`, stopped before the draft is unlocked. Under the actions lock
        // as in the real call, so the check for sent messages cannot run in between.
        let _actions = m2.core().inner.compose.actions();
        let d = Db::open(&m2.db.path, None).unwrap();
        assert!(croft_storage::drafts::unlink_outbox(d.connection(), m2.account, id2).unwrap());
        m2.core()
            .inner
            .handle
            .undo_send(m2.account, started.outbox_id)
            .unwrap();
    }
    m2.restart();
    wait_until("editable again", || {
        m2.core()
            .load_draft(m2.account, id2)
            .is_ok_and(|d| d.outbox_id.is_none())
    });
    assert!(m2
        .core()
        .save_draft(
            m2.account,
            Some(id2),
            content(&["bob@example.org"], "still here", "x")
        )
        .is_ok());
    stays(400, || m2.accepted() == 0);
}

// ------------------------------------------------------------------------------------------
// The draft's copy on the server
// ------------------------------------------------------------------------------------------

#[test]
fn a_draft_is_appended_then_replaced_and_deleted_on_the_server() {
    let m = Mail::new();
    let id = m.draft("First version");
    // "After 5 s: server draft present."
    assert_eq!(
        block(m.core().sync_draft_to_server(m.account, id, false)).unwrap(),
        DraftSyncOutcome::Saved
    );
    assert_eq!(drafts_on_server(&m).len(), 1);
    assert_eq!(
        m.core().load_draft(m.account, id).unwrap().server_copy,
        ServerCopy::Current
    );
    assert_eq!(
        block(m.core().sync_draft_to_server(m.account, id, false)).unwrap(),
        DraftSyncOutcome::AlreadyCurrent
    );

    // Edited: behind, and paced.
    m.core()
        .save_draft(
            m.account,
            Some(id),
            content(&["bob@example.org"], "Second version", "New body"),
        )
        .unwrap();
    assert_eq!(
        m.core().load_draft(m.account, id).unwrap().server_copy,
        ServerCopy::Behind
    );
    assert_eq!(
        block(m.core().sync_draft_to_server(m.account, id, false)).unwrap(),
        DraftSyncOutcome::Deferred
    );
    assert_eq!(drafts_on_server(&m).len(), 1);
    assert!(drafts_on_server(&m)[0].raw.contains("First version"));
    // On pause the app forces it: replaced, never two.
    assert_eq!(
        block(m.core().sync_draft_to_server(m.account, id, true)).unwrap(),
        DraftSyncOutcome::Saved
    );
    let on_server = drafts_on_server(&m);
    assert_eq!(on_server.len(), 1);
    assert!(on_server[0].raw.contains("Second version") && on_server[0].raw.contains("New body"));
    let message_id = m.core().load_draft(m.account, id).unwrap().message_id;
    assert!(on_server[0]
        .raw
        .contains(&format!("Message-ID: {message_id}")));

    // Discard deletes the copy.
    m.core().discard_draft(m.account, id).unwrap();
    wait_until("the copy is deleted", || drafts_on_server(&m).is_empty());
}

#[test]
fn an_empty_draft_is_not_copied_and_a_draft_with_attachments_is() {
    let m = Mail::new();
    let id = m
        .core()
        .save_draft(m.account, None, content(&[], "", ""))
        .unwrap()
        .id;
    assert_eq!(
        block(m.core().sync_draft_to_server(m.account, id, true)).unwrap(),
        DraftSyncOutcome::AlreadyCurrent
    );
    assert!(drafts_on_server(&m).is_empty());
    block(m.core().add_draft_attachment_bytes(
        m.account,
        id,
        "n.txt".into(),
        "text/plain".into(),
        b"note".to_vec(),
    ))
    .unwrap();
    assert_eq!(
        block(m.core().sync_draft_to_server(m.account, id, true)).unwrap(),
        DraftSyncOutcome::Saved
    );
    let raw = &drafts_on_server(&m)[0].raw;
    assert!(raw.contains("multipart/mixed") && raw.contains("filename=\"n.txt\""));
}

#[test]
fn a_draft_with_an_address_still_being_typed_is_copied_without_it() {
    let m = Mail::new();
    let id = m
        .core()
        .save_draft(
            m.account,
            None,
            content(&["bob@example.org", "half@"], "s", "b"),
        )
        .unwrap()
        .id;
    assert_eq!(
        block(m.core().sync_draft_to_server(m.account, id, true)).unwrap(),
        DraftSyncOutcome::Saved
    );
    let raw = &drafts_on_server(&m)[0].raw;
    assert!(raw.contains("bob@example.org") && !raw.contains("half@"));
}

#[test]
fn server_trouble_leaves_the_draft_on_the_device_and_is_not_an_error() {
    let mut m = Mail::new();
    let id = m.draft("Safe");
    // The connection the sync opened is closed, and the server refuses new ones.
    m.imap
        .set_misbehavior(croft_test_support::Misbehavior::ByeOnLogin);
    m.imap.drop_connections();
    let outcome = block(m.core().sync_draft_to_server(m.account, id, true)).unwrap();
    assert!(
        matches!(
            outcome,
            DraftSyncOutcome::TryLater | DraftSyncOutcome::NeedsSignIn
        ),
        "{outcome:?}"
    );
    assert!(drafts_on_server(&m).is_empty());
    assert_eq!(
        m.core().load_draft(m.account, id).unwrap().server_copy,
        ServerCopy::NotCopied
    );
    // After a restart it is still there and gets copied when the server is fine.
    m.imap
        .set_misbehavior(croft_test_support::Misbehavior::None);
    m.restart();
    wait_until("copied at the next start", || {
        drafts_on_server(&m).len() == 1
    });
}

#[test]
fn the_sample_mailbox_keeps_drafts_on_the_device_only() {
    let (_db, core, a) = sample_core();
    let id = core
        .save_draft(a, None, content(&["b@example.org"], "s", "b"))
        .unwrap()
        .id;
    assert_eq!(
        block(core.sync_draft_to_server(a, id, true)).unwrap(),
        DraftSyncOutcome::NotApplicable
    );
    // And it can send: the sample mailbox files the copy in its own Sent folder.
    core.send(a, id).unwrap();
    wait_until("sent from the sample mailbox", || {
        core.list_outbox(a).unwrap().is_empty()
    });
    wait_until("draft finished", || core.list_drafts(a).unwrap().is_empty());
}

// ------------------------------------------------------------------------------------------
// Credentials and certificates
// ------------------------------------------------------------------------------------------

#[test]
fn the_password_is_asked_for_at_each_send_and_never_stored() {
    let m = Mail::new();
    let id = m.draft("Secret handling");
    let before = m.provider.asked().len();
    m.core().send(m.account, id).unwrap();
    wait_until("sent", || m.accepted() == 1);
    assert!(
        m.provider.asked().len() > before,
        "the provider is asked when a message goes out"
    );
    wait_until("draft finished", || {
        m.core().list_drafts(m.account).unwrap().is_empty()
    });
    // The password is in no file of the database (main, WAL).
    let dir = m.db.path.parent().unwrap();
    for entry in std::fs::read_dir(dir).unwrap() {
        let bytes = std::fs::read(entry.unwrap().path()).unwrap();
        assert!(
            !bytes
                .windows(PASSWORD.len())
                .any(|w| w == PASSWORD.as_bytes()),
            "the password is in the database"
        );
    }
}

#[test]
fn a_wrong_password_pauses_sending_and_the_message_waits_for_the_sign_in() {
    let m = Mail::new();
    m.provider
        .store(m.account, password_cred("not the password"));
    let id = m.draft("Needs sign-in");
    m.core().send(m.account, id).unwrap();
    wait_until("sign-in needed", || {
        m.provider.sign_ins().contains(&m.account)
    });
    assert_eq!(m.accepted(), 0);
    let item = first_outbox(&m);
    assert_eq!(
        item.state,
        OutboxItemState::Queued,
        "the message is kept, not failed"
    );
    // The person signs in again: it goes out once.
    m.provider.store(m.account, password_cred(PASSWORD));
    block(m.core().signed_in(m.account)).unwrap();
    wait_until("sent after sign-in", || m.accepted() == 1);
    stays(500, || m.accepted() == 1);
}

#[test]
fn the_accounts_pin_is_used_for_smtp_on_the_same_host_and_not_on_another_host() {
    // Same host as IMAP (127.0.0.1) and not a provider's: the pin applies, a self-signed
    // certificate with that fingerprint is accepted, and the message is sent over TLS.
    let m = Mail::with_smtp(TlsMode::Implicit, |s, smtp| {
        let pin = croft_protocols::CertFingerprint::of_der(&smtp.certificate_der().unwrap());
        s.pinned_fingerprint = Some(pin.to_string());
        s.smtp.as_mut().unwrap().security = StoredSecurity::Tls;
    });
    let id = m.draft("Pinned");
    m.core().send(m.account, id).unwrap();
    wait_until("sent over pinned TLS", || m.accepted() == 1);
    assert!(m.smtp.accepted()[0].over_tls);

    // The same server under another host name ("localhost"): the pin does not apply, the
    // self-signed certificate is not trusted, and nothing is sent.
    let m = Mail::with_smtp(TlsMode::Implicit, |s, smtp| {
        let pin = croft_protocols::CertFingerprint::of_der(&smtp.certificate_der().unwrap());
        s.pinned_fingerprint = Some(pin.to_string());
        let smtp = s.smtp.as_mut().unwrap();
        smtp.security = StoredSecurity::Tls;
        smtp.host = "localhost".into();
    });
    let id = m.draft("Not pinned");
    m.core().send(m.account, id).unwrap();
    wait_until("the server was contacted", || {
        m.smtp.connection_count() >= 1
    });
    stays(700, || m.accepted() == 0);
    assert_eq!(first_outbox(&m).state, OutboxItemState::Queued);
}

#[test]
fn an_account_without_an_smtp_server_cannot_send() {
    let m = Mail::with(|s| s.smtp = None);
    let id = m.draft("No server");
    assert_eq!(
        m.core().send(m.account, id).unwrap_err(),
        ComposeError::CannotSend
    );
    assert!(m.core().list_outbox(m.account).unwrap().is_empty());
    // The draft is still editable.
    assert!(m
        .core()
        .save_draft(m.account, Some(id), content(&["bob@example.org"], "x", "y"))
        .is_ok());
}

#[test]
fn the_pacing_of_copies_to_the_server_can_be_shortened_for_tests_and_forced_by_the_app() {
    let m = Mail::new();
    m.core().inner.compose.set_interval_ms(0);
    let id = m.draft("Quick");
    for round in 0..3 {
        m.core()
            .save_draft(
                m.account,
                Some(id),
                content(&["bob@example.org"], "Quick", &format!("edit {round}")),
            )
            .unwrap();
        assert_eq!(
            block(m.core().sync_draft_to_server(m.account, id, false)).unwrap(),
            DraftSyncOutcome::Saved
        );
        assert_eq!(drafts_on_server(&m).len(), 1);
    }
    assert!(drafts_on_server(&m)[0].raw.contains("edit 2"));
}

fn message_id_of(data: &[u8]) -> String {
    String::from_utf8_lossy(data)
        .lines()
        .find_map(|l| l.strip_prefix("Message-ID: "))
        .unwrap()
        .trim()
        .to_owned()
}

/// A message the server may have taken is edited and sent again: the corrected one must not look like
/// the old one (the copy in Sent is found by Message-ID and would not be filed; a recipient's
/// server may drop a repeat), and the old draft copy on the server must go.
#[test]
fn editing_a_message_that_may_have_been_sent_gives_the_draft_a_new_message_id() {
    let m = Mail::new();
    let id = m.draft("Maybe sent");
    let old = m.core().load_draft(m.account, id).unwrap().message_id;
    assert_eq!(
        block(m.core().sync_draft_to_server(m.account, id, true)).unwrap(),
        DraftSyncOutcome::Saved
    );
    assert!(drafts_on_server(&m)[0].raw.contains(&old));

    m.smtp.set_misbehavior(SmtpMisbehavior::AcceptThenDrop);
    m.core().send(m.account, id).unwrap();
    wait_until("may have been sent", || {
        m.core()
            .list_outbox(m.account)
            .unwrap()
            .first()
            .is_some_and(|i| i.may_have_been_sent)
    });
    m.smtp.set_misbehavior(SmtpMisbehavior::None);
    let first = message_id_of(&m.smtp.accepted()[0].data);
    assert_eq!(
        first, old,
        "the message that went out carries the draft's id"
    );

    // Edit: back to the draft, with a new Message-ID.
    let item = first_outbox(&m);
    assert_eq!(m.core().edit_outbox_item(m.account, item.id).unwrap(), id);
    let edited = m.core().load_draft(m.account, id).unwrap();
    assert_ne!(edited.message_id, old);
    assert!(edited.message_id.starts_with('<') && edited.message_id.ends_with("@example.org>"));
    m.core()
        .save_draft(
            m.account,
            Some(id),
            content(&["bob@example.org"], "Maybe sent", "Corrected text"),
        )
        .unwrap();
    // The copy on the server is replaced under the new id, and the old one is gone.
    assert_eq!(
        block(m.core().sync_draft_to_server(m.account, id, true)).unwrap(),
        DraftSyncOutcome::Saved
    );
    let on_server = drafts_on_server(&m);
    assert_eq!(on_server.len(), 1, "{}", on_server.len());
    assert!(on_server[0].raw.contains(&edited.message_id) && !on_server[0].raw.contains(&old));
    assert!(on_server[0].raw.contains("Corrected text"));

    // Sent again: a message with the new id, and its copy is filed in Sent (the first was never
    // filed: it was not known to be sent).
    m.core().send(m.account, id).unwrap();
    wait_until("the corrected message is sent", || m.accepted() == 2);
    let second = message_id_of(&m.smtp.accepted()[1].data);
    assert_eq!(second, edited.message_id);
    assert_ne!(second, first);
    wait_until("filed in Sent under the new id", || {
        let sent = m.imap.messages("Sent");
        sent.len() == 1 && sent[0].raw.contains(&second)
    });
    wait_until("draft and its copy gone", || {
        m.core().list_drafts(m.account).unwrap().is_empty() && drafts_on_server(&m).is_empty()
    });
}

/// A message queued together with its draft, and the app dying before the draft was marked: when
/// the account runs again and sends it, the draft must not be left behind as an editable draft of a
/// message that went out. The check runs before any account starts.
#[test]
fn a_crash_between_the_outbox_write_and_the_draft_mark_never_leaves_an_editable_draft_of_a_sent_message(
) {
    let mut m = Mail::new();
    m.core().set_undo_window_ms(60_000);
    let id = m.draft("Sent after a crash");
    let message_id = m.core().load_draft(m.account, id).unwrap().message_id;
    let mime = format!(
        "From: alice@example.org\r\nTo: bob@example.org\r\nSubject: Sent after a crash\r\nMessage-ID: {message_id}\r\n\r\nbody\r\n"
    );
    // What `send` does, stopped after the Outbox write and before the draft is marked.
    m.core()
        .inner
        .handle
        .send(
            croft_engine::NewMessage {
                account: m.account,
                identity: None,
                draft: Some(id),
                envelope_from: "alice@example.org".into(),
                recipients: vec!["bob@example.org".into()],
            },
            mime.into_bytes(),
        )
        .unwrap();
    m.kill();
    // The Undo window is over by the time the app is back: the message is due at once.
    {
        let c = rusqlite::Connection::open(&m.db.path).unwrap();
        c.execute("UPDATE outbox SET send_at = 0", []).unwrap();
        // The state the crash left: the Outbox row is there, the draft was not marked. (The
        // check that follows a send may have marked it before the kill; put it back.)
        c.execute(
            "UPDATE draft SET state = 'draft', outbox_id = NULL WHERE id = ?1",
            [id],
        )
        .unwrap();
    }
    m.reopen();
    // The draft is locked from the first moment (before the account could send anything), and it
    // is finished, never left as an editable draft, once the message is out.
    let state = |m: &Mail| -> Option<String> {
        let c = rusqlite::Connection::open(&m.db.path).unwrap();
        c.query_row("SELECT state FROM draft WHERE id = ?1", [id], |r| r.get(0))
            .ok()
    };
    assert_ne!(state(&m).as_deref(), Some("draft"));
    wait_until("sent", || m.accepted() == 1);
    wait_until("filed in Sent", || m.imap.messages("Sent").len() == 1);
    let deadline = Instant::now() + Duration::from_secs(20);
    while state(&m).is_some_and(|s| s != "done") {
        assert_ne!(
            state(&m).as_deref(),
            Some("draft"),
            "an editable draft of a sent message"
        );
        assert!(Instant::now() < deadline, "the draft was never finished");
        std::thread::sleep(Duration::from_millis(20));
    }
    wait_until("draft gone", || {
        m.core().list_drafts(m.account).unwrap().is_empty()
    });
    assert_eq!(m.accepted(), 1);
}

/// The copy of a draft was made on the server but never recorded (a failure, a crash): the draft
/// had noted the folder before the copy, so discarding it still deletes the copy.
#[test]
fn a_server_copy_that_was_never_recorded_is_deleted_when_the_draft_is_discarded() {
    let m = Mail::new();
    let id = m.draft("Unrecorded copy");
    let message_id = m.core().load_draft(m.account, id).unwrap().message_id;
    m.imap.add_message_with_flags(
        "Drafts",
        &format!(
            "From: alice@example.org\r\nSubject: Unrecorded copy\r\nMessage-ID: {message_id}\r\n\r\nbody\r\n"
        ),
        &["\\Draft"],
    );
    {
        // What the core writes before it appends: the folder, and nothing else.
        let d = Db::open(&m.db.path, None).unwrap();
        assert!(
            croft_storage::drafts::plan_server_copy(d.connection(), m.account, id, "Drafts")
                .unwrap()
        );
    }
    assert_eq!(drafts_on_server(&m).len(), 1);
    m.core().discard_draft(m.account, id).unwrap();
    wait_until("the unrecorded copy is deleted", || {
        drafts_on_server(&m).is_empty()
    });
}

#[test]
fn a_scheduled_message_waits_for_its_time_and_can_be_taken_back_until_then() {
    let m = Mail::new();
    m.core().set_undo_window_ms(0);
    let id = m.draft("Later");
    let at = now_ms() + 60_000;
    let started = m.core().schedule_send(m.account, id, at).unwrap();
    assert_eq!(started.send_at_ms, at);
    let item = first_outbox(&m);
    assert_eq!(item.state, OutboxItemState::Queued);
    assert_eq!(item.send_at_ms, at);
    // Locked like any queued message.
    assert_eq!(
        m.core().send(m.account, id).unwrap_err(),
        ComposeError::InOutbox
    );
    stays(600, || m.accepted() == 0);
    // Cancelled long after any Undo window: nothing goes, the draft is back.
    assert_eq!(
        m.core().undo_send(m.account, started.outbox_id).unwrap(),
        id
    );
    assert!(m.core().list_outbox(m.account).unwrap().is_empty());
    assert_eq!(m.core().load_draft(m.account, id).unwrap().outbox_id, None);
    stays(400, || m.accepted() == 0);
}

#[test]
fn a_scheduled_message_goes_out_once_at_its_time_even_after_a_restart() {
    let mut m = Mail::new();
    m.core().set_undo_window_ms(0);
    let id = m.draft("Timed");
    let at = now_ms() + 1_500;
    m.core().schedule_send(m.account, id, at).unwrap();
    m.restart();
    assert_eq!(first_outbox(&m).send_at_ms, at);
    stays(800, || m.accepted() == 0);
    wait_until("sent", || m.accepted() == 1);
    assert!(now_ms() >= at);
    stays(500, || m.accepted() == 1);
    wait_until("draft finished", || {
        m.core().list_drafts(m.account).unwrap().is_empty()
    });
}

#[test]
fn a_time_too_far_ahead_is_refused_and_one_in_the_past_is_an_ordinary_send() {
    let m = Mail::new();
    m.core().set_undo_window_ms(60_000);
    let id = m.draft("Far");
    let too_far = now_ms() + MAX_SCHEDULE_AHEAD_MS + 60_000;
    assert_eq!(
        m.core().schedule_send(m.account, id, too_far).unwrap_err(),
        ComposeError::BadSendTime
    );
    // Nothing was queued and the draft is still editable.
    assert!(m.core().list_outbox(m.account).unwrap().is_empty());
    assert_eq!(m.core().load_draft(m.account, id).unwrap().outbox_id, None);
    // A time already past waits only the Undo window, as Send does.
    let before = now_ms();
    let started = m
        .core()
        .schedule_send(m.account, id, before - 5_000)
        .unwrap();
    assert!(
        started.send_at_ms >= before + 60_000,
        "{}",
        started.send_at_ms - before
    );
    assert_eq!(
        m.core().undo_send(m.account, started.outbox_id).unwrap(),
        id
    );
}
