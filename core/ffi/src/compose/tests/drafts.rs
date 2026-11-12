//! Drafts on the device: storing, restoring, limits, the Send button's facts, attachments,
//! identities, and that no call touches another account's data.

use super::*;

#[test]
fn a_draft_is_stored_loaded_listed_and_keeps_its_message_id() {
    let (_db, core, a) = sample_core();
    let first = core
        .save_draft(a, None, content(&["bob@example.org"], "Plans", "Hi"))
        .unwrap();
    assert!(first.message_id.starts_with('<') && first.message_id.ends_with('>'));
    assert!(
        first.message_id.contains("@example.org>"),
        "{}",
        first.message_id
    );
    let second = core
        .save_draft(
            a,
            Some(first.id),
            content(
                &["bob@example.org", "cy@example.org"],
                "Plans 2",
                "Hi again",
            ),
        )
        .unwrap();
    assert_eq!(
        (second.id, &second.message_id),
        (first.id, &first.message_id)
    );
    let d = core.load_draft(a, first.id).unwrap();
    assert_eq!(d.content.subject, "Plans 2");
    assert_eq!(d.content.to.len(), 2);
    assert_eq!((d.server_copy, d.outbox_id), (ServerCopy::NotCopied, None));
    assert_eq!(d.message_id, first.message_id);
    let list = core.list_drafts(a).unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].subject, "Plans 2");
    assert_eq!(list[0].to_preview, ["bob@example.org", "cy@example.org"]);
    assert_eq!(list[0].snippet, "Hi again");
    // Another draft has its own Message-ID.
    let other = core
        .save_draft(a, None, content(&["x@example.org"], "", ""))
        .unwrap();
    assert_ne!(other.message_id, first.message_id);
}

/// "Kill after 1 s: local draft restored": the app stores, the process dies, the next start finds it.
#[test]
fn a_saved_draft_is_there_after_a_restart() {
    let db = TempDb::new();
    let provider = MemProvider::new();
    let (a, id) = {
        let core = db.open(&provider);
        let a = block(core.add_demo_account()).unwrap().id;
        let mut c = content(
            &["bob@example.org"],
            "Unsent thoughts",
            "Dear Bob,\nline two",
        );
        c.bcc = vec![address(Some("Hidden"), "hidden@example.org")];
        let id = core.save_draft(a, None, c).unwrap().id;
        (a, id)
    };
    let core = db.open(&provider);
    let list = core.list_drafts(a).unwrap();
    assert_eq!(list.len(), 1);
    let d = core.load_draft(a, list[0].id).unwrap();
    assert_eq!(d.id, id);
    assert_eq!(d.content.body, "Dear Bob,\nline two");
    assert_eq!(d.content.bcc[0].address, "hidden@example.org");
}

#[test]
fn a_discarded_draft_is_gone_from_the_lists_and_cannot_be_loaded() {
    let (_db, core, a) = sample_core();
    let id = core
        .save_draft(a, None, content(&["b@example.org"], "x", "y"))
        .unwrap()
        .id;
    core.discard_draft(a, id).unwrap();
    assert!(core.list_drafts(a).unwrap().is_empty());
    assert_eq!(
        core.load_draft(a, id).unwrap_err(),
        ComposeError::NoSuchDraft
    );
    assert_eq!(
        core.save_draft(a, Some(id), content(&[], "", ""))
            .unwrap_err(),
        ComposeError::NoSuchDraft
    );
    assert_eq!(
        core.discard_draft(a, id).unwrap_err(),
        ComposeError::NoSuchDraft
    );
}

