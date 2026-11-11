// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Reading a message: the core's side of the conversation screen (S20) and its blocked-content
//! bar and attachment chips (S21).
//!
//! Everything here is pure and synchronous: raw message bytes in, plain text, **sanitised** HTML
//! and attachment metadata out. Nothing logs, and no error carries message content.
//!
//! - [`render`] parses a whole RFC 5322 message ([`mail_parser`], with limits on size, parts and
//!   nesting), sanitises its HTML ([`sanitize`]) and lists its attachments.
//! - [`part_content`] gives the decoded bytes of one part, for saving an attachment.
//! - [`files`] makes attachment names safe to write to disk and marks executables.
//! - [`remote`] decides whether a message may load remote images.
//!
//! # Limits
//!
//! | What | Limit | Past it |
//! | --- | --- | --- |
//! | Message size | [`MAX_MESSAGE_BYTES`] (50 MiB, the same as the protocol layer's) | [`ParseError::TooLarge`] |
//! | MIME parts | [`MAX_PARTS`] | the rest stays unparsed, [`Rendered::truncated`] |
//! | Nesting (multiparts and attached messages) | [`MAX_DEPTH`] | kept as one opaque part |
//! | Plain text kept | [`MAX_TEXT_BYTES`] | cut, [`Rendered::truncated`] |
//! | HTML given to the sanitiser | [`MAX_HTML_BYTES`] | cut, [`Rendered::truncated`] |
//! | One inline image | [`MAX_INLINE_IMAGE_BYTES`] | not drawn (offered as an attachment) |
//! | All inline images | [`MAX_INLINE_TOTAL_BYTES`] | the rest not drawn |
//!
//! Malformed MIME never panics: the parser reports damage instead of failing, and the fuzz-style
//! test (`tests/fuzz.rs`) throws truncated and mutated messages at [`render`].

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

pub mod files;
mod parse;
pub mod remote;
pub mod sanitize;

pub use parse::{
    part_content, render, snippet, Address, Attachment, ParseError, Rendered, MAX_DEPTH,
    MAX_HTML_BYTES, MAX_INLINE_IMAGE_BYTES, MAX_INLINE_TOTAL_BYTES, MAX_MESSAGE_BYTES, MAX_PARTS,
    MAX_TEXT_BYTES,
};
pub use remote::{normalise_address, remote_allowed, RemoteImages};
pub use sanitize::{sanitize_html, InlineImage, Sanitized, BLOCKED_IMAGE_PLACEHOLDER};
