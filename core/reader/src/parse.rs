// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Parsing a fetched message into what the conversation screen shows.

use std::collections::HashMap;
use std::fmt;

use mail_parser::{Message, MessageParser, MessagePart, MessageRef, PartFlags, PartKind};

use crate::files;
use crate::sanitize::{self, InlineImage};

/// Largest message accepted, in bytes (50 MiB). The same as `croft_protocols::MAX_MESSAGE_BYTES`,
/// which stops bigger messages before they are downloaded (a test in the engine checks the two
/// agree).
pub const MAX_MESSAGE_BYTES: u64 = 50 * 1024 * 1024;
/// Most MIME parts parsed in one message (all levels together). Past it the rest of the message
/// stays one unparsed part.
pub const MAX_PARTS: usize = 500;
/// Most levels of multiparts and attached messages, the message itself included. A part deeper
/// than this is kept as one opaque part.
pub const MAX_DEPTH: usize = 16;
/// Most levels of attached messages that are themselves base64 or quoted-printable encoded and
/// get decoded and parsed.
const MAX_ENCODED_NESTING: usize = 2;
/// Most plain text kept for one message, in bytes (2 MiB). Longer text is cut.
pub const MAX_TEXT_BYTES: usize = 2 * 1024 * 1024;
/// Most HTML given to the sanitiser, in bytes (4 MiB). Longer HTML is cut first (the HTML parser
/// closes whatever the cut left open).
pub const MAX_HTML_BYTES: usize = 4 * 1024 * 1024;
/// Largest inline image drawn inside the message, in bytes (5 MiB). A bigger one is not drawn
/// and stays in the attachment list.
pub const MAX_INLINE_IMAGE_BYTES: usize = 5 * 1024 * 1024;
/// Most bytes of inline images drawn inside one message (15 MiB).
pub const MAX_INLINE_TOTAL_BYTES: usize = 15 * 1024 * 1024;
/// Most bytes of the header block kept (for "Show original" headers and search).
const MAX_HEADER_BYTES: usize = 64 * 1024;
/// Characters in a list preview.
const SNIPPET_CHARS: usize = 200;
/// Most recipients kept per header (To, Cc).
const MAX_ADDRESSES: usize = 500;

/// Image types drawn inline from `cid:` references, as `data:` URIs. SVG is not on the list: it is
/// a document format that can carry script and links.
const INLINE_IMAGE_TYPES: &[&str] = &[
    "image/png",
    "image/jpeg",
    "image/gif",
    "image/webp",
    "image/bmp",
];

/// Why a message could not be read. Fixed wording: never message content.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParseError {
    /// The message is larger than [`MAX_MESSAGE_BYTES`].
    TooLarge {
        /// Its size in bytes.
        size: u64,
    },
    /// The bytes are not a message (no header and no blank line).
    NotAMessage,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooLarge { size } => write!(f, "the message is too large ({size} bytes)"),
            Self::NotAMessage => f.write_str("the data is not an email message"),
        }
    }
}

impl std::error::Error for ParseError {}

/// One mailbox of an address header. `Debug` does not print it.
#[derive(Clone, PartialEq, Eq, Default)]
pub struct Address {
    /// Display name, decoded.
    pub name: Option<String>,
    /// The address itself.
    pub address: Option<String>,
}

impl fmt::Debug for Address {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Address { .. }")
    }
}

/// One attachment of a message (a part offered for download), as the chips show it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attachment {
    /// The part's number in the message (document order, every level counted): what
    /// [`part_content`] takes. Stable for the same bytes.
    pub part_id: String,
    /// A file name safe to show and to write ([`files::safe_file_name`]): never a path, never
    /// empty.
    pub file_name: String,
    /// The MIME type, lower case (`application/octet-stream` when the part names none).
    pub mime_type: String,
    /// Decoded size in bytes.
    pub size: u64,
    /// The `Content-ID` without angle brackets, if the part has one.
    pub content_id: Option<String>,
    /// True when the part is drawn inside the HTML body (a `cid:` image the sanitiser resolved).
    /// The screen shows chips for the others.
    pub shown_inline: bool,
    /// True for a program or script ([`files::is_executable`]): shown as blocked, never opened or
    /// saved.
    pub executable: bool,
}

