// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Writing a message: addresses, the RFC 5322 text, and the fields of a reply or a forward.
//!
//! Everything here is pure (no clock, no randomness, no I/O): the caller passes the time and the
//! random bytes in, so every function is tested without a network or a database.
//!
//! # What the output is
//!
//! [`build_message`] writes **7-bit text** only: headers are ASCII (a non-ASCII subject or name is
//! an RFC 2047 encoded word), the body is `7bit` or quoted-printable, attachments are base64. The
//! one exception is an address with non-ASCII letters, which is written as UTF-8 (RFC 6532); the
//! SMTP adapter then needs `SMTPUTF8` from the server and says so if it has none. The result is
//! always valid UTF-8, which the copy for the Sent and Drafts folders needs (the IMAP adapter
//! appends text).
//!
//! # Safety of header text
//!
//! Nothing the person typed is ever put into a header as it is. A control character (a line break
//! above all) in a subject or a name becomes a space, an address with one is invalid and refused,
//! and a Message-ID, In-Reply-To or References id that is not `<...>` made of printable ASCII is
//! left out (or refused, for the message's own id). So no text can add a header line, such as a
//! `Bcc:`, to a message.
//!
//! # Bcc
//!
//! [`Outgoing::include_bcc`] is false for a message that is sent: the `Bcc:` header would show the
//! recipient to everyone else, and the engine refuses a stored message that has one. It is true
//! for the copy in the server's Drafts folder, which only the person's own mailbox holds, so that
//! reopening the draft elsewhere restores the Bcc recipients.

use base64::Engine as _;

/// An address with an optional display name.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct Address {
    /// The display name, without quotes (`Ann Example`), if there is one.
    pub name: Option<String>,
    /// The bare address (`ann@example.org`).
    pub address: String,
}

impl std::fmt::Debug for Address {
    /// Never prints the address or the name.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Address(***)")
    }
}

impl Address {
    /// An address with no display name.
    pub fn bare(address: impl Into<String>) -> Self {
        Self {
            name: None,
            address: address.into(),
        }
    }
}

/// The longest address Croft accepts (RFC 5321: 254 bytes in a path).
const MAX_ADDRESS_BYTES: usize = 254;
/// The longest local part (RFC 5321).
const MAX_LOCAL_BYTES: usize = 64;

/// True when `address` is something a server could deliver to: one `@`, a local part of dot-separated
/// words (no spaces, quotes, brackets or other specials) and a domain of dot-separated labels. A
/// single-label domain (`alice@localhost`) is allowed (private servers use them). Non-ASCII letters
/// are allowed (the server needs `SMTPUTF8` then). Comments, quoted local parts and IP-address
/// domains are not accepted: they are rare, and an address the person typed that looks like one is
/// far more often a typing mistake.
pub fn is_valid_address(address: &str) -> bool {
    if address.is_empty() || address.len() > MAX_ADDRESS_BYTES {
        return false;
    }
    let Some((local, domain)) = address.rsplit_once('@') else {
        return false;
    };
    if local.is_empty() || local.len() > MAX_LOCAL_BYTES || domain.is_empty() {
        return false;
    }
    let local_ok = local.split('.').all(|word| {
        !word.is_empty()
            && word.chars().all(|c| {
                !c.is_control()
                    && !c.is_whitespace()
                    && !matches!(
                        c,
                        '(' | ')' | '<' | '>' | '[' | ']' | ':' | ';' | '@' | '\\' | ',' | '"'
                    )
            })
    });
    let domain_ok = domain.len() <= 253
        && domain.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label
                    .chars()
                    .all(|c| c == '-' || c.is_alphanumeric() || (!c.is_ascii() && !c.is_control()))
        });
    local_ok && domain_ok
}

