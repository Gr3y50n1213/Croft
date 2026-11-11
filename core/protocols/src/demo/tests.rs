// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use super::*;
use crate::SpecialUse;

/// A fixed "now": 11 November 2026, 12:00 UTC.
const NOW: i64 = 1_794_398_400_000;

/// Runs a future on a runtime **without an I/O driver**: any socket (tokio's TCP, UDP or DNS) would
/// panic with "IO is disabled". The demo backend must never need one.
fn run<F: std::future::Future>(f: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap()
        .block_on(f)
}

fn all_sources(b: &DemoBackend) -> Vec<String> {
    run(async {
        let mut out = Vec::new();
        for f in b.list_folders().await.unwrap() {
            for uid in b.snapshot(&f.path).await.unwrap().messages.keys() {
                out.push(b.message_source(&f.path, *uid).unwrap());
            }
        }
        out
    })
}

#[test]
fn it_has_the_five_folders_with_their_special_uses() {
    let b = DemoBackend::new(NOW);
    let folders = run(b.list_folders()).unwrap();
    let got: Vec<(&str, SpecialUse)> = folders
        .iter()
        .map(|f| (f.path.as_str(), f.special_use))
        .collect();
    assert_eq!(
        got,
        [
            (INBOX, SpecialUse::Inbox),
            (SENT, SpecialUse::Sent),
            (DRAFTS, SpecialUse::Drafts),
            (TRASH, SpecialUse::Trash),
            (ARCHIVE, SpecialUse::Archive),
        ]
    );
    assert!(folders.iter().all(|f| f.selectable));
}

#[test]
fn the_snapshot_has_every_message_with_its_flags() {
    let b = DemoBackend::new(NOW);
    let mut total = 0;
    for (path, n) in [
        (INBOX, 10),
        (SENT, 3),
        (DRAFTS, 2),
        (TRASH, 2),
        (ARCHIVE, 3),
    ] {
        let snap = run(b.snapshot(path)).unwrap();
        assert_eq!(snap.messages.len(), n, "{path}");
        assert_eq!(snap.status.exists as usize, n);
        assert_eq!(snap.status.uid_validity, data::UID_VALIDITY);
        assert!(snap.status.uid_next as usize > n);
        assert_eq!(snap, run(b.snapshot(path)).unwrap(), "stable");
        total += n;
    }
    assert!((15..=25).contains(&total), "{total}");
    let inbox = run(b.snapshot(INBOX)).unwrap();
    assert!(inbox.messages.values().any(|f| !f.seen), "some unread");
    assert!(inbox.messages.values().any(|f| f.flagged), "some starred");
    let drafts = run(b.snapshot(DRAFTS)).unwrap();
    assert!(drafts.messages.values().all(|f| f.draft));
    assert!(matches!(
        run(b.snapshot("Nowhere")),
        Err(BackendError::Permanent(_))
    ));
}

#[test]
fn headers_come_newest_first_and_are_the_same_for_the_same_now() {
    let b = DemoBackend::new(NOW);
    let uids: Vec<Uid> = run(b.snapshot(INBOX))
        .unwrap()
        .messages
        .keys()
        .copied()
        .collect();
    let headers = run(b.fetch_headers(INBOX, &uids)).unwrap();
    assert_eq!(headers.len(), uids.len());
    assert!(headers.windows(2).all(|w| w[0].date_ms >= w[1].date_ms));
    assert!(headers.iter().all(|h| h.date_ms < NOW));
    // UIDs grow with the date, as on a real server.
    assert!(headers.windows(2).all(|w| w[0].uid > w[1].uid));
    assert_eq!(headers[0].subject, "Welcome to the sample mailbox");
    let again = DemoBackend::new(NOW);
    assert_eq!(run(again.fetch_headers(INBOX, &uids)).unwrap(), headers);
    // Unknown UIDs are left out.
    assert!(run(b.fetch_headers(INBOX, &[999])).unwrap().is_empty());
}