/// A parsed, sanitised message. `Debug` prints sizes and counts only.
#[derive(Clone, PartialEq, Eq, Default)]
pub struct Rendered {
    /// Subject, decoded.
    pub subject: Option<String>,
    /// `From`.
    pub from: Vec<Address>,
    /// `To`.
    pub to: Vec<Address>,
    /// `Cc`.
    pub cc: Vec<Address>,
    /// `Reply-To`.
    pub reply_to: Vec<Address>,
    /// `Date`, Unix milliseconds.
    pub date_ms: Option<i64>,
    /// `Message-ID` with angle brackets.
    pub message_id: Option<String>,
    /// `References` then `In-Reply-To`, with angle brackets, without duplicates.
    pub references: Vec<String>,
    /// The message's header block as text (at most 64 KiB).
    pub headers_raw: String,
    /// The plain text body (HTML-only messages are converted), at most [`MAX_TEXT_BYTES`].
    pub text: Option<String>,
    /// The **sanitised** HTML body, when the message has an HTML part. Safe to give to a
    /// renderer with scripts off (see [`crate::sanitize`]).
    pub html: Option<String>,
    /// Remote images left out because remote content is blocked (0 when it was allowed).
    pub blocked_remote: u32,
    /// Tracking pixels removed (in every mode).
    pub trackers_removed: u32,
    /// The attachments, in document order.
    pub attachments: Vec<Attachment>,
    /// True when something was cut: a body over its limit, or more parts than [`MAX_PARTS`].
    pub truncated: bool,
}

impl fmt::Debug for Rendered {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Rendered")
            .field("text_bytes", &self.text.as_ref().map(String::len))
            .field("html_bytes", &self.html.as_ref().map(String::len))
            .field("blocked_remote", &self.blocked_remote)
            .field("trackers_removed", &self.trackers_removed)
            .field("attachments", &self.attachments.len())
            .field("truncated", &self.truncated)
            .finish_non_exhaustive()
    }
}

fn parser() -> MessageParser {
    MessageParser::new()
        .max_depth(MAX_DEPTH)
        .max_parts(MAX_PARTS)
        .max_encoded_nesting(MAX_ENCODED_NESTING)
}

fn parse_checked(raw: &[u8]) -> Result<Message<'_>, ParseError> {
    let size = raw.len() as u64;
    if size > MAX_MESSAGE_BYTES {
        return Err(ParseError::TooLarge { size });
    }
    parser().parse(raw).ok_or(ParseError::NotAMessage)
}