/// Reads one address as a person or a header writes it: `ann@example.org`, `<ann@example.org>`,
/// `Ann Example <ann@example.org>`, `"Example, Ann" <ann@example.org>`. Returns `None` when no
/// address can be found; it does **not** check that the address is valid (use
/// [`is_valid_address`]), so a chip with a typing mistake can still be shown and corrected.
pub fn parse_address(text: &str) -> Option<Address> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    if let (Some(open), true) = (text.rfind('<'), text.ends_with('>')) {
        let address = text[open + 1..text.len() - 1].trim();
        if address.is_empty() {
            return None;
        }
        let name = unquote_name(text[..open].trim());
        return Some(Address {
            name: (!name.is_empty()).then_some(name),
            address: address.to_owned(),
        });
    }
    Some(Address::bare(text))
}

/// A display name as written, without its quotes and escapes.
fn unquote_name(raw: &str) -> String {
    let raw = raw.trim();
    let inner = raw
        .strip_prefix('"')
        .and_then(|r| r.strip_suffix('"'))
        .unwrap_or(raw);
    let mut out = String::with_capacity(inner.len());
    let mut escaped = false;
    for c in inner.chars() {
        if escaped {
            out.push(c);
            escaped = false;
        } else if c == '\\' {
            escaped = true;
        } else {
            out.push(c);
        }
    }
    out.trim().to_owned()
}

/// Reads a header's address list (`To:` or `Cc:` value): items separated by commas, but not the
/// commas inside quotes or angle brackets. Items with no address are skipped. At most `limit` are
/// returned.
pub fn parse_address_list(text: &str, limit: usize) -> Vec<Address> {
    let mut items = Vec::new();
    let mut current = String::new();
    let (mut quoted, mut escaped, mut angle) = (false, false, false);
    for c in text.chars() {
        if escaped {
            escaped = false;
            current.push(c);
            continue;
        }
        match c {
            '\\' if quoted => {
                escaped = true;
                current.push(c);
            }
            '"' => {
                quoted = !quoted;
                current.push(c);
            }
            '<' if !quoted => {
                angle = true;
                current.push(c);
            }
            '>' if !quoted => {
                angle = false;
                current.push(c);
            }
            ',' | ';' if !quoted && !angle => {
                items.push(std::mem::take(&mut current));
            }
            _ => current.push(c),
        }
    }
    items.push(current);
    items
        .iter()
        .filter_map(|item| parse_address(item))
        .take(limit)
        .collect()
}

// ------------------------------------------------------------------------------------------
// Header text
// ------------------------------------------------------------------------------------------

/// A header value from person-typed text: control characters (line breaks included) become spaces,
/// and runs of spaces are kept as one.
fn clean_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut last_space = false;
    for c in text.chars() {
        let c = if c.is_control() || c == '\u{2028}' || c == '\u{2029}' {
            ' '
        } else {
            c
        };
        if c == ' ' {
            if last_space {
                continue;
            }
            last_space = true;
        } else {
            last_space = false;
        }
        out.push(c);
    }
    out.trim().to_owned()
}

/// The longest line a header is folded to, including the field name.
const FOLD_AT: usize = 76;

/// `text` as header text: unchanged when it is printable ASCII, else RFC 2047 encoded words
/// (UTF-8, base64), one per continuation line, each at most 75 characters.
fn encode_text(text: &str) -> String {
    let text = clean_text(text);
    if text.is_ascii() {
        return text;
    }
    // 45 bytes of text make 60 characters of base64: with the 12 of `=?UTF-8?B?` and `?=` that is
    // 72, inside the limit of 75.
    const CHUNK: usize = 45;
    let mut words = Vec::new();
    let mut chunk = String::new();
    for c in text.chars() {
        if chunk.len() + c.len_utf8() > CHUNK {
            words.push(std::mem::take(&mut chunk));
        }
        chunk.push(c);
    }
    if !chunk.is_empty() {
        words.push(chunk);
    }
    words
        .iter()
        .map(|w| {
            format!(
                "=?UTF-8?B?{}?=",
                base64::engine::general_purpose::STANDARD.encode(w.as_bytes())
            )
        })
        .collect::<Vec<_>>()
        .join("\r\n ")
}