#[test]
fn every_call_refuses_another_accounts_ids() {
    let db = TempDb::new();
    let provider = MemProvider::new();
    // Two accounts: the sample mailbox and a stored one with no server.
    let id2 = {
        let d = Db::open(&db.path, None).unwrap();
        crate::tests::insert_account(&d, 2)
    };
    let core = db.open(&provider);
    let a = block(core.add_demo_account()).unwrap().id;
    let b = id2;
    core.set_undo_window_ms(60_000);
    let draft = core
        .save_draft(
            a,
            None,
            content(&["bob@example.org"], "Mine", "secret body"),
        )
        .unwrap()
        .id;
    let att = block(core.add_draft_attachment_bytes(
        a,
        draft,
        "a.txt".into(),
        "text/plain".into(),
        b"x".to_vec(),
    ))
    .unwrap();
    let sent = core.send(a, draft).unwrap();

    let no_draft = ComposeError::NoSuchDraft;
    assert_eq!(core.load_draft(b, draft).unwrap_err(), no_draft);
    assert_eq!(
        core.save_draft(b, Some(draft), content(&["x@example.org"], "", ""))
            .unwrap_err(),
        no_draft
    );
    assert_eq!(core.discard_draft(b, draft).unwrap_err(), no_draft);
    assert_eq!(core.check_draft(b, draft).unwrap_err(), no_draft);
    assert_eq!(core.send(b, draft).unwrap_err(), no_draft);
    assert_eq!(
        block(core.sync_draft_to_server(b, draft, true)).unwrap_err(),
        no_draft
    );
    assert_eq!(
        block(core.add_draft_attachment_bytes(b, draft, "x".into(), "a/b".into(), vec![1]))
            .unwrap_err(),
        no_draft
    );
    assert_eq!(
        core.remove_draft_attachment(b, draft, att.id).unwrap_err(),
        no_draft
    );
    assert!(core.list_drafts(b).unwrap().is_empty());
    assert!(core.list_outbox(b).unwrap().is_empty());
    let gone = ComposeError::NoSuchOutboxItem;
    assert_eq!(core.undo_send(b, sent.outbox_id).unwrap_err(), gone);
    assert_eq!(
        core.retry_outbox_item(b, sent.outbox_id, true).unwrap_err(),
        gone
    );
    assert_eq!(core.edit_outbox_item(b, sent.outbox_id).unwrap_err(), gone);
    assert_eq!(
        core.discard_outbox_item(b, sent.outbox_id).unwrap_err(),
        gone
    );
    assert!(core
        .suggest_recipients(b, "bo".into(), 5)
        .unwrap()
        .is_empty());
    assert_eq!(
        core.reply_draft(b, 1).unwrap_err(),
        ComposeError::NoSuchMessage
    );
    // An identity of one account cannot be used in another.
    let ident = core.identities(a).unwrap()[0].id;
    let mut c = content(&["x@example.org"], "", "");
    c.identity = Some(ident);
    assert_eq!(
        core.save_draft(b, None, c).unwrap_err(),
        ComposeError::NoSuchIdentity
    );
    // And nothing of account a changed: the message is still queued, the draft locked.
    assert_eq!(core.list_outbox(a).unwrap().len(), 1);
    assert_eq!(
        core.load_draft(a, draft).unwrap().outbox_id,
        Some(sent.outbox_id)
    );
    // Unknown accounts.
    assert_eq!(
        core.list_drafts(9999).unwrap_err(),
        ComposeError::NoSuchAccount
    );
    assert_eq!(
        core.identities(9999).unwrap_err(),
        ComposeError::NoSuchAccount
    );
}

#[test]
fn text_over_the_limits_is_refused_and_nothing_is_stored() {
    let (_db, core, a) = sample_core();
    let mut c = content(&["x@example.org"], "s", "b");
    c.subject = "s".repeat(MAX_SUBJECT_CHARS as usize + 1);
    assert_eq!(
        core.save_draft(a, None, c).unwrap_err(),
        ComposeError::DraftTooLarge
    );
    let mut c = content(&[], "s", "b");
    c.to = (0..=MAX_RECIPIENTS)
        .map(|i| address(None, &format!("p{i}@example.org")))
        .collect();
    assert_eq!(
        core.save_draft(a, None, c).unwrap_err(),
        ComposeError::DraftTooLarge
    );
    let mut c = content(&["x@example.org"], "s", "b");
    c.body = "x".repeat(MAX_BODY_BYTES as usize + 1);
    assert_eq!(
        core.save_draft(a, None, c).unwrap_err(),
        ComposeError::DraftTooLarge
    );
    let mut c = content(&["x@example.org"], "s", "b");
    c.to[0].address = "a".repeat(400);
    assert_eq!(
        core.save_draft(a, None, c).unwrap_err(),
        ComposeError::DraftTooLarge
    );
    assert!(core.list_drafts(a).unwrap().is_empty());
}

