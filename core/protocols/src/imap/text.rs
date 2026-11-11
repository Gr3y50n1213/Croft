// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Small pure helpers for IMAP text: folder-name encoding (RFC 3501 modified UTF-7), RFC 2047
//! encoded words, UID sets, quoting and `References` parsing.

use crate::types::Uid;
use base64::engine::general_purpose::{GeneralPurpose, GeneralPurposeConfig, STANDARD_NO_PAD};
use base64::engine::DecodePaddingMode;
use base64::{alphabet, Engine};

/// Standard alphabet, padding optional and trailing bits ignored: what real servers send.
const LENIENT: GeneralPurpose = GeneralPurpose::new(
    &alphabet::STANDARD,
    GeneralPurposeConfig::new()
        .with_decode_padding_mode(DecodePaddingMode::Indifferent)
        .with_decode_allow_trailing_bits(true),
);

/// Encodes a folder name as IMAP modified UTF-7 (RFC 3501 section 5.1.3).
pub(crate) fn utf7_encode(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut pending: Vec<u16> = Vec::new();
    fn flush(out: &mut String, pending: &mut Vec<u16>) {
        if pending.is_empty() {
            return;
        }
        let mut bytes = Vec::with_capacity(pending.len() * 2);
        for unit in pending.drain(..) {
            bytes.extend_from_slice(&unit.to_be_bytes());
        }
        out.push('&');
        out.push_str(&STANDARD_NO_PAD.encode(bytes).replace('/', ","));
        out.push('-');
    }
    for ch in name.chars() {
        match ch {
            '&' => {
                flush(&mut out, &mut pending);
                out.push_str("&-");
            }
            '\u{20}'..='\u{7e}' => {
                flush(&mut out, &mut pending);
                out.push(ch);
            }
            _ => {
                let mut buf = [0u16; 2];
                pending.extend_from_slice(ch.encode_utf16(&mut buf));
            }
        }
    }
    flush(&mut out, &mut pending);
    out
}

/// Decodes an IMAP modified UTF-7 folder name. Malformed sequences are kept as they came, so a
/// strange name still round-trips to the server.
pub(crate) fn utf7_decode(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut rest = name;
    while let Some(start) = rest.find('&') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        let Some(end) = after.find('-') else {
            out.push_str(&rest[start..]);
            return out;
        };
        let encoded = &after[..end];
        if encoded.is_empty() {
            out.push('&');
        } else {
            let std_b64 = encoded.replace(',', "/");
            match LENIENT.decode(std_b64.as_bytes()) {
                Ok(bytes) if bytes.len() % 2 == 0 => {
                    let units: Vec<u16> = bytes
                        .chunks_exact(2)
                        .map(|c| u16::from_be_bytes([c[0], c[1]]))
                        .collect();
                    match String::from_utf16(&units) {
                        Ok(s) => out.push_str(&s),
                        Err(_) => out.push_str(&rest[start..start + end + 2]),
                    }
                }
                _ => out.push_str(&rest[start..start + end + 2]),
            }
        }
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    out
}

/// Quotes a string for an IMAP command. Returns `None` for text that cannot go in a quoted
/// string (CR, LF, NUL, or anything outside ASCII, which would need a literal).
pub(crate) fn quote(s: &str) -> Option<String> {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for ch in s.chars() {
        match ch {
            '\r' | '\n' | '\0' => return None,
            c if !c.is_ascii() => return None,
            '"' | '\\' => {
                out.push('\\');
                out.push(ch);
            }
            c => out.push(c),
        }
    }
    out.push('"');
    Some(out)
}

/// Quoted, modified-UTF-7 folder name ready for a command line.
pub(crate) fn folder_arg(name: &str) -> Option<String> {
    quote(&utf7_encode(name))
}

/// Compresses sorted UIDs into an IMAP set such as `1:3,5,7:9`, in chunks of at most `max` UIDs
/// so a huge selection never makes one huge command line.
pub(crate) fn uid_sets(uids: &[Uid], max: usize) -> Vec<String> {
    let mut sorted: Vec<Uid> = uids.to_vec();
    sorted.sort_unstable();
    sorted.dedup();
    let max = max.max(1);
    sorted.chunks(max).map(uid_set_sorted).collect()
}