/// Folds already-encoded `value` after `name: `: at spaces when the line would be longer than
/// [`FOLD_AT`], and before every encoded word that was joined with `\r\n ` by [`encode_text`]. A
/// word longer than a line stays whole (it is an encoded word or an address).
fn fold(name: &str, value: &str) -> String {
    let mut out = format!("{name}:");
    let mut len = out.len();
    let mut on_line = 0usize;
    for (i, piece) in value.split("\r\n ").enumerate() {
        let mut force = i > 0;
        for word in piece.split(' ').filter(|w| !w.is_empty()) {
            let too_long = len + 1 + word.len() > FOLD_AT;
            if force || (too_long && (on_line > 0 || word.len() < FOLD_AT)) {
                out.push_str("\r\n ");
                len = 1;
                on_line = 0;
                force = false;
            } else {
                out.push(' ');
                len += 1;
            }
            out.push_str(word);
            len += word.len();
            on_line += 1;
        }
    }
    if on_line == 0 && out.ends_with(':') {
        out.push(' ');
    }
    out
}

/// One address as a header writes it: `Name <a@b>` with the name quoted or encoded as needed.
fn format_address(a: &Address) -> String {
    let name = a.name.as_deref().map(clean_text).unwrap_or_default();
    if name.is_empty() {
        return a.address.clone();
    }
    let shown = if name.is_ascii() {
        if name.chars().all(|c| c.is_ascii_alphanumeric() || c == ' ') {
            name
        } else {
            let mut q = String::from("\"");
            for c in name.chars() {
                if c == '"' || c == '\\' {
                    q.push('\\');
                }
                q.push(c);
            }
            q.push('"');
            q
        }
    } else {
        // An encoded word is one token; keep it on one line.
        encode_text(&name).replace("\r\n ", " ")
    };
    format!("{shown} <{}>", a.address)
}

/// An address list as one header value, folded after the commas by [`fold`]'s spaces.
fn format_list(list: &[Address]) -> String {
    list.iter()
        .map(format_address)
        .collect::<Vec<_>>()
        .join(", ")
}

/// A message id (`<...>`) written to a header, if it is one: printable ASCII with no spaces and no
/// angle brackets inside. `None` for anything else, so it is never copied into a header.
pub fn clean_message_id(id: &str) -> Option<String> {
    let id = id.trim();
    let inner = id.strip_prefix('<')?.strip_suffix('>')?;
    (!inner.is_empty()
        && inner.len() <= 200
        && inner
            .bytes()
            .all(|b| (0x21..=0x7e).contains(&b) && b != b'<' && b != b'>'))
    .then(|| id.to_owned())
}

// ------------------------------------------------------------------------------------------
// Dates and ids
// ------------------------------------------------------------------------------------------

const WEEKDAYS: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

/// Year, month (1 to 12), day, hour, minute, second and weekday (0 = Monday) of a Unix time in
/// milliseconds, in UTC.
fn civil(ms: i64) -> (i64, usize, i64, i64, i64, i64, usize) {
    let secs = ms.div_euclid(1000);
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    // Howard Hinnant's days-to-civil.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    // 1970-01-01 was a Thursday (index 3 with Monday as 0).
    let weekday = usize::try_from((days + 3).rem_euclid(7)).unwrap_or(0);
    (
        y,
        usize::try_from(m - 1).unwrap_or(0),
        d,
        rem / 3600,
        rem % 3600 / 60,
        rem % 60,
        weekday,
    )
}

