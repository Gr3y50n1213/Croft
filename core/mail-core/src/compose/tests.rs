#![allow(clippy::unwrap_used, clippy::expect_used)]

use super::*;

fn addr(name: Option<&str>, a: &str) -> Address {
    Address {
        name: name.map(str::to_owned),
        address: a.to_owned(),
    }
}

fn build<'a>(body: &'a str, f: impl FnOnce(&mut Outgoing<'a>)) -> String {
    // Leaked on purpose (tests only): the closure may borrow from its caller for `'a`.
    let from: &'a Address = Box::leak(Box::new(addr(Some("Alice Writer"), "alice@example.org")));
    let to: &'a [Address] = Box::leak(Box::new([addr(None, "bob@example.org")]));
    let mut m = Outgoing {
        from,
        to,
        cc: &[],
        bcc: &[],
        subject: "Hello",
        body,
        in_reply_to: None,
        references: &[],
        message_id: "<abc@example.org>",
        date_ms: 1_794_393_000_000,
        as_draft: false,
        attachments: &[],
        boundary_seed: [7; 8],
    };
    f(&mut m);
    String::from_utf8(build_message(&m).unwrap()).unwrap()
}

/// The header lines of a built message, unfolded.
fn headers(text: &str) -> Vec<(String, String)> {
    let head = text.split("\r\n\r\n").next().unwrap();
    let mut out: Vec<(String, String)> = Vec::new();
    for line in head.split("\r\n") {
        if line.starts_with(' ') {
            let last = out.last_mut().unwrap();
            last.1.push_str(line);
        } else {
            let (n, v) = line.split_once(':').unwrap();
            out.push((n.to_owned(), v.trim().to_owned()));
        }
    }
    out
}

fn header(text: &str, name: &str) -> Option<String> {
    headers(text)
        .into_iter()
        .find(|(n, _)| n.eq_ignore_ascii_case(name))
        .map(|(_, v)| v)
}