#[test]
fn the_inbox_has_a_long_thread_an_attachment_and_a_remote_image() {
    let b = DemoBackend::new(NOW);
    let snap = run(b.snapshot(INBOX)).unwrap();
    let uids: Vec<Uid> = snap.messages.keys().copied().collect();
    let headers = run(b.fetch_headers(INBOX, &uids)).unwrap();
    let thread = headers
        .iter()
        .filter(|h| {
            h.message_id.as_deref() == Some("<hike-1@sample.example.org>")
                || h.references
                    .contains(&"<hike-1@sample.example.org>".to_owned())
        })
        .count();
    assert!(thread >= 3, "{thread}");
    let last = headers
        .iter()
        .find(|h| h.message_id.as_deref() == Some("<hike-4@sample.example.org>"))
        .unwrap();
    assert_eq!(
        last.references,
        [
            "<hike-1@sample.example.org>",
            "<hike-2@sample.example.org>",
            "<hike-3@sample.example.org>"
        ]
    );

    let sources: Vec<String> = uids
        .iter()
        .map(|u| b.message_source(INBOX, *u).unwrap())
        .collect();
    assert!(sources
        .iter()
        .any(|s| s.contains("Content-Disposition: attachment; filename=\"agenda.txt\"")));
    assert!(sources.iter().any(|s| s.contains("Content-Type: text/html")
        && s.contains("<img src=\"https://images.example.com/")));
    // Well-formed: CRLF only, a blank line after the header, a size that matches.
    for (uid, s) in uids.iter().zip(&sources) {
        assert!(!s.replace("\r\n", "").contains('\n'), "bare LF");
        assert!(s.contains("\r\n\r\n"));
        let h = headers.iter().find(|h| h.uid == *uid).unwrap();
        assert_eq!(h.size as usize, s.len());
    }
}

/// Every address and every link host in the sample set is at example.org or example.com, so the
/// public repository and the shipped app hold no real person's address.
#[test]
fn every_address_and_link_is_at_an_example_domain() {
    let b = DemoBackend::new(NOW);
    let ok = |domain: &str| {
        let d = domain.to_ascii_lowercase();
        ["example.org", "example.com"]
            .iter()
            .any(|e| d == *e || d.ends_with(&format!(".{e}")))
    };
    let mut checked = 0;
    for source in all_sources(&b) {
        for word in source.split(|c: char| c.is_whitespace() || "<>\"(),;".contains(c)) {
            if let Some((_, domain)) = word.split_once('@') {
                assert!(ok(domain), "address at {domain}");
                checked += 1;
            }
            if let Some(rest) = word.strip_prefix("https://") {
                let host = rest.split('/').next().unwrap();
                assert!(ok(host), "link to {host}");
                checked += 1;
            }
            assert!(!word.starts_with("http://"), "plain http link");
        }
    }
    assert!(checked > 40, "{checked}");
    assert!(DEMO_ADDRESS.ends_with("@example.org"));
}

#[test]
fn a_flag_change_is_kept_in_the_copy_and_never_changes_the_built_in_set() {
    let b = DemoBackend::new(NOW);
    let snap = run(b.snapshot(INBOX)).unwrap();
    let (uid, before) = snap.messages.iter().find(|(_, f)| !f.flagged).unwrap();
    run(b.set_flag(INBOX, &[*uid], FlagChange::Flagged(true))).unwrap();
    run(b.set_flag(INBOX, &[*uid], FlagChange::Seen(!before.seen))).unwrap();
    // Unknown UIDs are ignored.
    run(b.set_flag(INBOX, &[999], FlagChange::Seen(true))).unwrap();
    let after = run(b.snapshot(INBOX)).unwrap();
    assert!(after.messages[uid].flagged);
    assert_eq!(after.messages[uid].seen, !before.seen);
    assert_eq!(
        run(b.fetch_headers(INBOX, &[*uid])).unwrap()[0].flags,
        after.messages[uid]
    );
    // A fresh sample mailbox still has the original flags: nothing was written anywhere else.
    assert_eq!(
        run(DemoBackend::new(NOW).snapshot(INBOX)).unwrap().messages[uid],
        *before
    );
}

#[test]
fn a_move_gives_the_new_uid_and_can_be_repeated() {
    let b = DemoBackend::new(NOW);
    let inbox = run(b.snapshot(INBOX)).unwrap();
    let uid = *inbox.messages.keys().next().unwrap();
    let id = run(b.fetch_headers(INBOX, &[uid])).unwrap()[0]
        .message_id
        .clone()
        .unwrap();
    let next = run(b.folder_status(ARCHIVE)).unwrap().uid_next;
    let moved = run(b.move_to(INBOX, &[uid], ARCHIVE)).unwrap();
    assert_eq!(moved.dest_uids.get(&uid), Some(&next));
    assert!(!run(b.snapshot(INBOX)).unwrap().messages.contains_key(&uid));
    assert_eq!(run(b.find_by_message_id(ARCHIVE, &id)).unwrap(), Some(next));
    assert_eq!(run(b.find_by_message_id(INBOX, &id)).unwrap(), None);
    assert!(b.message_source(ARCHIVE, next).unwrap().contains(&id));
    // A repeated move (lost reply) is harmless: the UID is gone, the answer is empty.
    assert!(run(b.move_to(INBOX, &[uid], ARCHIVE))
        .unwrap()
        .dest_uids
        .is_empty());
    assert_eq!(run(b.folder_status(ARCHIVE)).unwrap().uid_next, next + 1);
    assert!(matches!(
        run(b.move_to(INBOX, &[uid], "Nowhere")),
        Err(BackendError::Permanent(_))
    ));
}