/// The `Date:` header value for a Unix time in milliseconds: `Wed, 11 Nov 2026 10:30:00 +0000`.
/// Always UTC, so the message does not tell where the person is.
pub fn rfc5322_date(ms: i64) -> String {
    let (y, m, d, h, mi, s, wd) = civil(ms);
    format!(
        "{}, {d:02} {} {y:04} {h:02}:{mi:02}:{s:02} +0000",
        WEEKDAYS[wd], MONTHS[m]
    )
}

/// A date for a quote's first line: `11 Nov 2026, 10:30 UTC`.
fn quote_date(ms: i64) -> String {
    let (y, m, d, h, mi, ..) = civil(ms);
    format!("{d} {} {y}, {h:02}:{mi:02} UTC", MONTHS[m])
}

/// A new Message-ID: `<` + the random bytes in hex + `@` + the domain of `from` + `>`. The domain is
/// the one the person's address has (as every mail client does), or `croft.invalid` when that is
/// not plain ASCII. The 128 random bits make it unique; the caller supplies them (from the system's
/// random source).
pub fn new_message_id(random: [u8; 16], from_address: &str) -> String {
    let domain = from_address
        .rsplit_once('@')
        .map(|(_, d)| d)
        .filter(|d| {
            !d.is_empty()
                && d.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-')
        })
        .unwrap_or("croft.invalid");
    let hex: String = random.iter().map(|b| format!("{b:02x}")).collect();
    format!("<{hex}@{domain}>")
}

// ------------------------------------------------------------------------------------------
// Building the message
// ------------------------------------------------------------------------------------------

/// A file to attach.
#[derive(Clone, Copy)]
pub struct AttachmentData<'a> {
    /// The file name shown to the recipient. Folders, control characters and quotes are removed.
    pub name: &'a str,
    /// The media type (`application/pdf`). Anything that is not a plain `type/subtype` becomes
    /// `application/octet-stream`.
    pub mime_type: &'a str,
    /// The bytes.
    pub bytes: &'a [u8],
}

impl std::fmt::Debug for AttachmentData<'_> {
    /// Prints the size only.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "AttachmentData({} bytes)", self.bytes.len())
    }
}

/// Everything [`build_message`] writes.
#[derive(Debug, Clone, Copy)]
pub struct Outgoing<'a> {
    /// The sender (an identity of the account).
    pub from: &'a Address,
    /// To.
    pub to: &'a [Address],
    /// Cc.
    pub cc: &'a [Address],
    /// Bcc. Written only when [`Outgoing::include_bcc`] is set; the caller sends to them either way.
    pub bcc: &'a [Address],
    /// The subject, as typed.
    pub subject: &'a str,
    /// The body text, as typed (any line ending).
    pub body: &'a str,
    /// The `In-Reply-To` id, if this answers a message.
    pub in_reply_to: Option<&'a str>,
    /// The `References` ids, oldest first.
    pub references: &'a [String],
    /// The message's own Message-ID, `<...>`.
    pub message_id: &'a str,
    /// The `Date`, Unix milliseconds.
    pub date_ms: i64,
    /// Whether to write the `Bcc:` header (see the module notes): false for a message that is sent.
    pub include_bcc: bool,
    /// The attachments.
    pub attachments: &'a [AttachmentData<'a>],
    /// Random bytes for the part boundary. Must differ from message to message.
    pub boundary_seed: [u8; 8],
}

/// Why a message could not be written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuildError {
    /// Nobody to send it to (To, Cc and Bcc are all empty).
    NoRecipient,
    /// An address is not valid (a recipient or the sender).
    InvalidAddress,
    /// The Message-ID is not a `<...>` of printable ASCII.
    BadMessageId,
}

/// Whether the outgoing message has anyone to deliver to.
pub fn has_recipient(to: &[Address], cc: &[Address], bcc: &[Address]) -> bool {
    !(to.is_empty() && cc.is_empty() && bcc.is_empty())
}

