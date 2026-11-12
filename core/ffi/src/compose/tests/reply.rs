//! Reply, reply all and forward from stored messages, recipient suggestions, and the identity
//! picked for an answer.

use croft_protocols::{Flags, RemoteFolder, RemoteHeader, SpecialUse};
use croft_storage::{folders, messages};

use super::*;

struct Stored {
    db: TempDb,
    core: Arc<Core>,
    account: i64,
    inbox: i64,
}

fn stored() -> Stored {
    let db = TempDb::new();
    let provider = MemProvider::new();
    let id = {
        let d = Db::open(&db.path, None).unwrap();
        crate::tests::insert_account(&d, 1)
    };
    let core = db.open(&provider);
    let inbox = {
        let d = Db::open(&db.path, None).unwrap();
        folders::upsert(
            d.connection(),
            id,
            &RemoteFolder {
                path: "INBOX".into(),
                delimiter: Some('/'),
                special_use: SpecialUse::Inbox,
                selectable: true,
            },
        )
        .unwrap()
    };
    Stored {
        db,
        core,
        account: id,
        inbox,
    }
}

impl Stored {
    /// A message from Ann with the given stored To and Cc (JSON) and body (text and header block).
    fn message(&self, uid: u32, to: &str, cc: &str, body: Option<(&str, &str)>) -> i64 {
        let d = Db::open(&self.db.path, None).unwrap();
        let c = d.connection();
        let id = messages::upsert(
            c,
            self.account,
            self.inbox,
            &RemoteHeader {
                uid,
                message_id: Some(format!("<orig{uid}@example.com>")),
                references: vec![],
                subject: "Plans for Friday".into(),
                from_name: Some("Ann Lee".into()),
                from_addr: Some("ann@example.com".into()),
                date_ms: 1_794_393_000_000,
                size: 100,
                flags: Flags::default(),
            },
        )
        .unwrap();
        c.execute(
            "UPDATE message SET to_json = ?2, cc_json = ?3 WHERE id = ?1",
            (id, to, cc),
        )
        .unwrap();
        if let Some((headers, text)) = body {
            c.execute(
                "INSERT INTO message_body (message_id, headers_raw, text_plain) VALUES (?1, ?2, ?3)",
                (id, headers, text),
            )
            .unwrap();
        }
        id
    }
}

const TO: &str = r#"[{"name":"Me","address":"user1@example.org"},{"address":"carl@example.com"}]"#;
const CC: &str = r#"["Dee <dee@example.com>", "USER1@example.org"]"#;

fn addrs(list: &[FfiAddress]) -> Vec<&str> {
    list.iter().map(|a| a.address.as_str()).collect()
}

#[test]
fn a_reply_goes_to_the_sender_and_threads_with_the_stored_text() {
    let s = stored();
    let id = s.message(
        1,
        TO,
        CC,
        Some((
            "From: Ann Lee <ann@example.com>\r\nReferences: <root@example.com>\r\n",
            "Line one\n\nLine three",
        )),
    );
    let r = s.core.reply_draft(s.account, id).unwrap();
    assert_eq!(addrs(&r.content.to), ["ann@example.com"]);
    assert!(r.content.cc.is_empty());
    assert_eq!(r.content.subject, "Re: Plans for Friday");
    assert_eq!(
        r.content.in_reply_to.as_deref(),
        Some("<orig1@example.com>")
    );
    assert_eq!(
        r.content.references,
        ["<root@example.com>", "<orig1@example.com>"]
    );
    assert!(r.quote_included);
    assert!(
        r.content
            .body
            .contains("Ann Lee wrote:\n> Line one\n>\n> Line three\n"),
        "{}",
        r.content.body
    );
    // Answered from the address the message was sent to.
    let me = s.core.identities(s.account).unwrap()[0].id;
    assert_eq!(r.content.identity, Some(me));
    // The result is a plain draft: the app stores it when the person starts to write.
    let saved = s.core.save_draft(s.account, None, r.content).unwrap();
    assert_eq!(
        s.core
            .load_draft(s.account, saved.id)
            .unwrap()
            .content
            .in_reply_to
            .as_deref(),
        Some("<orig1@example.com>")
    );
}

#[test]
fn reply_all_leaves_out_the_person_and_uses_the_stored_to_and_cc() {
    let s = stored();
    let id = s.message(1, TO, CC, None);
    let r = s.core.reply_all_draft(s.account, id).unwrap();
    assert_eq!(addrs(&r.content.to), ["ann@example.com"]);
    assert_eq!(
        addrs(&r.content.cc),
        ["carl@example.com", "dee@example.com"]
    );
    assert!(!r.quote_included);
    assert_eq!(r.content.body, "", "no stored body: the quote is empty");
    // Another identity of the account is not answered either.
    {
        let d = Db::open(&s.db.path, None).unwrap();
        s.core.identities(s.account).unwrap();
        d.connection()
            .execute(
                "INSERT INTO identity (account_id, display_name, address, is_default) VALUES (?1, 'Work', 'carl@example.com', 0)",
                [s.account],
            )
            .unwrap();
    }
    let r = s.core.reply_all_draft(s.account, id).unwrap();
    assert_eq!(addrs(&r.content.cc), ["dee@example.com"]);
}