#[test]
fn delete_forever_removes_and_can_be_repeated() {
    let b = DemoBackend::new(NOW);
    let uid = *run(b.snapshot(TRASH))
        .unwrap()
        .messages
        .keys()
        .next()
        .unwrap();
    run(b.delete_forever(TRASH, &[uid])).unwrap();
    run(b.delete_forever(TRASH, &[uid])).unwrap();
    assert!(!run(b.snapshot(TRASH)).unwrap().messages.contains_key(&uid));
    assert_eq!(run(b.folder_status(TRASH)).unwrap().exists, 1);
}

fn written(id: &str) -> Vec<u8> {
    format!(
        "Message-ID: <{id}@example.org>\r\n\
         Date: Wed, 11 Nov 2026 12:00:00 +0000\r\n\
         From: Robin Sample <robin@example.org>\r\n\
         To: Maya Okafor <maya@example.org>\r\n\
         Subject: =?UTF-8?Q?Caf=C3=A9_on_Sunday?=\r\n\
         In-Reply-To: <hike-4@sample.example.org>\r\n\
         \r\n\
         See you there.\r\n"
    )
    .into_bytes()
}

#[test]
fn sending_sends_nothing_and_files_one_copy_in_sent() {
    let b = Arc::new(DemoBackend::new(NOW));
    let s = b.submitter();
    let mime = written("new-1");
    let before = run(b.snapshot(SENT)).unwrap();
    run(s.send("robin@example.org", &["maya@example.org".into()], &mime)).unwrap();
    // Sending alone changes nothing; the copy is the engine's next step.
    assert_eq!(run(b.snapshot(SENT)).unwrap(), before);
    assert!(!s.files_sent_itself());
    run(s.file_in_sent(Some("<new-1@example.org>"), &mime)).unwrap();
    run(s.file_in_sent(Some("<new-1@example.org>"), &mime)).unwrap();
    let after = run(b.snapshot(SENT)).unwrap();
    assert_eq!(after.messages.len(), before.messages.len() + 1);
    let uid = run(b.find_by_message_id(SENT, "<new-1@example.org>"))
        .unwrap()
        .unwrap();
    assert!(after.messages[&uid].seen);
    let h = &run(b.fetch_headers(SENT, &[uid])).unwrap()[0];
    assert_eq!(h.subject, "Café on Sunday");
    assert_eq!(h.from_name.as_deref(), Some("Robin Sample"));
    assert_eq!(h.from_addr.as_deref(), Some("robin@example.org"));
    assert_eq!(h.references, ["<hike-4@sample.example.org>"]);
    assert_eq!(h.date_ms, NOW);
    assert_eq!(b.message_source(SENT, uid).unwrap().as_bytes(), mime);
}

#[test]
fn resume_keeps_what_the_device_stored_and_fills_the_rest_from_the_built_in_set() {
    let first = Arc::new(DemoBackend::new(NOW));
    // The person starred one message, archived another and sent one.
    let inbox = run(first.snapshot(INBOX)).unwrap();
    let mut uids = inbox.messages.keys().copied();
    let (starred, archived) = (uids.next().unwrap(), uids.next().unwrap());
    run(first.set_flag(INBOX, &[starred], FlagChange::Flagged(true))).unwrap();
    let moved = run(first.move_to(INBOX, &[archived], ARCHIVE)).unwrap();
    run(first.submitter().file_in_sent(None, &written("new-2"))).unwrap();

    // What the device would have stored after syncing: every folder in full, minus references.
    // Drafts never finished its first sync: only one of its two messages is stored.
    let mut saved = Vec::new();
    for f in run(first.list_folders()).unwrap() {
        let snap = run(first.snapshot(&f.path)).unwrap();
        let uids: Vec<Uid> = snap.messages.keys().copied().collect();
        let mut messages = run(first.fetch_headers(&f.path, &uids)).unwrap();
        for m in &mut messages {
            m.references.clear();
        }
        let synced = f.path != DRAFTS;
        if !synced {
            messages.truncate(1);
        }
        saved.push(DemoFolderState {
            path: f.path.clone(),
            synced,
            uid_next: Some(snap.status.uid_next),
            messages,
        });
    }
    saved.push(DemoFolderState {
        path: "Not a sample folder".into(),
        synced: true,
        uid_next: None,
        messages: vec![],
    });

    let again = DemoBackend::resume(NOW, saved);
    for f in run(first.list_folders()).unwrap() {
        assert_eq!(
            run(again.snapshot(&f.path)).unwrap(),
            run(first.snapshot(&f.path)).unwrap(),
            "{}",
            f.path
        );
        let uids: Vec<Uid> = run(again.snapshot(&f.path))
            .unwrap()
            .messages
            .keys()
            .copied()
            .collect();
        // References come back from the built-in set; a message written before the restart has
        // none (the device does not store them; its thread key is already stored).
        let mut expected = run(first.fetch_headers(&f.path, &uids)).unwrap();
        for h in &mut expected {
            if !h
                .message_id
                .as_deref()
                .is_some_and(|id| id.ends_with("@sample.example.org>"))
            {
                h.references.clear();
            }
        }
        assert_eq!(
            run(again.fetch_headers(&f.path, &uids)).unwrap(),
            expected,
            "{}",
            f.path
        );
    }
    let new_uid = moved.dest_uids[&archived];
    assert!(again.message_source(ARCHIVE, new_uid).is_some());
    // A message written before the restart keeps its header only.
    let sent = run(again.find_by_message_id(SENT, "<new-2@example.org>"))
        .unwrap()
        .unwrap();
    assert!(again.message_source(SENT, sent).is_none());
    // New UIDs never reuse an old one.
    assert!(run(again.folder_status(ARCHIVE)).unwrap().uid_next > new_uid);
}

