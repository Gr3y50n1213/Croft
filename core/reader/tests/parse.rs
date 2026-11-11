// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! `render` and `part_content` over the fixtures in `tests/fixtures` (plain, multipart/alternative,
//! inline `cid:` image, nested attachments, broken charsets, truncated MIME) and over generated
//! messages (a huge part, too many parts, too deep a nesting).
#![allow(clippy::unwrap_used, clippy::expect_used)]

use croft_reader::*;

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(format!(
        "{}/tests/fixtures/{name}",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}

#[test]
fn a_plain_message_gives_text_headers_and_no_html() {
    let r = render(&fixture("plain.eml"), false).unwrap();
    assert_eq!(r.subject.as_deref(), Some("Café on Friday"));
    assert_eq!(r.from[0].name.as_deref(), Some("Ana María"));
    assert_eq!(r.from[0].address.as_deref(), Some("ana@example.org"));
    let to: Vec<_> = r.to.iter().filter_map(|a| a.address.as_deref()).collect();
    assert_eq!(to, ["ben@example.com", "carla@example.com"]);
    assert_eq!(r.cc[0].address.as_deref(), Some("dee@example.net"));
    assert_eq!(r.date_ms, Some(1_772_443_800_000));
    assert_eq!(r.message_id.as_deref(), Some("<plain-1@example.org>"));
    assert_eq!(r.references, ["<root-0@example.org>"]);
    assert!(r
        .text
        .as_deref()
        .unwrap()
        .contains("See you at the café on Friday."));
    assert!(r.html.is_none());
    assert!(r.attachments.is_empty());
    assert!(!r.truncated);
    assert!(r.headers_raw.contains("Message-ID: <plain-1@example.org>"));
    assert_eq!(
        snippet(r.text.as_deref().unwrap()),
        "Hello Ben, See you at the café on Friday. Ana"
    );
}

#[test]
fn multipart_alternative_gives_both_bodies_with_remote_content_blocked() {
    let raw = fixture("alternative.eml");
    let r = render(&raw, false).unwrap();
    assert_eq!(
        r.text.as_deref().unwrap().trim(),
        "Weekly news in plain text."
    );
    let html = r.html.as_deref().unwrap();
    assert!(html.contains("Weekly news"));
    // The two real images are blocked and counted; the pixel is removed and counted apart.
    assert_eq!(r.blocked_remote, 2, "{html}");
    assert_eq!(r.trackers_removed, 1, "{html}");
    assert!(!html.contains("cdn.example.org"), "{html}");
    assert!(!html.contains("t.example"), "{html}");
    for gone in [
        "<script", "alert", "<iframe", "onload", "onclick", "<meta", "<base", "<style", "url(",
        "target",
    ] {
        assert!(!html.to_lowercase().contains(gone), "{gone} in {html}");
    }
    // The link keeps its real destination.
    assert!(
        html.contains(r#"href="https://example.org/story?id=7""#),
        "{html}"
    );
    assert!(html.contains(r#"rel="noopener noreferrer""#), "{html}");
    assert!(
        html.contains("color:#336699") || html.contains("color:#369"),
        "{html}"
    );

    let allowed = render(&raw, true).unwrap();
    let html = allowed.html.as_deref().unwrap();
    assert_eq!(allowed.blocked_remote, 0);
    assert_eq!(allowed.trackers_removed, 1);
    assert!(
        html.contains(r#"src="https://cdn.example.org/photo.jpg""#),
        "{html}"
    );
    // Plain http is upgraded, never fetched in clear text.
    assert!(
        html.contains(r#"src="https://cdn.example.org/old.jpg""#),
        "{html}"
    );
    assert!(!html.contains("http://"), "{html}");
    assert!(
        !html.contains("t.example"),
        "trackers and CSS images stay out: {html}"
    );
}

#[test]
fn inline_cid_images_are_drawn_from_the_message_and_svg_is_not() {
    let r = render(&fixture("inline_cid.eml"), false).unwrap();
    let html = r.html.as_deref().unwrap();
    assert!(
        html.contains(r#"src="data:image/png;base64,iVBORw0KGgo"#),
        "{html}"
    );
    // An unknown Content-ID and an SVG part are not drawn.
    assert!(!html.contains("cid:"), "{html}");
    assert!(!html.contains("svg"), "{html}");
    assert_eq!(r.blocked_remote, 0);
    let logo = r
        .attachments
        .iter()
        .find(|a| a.content_id.as_deref() == Some("logo@example.org"));
    if let Some(logo) = logo {
        assert!(logo.shown_inline, "a drawn image needs no chip");
    }
    // The SVG, if listed, is a plain attachment chip.
    for a in &r.attachments {
        if a.mime_type == "image/svg+xml" {
            assert!(!a.shown_inline);
        }
    }
}

#[test]
fn nested_attachments_are_listed_with_safe_names_and_executables_marked() {
    let raw = fixture("nested_attachments.eml");
    let r = render(&raw, false).unwrap();
    assert!(r.text.as_deref().unwrap().contains("Here are the files."));
    assert!(r.html.as_deref().unwrap().contains("<b>files</b>"));
    let names: Vec<&str> = r.attachments.iter().map(|a| a.file_name.as_str()).collect();
    assert_eq!(
        names,
        [
            "report.pdf",
            "evil.sh",
            "invoice.pdf.exe",
            "Forwarded inner"
        ],
        "{:?}",
        r.attachments
    );
    let by_name = |n: &str| r.attachments.iter().find(|a| a.file_name == n).unwrap();
    let pdf = by_name("report.pdf");
    assert_eq!(pdf.mime_type, "application/pdf");
    assert!(!pdf.executable && !pdf.shown_inline);
    let content = part_content(&raw, &pdf.part_id).unwrap().unwrap();
    assert_eq!(content, b"%PDF-1.4 Croft test attachment, made up.\n");
    assert_eq!(pdf.size, content.len() as u64);
    assert!(by_name("evil.sh").executable);
    assert!(by_name("invoice.pdf.exe").executable);
    // The attached message is one attachment (its own attachment is not listed at the top), and
    // its content is the whole message.
    let inner = by_name("Forwarded inner");
    assert_eq!(inner.mime_type, "message/rfc822");
    let eml = part_content(&raw, &inner.part_id).unwrap().unwrap();
    assert!(String::from_utf8_lossy(&eml).contains("Subject: Forwarded inner"));
    assert!(!names.contains(&"inner.txt"));
    assert_eq!(part_content(&raw, "9999").unwrap(), None);
    assert_eq!(part_content(&raw, "not-a-number").unwrap(), None);
}

#[test]
fn broken_and_unknown_charsets_still_give_text() {
    let r = render(&fixture("broken_charset.eml"), false).unwrap();
    let text = r.text.unwrap();
    assert!(text.contains("Unknown"), "{text}");
    assert!(text.contains("日本語"), "Shift_JIS decodes: {text}");
    assert!(r.subject.unwrap().contains("Hello"));
}

#[test]
fn truncated_mime_still_renders_what_arrived() {
    let raw = fixture("truncated.eml");
    let r = render(&raw, false).unwrap();
    let html = r.html.unwrap();
    assert!(html.contains("Start of a"), "{html}");
    assert!(!html.contains("t.example"), "{html}");
    for a in &r.attachments {
        let _ = part_content(&raw, &a.part_id).unwrap();
    }
}

#[test]
fn a_huge_part_is_cut_at_the_limits() {
    let text_body = "line of text\r\n".repeat(MAX_TEXT_BYTES / 10);
    let raw = format!(
        "From: x@example.org\r\nSubject: Big\r\nContent-Type: text/plain\r\n\r\n{text_body}"
    );
    let r = render(raw.as_bytes(), false).unwrap();
    assert!(r.truncated);
    assert!(r.text.unwrap().len() <= MAX_TEXT_BYTES);

    let paragraph = format!("<p>{}</p>\r\n", "paragraph é ".repeat(100));
    let html_body = paragraph.repeat(MAX_HTML_BYTES / paragraph.len() + 100);
    let raw = format!(
        "From: x@example.org\r\nSubject: Big\r\nContent-Type: text/html; charset=utf-8\r\n\r\n{html_body}"
    );
    let r = render(raw.as_bytes(), false).unwrap();
    assert!(r.truncated);
    assert!(r.html.unwrap().len() <= MAX_HTML_BYTES + 1024);

    // HTML too deep to parse in reasonable time is shown as text.
    let deep = "<div>".repeat(5_000) + "deep words";
    let raw =
        format!("From: x@example.org\r\nSubject: Deep\r\nContent-Type: text/html\r\n\r\n{deep}");
    let r = render(raw.as_bytes(), false).unwrap();
    assert!(r.html.is_none() && r.truncated);
    assert!(r.text.unwrap().contains("deep words"));
}

#[test]
fn too_many_parts_and_too_deep_nesting_are_cut_without_failing() {
    let mut many = String::from(
        "From: x@example.org\r\nSubject: Many\r\nContent-Type: multipart/mixed; boundary=M\r\n\r\n",
    );
    for n in 0..(MAX_PARTS * 3) {
        many.push_str(&format!(
            "--M\r\nContent-Type: application/octet-stream\r\nContent-Disposition: attachment; filename=\"f{n}.bin\"\r\n\r\nx\r\n"
        ));
    }
    many.push_str("--M--\r\n");
    let r = render(many.as_bytes(), false).unwrap();
    assert!(r.truncated);
    assert!(r.attachments.len() <= MAX_PARTS);

    let depth = MAX_DEPTH * 4;
    let mut deep = String::from("From: x@example.org\r\nSubject: Deep\r\n");
    for n in 0..depth {
        deep.push_str(&format!(
            "Content-Type: multipart/mixed; boundary=D{n}\r\n\r\n--D{n}\r\n"
        ));
    }
    deep.push_str("Content-Type: text/plain\r\n\r\nbottom\r\n");
    for n in (0..depth).rev() {
        deep.push_str(&format!("--D{n}--\r\n"));
    }
    let r = render(deep.as_bytes(), false).unwrap();
    assert!(r.truncated);
}

#[test]
fn bytes_that_are_not_a_message_are_refused_plainly() {
    assert_eq!(render(b"", false).unwrap_err(), ParseError::NotAMessage);
    let err = render(b"", false).unwrap_err().to_string();
    assert!(!err.contains('@'));
}