/// Writes the message. See the module notes for what the output looks like and what it never
/// contains. Fails when there is no recipient, when an address is invalid or when the Message-ID
/// is not usable.
pub fn build_message(m: &Outgoing<'_>) -> Result<Vec<u8>, BuildError> {
    if !has_recipient(m.to, m.cc, m.bcc) {
        return Err(BuildError::NoRecipient);
    }
    let all_valid = std::iter::once(m.from)
        .chain(m.to)
        .chain(m.cc)
        .chain(m.bcc)
        .all(|a| is_valid_address(&a.address));
    if !all_valid {
        return Err(BuildError::InvalidAddress);
    }
    let message_id = clean_message_id(m.message_id).ok_or(BuildError::BadMessageId)?;

    let mut head = String::new();
    let mut line = |name: &str, value: &str| {
        head.push_str(&fold(name, value));
        head.push_str("\r\n");
    };
    line("From", &format_address(m.from));
    if !m.to.is_empty() {
        line("To", &format_list(m.to));
    }
    if !m.cc.is_empty() {
        line("Cc", &format_list(m.cc));
    }
    if m.include_bcc && !m.bcc.is_empty() {
        line("Bcc", &format_list(m.bcc));
    }
    let subject = encode_text(m.subject);
    line("Subject", &subject);
    line("Date", &rfc5322_date(m.date_ms));
    line("Message-ID", &message_id);
    if let Some(parent) = m.in_reply_to.and_then(clean_message_id) {
        line("In-Reply-To", &parent);
    }
    let references: Vec<String> = m
        .references
        .iter()
        .filter_map(|r| clean_message_id(r))
        .collect();
    if !references.is_empty() {
        line("References", &references.join(" "));
    }
    line("MIME-Version", "1.0");

    let (text_headers, text_body) = encode_body(m.body);
    let mut out = head;
    if m.attachments.is_empty() {
        out.push_str(&text_headers);
        out.push_str("\r\n");
        out.push_str(&text_body);
        return Ok(out.into_bytes());
    }

    let boundary = boundary(m);
    out.push_str(&format!(
        "Content-Type: multipart/mixed; boundary=\"{boundary}\"\r\n\r\n"
    ));
    out.push_str("This is a multi-part message in MIME format.\r\n");
    out.push_str(&format!("--{boundary}\r\n"));
    out.push_str(&text_headers);
    out.push_str("\r\n");
    out.push_str(&text_body);
    if !text_body.ends_with("\r\n") {
        out.push_str("\r\n");
    }
    for a in m.attachments {
        out.push_str(&format!("--{boundary}\r\n"));
        out.push_str(&attachment_headers(a));
        out.push_str("\r\n");
        let encoded = base64::engine::general_purpose::STANDARD.encode(a.bytes);
        for chunk in encoded.as_bytes().chunks(76) {
            // Base64 is ASCII.
            out.push_str(&String::from_utf8_lossy(chunk));
            out.push_str("\r\n");
        }
    }
    out.push_str(&format!("--{boundary}--\r\n"));
    Ok(out.into_bytes())
}

/// A boundary that appears nowhere in the message's own text: random, and checked against the body
/// and the file names (the base64 parts cannot contain the `=_` that starts it... except in the
/// padding, which is why the check is on the whole line start).
fn boundary(m: &Outgoing<'_>) -> String {
    let seed: String = m.boundary_seed.iter().map(|b| format!("{b:02x}")).collect();
    let mut n = 0u32;
    loop {
        let candidate = format!("=_croft_{seed}{n}");
        if !m.body.contains(&candidate) && !m.subject.contains(&candidate) {
            return candidate;
        }
        n += 1;
    }
}