#[test]
fn every_call_works_on_a_runtime_that_has_no_network_driver() {
    // `run` builds a runtime without `enable_io`: if any of these opened a socket, tokio would
    // panic. Every call of both traits is made once.
    let b = Arc::new(DemoBackend::new(NOW));
    let s = b.submitter();
    run(async {
        b.capabilities().await.unwrap();
        let folders = b.list_folders().await.unwrap();
        for f in &folders {
            b.folder_status(&f.path).await.unwrap();
            let snap = b.snapshot(&f.path).await.unwrap();
            let uids: Vec<Uid> = snap.messages.keys().copied().collect();
            b.fetch_headers(&f.path, &uids).await.unwrap();
        }
        let uid = *b
            .snapshot(INBOX)
            .await
            .unwrap()
            .messages
            .keys()
            .next()
            .unwrap();
        b.find_by_message_id(INBOX, "<welcome@sample.example.org>")
            .await
            .unwrap();
        b.set_flag(INBOX, &[uid], FlagChange::Seen(true))
            .await
            .unwrap();
        let moved = b.move_to(INBOX, &[uid], TRASH).await.unwrap();
        b.delete_forever(
            TRASH,
            &moved.dest_uids.values().copied().collect::<Vec<_>>(),
        )
        .await
        .unwrap();
        s.send(
            "robin@example.org",
            &["maya@example.org".into()],
            &written("n3"),
        )
        .await
        .unwrap();
        s.file_in_sent(None, &written("n3")).await.unwrap();
    });
}

/// The demo module's own source names no socket, TLS, DNS or protocol-adapter API, so it has no way
/// to open a connection. (This file is not scanned: it names them to look for them.)
#[test]
fn the_demo_module_uses_no_network_api() {
    let forbidden = [
        "std::net",
        "tokio::net",
        "TcpStream",
        "TcpSocket",
        "UdpSocket",
        "TlsConnector",
        "tokio_rustls",
        "rustls",
        "hickory",
        "ToSocketAddrs",
        "lookup_host",
        "ImapBackend",
        "ImapConfig",
        "JmapBackend",
        "SmtpSender",
        "SmtpConfig",
        "Discoverer",
        "Credential",
        "crate::discovery",
        "crate::jmap",
    ];
    for (name, source) in [
        ("demo/mod.rs", include_str!("mod.rs")),
        ("demo/data.rs", include_str!("data.rs")),
    ] {
        for token in forbidden {
            assert!(!source.contains(token), "{name} mentions {token}");
        }
        // The one `crate::imap` item it uses is the text helpers module, which has no I/O.
        for line in source.lines().filter(|l| l.contains("crate::imap")) {
            assert!(line.contains("crate::imap::text::"), "{name}: {line}");
        }
    }
    // And the text helpers it borrows have no I/O either.
    let text = include_str!("../imap/text.rs");
    for token in ["std::net", "tokio", "TcpStream", "rustls"] {
        assert!(!text.contains(token), "imap/text.rs mentions {token}");
    }
}

/// The control for the test above: on such a runtime a socket call through tokio really panics
/// (so a demo call that opened one could not pass unnoticed). Nothing is sent: the panic comes
/// before any connection attempt.
#[test]
fn a_socket_call_panics_on_the_runtime_without_a_network_driver() {
    let attempt = std::panic::catch_unwind(|| {
        run(async {
            let _ = tokio::net::TcpStream::connect(("127.0.0.1", 9)).await;
        })
    });
    assert!(attempt.is_err());
}
