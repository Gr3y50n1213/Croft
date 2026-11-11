// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Where a line of a message ends. Pure text handling, no I/O: the SMTP adapter encodes `DATA`
//! with it, and code that only reads a message (the engine's checks before sending, the sample
//! mailbox) uses the same rule. Also re-exported as `smtp::data_lines`.

/// Splits a message into lines exactly as the SMTP adapter's `DATA` encoding does: a line ends at `CRLF`, at a
/// bare `LF` or at a bare `CR` (a `CR` followed by `LF` is one terminator, not two). The
/// terminators are not part of the lines. A text that ends in a terminator has no extra empty
/// line at the end, and a last line with no terminator is yielded as it is.
///
/// This is the one definition of "where a line ends" for what goes to the server. Code that
/// inspects the header section of a message before it is sent (for example to refuse a `Bcc:`
/// header) must split with this function: a different rule (only `LF`, say) would read as one line
/// what the server receives as two.
pub fn data_lines(mime: &[u8]) -> impl Iterator<Item = &[u8]> {
    let mut rest = mime;
    std::iter::from_fn(move || {
        if rest.is_empty() {
            return None;
        }
        match rest.iter().position(|b| matches!(b, b'\r' | b'\n')) {
            None => Some(std::mem::take(&mut rest)),
            Some(i) => {
                let line = &rest[..i];
                let crlf = rest[i] == b'\r' && rest.get(i + 1) == Some(&b'\n');
                rest = &rest[i + if crlf { 2 } else { 1 }..];
                Some(line)
            }
        }
    })
}