#[test]
fn reply_all_reads_a_stored_header_block_before_the_stored_lists() {
    let s = stored();
    let headers = "From: Ann Lee <ann@example.com>\r\nTo: Me <user1@example.org>, Eve <eve@example.com>\r\nCc: Fay <fay@example.com>\r\nReply-To: list@example.com\r\n";
    let id = s.message(1, "[]", "[]", Some((headers, "text")));
    let r = s.core.reply_all_draft(s.account, id).unwrap();
    assert_eq!(addrs(&r.content.to), ["list@example.com"]);
    assert_eq!(addrs(&r.content.cc), ["eve@example.com", "fay@example.com"]);
}

#[test]
fn a_forward_has_no_recipients_and_the_forwarded_block() {
    let s = stored();
    let id = s.message(1, TO, CC, Some(("", "Original text")));
    let f = s.core.forward_draft(s.account, id).unwrap();
    assert!(f.content.to.is_empty() && f.content.cc.is_empty());
    assert_eq!(f.content.subject, "Fwd: Plans for Friday");
    assert_eq!(f.content.in_reply_to, None);
    assert!(f.quote_included);
    assert!(f
        .content
        .body
        .contains("---------- Forwarded message ----------"));
    assert!(f.content.body.contains("From: Ann Lee <ann@example.com>"));
    assert!(f.content.body.contains("Original text"));
    assert_eq!(f.content.identity, None);
    let no_body = s.message(2, TO, CC, None);
    let f = s.core.forward_draft(s.account, no_body).unwrap();
    assert!(!f.quote_included);
}

#[test]
fn the_prefix_is_added_once_and_another_accounts_message_is_refused() {
    let s = stored();
    let id = s.message(1, TO, CC, None);
    {
        let d = Db::open(&s.db.path, None).unwrap();
        d.connection()
            .execute(
                "UPDATE message SET subject = 'RE: Re: Plans' WHERE id = ?1",
                [id],
            )
            .unwrap();
    }
    assert_eq!(
        s.core.reply_draft(s.account, id).unwrap().content.subject,
        "Re: Re: Plans"
    );
    let other = {
        let d = Db::open(&s.db.path, None).unwrap();
        crate::tests::insert_account(&d, 2)
    };
    assert_eq!(
        s.core.reply_draft(other, id).unwrap_err(),
        ComposeError::NoSuchMessage
    );
    assert_eq!(
        s.core.forward_draft(other, id).unwrap_err(),
        ComposeError::NoSuchMessage
    );
    assert_eq!(
        s.core.reply_all_draft(s.account, id + 99).unwrap_err(),
        ComposeError::NoSuchMessage
    );
}

#[test]
fn suggestions_come_from_the_sent_folder_when_the_sync_stored_recipients() {
    let s = stored();
    let sent = {
        let d = Db::open(&s.db.path, None).unwrap();
        folders::upsert(
            d.connection(),
            s.account,
            &RemoteFolder {
                path: "Sent".into(),
                delimiter: Some('/'),
                special_use: SpecialUse::Sent,
                selectable: true,
            },
        )
        .unwrap()
    };
    {
        let d = Db::open(&s.db.path, None).unwrap();
        let c = d.connection();
        let id = messages::upsert(
            c,
            s.account,
            sent,
            &RemoteHeader {
                uid: 1,
                message_id: Some("<s1@example.org>".into()),
                references: vec![],
                subject: "x".into(),
                from_name: None,
                from_addr: None,
                date_ms: 1000,
                size: 1,
                flags: Flags::default(),
            },
        )
        .unwrap();
        c.execute(
            "UPDATE message SET to_json = ?2, cc_json = ?3 WHERE id = ?1",
            (
                id,
                r#"[{"name":"Gina Park","address":"gina@example.com"}]"#,
                r#"["gus@example.com"]"#,
            ),
        )
        .unwrap();
    }
    let found = |p: &str| addrs_owned(s.core.suggest_recipients(s.account, p.into(), 10).unwrap());
    assert_eq!(found("gi"), ["gina@example.com"]);
    assert_eq!(found("park"), ["gina@example.com"]);
    assert_eq!(found("g"), ["gina@example.com", "gus@example.com"]);
    assert!(found("").is_empty());
}

fn addrs_owned(list: Vec<FfiAddress>) -> Vec<String> {
    list.into_iter().map(|a| a.address).collect()
}