/// Parses `raw` and sanitises its HTML. With `remote_allowed` false (the default for every
/// message, see [`crate::remote`]) remote images are replaced by a placeholder and counted in
/// [`Rendered::blocked_remote`]; with it true they are kept (`https` only). Everything else the
/// sanitiser removes is removed in both modes.
pub fn render(raw: &[u8], remote_allowed: bool) -> Result<Rendered, ParseError> {
    let message = parse_checked(raw)?;
    let root = message.root();
    let mut out = Rendered {
        subject: root.subject().map(str::to_owned),
        from: addresses(root.from()),
        to: addresses(root.to()),
        cc: addresses(root.cc()),
        reply_to: addresses(root.reply_to()),
        date_ms: root.date().map(|d| d.to_timestamp().saturating_mul(1000)),
        message_id: root.message_id().map(bracket),
        references: references(&root),
        headers_raw: header_text(root.root_part().raw_headers()),
        ..Rendered::default()
    };
    out.truncated = message
        .parts()
        .any(|p| p.flags().contains(PartFlags::LIMIT_REACHED));

    // Plain text: every part of the text body, HTML parts converted to text.
    let mut text = String::new();
    for part in root.text_body() {
        let room = MAX_TEXT_BYTES.saturating_sub(text.len());
        if room == 0 {
            out.truncated = true;
            break;
        }
        let piece = match part.kind() {
            PartKind::Text => part.text_prefix_bytes(room).map(|t| t.into_owned()),
            PartKind::Html => part
                .text_prefix_bytes(MAX_HTML_BYTES)
                .map(|h| mail_parser::html_to_text(&h)),
            _ => None,
        };
        if let Some(piece) = piece {
            if !text.is_empty() {
                text.push('\n');
            }
            let cut = piece.floor_char_boundary(room);
            out.truncated |= cut < piece.len() || part_longer_than(&part, piece.len(), room);
            text.push_str(&piece[..cut]);
        }
    }
    if !text.trim().is_empty() {
        out.text = Some(text);
    }

    // HTML: only when the message has a real HTML part (a text-only message is shown as text).
    let has_html = root.html_body().any(|p| matches!(p.kind(), PartKind::Html));
    let mut shown = Vec::new();
    if has_html {
        let mut html = String::new();
        for part in root.html_body() {
            let room = MAX_HTML_BYTES.saturating_sub(html.len());
            if room == 0 {
                out.truncated = true;
                break;
            }
            let piece = match part.kind() {
                PartKind::Html => part.text_prefix_bytes(room).map(|h| h.into_owned()),
                PartKind::Text => part
                    .text_prefix_bytes(room)
                    .map(|t| mail_parser::text_to_html(&t)),
                _ => None,
            };
            if let Some(piece) = piece {
                let cut = piece.floor_char_boundary(room);
                out.truncated |= cut < piece.len() || part_longer_than(&part, piece.len(), room);
                html.push_str(&piece[..cut]);
            }
        }
        let images = inline_images(&message);
        let clean = sanitize::sanitize_html(&html, remote_allowed, &images);
        out.blocked_remote = clean.blocked_remote;
        out.trackers_removed = clean.trackers_removed;
        shown = clean.cids_used;
        if clean.too_complex {
            // Shown as text: the plain text body, else the HTML converted to text.
            out.truncated = true;
            if out.text.is_none() {
                let text = mail_parser::html_to_text(&html);
                let cut = text.floor_char_boundary(MAX_TEXT_BYTES);
                out.text = Some(text[..cut].to_owned());
            }
        } else {
            out.html = Some(clean.html);
        }
    }

    for part in root.attachments() {
        let mime_type = mime_type_of(&part);
        let content_id = part.content_id().map(|c| c.trim().to_owned());
        let name = part.attachment_name().or_else(|| match part.kind() {
            PartKind::Message(m) => m.subject(),
            _ => None,
        });
        let file_name = files::safe_file_name(name, &mime_type);
        let shown_inline = content_id
            .as_deref()
            .is_some_and(|c| shown.iter().any(|s| s == c));
        out.attachments.push(Attachment {
            part_id: part.id().to_string(),
            executable: files::is_executable(&file_name, &mime_type),
            file_name,
            size: part.decoded_len() as u64,
            content_id,
            mime_type,
            shown_inline,
        });
    }
    Ok(out)
}

/// The decoded bytes of part `part_id` (an [`Attachment::part_id`]) of `raw`, for saving an
/// attachment. `Ok(None)` when the message has no such part. An attached message is returned
/// whole (an `.eml` file).
pub fn part_content(raw: &[u8], part_id: &str) -> Result<Option<Vec<u8>>, ParseError> {
    let message = parse_checked(raw)?;
    let Ok(id) = part_id.parse::<u32>() else {
        return Ok(None);
    };
    Ok(message.part(id).map(|p| p.decoded().into_owned()))
}

/// A one-line preview of a body text: whitespace collapsed, at most 200 characters.
pub fn snippet(text: &str) -> String {
    let mut out = String::new();
    for word in text.split_whitespace() {
        if out.chars().count() >= SNIPPET_CHARS {
            break;
        }
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(word);
    }
    out.chars().take(SNIPPET_CHARS).collect()
}

/// True when the part did not fit in `room`: its decoded body is longer, or its text (after the
/// charset conversion, which can grow it) filled all of the room.
fn part_longer_than(part: &MessagePart<'_>, piece_len: usize, room: usize) -> bool {
    piece_len >= room || part.decoded_len() > room
}