/// The rows of the Send button table in `docs/ux/controls.md`, as the core can answer them.
#[test]
fn the_send_button_facts_follow_the_condition_table() {
    let (_db, core, a) = sample_core();
    let check = |c: DraftContent| {
        let id = core.save_draft(a, None, c).unwrap().id;
        let r = core.check_draft(a, id).unwrap();
        (id, r)
    };
    // No recipient.
    let (_, r) = check(content(&[], "Hi", "Text"));
    assert_eq!(r.problems, [DraftProblem::NoRecipient]);
    // An invalid chip.
    let (_, r) = check(content(
        &["bob@example.org", "not an address", "x@"],
        "Hi",
        "Text",
    ));
    assert_eq!(r.problems, [DraftProblem::InvalidRecipient]);
    assert_eq!(r.invalid_recipients, ["not an address", "x@"]);
    // Recipient but empty subject and body: enabled, and the app asks.
    let (_, r) = check(content(&["bob@example.org"], "  ", " \n"));
    assert!(r.problems.is_empty());
    assert!(r.subject_empty && r.body_empty);
    // Everything present.
    let (id, r) = check(content(&["bob@example.org"], "Hi", "Text"));
    assert!(r.problems.is_empty() && !r.subject_empty && !r.body_empty);
    assert_eq!(r.size_limit_bytes, ATTACHMENT_WARN_BYTES);
    // An attachment makes the body count as present, and a big one disables Send.
    let (id2, _) = check(content(&["bob@example.org"], "", ""));
    block(core.add_draft_attachment_bytes(
        a,
        id2,
        "a.bin".into(),
        "application/octet-stream".into(),
        vec![0; 10],
    ))
    .unwrap();
    assert!(!core.check_draft(a, id2).unwrap().body_empty);
    block(core.add_draft_attachment_bytes(
        a,
        id,
        "big.bin".into(),
        "application/octet-stream".into(),
        vec![0; ATTACHMENT_WARN_BYTES as usize + 1],
    ))
    .unwrap();
    let r = core.check_draft(a, id).unwrap();
    assert_eq!(r.problems, [DraftProblem::AttachmentsTooLarge]);
    assert_eq!(r.attachments_bytes, ATTACHMENT_WARN_BYTES + 1);
    // Offline or signed out never disables it: nothing here depends on the network.
    core.set_network_available(false).unwrap();
    assert!(core.check_draft(a, id2).unwrap().problems.is_empty());
    // Send itself refuses what the table disables.
    let (none, _) = check(content(&[], "Hi", "Text"));
    assert_eq!(core.send(a, none).unwrap_err(), ComposeError::NoRecipient);
    let (bad, _) = check(content(&["nope"], "Hi", "Text"));
    assert_eq!(core.send(a, bad).unwrap_err(), ComposeError::InvalidAddress);
    assert!(core.list_outbox(a).unwrap().is_empty());
}

#[test]
fn recipients_are_read_from_typed_text() {
    let one = parse_recipient("Ann Example <ann@example.org>".into()).unwrap();
    assert_eq!(one, address(Some("Ann Example"), "ann@example.org"));
    assert!(parse_recipient("  ".into()).is_none());
    assert_eq!(
        parse_recipients("a@x.org, B <b@x.org>; c@x.org".into()).len(),
        3
    );
    assert!(is_valid_address("ann@example.org".into()));
    assert!(!is_valid_address("ann@".into()));
    let limits = compose_limits();
    assert_eq!(limits.attachment_warn_bytes, 25 * 1024 * 1024);
}

#[test]
fn identities_always_have_the_accounts_own_address() {
    let (_db, core, a) = sample_core();
    let ids = core.identities(a).unwrap();
    assert_eq!(ids.len(), 1);
    assert!(ids[0].is_default);
    assert_eq!(ids[0].address, croft_protocols::demo::DEMO_ADDRESS);
    // Asking twice gives the same row.
    assert_eq!(core.identities(a).unwrap(), ids);
    // The default is used when none is chosen, and a chosen one is kept.
    let id = core
        .save_draft(a, None, content(&["x@example.org"], "s", "b"))
        .unwrap()
        .id;
    assert_eq!(core.load_draft(a, id).unwrap().content.identity, None);
    let mut c = content(&["x@example.org"], "s", "b");
    c.identity = Some(ids[0].id);
    core.save_draft(a, Some(id), c).unwrap();
    assert_eq!(
        core.load_draft(a, id).unwrap().content.identity,
        Some(ids[0].id)
    );
    let mut c = content(&["x@example.org"], "s", "b");
    c.identity = Some(ids[0].id + 1000);
    assert_eq!(
        core.save_draft(a, Some(id), c).unwrap_err(),
        ComposeError::NoSuchIdentity
    );
}