/// The part headers and the encoded text of the body. `7bit` when the text is ASCII with lines no
/// longer than 998 characters (RFC 5322), else quoted-printable.
fn encode_body(body: &str) -> (String, String) {
    // CRLF everywhere, whatever the input used.
    let normalised = body.replace("\r\n", "\n").replace('\r', "\n");
    let mut text = normalised.replace('\n', "\r\n");
    if !text.is_empty() && !text.ends_with("\r\n") {
        text.push_str("\r\n");
    }
    let seven_bit = text.is_ascii()
        && !text.contains('\0')
        && text.split("\r\n").all(|l| l.len() <= 998)
        && !text.lines().any(|l| l.starts_with("--=_croft_"));
    if seven_bit {
        (
            "Content-Type: text/plain; charset=utf-8\r\nContent-Transfer-Encoding: 7bit\r\n"
                .to_owned(),
            text,
        )
    } else {
        (
            "Content-Type: text/plain; charset=utf-8\r\nContent-Transfer-Encoding: quoted-printable\r\n"
                .to_owned(),
            quoted_printable(&text),
        )
    }
}

/// Quoted-printable (RFC 2045) of text that already uses CRLF: lines of at most 76 characters,
/// a space or tab at the end of a line encoded, everything outside printable ASCII as `=XX`.
pub fn quoted_printable(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + text.len() / 8);
    for (i, line) in text.split("\r\n").enumerate() {
        if i > 0 {
            out.push_str("\r\n");
        }
        let bytes = line.as_bytes();
        let mut col = 0usize;
        for (j, &b) in bytes.iter().enumerate() {
            let last = j + 1 == bytes.len();
            let piece = match b {
                b'=' => format!("={b:02X}"),
                b' ' | b'\t' if last => format!("={b:02X}"),
                0x20..=0x7e | b'\t' => (b as char).to_string(),
                _ => format!("={b:02X}"),
            };
            // Leave room for the `=` of a soft line break unless this is the last piece.
            let limit = if last { 76 } else { 75 };
            if col + piece.len() > limit {
                out.push_str("=\r\n");
                col = 0;
            }
            out.push_str(&piece);
            col += piece.len();
        }
    }
    out
}

/// A file name that is safe in a header: no folders, no control characters, no quotes.
fn safe_file_name(name: &str) -> String {
    let base = name.rsplit(['/', '\\']).next().unwrap_or("");
    let cleaned: String = clean_text(base)
        .chars()
        .filter(|c| !matches!(c, '"' | '/' | '\\' | ':'))
        .collect();
    let cleaned: String = cleaned.chars().take(120).collect();
    if cleaned.trim().is_empty() || cleaned == "." || cleaned == ".." {
        "attachment".to_owned()
    } else {
        cleaned
    }
}

/// The percent-encoding RFC 2231 wants for `filename*=UTF-8''...`.
fn percent_encode(name: &str) -> String {
    let mut out = String::new();
    for b in name.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

fn attachment_headers(a: &AttachmentData<'_>) -> String {
    let name = safe_file_name(a.name);
    let ascii: String = name
        .chars()
        .map(|c| if c.is_ascii() { c } else { '_' })
        .collect();
    let mime = clean_mime_type(a.mime_type);
    let mut filename = format!("filename=\"{ascii}\"");
    if !name.is_ascii() {
        filename.push_str(&format!("; filename*=UTF-8''{}", percent_encode(&name)));
    }
    format!(
        "Content-Type: {mime}; name=\"{ascii}\"\r\nContent-Transfer-Encoding: base64\r\nContent-Disposition: attachment; {filename}\r\n"
    )
}

/// `type/subtype` when `text` is one (letters, digits and `+.-_` only), else `application/octet-stream`.
fn clean_mime_type(text: &str) -> String {
    let t = text.trim().to_ascii_lowercase();
    let ok = t.len() <= 100
        && t.split_once('/').is_some_and(|(a, b)| {
            let part = |s: &str| {
                !s.is_empty()
                    && s.bytes().all(|c| {
                        c.is_ascii_alphanumeric() || matches!(c, b'+' | b'.' | b'-' | b'_')
                    })
            };
            part(a) && part(b)
        });
    if ok {
        t
    } else {
        "application/octet-stream".to_owned()
    }
}

// ------------------------------------------------------------------------------------------
// Reply, reply all, forward
// ------------------------------------------------------------------------------------------

/// Which draft to start from a stored message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplyKind {
    /// Answer the sender (or the `Reply-To`).
    Reply,
    /// Answer the sender and everyone else on To and Cc, except the person.
    ReplyAll,
    /// Send the message on to someone else.
    Forward,
}