fn uid_set_sorted(uids: &[Uid]) -> String {
    let mut parts: Vec<String> = Vec::new();
    let mut i = 0;
    while i < uids.len() {
        let start = uids[i];
        let mut end = start;
        while i + 1 < uids.len() && uids[i + 1] == end.saturating_add(1) && uids[i + 1] != end {
            i += 1;
            end = uids[i];
        }
        if start == end {
            parts.push(start.to_string());
        } else {
            parts.push(format!("{start}:{end}"));
        }
        i += 1;
    }
    parts.join(",")
}

/// Decodes RFC 2047 encoded words in a header value (`=?UTF-8?Q?caf=C3=A9?=`). Text that is not
/// an encoded word is kept as it is. Whitespace between two encoded words is dropped.
pub(crate) fn decode_encoded_words(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut rest = input;
    let mut last_was_word = false;
    while !rest.is_empty() {
        if let Some((decoded, used)) = parse_encoded_word(rest) {
            out.push_str(&decoded);
            rest = &rest[used..];
            last_was_word = true;
            continue;
        }
        // Whitespace run: dropped when it sits between two encoded words.
        let ws_len = rest.len() - rest.trim_start().len();
        if ws_len > 0 {
            let after = &rest[ws_len..];
            if last_was_word && parse_encoded_word(after).is_some() {
                rest = after;
                continue;
            }
            out.push_str(&rest[..ws_len]);
            rest = after;
            last_was_word = false;
            continue;
        }
        let mut chars = rest.chars();
        if let Some(c) = chars.next() {
            out.push(c);
            rest = chars.as_str();
        }
        last_was_word = false;
    }
    out
}

/// Parses one encoded word at the start of `s`. Returns the decoded text and bytes used.
fn parse_encoded_word(s: &str) -> Option<(String, usize)> {
    let body = s.strip_prefix("=?")?;
    let (charset, body) = body.split_once('?')?;
    let (encoding, body) = body.split_once('?')?;
    let end = body.find("?=")?;
    let text = &body[..end];
    if text.contains(char::is_whitespace) || charset.is_empty() || encoding.len() != 1 {
        return None;
    }
    // Language suffix (RFC 2231): `UTF-8*en`.
    let charset = charset.split('*').next().unwrap_or(charset);
    let bytes = match encoding {
        "B" | "b" => LENIENT.decode(text.as_bytes()).ok()?,
        "Q" | "q" => decode_q(text)?,
        _ => return None,
    };
    let decoded = match encoding_rs::Encoding::for_label(charset.as_bytes()) {
        Some(enc) => enc.decode_without_bom_handling(&bytes).0.into_owned(),
        None => String::from_utf8_lossy(&bytes).into_owned(),
    };
    let used = 2 + (s.len() - 2 - body.len()) + end + 2;
    Some((decoded, used))
}

fn decode_q(text: &str) -> Option<Vec<u8>> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'_' => out.push(b' '),
            b'=' => {
                let hex = bytes.get(i + 1..i + 3)?;
                let hex = std::str::from_utf8(hex).ok()?;
                out.push(u8::from_str_radix(hex, 16).ok()?);
                i += 2;
            }
            b => out.push(b),
        }
        i += 1;
    }
    Some(out)
}

/// Message ids (`<...>`) from the `References` and `In-Reply-To` headers of a raw header block,
/// References first (oldest first as written), then In-Reply-To, without duplicates.
pub(crate) fn reference_ids(raw_headers: &[u8]) -> Vec<String> {
    let text = String::from_utf8_lossy(raw_headers);
    // Unfold continuation lines.
    let unfolded = text
        .replace("\r\n", "\n")
        .replace("\n ", " ")
        .replace("\n\t", " ");
    let mut references: Vec<String> = Vec::new();
    let mut in_reply_to: Vec<String> = Vec::new();
    for line in unfolded.lines() {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        let target = match name.trim().to_ascii_lowercase().as_str() {
            "references" => &mut references,
            "in-reply-to" => &mut in_reply_to,
            _ => continue,
        };
        let mut rest = value;
        while let Some(open) = rest.find('<') {
            let Some(len) = rest[open..].find('>') else {
                break;
            };
            let id = &rest[open..open + len + 1];
            if id.len() > 2 && !id.contains(char::is_whitespace) {
                target.push(id.to_string());
            }
            rest = &rest[open + len + 1..];
        }
    }
    let mut all = references;
    for id in in_reply_to {
        if !all.contains(&id) {
            all.push(id);
        }
    }
    all
}

