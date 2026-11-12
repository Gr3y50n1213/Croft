// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Fuzz-style test: truncated and mutated copies of every fixture go through `render` (both
//! modes) and `part_content`, and nothing may panic. Deterministic (a fixed-seed generator), so a
//! failure is reproducible; the case number is in the panic message.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use croft_reader::{part_content, render, sanitize, sanitize_html};
use std::collections::HashMap;

/// The tags of sanitiser output: (name, [(attribute, value)]). The output is html5ever's
/// serialisation, where every literal `<` opens a tag (text has `&lt;`) and every attribute value
/// is in double quotes (with `"` escaped), so this small reader is exact for it.
fn tags(html: &str) -> Vec<(String, Vec<(String, String)>)> {
    let mut out = Vec::new();
    let b = html.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] != b'<' {
            i += 1;
            continue;
        }
        i += 1;
        let closing = b.get(i) == Some(&b'/');
        if closing {
            i += 1;
        }
        let start = i;
        while i < b.len() && !b[i].is_ascii_whitespace() && b[i] != b'>' && b[i] != b'/' {
            i += 1;
        }
        let name = html[start..i].to_ascii_lowercase();
        let mut attrs = Vec::new();
        loop {
            while i < b.len() && (b[i].is_ascii_whitespace() || b[i] == b'/') {
                i += 1;
            }
            if i >= b.len() || b[i] == b'>' {
                i += 1;
                break;
            }
            let a = i;
            while i < b.len() && b[i] != b'=' && b[i] != b'>' && !b[i].is_ascii_whitespace() {
                i += 1;
            }
            let attr = html[a..i].to_ascii_lowercase();
            let mut value = String::new();
            if i < b.len() && b[i] == b'=' {
                i += 2; // `="`
                let v = i;
                while i < b.len() && b[i] != b'"' {
                    i += 1;
                }
                value = html[v.min(b.len())..i.min(b.len())].to_owned();
                i += 1;
            }
            attrs.push((attr, value));
        }
        if !closing {
            out.push((name, attrs));
        }
    }
    out
}

/// Every tag is allowed, no attribute is an event handler, and no URL runs script.
fn assert_clean_output(case: &str, html: &str) {
    for (name, attrs) in tags(html) {
        assert!(
            sanitize::TAGS.contains(&name.as_str()),
            "{case}: tag {name} in {html}"
        );
        for (attr, value) in attrs {
            assert!(!attr.starts_with("on"), "{case}: {attr} in {html}");
            let v = value.to_ascii_lowercase();
            assert!(
                !v.contains("javascript:") && !v.contains("vbscript:"),
                "{case}: {attr}={value} in {html}"
            );
            if attr == "style" {
                assert!(!v.contains("url("), "{case}: {value} in {html}");
            }
        }
    }
}

/// xorshift64*: small, deterministic, good enough to pick positions and bytes.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn below(&mut self, n: usize) -> usize {
        if n == 0 {
            0
        } else {
            (self.next() % n as u64) as usize
        }
    }
}

/// Bytes that matter to MIME and HTML parsers, to splice in.
const SPICE: &[&[u8]] = &[
    b"\r\n",
    b"\r\n\r\n",
    b"--",
    b"--ALT",
    b"--MIX--",
    b"Content-Type: multipart/mixed; boundary=\"Z\"\r\n",
    b"Content-Transfer-Encoding: base64\r\n",
    b"Content-Transfer-Encoding: quoted-printable\r\n",
    b"charset=\"utf-16\"",
    b"=?utf-8?B?",
    b"=?x?Q?=",
    b"=\r\n",
    b"=FF",
    b"\xff\xfe\x00",
    b"<",
    b">",
    b"<img src=\"cid:",
    b"<svg><script>",
    b"</",
    b"<!--",
    b"\"",
    b"&#x",
    b"message/rfc822\r\n\r\n",
    b"\x00",
];

fn mutate(rng: &mut Rng, input: &[u8]) -> Vec<u8> {
    let mut data = input.to_vec();
    for _ in 0..(1 + rng.below(6)) {
        let at = rng.below(data.len() + 1);
        match rng.below(5) {
            0 if !data.is_empty() => {
                let i = rng.below(data.len());
                data[i] = rng.next() as u8;
            }
            1 => {
                let spice = SPICE[rng.below(SPICE.len())];
                data.splice(at..at, spice.iter().copied());
            }
            2 if at < data.len() => {
                let end = (at + 1 + rng.below(64)).min(data.len());
                data.drain(at..end);
            }
            3 if !data.is_empty() => {
                let start = rng.below(data.len());
                let end = (start + 1 + rng.below(256)).min(data.len());
                let copy = data[start..end].to_vec();
                data.splice(at..at, copy);
            }
            _ => data.truncate(at),
        }
    }
    data
}

fn exercise(case: &str, raw: &[u8]) {
    for allowed in [false, true] {
        if let Ok(r) = render(raw, allowed) {
            for a in &r.attachments {
                let _ = part_content(raw, &a.part_id);
            }
            // Whatever came out is sanitised: nothing executable slips through a damaged parse.
            if let Some(html) = &r.html {
                assert_clean_output(case, html);
            }
        }
    }
    let _ = part_content(raw, "0");
}

fn fixtures() -> Vec<(String, Vec<u8>)> {
    let dir = format!("{}/tests/fixtures", env!("CARGO_MANIFEST_DIR"));
    let mut out: Vec<(String, Vec<u8>)> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "eml"))
        .map(|p| {
            (
                p.file_name().unwrap().to_string_lossy().into_owned(),
                std::fs::read(&p).unwrap(),
            )
        })
        .collect();
    out.sort();
    assert!(out.len() >= 6, "fixtures missing");
    out
}

#[test]
fn the_output_reader_sees_tags_and_attributes() {
    let t = tags(r#"<p title="a &gt; b">x &lt;img onerror&gt;</p><img src="data:x" alt="">"#);
    assert_eq!(t.len(), 2);
    assert_eq!(t[0].1, [("title".to_owned(), "a &gt; b".to_owned())]);
    assert_eq!(t[1].0, "img");
    assert_eq!(t[1].1.len(), 2);
}

#[test]
fn every_truncation_of_every_fixture_is_handled() {
    for (name, raw) in fixtures() {
        for cut in 0..=raw.len() {
            exercise(&format!("{name} cut at {cut}"), &raw[..cut]);
        }
    }
}

#[test]
fn mutated_fixtures_never_panic() {
    let mut rng = Rng(0x00C0_FFEE_D00D_F00D);
    for (name, raw) in fixtures() {
        for case in 0..300 {
            let data = mutate(&mut rng, &raw);
            exercise(&format!("{name} mutation {case}"), &data);
        }
    }
}

#[test]
fn random_html_never_panics_the_sanitiser() {
    let mut rng = Rng(0x5EED_0000_1234_5678);
    let seed = br#"<div><a href="https://x.example/">a</a><img src="cid:a" width=1><table><tr><td style="color:red;background:url(x)">t</td></tr></table><svg><style><img src=x onerror=alert(1)></style></svg></div>"#;
    for case in 0..2000 {
        let data = mutate(&mut rng, seed);
        let html = String::from_utf8_lossy(&data);
        for allowed in [false, true] {
            let out = sanitize_html(&html, allowed, &HashMap::new());
            assert_clean_output(&format!("case {case}: {html}"), &out.html);
        }
    }
}