/// What is known about the message being answered.
#[derive(Debug, Clone, Default)]
pub struct SourceMessage {
    /// Its Message-ID, `<...>`.
    pub message_id: Option<String>,
    /// Its subject.
    pub subject: String,
    /// The sender.
    pub from: Option<Address>,
    /// `Reply-To`, if the stored headers have one.
    pub reply_to: Vec<Address>,
    /// The To recipients, as far as they are stored.
    pub to: Vec<Address>,
    /// The Cc recipients, as far as they are stored.
    pub cc: Vec<Address>,
    /// Its date, Unix milliseconds.
    pub date_ms: i64,
    /// Its `References`, oldest first, if stored.
    pub references: Vec<String>,
    /// Its text, if the body is stored.
    pub text: Option<String>,
}

/// The fields of a draft that a reply or a forward fills in. Nothing is stored yet.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DraftFields {
    /// To.
    pub to: Vec<Address>,
    /// Cc.
    pub cc: Vec<Address>,
    /// The subject, with `Re:` or `Fwd:` once.
    pub subject: String,
    /// The body: empty lines for the person to type in, then the quote or the forwarded message.
    pub body: String,
    /// `In-Reply-To` (a reply, not a forward).
    pub in_reply_to: Option<String>,
    /// `References` (a reply, not a forward).
    pub references: Vec<String>,
    /// False when the body of the original is not stored, so the quote (or the forwarded text) is
    /// missing. A forward in this state has only the header block of the original.
    pub quote_included: bool,
}

/// The most of the original's text that is quoted (bytes; cut at a line end).
const MAX_QUOTE_BYTES: usize = 32 * 1024;

fn lower_eq(a: &str, b: &str) -> bool {
    a.trim().to_lowercase() == b.trim().to_lowercase()
}

/// `subject` with the prefix once: `Re: x` stays `Re: x` (also `RE:`, `re:`); `x` becomes `Re: x`.
/// For a forward the prefix is `Fwd:` (`Fw:` and `FWD:` are recognised and written as `Fwd:`).
pub fn prefixed_subject(subject: &str, forward: bool) -> String {
    let s = clean_text(subject);
    let lower = s.to_lowercase();
    let strip = |prefixes: &[&str]| -> Option<String> {
        prefixes.iter().find_map(|p| {
            lower
                .starts_with(p)
                .then(|| s[p.len()..].trim_start().to_owned())
        })
    };
    if forward {
        let rest = strip(&["fwd:", "fw:"]).unwrap_or(s);
        format!("Fwd: {rest}")
    } else {
        let rest = strip(&["re:"]).unwrap_or(s);
        format!("Re: {rest}")
    }
}

/// Removes later copies of the same address (compared without regard to case).
fn dedupe(list: &mut Vec<Address>) {
    let mut seen: Vec<String> = Vec::new();
    list.retain(|a| {
        let key = a.address.trim().to_lowercase();
        if seen.contains(&key) {
            false
        } else {
            seen.push(key);
            true
        }
    });
}

/// `Name <a@b>` or the bare address, for a quote's header lines.
fn shown(a: &Address) -> String {
    match a.name.as_deref().map(str::trim).filter(|n| !n.is_empty()) {
        Some(n) => format!("{n} <{}>", a.address),
        None => a.address.clone(),
    }
}