/// Puts angle brackets around a Message-ID if it has none.
pub(crate) fn bracketed_message_id(id: &str) -> String {
    let id = id.trim();
    if id.starts_with('<') && id.ends_with('>') {
        id.to_string()
    } else {
        format!("<{}>", id.trim_matches(|c| c == '<' || c == '>'))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utf7_matches_the_rfc_example() {
        let name = "~peter/mail/\u{53f0}\u{5317}/\u{65e5}\u{672c}\u{8a9e}";
        let enc = utf7_encode(name);
        assert_eq!(enc, "~peter/mail/&U,BTFw-/&ZeVnLIqe-");
        assert_eq!(utf7_decode(&enc), name);
    }

    #[test]
    fn utf7_handles_ampersand_ascii_and_astral() {
        assert_eq!(utf7_encode("Tom & Jerry"), "Tom &- Jerry");
        assert_eq!(utf7_decode("Tom &- Jerry"), "Tom & Jerry");
        assert_eq!(utf7_decode("INBOX"), "INBOX");
        let emoji = "Post\u{1F600}";
        assert_eq!(utf7_decode(&utf7_encode(emoji)), emoji);
        assert_eq!(utf7_decode("bad&AAAA"), "bad&AAAA");
    }

    #[test]
    fn quoting_escapes_and_refuses_line_breaks() {
        assert_eq!(quote("a\"b\\c").as_deref(), Some("\"a\\\"b\\\\c\""));
        assert_eq!(quote("a\nb"), None);
        assert_eq!(quote("caf\u{e9}"), None);
        assert_eq!(folder_arg("Caf\u{e9}").as_deref(), Some("\"Caf&AOk-\""));
    }

    #[test]
    fn uid_sets_compress_and_chunk() {
        assert_eq!(uid_sets(&[5, 1, 2, 3, 7, 8, 9, 3], 100), vec!["1:3,5,7:9"]);
        assert_eq!(uid_sets(&[1, 2, 3, 4], 2), vec!["1:2", "3:4"]);
        assert!(uid_sets(&[], 10).is_empty());
        assert_eq!(uid_sets(&[u32::MAX], 10), vec![u32::MAX.to_string()]);
    }

    #[test]
    fn encoded_words_decode() {
        assert_eq!(decode_encoded_words("=?UTF-8?Q?caf=C3=A9?="), "caf\u{e9}");
        assert_eq!(decode_encoded_words("=?utf-8?B?Y2Fmw6k=?="), "caf\u{e9}");
        assert_eq!(
            decode_encoded_words("=?ISO-8859-1?Q?Andr=E9_Pirard?="),
            "Andr\u{e9} Pirard"
        );
        assert_eq!(
            decode_encoded_words("=?UTF-8?Q?a?= =?UTF-8?Q?b?= plain"),
            "ab plain"
        );
        assert_eq!(decode_encoded_words("no words here"), "no words here");
        assert_eq!(decode_encoded_words("=?broken"), "=?broken");
    }

    #[test]
    fn reference_ids_unfold_and_merge() {
        let raw = b"References: <a@x> <b@x>\r\n <c@x>\r\nIn-Reply-To: <c@x>\r\n\r\n";
        assert_eq!(reference_ids(raw), vec!["<a@x>", "<b@x>", "<c@x>"]);
        let only_reply = b"In-Reply-To: <z@x> (comment)\r\n\r\n";
        assert_eq!(reference_ids(only_reply), vec!["<z@x>"]);
        assert!(reference_ids(b"Subject: hi\r\n\r\n").is_empty());
    }

    #[test]
    fn message_ids_get_brackets() {
        assert_eq!(bracketed_message_id("a@b"), "<a@b>");
        assert_eq!(bracketed_message_id("<a@b>"), "<a@b>");
        assert_eq!(bracketed_message_id(" <a@b> "), "<a@b>");
    }
}
