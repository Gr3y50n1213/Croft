// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! One behaviour test over `&dyn MailBackend`, run against the IMAP adapter (in-process server)
//! and the JMAP adapter (in-process fake server, and Stalwart under the `integration` feature).
//!
//! The caller seeds the inbox with exactly three messages and leaves Trash empty:
//!
//! 1. `ids[0]`, subject "First — café" (sent as an encoded word), unread, no references;
//! 2. `ids[1]`, subject "Re: First", unread, `References` and `In-Reply-To` pointing at `ids[0]`;
//! 3. `ids[2]`, subject "Third", already read.
//!
//! The first message's Date header is 2 March 2026 09:30:00 UTC ([`DATE_HEADER_MS`]).
//! The test finds messages by Message-ID, never by the order of their UIDs.
#![allow(dead_code, clippy::unwrap_used, clippy::expect_used)]

use croft_protocols::*;
use std::collections::BTreeMap;

/// The first seeded message's Date header, in Unix milliseconds.
pub const DATE_HEADER_MS: i64 = 1_772_443_800_000;

/// The Message-IDs of the three seeded messages (without angle brackets).
pub struct Seeded {
    pub ids: [String; 3],
}

/// Builds the raw text of seeded message `n` (0, 1 or 2) for a recipient.
pub fn seed_message(ids: &[String; 3], n: usize, to: &str) -> String {
    let (subject, extra) = match n {
        0 => (
            "=?UTF-8?Q?First_=E2=80=94_caf=C3=A9?=".to_string(),
            String::new(),
        ),
        1 => (
            "Re: First".to_string(),
            format!("References: <{}>\r\nIn-Reply-To: <{}>\r\n", ids[0], ids[0]),
        ),
        _ => ("Third".to_string(), String::new()),
    };
    format!(
        "From: Ann Example <ann@example.org>\r\nTo: Test <{to}>\r\nSubject: {subject}\r\n\
         Date: Mon, 02 Mar 2026 09:30:0{n} +0000\r\nMessage-ID: <{}>\r\n{extra}\r\n\
         Body of {}.\r\n",
        ids[n], ids[n]
    )
}

async fn uid_by_message_id(b: &dyn MailBackend, folder: &str) -> BTreeMap<String, Uid> {
    let snap = b.snapshot(folder).await.unwrap();
    let uids: Vec<Uid> = snap.messages.keys().copied().collect();
    b.fetch_headers(folder, &uids)
        .await
        .unwrap()
        .into_iter()
        .map(|h| (h.message_id.unwrap_or_default(), h.uid))
        .collect()
}