fn quoted(text: &str) -> String {
    let mut used = 0usize;
    let mut out = String::new();
    for line in text.replace("\r\n", "\n").replace('\r', "\n").lines() {
        used += line.len() + 1;
        if used > MAX_QUOTE_BYTES {
            out.push_str("> [...]\n");
            break;
        }
        if line.is_empty() {
            out.push_str(">\n");
        } else {
            out.push_str("> ");
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

/// The draft fields for a reply, a reply to all, or a forward of `source`. `own` lists the
/// person's own addresses (the account's and every identity's): they are never put on To or Cc of
/// a reply to all. Replying to a message the person wrote answers the people it was sent to.
pub fn reply_fields(kind: ReplyKind, source: &SourceMessage, own: &[String]) -> DraftFields {
    let is_own = |a: &Address| own.iter().any(|o| lower_eq(o, &a.address));
    let mut fields = DraftFields {
        subject: prefixed_subject(&source.subject, kind == ReplyKind::Forward),
        ..DraftFields::default()
    };

    if kind != ReplyKind::Forward {
        // Who the answer goes to first: Reply-To if there is one, else the sender.
        let target: Vec<Address> = if source.reply_to.is_empty() {
            source.from.iter().cloned().collect()
        } else {
            source.reply_to.clone()
        };
        let from_me = source.from.as_ref().is_some_and(is_own);
        let (mut to, mut cc) = match (kind, from_me) {
            // Answering one's own message: the people it went to.
            (ReplyKind::Reply, true) => (source.to.clone(), Vec::new()),
            (_, true) => (source.to.clone(), source.cc.clone()),
            (ReplyKind::Reply, false) => (target, Vec::new()),
            (_, false) => (
                target,
                source.to.iter().chain(&source.cc).cloned().collect(),
            ),
        };
        if kind == ReplyKind::ReplyAll {
            // The person's own address is never answered.
            to.retain(|a| !is_own(a));
            cc.retain(|a| !is_own(a));
        }
        dedupe(&mut to);
        dedupe(&mut cc);
        cc.retain(|a| !to.iter().any(|t| lower_eq(&t.address, &a.address)));
        fields.to = to;
        fields.cc = cc;
        fields.in_reply_to = source.message_id.as_deref().and_then(clean_message_id);
        let mut refs: Vec<String> = source
            .references
            .iter()
            .filter_map(|r| clean_message_id(r))
            .collect();
        if let Some(id) = &fields.in_reply_to {
            if !refs.contains(id) {
                refs.push(id.clone());
            }
        }
        fields.references = refs;
    }

    let sender = source
        .from
        .as_ref()
        .map_or_else(|| "someone".to_owned(), shown);
    match (kind, source.text.as_deref()) {
        (ReplyKind::Forward, text) => {
            let mut body = String::from("\n\n---------- Forwarded message ----------\n");
            body.push_str(&format!("From: {sender}\n"));
            body.push_str(&format!("Date: {}\n", quote_date(source.date_ms)));
            body.push_str(&format!("Subject: {}\n", clean_text(&source.subject)));
            if !source.to.is_empty() {
                let to: Vec<String> = source.to.iter().map(shown).collect();
                body.push_str(&format!("To: {}\n", to.join(", ")));
            }
            body.push('\n');
            if let Some(text) = text {
                body.push_str(text.replace("\r\n", "\n").trim_end());
                body.push('\n');
            }
            fields.body = body;
            fields.quote_included = text.is_some();
        }
        (_, Some(text)) => {
            let name = source
                .from
                .as_ref()
                .map(|a| {
                    a.name
                        .as_deref()
                        .map(str::trim)
                        .filter(|n| !n.is_empty())
                        .map_or_else(|| a.address.clone(), str::to_owned)
                })
                .unwrap_or(sender);
            fields.body = format!(
                "\n\nOn {}, {name} wrote:\n{}",
                quote_date(source.date_ms),
                quoted(text)
            );
            fields.quote_included = true;
        }
        (_, None) => {
            fields.quote_included = false;
        }
    }
    fields
}

#[cfg(test)]
mod tests;