fn decode_qp(text: &str) -> Vec<u8> {
    let mut out = Vec::new();
    let b = text.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'=' {
            if b.get(i + 1) == Some(&b'\r') {
                i += 3;
                continue;
            }
            let hex = std::str::from_utf8(&b[i + 1..i + 3]).unwrap();
            out.push(u8::from_str_radix(hex, 16).unwrap());
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    out
}

#[test]
fn addresses_are_checked() {
    for good in [
        "a@example.org",
        "first.last+tag@sub.example.org",
        "alice@localhost",
        "jörg@example.org",
    ] {
        assert!(is_valid_address(good), "{good}");
    }
    for bad in [
        "",
        "a",
        "a@",
        "@b.org",
        "a b@example.org",
        "a@@example.org",
        "a@exa mple.org",
        ".a@example.org",
        "a..b@example.org",
        "a@example..org",
        "a@-example.org",
        "a\r\n@example.org",
        "a@example.org\r\nBcc: x@y.z",
        "<a@example.org>",
        "\"a\"@example.org",
        "a,b@example.org",
    ] {
        assert!(!is_valid_address(bad), "{bad:?}");
    }
    let long = format!("{}@example.org", "a".repeat(65));
    assert!(!is_valid_address(&long));
}

#[test]
fn an_address_is_read_as_typed_or_as_a_header_writes_it() {
    let a = parse_address("Ann Example <ann@example.org>").unwrap();
    assert_eq!(a.name.as_deref(), Some("Ann Example"));
    assert_eq!(a.address, "ann@example.org");
    let a = parse_address("\"Example, Ann\" <ann@example.org>").unwrap();
    assert_eq!(a.name.as_deref(), Some("Example, Ann"));
    assert_eq!(parse_address("<x@y.org>").unwrap().name, None);
    assert_eq!(parse_address(" x@y.org ").unwrap().address, "x@y.org");
    assert!(parse_address("   ").is_none());
    assert!(parse_address("Name <>").is_none());
    let list = parse_address_list("\"A, B\" <ab@x.org>, c@x.org; D <d@x.org>", 10);
    let addrs: Vec<&str> = list.iter().map(|a| a.address.as_str()).collect();
    assert_eq!(addrs, ["ab@x.org", "c@x.org", "d@x.org"]);
    assert_eq!(parse_address_list("a@x.org, b@x.org, c@x.org", 2).len(), 2);
}

#[test]
fn the_date_header_is_right() {
    assert_eq!(rfc5322_date(0), "Thu, 01 Jan 1970 00:00:00 +0000");
    assert_eq!(
        rfc5322_date(1_794_393_000_000),
        "Wed, 11 Nov 2026 10:30:00 +0000"
    );
    // A leap day.
    assert_eq!(
        rfc5322_date(1_709_164_800_000),
        "Thu, 29 Feb 2024 00:00:00 +0000"
    );
    // Before 1970.
    assert_eq!(rfc5322_date(-1000), "Wed, 31 Dec 1969 23:59:59 +0000");
}

#[test]
fn a_message_id_has_the_senders_domain_or_a_fixed_one() {
    let id = new_message_id([0xab; 16], "alice@example.org");
    assert_eq!(id, format!("<{}@example.org>", "ab".repeat(16)));
    assert!(new_message_id([1; 16], "jörg@münchen.de").ends_with("@croft.invalid>"));
    assert!(new_message_id([1; 16], "no-at-sign").ends_with("@croft.invalid>"));
    assert!(clean_message_id(&id).is_some());
}

#[test]
fn only_a_plain_id_is_copied_to_a_header() {
    assert!(clean_message_id("<a@b>").is_some());
    for bad in [
        "a@b",
        "<>",
        "<a b@c>",
        "<a\r\nBcc: x@y>",
        "<a<b>@c>",
        "<ä@c>",
    ] {
        assert!(clean_message_id(bad).is_none(), "{bad:?}");
    }
}

#[test]
fn a_plain_message_has_the_expected_headers_and_no_bcc() {
    let text = build("Hi Bob,\nsee you.", |m| {
        m.subject = "Lunch";
    });
    assert_eq!(
        header(&text, "From").unwrap(),
        "Alice Writer <alice@example.org>"
    );
    assert_eq!(header(&text, "To").unwrap(), "bob@example.org");
    assert_eq!(header(&text, "Subject").unwrap(), "Lunch");
    assert_eq!(header(&text, "Message-ID").unwrap(), "<abc@example.org>");
    assert_eq!(
        header(&text, "Date").unwrap(),
        "Wed, 11 Nov 2026 10:30:00 +0000"
    );
    assert_eq!(header(&text, "MIME-Version").unwrap(), "1.0");
    assert!(header(&text, "Bcc").is_none());
    assert!(
        text.ends_with("\r\n\r\nHi Bob,\r\nsee you.\r\n"),
        "{text:?}"
    );
    assert!(!text.contains("\r\n\n") && !text.replace("\r\n", "").contains('\n'));
}

#[test]
fn bcc_is_written_only_for_the_draft_copy() {
    let bcc = [addr(None, "secret@example.org")];
    let sent = build("x", |m| m.bcc = &bcc);
    assert!(!sent.to_lowercase().contains("bcc"));
    assert!(!sent.contains("secret@example.org"));
    let draft = build("x", |m| {
        m.bcc = &bcc;
        m.as_draft = true;
    });
    assert_eq!(header(&draft, "Bcc").unwrap(), "secret@example.org");
}

#[test]
fn a_message_with_only_bcc_recipients_is_allowed() {
    let bcc = [addr(None, "secret@example.org")];
    let from = addr(None, "a@example.org");
    let m = Outgoing {
        from: &from,
        to: &[],
        cc: &[],
        bcc: &bcc,
        subject: "s",
        body: "b",
        in_reply_to: None,
        references: &[],
        message_id: "<m@x>",
        date_ms: 0,
        as_draft: false,
        attachments: &[],
        boundary_seed: [0; 8],
    };
    let text = String::from_utf8(build_message(&m).unwrap()).unwrap();
    assert!(header(&text, "To").is_none());
}

#[test]
fn refusals() {
    let from = addr(None, "a@example.org");
    let good = [addr(None, "b@example.org")];
    let base = Outgoing {
        from: &from,
        to: &good,
        cc: &[],
        bcc: &[],
        subject: "s",
        body: "b",
        in_reply_to: None,
        references: &[],
        message_id: "<m@x>",
        date_ms: 0,
        as_draft: false,
        attachments: &[],
        boundary_seed: [0; 8],
    };
    assert!(build_message(&base).is_ok());
    let none = Outgoing { to: &[], ..base };
    assert_eq!(build_message(&none).unwrap_err(), BuildError::NoRecipient);
    let bad = [addr(None, "not an address")];
    let invalid = Outgoing { cc: &bad, ..base };
    assert_eq!(
        build_message(&invalid).unwrap_err(),
        BuildError::InvalidAddress
    );
    let bad_from = addr(None, "x");
    let invalid = Outgoing {
        from: &bad_from,
        ..base
    };
    assert_eq!(
        build_message(&invalid).unwrap_err(),
        BuildError::InvalidAddress
    );
    let bad_id = Outgoing {
        message_id: "no brackets",
        ..base
    };
    assert_eq!(
        build_message(&bad_id).unwrap_err(),
        BuildError::BadMessageId
    );
}

#[test]
fn no_typed_text_can_add_a_header_line() {
    let evil_name = addr(Some("Eve\r\nBcc: victim@example.org"), "eve@example.org");
    let to = [addr(Some("X\nBcc: v@example.org"), "bob@example.org")];
    let refs = vec!["<ok@x>".to_owned(), "<a>\r\nBcc: v@example.org".to_owned()];
    let text = build("body", |m| {
        m.from = &evil_name;
        m.to = &to;
        m.subject = "Hi\r\nBcc: victim@example.org\rBcc: v2@example.org";
        m.in_reply_to = Some("<p@x>\r\nBcc: v3@example.org");
        m.references = &refs;
    });
    let head = text.split("\r\n\r\n").next().unwrap();
    assert!(
        headers(&text)
            .iter()
            .all(|(n, _)| !n.eq_ignore_ascii_case("bcc")),
        "{head:?}"
    );
    assert!(!head.contains('\r') || head.replace("\r\n", "").find('\r').is_none());
    assert!(header(&text, "In-Reply-To").is_none());
    assert_eq!(header(&text, "References").unwrap(), "<ok@x>");
}

#[test]
fn a_non_ascii_subject_and_name_are_encoded_words_and_decode_back() {
    let name = addr(Some("Jörg Müller"), "joerg@example.org");
    let subject = "Grüße aus Köln – ein sehr langer Betreff mit vielen Wörtern, damit er über mehrere Zeilen läuft und gefaltet werden muss";
    let text = build("x", |m| {
        m.from = &name;
        m.subject = subject;
    });
    for line in text.split("\r\n") {
        assert!(line.len() <= 78, "{line:?}");
    }
    let decode = |value: &str| -> String {
        let mut out = Vec::new();
        for word in value.split(' ').filter(|w| w.starts_with("=?")) {
            let b64 = word
                .strip_prefix("=?UTF-8?B?")
                .unwrap()
                .strip_suffix("?=")
                .unwrap();
            out.extend(
                base64::engine::general_purpose::STANDARD
                    .decode(b64)
                    .unwrap(),
            );
        }
        String::from_utf8(out).unwrap()
    };
    // The folded header, unfolded by `headers`, is words separated by nothing; split them.
    let raw = text.split("\r\n\r\n").next().unwrap();
    let subject_value: String = raw
        .split("\r\n")
        .skip_while(|l| !l.starts_with("Subject:"))
        .take_while(|l| l.starts_with("Subject:") || l.starts_with(' '))
        .map(|l| l.trim_start_matches("Subject:").trim())
        .collect::<Vec<_>>()
        .join(" ");
    assert_eq!(decode(&subject_value), subject);
    let from = header(&text, "From").unwrap();
    assert!(from.ends_with(" <joerg@example.org>"));
    assert_eq!(decode(&from), "Jörg Müller");
    assert!(text.is_ascii());
}

#[test]
fn a_long_ascii_subject_is_folded_at_spaces() {
    let subject = "word ".repeat(40);
    let text = build("x", |m| m.subject = &subject);
    for line in text.split("\r\n") {
        assert!(line.len() <= 78, "{line:?}");
    }
    assert_eq!(
        header(&text, "Subject").unwrap().split_whitespace().count(),
        40
    );
}

#[test]
fn many_recipients_are_folded_and_kept_whole() {
    let to: Vec<Address> = (0..12)
        .map(|i| addr(Some("Some Person"), &format!("person{i}@example.org")))
        .collect();
    let text = build("x", |m| m.to = &to);
    for line in text.split("\r\n") {
        assert!(line.len() <= 78, "{line:?}");
    }
    let value = header(&text, "To").unwrap();
    assert_eq!(value.matches('@').count(), 12);
}

#[test]
fn a_name_with_specials_is_quoted() {
    let to = [addr(Some("Doe, \"J\" \\ Jr"), "j@example.org")];
    let text = build("x", |m| m.to = &to);
    assert_eq!(
        header(&text, "To").unwrap(),
        "\"Doe, \\\"J\\\" \\\\ Jr\" <j@example.org>"
    );
    let parsed = parse_address_list(&header(&text, "To").unwrap(), 5);
    assert_eq!(parsed[0].name.as_deref(), Some("Doe, \"J\" \\ Jr"));
}

#[test]
fn references_and_in_reply_to_are_written() {
    let refs = vec!["<root@x>".to_owned(), "<parent@x>".to_owned()];
    let text = build("x", |m| {
        m.in_reply_to = Some("<parent@x>");
        m.references = &refs;
    });
    assert_eq!(header(&text, "In-Reply-To").unwrap(), "<parent@x>");
    assert_eq!(header(&text, "References").unwrap(), "<root@x> <parent@x>");
}

#[test]
fn an_ascii_body_is_seven_bit_and_other_text_is_quoted_printable_that_decodes_back() {
    let text = build("plain\nlines\r\nmixed\rendings", |_| {});
    assert!(text.contains("Content-Transfer-Encoding: 7bit"));
    assert!(text.ends_with("plain\r\nlines\r\nmixed\r\nendings\r\n"));

    let body = "Grüße, 100% = fertig \nZeile mit Leerzeichen am Ende   \n\n=\n日本語のテキスト, ein langer Satz der über die Zeilengrenze von sechsundsiebzig Zeichen hinausgeht und umbrochen wird.";
    let text = build(body, |_| {});
    assert!(text.contains("Content-Transfer-Encoding: quoted-printable"));
    assert!(text.is_ascii());
    let encoded = text.split_once("\r\n\r\n").unwrap().1;
    for line in encoded.split("\r\n") {
        assert!(line.len() <= 76, "{line:?}");
    }
    let decoded = String::from_utf8(decode_qp(encoded)).unwrap();
    assert_eq!(decoded, format!("{}\r\n", body.replace('\n', "\r\n")));
}

#[test]
fn a_very_long_line_is_quoted_printable() {
    let body = "x".repeat(2000);
    let text = build(&body, |_| {});
    assert!(text.contains("quoted-printable"));
    let encoded = text.split_once("\r\n\r\n").unwrap().1;
    assert_eq!(
        String::from_utf8(decode_qp(encoded)).unwrap(),
        format!("{body}\r\n")
    );
}

#[test]
fn attachments_make_a_multipart_with_base64_parts() {
    let bytes: Vec<u8> = (0..=255u8).cycle().take(1000).collect();
    let files = [
        AttachmentData {
            name: "../../etc/pass\"wd\r\n.pdf",
            mime_type: "Application/PDF",
            bytes: &bytes,
        },
        AttachmentData {
            name: "Übersicht.txt",
            mime_type: "not a type",
            bytes: b"hello",
        },
    ];
    let text = build("See attached.", |m| m.attachments = &files);
    let ct = header(&text, "Content-Type").unwrap();
    let boundary = ct
        .split("boundary=\"")
        .nth(1)
        .unwrap()
        .trim_end_matches('"')
        .to_owned();
    assert!(text.ends_with(&format!("--{boundary}--\r\n")));
    let parts: Vec<&str> = text
        .split(&format!("--{boundary}\r\n"))
        .skip(1)
        .map(|p| p.trim_end_matches(&format!("--{boundary}--\r\n")))
        .collect();
    assert_eq!(parts.len(), 3);
    assert!(parts[0].contains("See attached."));
    assert!(
        parts[1].contains("Content-Type: application/pdf; name=\"passwd .pdf\""),
        "{}",
        parts[1]
    );
    assert!(!parts[1].contains("etc"));
    let (_, data) = parts[1].split_once("\r\n\r\n").unwrap();
    let joined: String = data.split("\r\n").collect();
    assert_eq!(
        base64::engine::general_purpose::STANDARD
            .decode(joined)
            .unwrap(),
        bytes
    );
    for line in data.split("\r\n") {
        assert!(line.len() <= 76);
    }
    assert!(parts[2].contains("Content-Type: application/octet-stream"));
    assert!(parts[2].contains("filename*=UTF-8''%C3%9Cbersicht.txt"));
    assert!(parts[2].contains("filename=\"_bersicht.txt\""));
    assert!(text.is_ascii());
}

#[test]
fn a_file_name_with_nothing_left_is_called_attachment() {
    assert_eq!(safe_file_name(""), "attachment");
    assert_eq!(safe_file_name(".."), "attachment");
    assert_eq!(safe_file_name("/"), "attachment");
    assert_eq!(safe_file_name("a/b\\c.txt"), "c.txt");
}

#[test]
fn the_subject_prefix_is_added_once() {
    assert_eq!(prefixed_subject("Lunch", false), "Re: Lunch");
    assert_eq!(prefixed_subject("Re: Lunch", false), "Re: Lunch");
    assert_eq!(prefixed_subject("RE: Lunch", false), "Re: Lunch");
    assert_eq!(prefixed_subject("re:Lunch", false), "Re: Lunch");
    assert_eq!(prefixed_subject("Fwd: Lunch", false), "Re: Fwd: Lunch");
    assert_eq!(prefixed_subject("Lunch", true), "Fwd: Lunch");
    assert_eq!(prefixed_subject("Fwd: Lunch", true), "Fwd: Lunch");
    assert_eq!(prefixed_subject("FW: Lunch", true), "Fwd: Lunch");
    assert_eq!(prefixed_subject("Re: Fwd: x", true), "Fwd: Re: Fwd: x");
    assert_eq!(prefixed_subject("Re:\r\nBcc: x", false), "Re: Bcc: x");
    assert_eq!(prefixed_subject("", false), "Re: ");
}

fn source() -> SourceMessage {
    SourceMessage {
        message_id: Some("<orig@example.com>".into()),
        subject: "Plans".into(),
        from: Some(addr(Some("Ann"), "ann@example.com")),
        reply_to: vec![],
        to: vec![
            addr(Some("Me"), "me@example.org"),
            addr(None, "carl@example.com"),
        ],
        cc: vec![addr(None, "Dee@example.com"), addr(None, "ME@example.org")],
        date_ms: 1_794_393_000_000,
        references: vec!["<root@example.com>".into()],
        text: Some("Line one\r\n\r\nLine three".into()),
    }
}

fn own() -> Vec<String> {
    vec!["me@example.org".into(), "alias@example.org".into()]
}

fn addrs(list: &[Address]) -> Vec<&str> {
    list.iter().map(|a| a.address.as_str()).collect()
}

#[test]
fn a_reply_goes_to_the_sender_with_the_quote_and_the_thread_headers() {
    let f = reply_fields(ReplyKind::Reply, &source(), &own());
    assert_eq!(addrs(&f.to), ["ann@example.com"]);
    assert!(f.cc.is_empty());
    assert_eq!(f.subject, "Re: Plans");
    assert_eq!(f.in_reply_to.as_deref(), Some("<orig@example.com>"));
    assert_eq!(f.references, ["<root@example.com>", "<orig@example.com>"]);
    assert!(f.quote_included);
    assert_eq!(
        f.body,
        "\n\nOn 11 Nov 2026, 10:30 UTC, Ann wrote:\n> Line one\n>\n> Line three\n"
    );
}

#[test]
fn a_reply_uses_reply_to_when_there_is_one() {
    let mut s = source();
    s.reply_to = vec![addr(None, "list@example.com")];
    let f = reply_fields(ReplyKind::Reply, &s, &own());
    assert_eq!(addrs(&f.to), ["list@example.com"]);
}

#[test]
fn reply_all_leaves_out_the_person_and_repeats_nobody() {
    let f = reply_fields(ReplyKind::ReplyAll, &source(), &own());
    assert_eq!(addrs(&f.to), ["ann@example.com"]);
    assert_eq!(addrs(&f.cc), ["carl@example.com", "Dee@example.com"]);
    // The person's other address too, in any case.
    let mut s = source();
    s.cc.push(addr(None, "Alias@Example.org"));
    s.to.push(addr(None, "ANN@example.com"));
    let f = reply_fields(ReplyKind::ReplyAll, &s, &own());
    assert!(f
        .to
        .iter()
        .chain(&f.cc)
        .all(|a| !a.address.to_lowercase().ends_with("@example.org")));
    assert_eq!(addrs(&f.to), ["ann@example.com"]);
    assert_eq!(addrs(&f.cc), ["carl@example.com", "Dee@example.com"]);
}

#[test]
fn answering_a_message_the_person_wrote_goes_to_its_recipients() {
    let mut s = source();
    s.from = Some(addr(Some("Me"), "me@example.org"));
    s.to = vec![addr(None, "ann@example.com")];
    s.cc = vec![addr(None, "carl@example.com")];
    let f = reply_fields(ReplyKind::Reply, &s, &own());
    assert_eq!(addrs(&f.to), ["ann@example.com"]);
    assert!(f.cc.is_empty());
    let f = reply_fields(ReplyKind::ReplyAll, &s, &own());
    assert_eq!(addrs(&f.to), ["ann@example.com"]);
    assert_eq!(addrs(&f.cc), ["carl@example.com"]);
}

#[test]
fn a_forward_has_no_recipients_no_thread_headers_and_the_original_text() {
    let f = reply_fields(ReplyKind::Forward, &source(), &own());
    assert!(f.to.is_empty() && f.cc.is_empty());
    assert_eq!(f.subject, "Fwd: Plans");
    assert_eq!(f.in_reply_to, None);
    assert!(f.references.is_empty());
    assert!(f.quote_included);
    assert!(f.body.starts_with("\n\n---------- Forwarded message ----------\nFrom: Ann <ann@example.com>\nDate: 11 Nov 2026, 10:30 UTC\nSubject: Plans\nTo: Me <me@example.org>, carl@example.com\n\nLine one\n\nLine three\n"));
}

#[test]
fn without_a_stored_body_the_quote_is_empty_and_says_so() {
    let mut s = source();
    s.text = None;
    let r = reply_fields(ReplyKind::Reply, &s, &own());
    assert_eq!(r.body, "");
    assert!(!r.quote_included);
    let f = reply_fields(ReplyKind::Forward, &s, &own());
    assert!(!f.quote_included);
    assert!(f.body.contains("Forwarded message"));
    assert!(!f.body.contains("Line one"));
}

#[test]
fn a_huge_original_is_cut() {
    let mut s = source();
    s.text = Some("a line of text\n".repeat(10_000));
    let f = reply_fields(ReplyKind::Reply, &s, &own());
    assert!(f.body.len() < 40_000);
    assert!(f.body.ends_with("> [...]\n"));
}

#[test]
fn a_message_id_that_is_not_plain_is_not_used_for_the_thread() {
    let mut s = source();
    s.message_id = Some("<a b>".into());
    s.references = vec!["junk".into()];
    let f = reply_fields(ReplyKind::Reply, &s, &own());
    assert_eq!(f.in_reply_to, None);
    assert!(f.references.is_empty());
}

#[test]
fn a_draft_copy_may_have_no_recipient_and_leaves_out_an_address_still_being_typed() {
    let half = [addr(None, "bob@"), addr(None, "ok@example.org")];
    let draft = build("x", |m| {
        m.to = &half;
        m.as_draft = true;
    });
    assert_eq!(header(&draft, "To").unwrap(), "ok@example.org");
    let none = build("x", |m| {
        m.to = &[];
        m.as_draft = true;
    });
    assert!(header(&none, "To").is_none());
    // Only the sender must be valid; a header can never be added through a recipient.
    let evil = [addr(None, "a@example.org\r\nBcc: v@example.org")];
    let text = build("x", |m| {
        m.to = &evil;
        m.as_draft = true;
    });
    assert!(headers(&text).iter().all(|(n, _)| n != "Bcc"));
    assert!(header(&text, "To").is_none());
}

#[test]
fn a_header_is_read_unfolded_from_a_block_or_a_message() {
    let raw =
        "From: a@x\r\nTo: One <one@x>,\r\n Two <two@x>\r\nCC: c@x\r\n\r\nTo: not a header\r\n";
    assert_eq!(header_value(raw, "to").unwrap(), "One <one@x>, Two <two@x>");
    assert_eq!(header_value(raw, "Cc").unwrap(), "c@x");
    assert!(header_value(raw, "Subject").is_none());
    assert_eq!(header_value("To: a@x\nCc: b@x\n", "cc").unwrap(), "b@x");
}