// ------------------------------------------------------------------------------------------
// Attachments
// ------------------------------------------------------------------------------------------

#[test]
fn attachments_from_bytes_are_stored_listed_and_removed() {
    let (_db, core, a) = sample_core();
    let id = core
        .save_draft(a, None, content(&["b@example.org"], "s", "b"))
        .unwrap()
        .id;
    let att = block(core.add_draft_attachment_bytes(
        a,
        id,
        "../../etc/passwd\r\nBcc: x@y.z".into(),
        "".into(),
        b"hello".to_vec(),
    ))
    .unwrap();
    assert_eq!(att.size_bytes, 5);
    assert!(
        !att.name.contains('/') && !att.name.contains('\r') && !att.name.contains('\n'),
        "{att:?}"
    );
    assert_eq!(att.mime_type, "application/octet-stream");
    let d = core.load_draft(a, id).unwrap();
    assert_eq!(d.attachments, vec![att.clone()]);
    assert!(core.list_drafts(a).unwrap()[0].has_attachments);
    core.remove_draft_attachment(a, id, att.id).unwrap();
    assert_eq!(
        core.remove_draft_attachment(a, id, att.id).unwrap_err(),
        ComposeError::NoSuchAttachment
    );
    assert!(core.load_draft(a, id).unwrap().attachments.is_empty());
}

#[test]
fn attachment_sizes_and_counts_are_capped() {
    let (_db, core, a) = sample_core();
    let id = core
        .save_draft(a, None, content(&["b@example.org"], "s", "b"))
        .unwrap()
        .id;
    let add = |bytes: Vec<u8>| {
        block(core.add_draft_attachment_bytes(a, id, "f".into(), "a/b".into(), bytes))
    };
    assert_eq!(
        add(vec![0; MAX_ATTACHMENT_BYTES as usize + 1]).unwrap_err(),
        ComposeError::AttachmentTooLarge
    );
    add(vec![0; MAX_DRAFT_ATTACHMENT_BYTES as usize - 10]).unwrap();
    assert_eq!(
        add(vec![0; 11]).unwrap_err(),
        ComposeError::AttachmentTooLarge
    );
    add(vec![0; 10]).unwrap();
    let id2 = core
        .save_draft(a, None, content(&["b@example.org"], "s", "b"))
        .unwrap()
        .id;
    for _ in 0..MAX_ATTACHMENTS {
        block(core.add_draft_attachment_bytes(a, id2, "f".into(), "a/b".into(), vec![1])).unwrap();
    }
    assert_eq!(
        block(core.add_draft_attachment_bytes(a, id2, "f".into(), "a/b".into(), vec![1]))
            .unwrap_err(),
        ComposeError::TooManyAttachments
    );
}

