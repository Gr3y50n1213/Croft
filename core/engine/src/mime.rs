// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Reading just enough of a stored message to apply two rules: a message to be sent has no `Bcc:`
//! header, and its Message-ID is how a copy in Sent is recognised. Nothing here parses addresses.

/// The header fields of a message as (name, unfolded value) pairs, in order. Stops at the first
/// empty line. A line that starts with a space or tab continues the field before it. Lines with no
/// colon that are not continuations are skipped.
fn header_fields(mime: &[u8]) -> Vec<(String, String)> {
    let mut fields: Vec<(String, String)> = Vec::new();
    for raw in mime.split(|b| *b == b'\n') {
        let line = raw.strip_suffix(b"\r").unwrap_or(raw);
        if line.is_empty() {
            break;
        }
        let text = String::from_utf8_lossy(line);
        if text.starts_with([' ', '\t']) {
            if let Some((_, value)) = fields.last_mut() {
                if !value.is_empty() {
                    value.push(' ');
                }
                value.push_str(text.trim());
            }
        } else if let Some((name, value)) = text.split_once(':') {
            fields.push((name.trim().to_owned(), value.trim().to_owned()));
        }
    }
    fields
}

/// True when the header section has a `Bcc` field (any capitalisation, with or without a value).
pub(crate) fn has_bcc_header(mime: &[u8]) -> bool {
    header_fields(mime)
        .iter()
        .any(|(name, _)| name.eq_ignore_ascii_case("bcc"))
}

/// The value of the first `Message-ID` field, angle brackets included, or `None` when there is
/// none or it is empty.
pub(crate) fn message_id(mime: &[u8]) -> Option<String> {
    header_fields(mime)
        .into_iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("message-id"))
        .map(|(_, value)| value)
        .filter(|value| !value.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_a_bcc_header_in_any_case_and_when_folded() {
        assert!(has_bcc_header(b"From: a\r\nBcc: b\r\n\r\nbody"));
        assert!(has_bcc_header(b"From: a\nBCC:\n\nbody"));
        assert!(has_bcc_header(b"From: a\r\nbcc : b\r\n\r\n"));
        assert!(has_bcc_header(
            b"Subject: x\r\nBcc:\r\n b@example.org\r\n\r\n"
        ));
    }

    #[test]
    fn a_word_in_the_body_or_another_field_is_not_a_bcc_header() {
        assert!(!has_bcc_header(
            b"From: a\r\nSubject: Bcc: b\r\n\r\nBcc: b\r\n"
        ));
        assert!(!has_bcc_header(b"From: a\r\nX-Bcc-Note: 1\r\n\r\n"));
        // A continuation line that looks like a header belongs to the field before it.
        assert!(!has_bcc_header(b"Subject: a\r\n Bcc: b\r\n\r\n"));
        assert!(!has_bcc_header(b""));
    }

    #[test]
    fn reads_the_message_id_with_its_brackets() {
        assert_eq!(
            message_id(b"From: a\r\nMessage-ID: <x@y>\r\n\r\nMessage-ID: <z>").as_deref(),
            Some("<x@y>")
        );
        assert_eq!(
            message_id(b"message-id:\r\n <folded@y>\r\n\r\n").as_deref(),
            Some("<folded@y>")
        );
        assert_eq!(message_id(b"From: a\r\n\r\nbody"), None);
        assert_eq!(message_id(b"Message-ID:\r\n\r\n"), None);
    }
}