/// The shared behaviour. `label` names the adapter in failure messages.
pub async fn behaviour(
    b: &dyn MailBackend,
    label: &str,
    seeded: &Seeded,
    date_header_ms: Option<i64>,
) {
    let ids = &seeded.ids;
    let mid = |n: usize| format!("<{}>", ids[n]);

    // Capabilities and folders.
    let caps = b.capabilities().await.unwrap();
    assert!(caps.move_cmd && caps.uidplus, "{label}: {caps:?}");
    let folders = b.list_folders().await.unwrap();
    let find = |s: SpecialUse| {
        folders
            .iter()
            .find(|f| f.special_use == s)
            .unwrap_or_else(|| panic!("{label}: no {s:?} in {folders:?}"))
            .path
            .clone()
    };
    let (inbox, trash) = (find(SpecialUse::Inbox), find(SpecialUse::Trash));
    for s in [SpecialUse::Sent, SpecialUse::Drafts, SpecialUse::Junk] {
        find(s);
    }
    assert!(folders.iter().all(|f| !f.path.is_empty()), "{label}");
    assert!(folders.iter().all(|f| f.selectable), "{label}");

    // Status and snapshot.
    let status = b.folder_status(&inbox).await.unwrap();
    assert_eq!(status.exists, 3, "{label}");
    assert!(
        status.uid_validity != 0 && status.uid_next >= 1,
        "{label}: {status:?}"
    );
    let snap = b.snapshot(&inbox).await.unwrap();
    assert_eq!(snap.messages.len(), 3, "{label}");
    assert_eq!(snap.status.uid_validity, status.uid_validity, "{label}");
    assert_eq!(snap.status.exists, 3, "{label}");
    assert!(
        snap.status.uid_next > *snap.messages.keys().max().unwrap(),
        "{label}"
    );
    let uids: Vec<Uid> = snap.messages.keys().copied().collect();

    // Headers.
    let headers = b.fetch_headers(&inbox, &uids).await.unwrap();
    assert_eq!(headers.len(), 3, "{label}");
    let by_mid: BTreeMap<String, &RemoteHeader> = headers
        .iter()
        .map(|h| (h.message_id.clone().unwrap_or_default(), h))
        .collect();
    let (h1, h2, h3) = (&by_mid[&mid(0)], &by_mid[&mid(1)], &by_mid[&mid(2)]);
    assert_eq!(h1.subject, "First \u{2014} caf\u{e9}", "{label}");
    assert_eq!(h2.subject, "Re: First", "{label}");
    assert_eq!(h2.references, vec![mid(0)], "{label}");
    assert_eq!(h1.from_name.as_deref(), Some("Ann Example"), "{label}");
    assert_eq!(h1.from_addr.as_deref(), Some("ann@example.org"), "{label}");
    // `date_header_ms` is the first message's Date header when the adapter reports that (JMAP's
    // `sentAt`); the IMAP adapter reports the server's internal date, which differs per server.
    assert!(h1.date_ms > 0, "{label}");
    if let Some(expected) = date_header_ms {
        assert_eq!(h1.date_ms, expected, "{label}: the Date header");
    }
    assert!(h1.size > 100, "{label}");
    assert!(!h1.flags.seen && !h2.flags.seen && h3.flags.seen, "{label}");
    // Newest first.
    assert!(
        headers.windows(2).all(|w| w[0].date_ms >= w[1].date_ms),
        "{label}"
    );
    let (u1, u2, u3) = (h1.uid, h2.uid, h3.uid);
    assert_eq!(snap.messages.len(), 3);
    assert!(
        snap.messages[&u3].seen && !snap.messages[&u1].seen,
        "{label}"
    );

    // The whole message, without marking it read; a UID that is not there is gone.
    match b.fetch_body(&inbox, u1).await.unwrap() {
        RawMessage::Found(raw) => {
            let text = String::from_utf8_lossy(&raw);
            assert!(text.contains(&format!("Body of {}.", ids[0])), "{label}");
            assert!(text.contains(&mid(0)), "{label}: the Message-ID header");
        }
        other => panic!("{label}: {other:?}"),
    }
    assert!(
        !b.snapshot(&inbox).await.unwrap().messages[&u1].seen,
        "{label}: fetching a message must not mark it read"
    );
    let absent = u1.max(u2).max(u3) + 1000;
    assert_eq!(
        b.fetch_body(&inbox, absent).await.unwrap(),
        RawMessage::Gone,
        "{label}"
    );

    // Find by Message-ID, and an id that is not there.
    assert_eq!(
        b.find_by_message_id(&inbox, &mid(1)).await.unwrap(),
        Some(u2),
        "{label}"
    );
    assert_eq!(
        b.find_by_message_id(&inbox, "<nobody@example.org>")
            .await
            .unwrap(),
        None,
        "{label}"
    );

    // Flags: set, repeat, clear; star.
    b.set_flag(&inbox, &[u1], FlagChange::Seen(true))
        .await
        .unwrap();
    b.set_flag(&inbox, &[u1], FlagChange::Seen(true))
        .await
        .unwrap();
    let s = b.snapshot(&inbox).await.unwrap();
    assert!(s.messages[&u1].seen && !s.messages[&u2].seen, "{label}");
    b.set_flag(&inbox, &[u1], FlagChange::Seen(false))
        .await
        .unwrap();
    b.set_flag(&inbox, &[u2], FlagChange::Flagged(true))
        .await
        .unwrap();
    let s = b.snapshot(&inbox).await.unwrap();
    assert!(!s.messages[&u1].seen && s.messages[&u2].flagged, "{label}");
    b.set_flag(&inbox, &[u2], FlagChange::Flagged(false))
        .await
        .unwrap();
    // A UID that does not exist is ignored.
    b.set_flag(&inbox, &[99_999], FlagChange::Seen(true))
        .await
        .unwrap();
    assert!(
        b.fetch_headers(&inbox, &[99_999]).await.unwrap().is_empty(),
        "{label}"
    );
    assert_eq!(
        b.snapshot(&inbox).await.unwrap().messages.len(),
        3,
        "{label}"
    );

    // Move to Trash: the new UID is reported, the old one is gone.
    let moved = b.move_to(&inbox, &[u1], &trash).await.unwrap();
    let new_uid = *moved
        .dest_uids
        .get(&u1)
        .unwrap_or_else(|| panic!("{label}: no destination UID in {moved:?}"));
    let inbox_now = b.snapshot(&inbox).await.unwrap();
    assert_eq!(inbox_now.messages.len(), 2, "{label}");
    assert!(!inbox_now.messages.contains_key(&u1), "{label}");
    let in_trash = uid_by_message_id(b, &trash).await;
    assert_eq!(in_trash.len(), 1, "{label}");
    assert_eq!(in_trash[&mid(0)], new_uid, "{label}");
    assert_eq!(
        b.find_by_message_id(&trash, &mid(0)).await.unwrap(),
        Some(new_uid),
        "{label}"
    );
    assert_eq!(b.folder_status(&inbox).await.unwrap().exists, 2, "{label}");
    // The queue re-points later operations at the new UID: flags work there.
    b.set_flag(&trash, &[new_uid], FlagChange::Seen(true))
        .await
        .unwrap();
    assert!(
        b.snapshot(&trash).await.unwrap().messages[&new_uid].seen,
        "{label}"
    );
    // Moving again from the old address is harmless and reports nothing.
    let again = b.move_to(&inbox, &[u1], &trash).await.unwrap();
    assert!(again.dest_uids.is_empty(), "{label}: {again:?}");
    assert_eq!(uid_by_message_id(b, &trash).await.len(), 1, "{label}");

    // Delete forever; repeating is harmless.
    b.delete_forever(&trash, &[new_uid]).await.unwrap();
    assert!(
        b.snapshot(&trash).await.unwrap().messages.is_empty(),
        "{label}"
    );
    b.delete_forever(&trash, &[new_uid]).await.unwrap();
    assert_eq!(
        b.snapshot(&inbox).await.unwrap().messages.len(),
        2,
        "{label}"
    );
    assert_eq!(
        b.find_by_message_id(&trash, &mid(0)).await.unwrap(),
        None,
        "{label}"
    );

    // A folder that does not exist is a permanent refusal.
    let err = b.snapshot("No/Such/Folder").await.unwrap_err();
    assert!(
        matches!(err, BackendError::Permanent(_)),
        "{label}: {err:?}"
    );
}