#[test]
fn an_attachment_from_a_path_reads_only_that_plain_file() {
    let (_db, core, a) = sample_core();
    let id = core
        .save_draft(a, None, content(&["b@example.org"], "s", "b"))
        .unwrap()
        .id;
    let dir = tempfile::tempdir().unwrap();
    core.set_attachment_directory(dir.path().display().to_string())
        .unwrap();
    let file = dir.path().join("picked.bin");
    std::fs::write(&file, b"file bytes").unwrap();
    let add = |p: &Path| {
        block(core.add_draft_attachment_file(
            a,
            id,
            p.display().to_string(),
            "My file.bin".into(),
            "application/pdf".into(),
        ))
    };
    let att = add(&file).unwrap();
    assert_eq!((att.name.as_str(), att.size_bytes), ("My file.bin", 10));
    // The name shown is the one the app gave, never the path's.
    assert!(!att.name.contains("picked"));

    // Not absolute, with `..`, a folder, missing.
    assert_eq!(
        add(Path::new("picked.bin")).unwrap_err(),
        ComposeError::AttachmentUnreadable
    );
    let tricky = dir.path().join("sub").join("..").join("picked.bin");
    assert_eq!(
        add(&tricky).unwrap_err(),
        ComposeError::AttachmentUnreadable
    );
    assert_eq!(
        add(dir.path()).unwrap_err(),
        ComposeError::AttachmentUnreadable
    );
    assert_eq!(
        add(&dir.path().join("missing")).unwrap_err(),
        ComposeError::AttachmentUnreadable
    );
    // A link to another file is not followed.
    #[cfg(unix)]
    {
        let secret = dir.path().join("secret.txt");
        std::fs::write(&secret, b"do not read me").unwrap();
        let link = dir.path().join("link");
        std::os::unix::fs::symlink(&secret, &link).unwrap();
        assert_eq!(add(&link).unwrap_err(), ComposeError::AttachmentUnreadable);
        // A device is not a plain file either (and reading it would never end).
        assert_eq!(
            add(Path::new("/dev/zero")).unwrap_err(),
            ComposeError::AttachmentUnreadable
        );
    }
    // Too large: refused from its size, before it is read.
    let big = dir.path().join("big.bin");
    let f = std::fs::File::create(&big).unwrap();
    f.set_len(MAX_ATTACHMENT_BYTES + 1).unwrap();
    assert_eq!(add(&big).unwrap_err(), ComposeError::AttachmentTooLarge);
    // A draft that is not this account's reads nothing: the path is not even looked at.
    assert_eq!(
        block(core.add_draft_attachment_file(
            a,
            id + 100,
            file.display().to_string(),
            "x".into(),
            "a/b".into()
        ))
        .unwrap_err(),
        ComposeError::NoSuchDraft
    );
    assert_eq!(core.load_draft(a, id).unwrap().attachments.len(), 1);
}

#[test]
fn a_locked_draft_takes_no_attachments_and_no_edits() {
    let (_db, core, a) = sample_core();
    core.set_undo_window_ms(60_000);
    let id = core
        .save_draft(a, None, content(&["b@example.org"], "s", "b"))
        .unwrap()
        .id;
    let att =
        block(core.add_draft_attachment_bytes(a, id, "f".into(), "a/b".into(), vec![1])).unwrap();
    core.send(a, id).unwrap();
    assert_eq!(
        core.save_draft(a, Some(id), content(&["b@example.org"], "changed", "b"))
            .unwrap_err(),
        ComposeError::InOutbox
    );
    assert_eq!(
        block(core.add_draft_attachment_bytes(a, id, "g".into(), "a/b".into(), vec![1]))
            .unwrap_err(),
        ComposeError::InOutbox
    );
    assert_eq!(
        core.remove_draft_attachment(a, id, att.id).unwrap_err(),
        ComposeError::InOutbox
    );
    assert_eq!(
        core.discard_draft(a, id).unwrap_err(),
        ComposeError::InOutbox
    );
    assert_eq!(core.send(a, id).unwrap_err(), ComposeError::InOutbox);
    assert_eq!(core.list_outbox(a).unwrap().len(), 1, "no second message");
}

