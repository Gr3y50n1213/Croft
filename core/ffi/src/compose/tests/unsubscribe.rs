//! T37: unsubscribing from a message's `List-Unsubscribe`, end to end against the in-process IMAP
//! and SMTP servers: the email way through the Outbox, the website way handed to the app, a
//! one-click address on a private network refused before anything is sent, and the delete of the
//! older mail from that sender.

use croft_test_support::sample_message;

use super::*;

/// A message from `from` with `List-Unsubscribe: header` (and the one-click header when asked).
fn list_mail(id: &str, from: &str, date: &str, header: &str, one_click: bool) -> String {
    let post = if one_click {
        "List-Unsubscribe-Post: List-Unsubscribe=One-Click\r\n"
    } else {
        ""
    };
    format!(
        "From: News <{from}>\r\nTo: alice@example.org\r\nSubject: News {id}\r\nDate: {date}\r\n\
         Message-ID: <{id}@list.example>\r\nList-Unsubscribe: {header}\r\n{post}\r\nBody.\r\n"
    )
}

/// The inbox rows, newest first, once `n` are there.
fn inbox_rows(m: &Mail, n: usize) -> Vec<MessageRow> {
    let mut rows = Vec::new();
    wait_until("the inbox has the messages", || {
        let _ = m.core().sync_now(m.account);
        rows = m
            .core()
            .list_messages(MessageScope::UnifiedInbox, None, 50)
            .unwrap()
            .rows;
        rows.len() >= n
    });
    rows
}

#[test]
fn the_email_way_goes_through_the_outbox_remembers_the_sender_and_deletes_older_mail() {
    let m = Mail::new();
    let header =
        "<http://plain.example/u>, <mailto:leave@list.example?subject=stop&cc=boss@x.example>";
    m.imap.add_message(
        "INBOX",
        &list_mail(
            "old1",
            "news@list.example",
            "Mon, 02 Mar 2026 09:00:00 +0000",
            header,
            false,
        ),
    );
    m.imap.add_message(
        "INBOX",
        &list_mail(
            "old2",
            "News@List.example",
            "Tue, 03 Mar 2026 09:00:00 +0000",
            header,
            false,
        ),
    );
    m.imap.add_message(
        "INBOX",
        &list_mail(
            "new",
            "news@list.example",
            "Wed, 04 Mar 2026 09:00:00 +0000",
            header,
            false,
        ),
    );
    m.imap.add_message(
        "INBOX",
        &sample_message("other@x.example", "Unrelated", "someone@x.example"),
    );
    let rows = inbox_rows(&m, 4);
    let newest = rows.iter().find(|r| r.subject == "News new").unwrap().id;
    let unrelated = rows.iter().find(|r| r.subject == "Unrelated").unwrap().id;
    assert_eq!(m.core().unsubscribe_info(unrelated).unwrap(), None);

    let info = m.core().unsubscribe_info(newest).unwrap().unwrap();
    assert_eq!(info.way, UnsubscribeWay::Email);
    assert_eq!(info.address.as_deref(), Some("leave@list.example"));
    assert_eq!(info.host, None);
    assert_eq!(info.sender, "News");
    assert_eq!(info.older_count, 2);
    assert!(!info.already);

    let out = block(m.core().unsubscribe(newest, true)).unwrap();
    assert!(out.done && out.failure.is_none() && out.open_url.is_none());
    assert!(out.outbox_id.is_some());
    assert_eq!(out.deleted_ops.len(), 2);
    wait_until("the email went out", || m.accepted() == 1);
    let sent = &m.smtp.accepted()[0];
    assert_eq!(
        sent.rcpt_to,
        ["leave@list.example"],
        "only the list's address, never the cc"
    );
    let text = String::from_utf8(sent.data.clone()).unwrap();
    assert!(text.contains("Subject: stop\r\n"), "{text}");
    let after = m.core().unsubscribe_info(newest).unwrap().unwrap();
    assert!(after.already, "the sender is remembered");
    let left: Vec<String> = m
        .core()
        .list_messages(MessageScope::UnifiedInbox, None, 50)
        .unwrap()
        .rows
        .into_iter()
        .map(|r| r.subject)
        .collect();
    assert!(!left.iter().any(|s| s.starts_with("News old")), "{left:?}");
    assert!(left.contains(&"News new".to_owned()) && left.contains(&"Unrelated".to_owned()));
}

#[test]
fn the_website_way_is_handed_to_the_app_and_a_private_one_click_address_is_refused() {
    let m = Mail::new();
    m.imap.add_message(
        "INBOX",
        &list_mail(
            "page",
            "a@list.example",
            "Mon, 02 Mar 2026 09:00:00 +0000",
            "<https://list.example/manage?u=1>",
            false,
        ),
    );
    m.imap.add_message(
        "INBOX",
        &list_mail(
            "lan",
            "b@list.example",
            "Tue, 03 Mar 2026 09:00:00 +0000",
            "<https://192.168.1.1/unsub>",
            true,
        ),
    );
    m.imap.add_message(
        "INBOX",
        &list_mail(
            "lan-old",
            "b@list.example",
            "Mon, 02 Mar 2026 08:00:00 +0000",
            "<https://192.168.1.1/unsub>",
            true,
        ),
    );
    let rows = inbox_rows(&m, 3);
    let page = rows.iter().find(|r| r.subject == "News page").unwrap().id;
    let lan = rows.iter().find(|r| r.subject == "News lan").unwrap().id;

    let info = m.core().unsubscribe_info(page).unwrap().unwrap();
    assert_eq!(info.way, UnsubscribeWay::Website);
    assert_eq!(info.host.as_deref(), Some("list.example"));
    let out = block(m.core().unsubscribe(page, false)).unwrap();
    assert_eq!(
        out.open_url.as_deref(),
        Some("https://list.example/manage?u=1")
    );
    assert!(!out.done);
    assert!(!m.core().unsubscribe_info(page).unwrap().unwrap().already);

    let info = m.core().unsubscribe_info(lan).unwrap().unwrap();
    assert_eq!(info.way, UnsubscribeWay::OneClick);
    assert_eq!(info.host.as_deref(), Some("192.168.1.1"));
    let out = block(m.core().unsubscribe(lan, true)).unwrap();
    assert_eq!(out.failure, Some(UnsubscribeFailure::Refused));
    assert!(
        out.deleted_ops.is_empty(),
        "nothing is deleted when unsubscribing failed"
    );
    assert!(!m.core().unsubscribe_info(lan).unwrap().unwrap().already);
    assert_eq!(m.accepted(), 0);
}