fn bracket(id: &str) -> String {
    let id = id.trim().trim_start_matches('<').trim_end_matches('>');
    format!("<{id}>")
}

fn references(root: &MessageRef<'_>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let lists = [root.references(), root.in_reply_to()];
    for id in lists.iter().flatten().flat_map(|l| l.iter()) {
        if id.trim().is_empty() {
            continue;
        }
        let id = bracket(id);
        if !out.contains(&id) {
            out.push(id);
        }
    }
    out
}

fn addresses(list: Option<mail_parser::AddressList<'_>>) -> Vec<Address> {
    let Some(list) = list else {
        return Vec::new();
    };
    list.mailboxes()
        .take(MAX_ADDRESSES)
        .map(|m| Address {
            name: m
                .name()
                .map(str::trim)
                .filter(|n| !n.is_empty())
                .map(str::to_owned),
            address: m
                .address()
                .map(str::trim)
                .filter(|a| !a.is_empty())
                .map(str::to_owned),
        })
        .filter(|a| a.name.is_some() || a.address.is_some())
        .collect()
}

fn header_text(raw: &[u8]) -> String {
    let cut = raw.len().min(MAX_HEADER_BYTES);
    let text = String::from_utf8_lossy(&raw[..cut]);
    text.into_owned()
}

fn mime_type_of(part: &MessagePart<'_>) -> String {
    match part.content_type() {
        Some(ct) => match ct.subtype() {
            Some(sub) => format!("{}/{}", ct.ctype(), sub).to_ascii_lowercase(),
            None => ct.ctype().to_ascii_lowercase(),
        },
        None => match part.kind() {
            PartKind::Message(_) => "message/rfc822".to_owned(),
            PartKind::Text => "text/plain".to_owned(),
            PartKind::Html => "text/html".to_owned(),
            _ => "application/octet-stream".to_owned(),
        },
    }
}

/// The images of the message itself (not of attached messages) that a `cid:` reference may
/// draw: an allowed type, a `Content-ID`, within the size limits. Keyed by Content-ID without
/// angle brackets.
fn inline_images(message: &Message<'_>) -> HashMap<String, InlineImage> {
    let mut out = HashMap::new();
    let mut total = 0usize;
    for part in message.parts() {
        if part.message().id() != 0 || part.is_multipart() {
            continue;
        }
        let Some(cid) = part.content_id().map(str::trim).filter(|c| !c.is_empty()) else {
            continue;
        };
        let mime_type = mime_type_of(&part);
        if !INLINE_IMAGE_TYPES.contains(&mime_type.as_str()) {
            continue;
        }
        let size = part.decoded_len();
        if size == 0
            || size > MAX_INLINE_IMAGE_BYTES
            || total.saturating_add(size) > MAX_INLINE_TOTAL_BYTES
        {
            continue;
        }
        total += size;
        out.entry(cid.to_owned()).or_insert_with(|| InlineImage {
            mime_type,
            data: part.decoded().into_owned(),
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snippets_collapse_whitespace_and_stop_at_200_characters() {
        assert_eq!(
            snippet("  Hello\r\n\r\n  there\tfriend "),
            "Hello there friend"
        );
        let long = "word ".repeat(100);
        assert_eq!(snippet(&long).chars().count(), 200);
        assert_eq!(snippet(""), "");
    }

    #[test]
    fn errors_name_no_content() {
        assert_eq!(
            ParseError::TooLarge { size: 7 }.to_string(),
            "the message is too large (7 bytes)"
        );
        assert_eq!(
            ParseError::NotAMessage.to_string(),
            "the data is not an email message"
        );
    }

    #[test]
    fn oversized_input_is_refused_before_parsing() {
        let raw = vec![b'a'; usize::try_from(MAX_MESSAGE_BYTES).unwrap() + 1];
        assert!(matches!(
            render(&raw, false),
            Err(ParseError::TooLarge { .. })
        ));
        assert!(matches!(
            part_content(&raw, "1"),
            Err(ParseError::TooLarge { .. })
        ));
    }
}