/// The folder the app registers is the only place a file can be read from: not the database, not
/// `/proc`, not a path with a link in it, not a pipe, not a file with two names.
#[test]
fn an_attachment_file_is_read_only_from_the_registered_folder() {
    let db = TempDb::new();
    let provider = MemProvider::new();
    let core = db.open(&provider);
    let a = block(core.add_demo_account()).unwrap().id;
    let id = core
        .save_draft(a, None, content(&["b@example.org"], "s", "b"))
        .unwrap()
        .id;
    let add = |p: &Path| {
        block(core.add_draft_attachment_file(
            a,
            id,
            p.display().to_string(),
            "f.bin".into(),
            "a/b".into(),
        ))
    };
    let root = tempfile::tempdir().unwrap();
    let cache = root.path().join("cache");
    let attach = cache.join("attach");
    std::fs::create_dir_all(attach.join("sub")).unwrap();
    std::fs::write(attach.join("ok.bin"), b"fine").unwrap();
    std::fs::write(attach.join("sub").join("deep.bin"), b"deeper").unwrap();
    std::fs::write(cache.join("outside.bin"), b"outside").unwrap();

    // Nothing is read before the app has registered a folder.
    assert_eq!(
        add(&attach.join("ok.bin")).unwrap_err(),
        ComposeError::NoAttachmentDirectory
    );
    // The folder must be absolute, existing, a folder, and not hold the database.
    for bad in [
        "relative/dir".to_owned(),
        attach.join("missing").display().to_string(),
        attach.join("ok.bin").display().to_string(),
        format!("{}/../cache", attach.display()),
        db.path.parent().unwrap().display().to_string(),
        "/".to_owned(),
    ] {
        assert_eq!(
            core.set_attachment_directory(bad.clone()).unwrap_err(),
            ComposeError::BadAttachmentDirectory,
            "{bad}"
        );
    }
    core.set_attachment_directory(attach.display().to_string())
        .unwrap();

    // Inside: a file, and one in a sub-folder.
    assert_eq!(add(&attach.join("ok.bin")).unwrap().size_bytes, 4);
    assert_eq!(
        add(&attach.join("sub").join("deep.bin"))
            .unwrap()
            .size_bytes,
        6
    );

    // Outside the folder: a sibling, the parent, the database, the files of /proc.
    let refused = ComposeError::AttachmentUnreadable;
    assert_eq!(add(&cache.join("outside.bin")).unwrap_err(), refused);
    assert_eq!(add(&db.path).unwrap_err(), refused);
    assert_eq!(add(Path::new("/proc/self/environ")).unwrap_err(), refused);
    assert_eq!(
        add(Path::new("/proc/self/root/etc/hostname")).unwrap_err(),
        refused
    );
    assert_eq!(add(Path::new("/etc/hostname")).unwrap_err(), refused);
    // `..` out of the folder, and a name that only starts like the folder.
    assert_eq!(
        add(&attach.join("..").join("outside.bin")).unwrap_err(),
        refused
    );
    let twin = cache.join("attach-twin");
    std::fs::create_dir_all(&twin).unwrap();
    std::fs::write(twin.join("x.bin"), b"x").unwrap();
    assert_eq!(add(&twin.join("x.bin")).unwrap_err(), refused);
    // The folder itself, and a sub-folder, are not files.
    assert_eq!(add(&attach).unwrap_err(), refused);
    assert_eq!(add(&attach.join("sub")).unwrap_err(), refused);

    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;
        // A link as the file, to a file inside and to one outside.
        symlink(attach.join("ok.bin"), attach.join("link-in")).unwrap();
        symlink(cache.join("outside.bin"), attach.join("link-out")).unwrap();
        assert_eq!(add(&attach.join("link-in")).unwrap_err(), refused);
        assert_eq!(add(&attach.join("link-out")).unwrap_err(), refused);
        // A link anywhere in the path: a folder inside pointing at a folder inside, one pointing
        // outside, and one pointing at the root of the system.
        symlink(attach.join("sub"), attach.join("sub-link")).unwrap();
        symlink(&cache, attach.join("cache-link")).unwrap();
        symlink("/", attach.join("root-link")).unwrap();
        assert_eq!(
            add(&attach.join("sub-link").join("deep.bin")).unwrap_err(),
            refused
        );
        assert_eq!(
            add(&attach.join("cache-link").join("outside.bin")).unwrap_err(),
            refused
        );
        assert_eq!(
            add(&attach.join("root-link").join("etc").join("hostname")).unwrap_err(),
            refused
        );
        // A pipe: refused at once, never waited for.
        let fifo = attach.join("pipe");
        let made = std::process::Command::new("mkfifo").arg(&fifo).status();
        if made.is_ok_and(|s| s.success()) {
            let started = std::time::Instant::now();
            assert_eq!(add(&fifo).unwrap_err(), refused);
            assert!(
                started.elapsed() < Duration::from_secs(5),
                "waited for a pipe"
            );
        }
        // A file with two names: both are refused.
        std::fs::hard_link(attach.join("ok.bin"), attach.join("ok-twin.bin")).unwrap();
        assert_eq!(add(&attach.join("ok.bin")).unwrap_err(), refused);
        assert_eq!(add(&attach.join("ok-twin.bin")).unwrap_err(), refused);
    }
    // Too large is refused from the size, before the file is read.
    let big = attach.join("big.bin");
    std::fs::File::create(&big)
        .unwrap()
        .set_len(MAX_ATTACHMENT_BYTES + 1)
        .unwrap();
    assert_eq!(add(&big).unwrap_err(), ComposeError::AttachmentTooLarge);
    // Nothing but the two good files was added.
    assert_eq!(core.load_draft(a, id).unwrap().attachments.len(), 2);
}
