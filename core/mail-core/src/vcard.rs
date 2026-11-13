// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! vCard 3.0 (RFC 2426) and 4.0 (RFC 6350) for contacts (T38a). Pure and bounded: a card comes
//! from a server, so sizes, lines and properties are capped and nothing in it is trusted.
//!
//! A [`Card`] keeps every line it was read from. [`Card::contact`] reads the properties Croft
//! shows; [`Card::apply`] writes an edited [`Contact`] back, rewriting only the kinds of property
//! that changed (so an untouched kind keeps its exact lines, Apple's `item1.` groups and labels
//! included) and keeping every other line as it was. [`Card::to_text`] folds lines at 75 octets.

/// The largest card read, in bytes.
pub const MAX_CARD: usize = 256 * 1024;
/// The most lines (after unfolding) in a card.
pub const MAX_LINES: usize = 2000;
/// The most values of one kind (emails, phones, addresses) a contact keeps.
pub const MAX_VALUES: usize = 50;
/// The longest single value kept, in characters.
pub const MAX_VALUE_CHARS: usize = 8000;

/// Why a card could not be read. Carries nothing of the card.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VcardError {
    /// Larger than [`MAX_CARD`] or [`MAX_LINES`].
    TooLarge,
    /// Not one `BEGIN:VCARD` ... `END:VCARD` with a `VERSION` of 3.0 or 4.0.
    NotACard,
}

impl std::fmt::Display for VcardError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::TooLarge => "the contact card is too large",
            Self::NotACard => "not a vCard 3.0 or 4.0",
        })
    }
}

impl std::error::Error for VcardError {}

/// The vCard version.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Version {
    /// RFC 2426.
    V3,
    /// RFC 6350.
    V4,
}

/// A name in parts (`N`): family, given, additional, prefix, suffix.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Name {
    /// Family name.
    pub family: String,
    /// Given name.
    pub given: String,
    /// Additional (middle) names.
    pub additional: String,
    /// Honorific prefix.
    pub prefix: String,
    /// Honorific suffix.
    pub suffix: String,
}

impl Name {
    fn is_empty(&self) -> bool {
        [
            &self.family,
            &self.given,
            &self.additional,
            &self.prefix,
            &self.suffix,
        ]
        .iter()
        .all(|p| p.trim().is_empty())
    }
}

/// An email or phone number with its kinds (`home`, `work`, `cell`, ... in lower case).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Typed {
    /// The address or number.
    pub value: String,
    /// The kinds, lower case, without `pref`.
    pub kinds: Vec<String>,
    /// Marked preferred.
    pub preferred: bool,
}

/// A postal address (`ADR`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PostalAddress {
    /// Street (with the post-office box and extended address folded in front, when given).
    pub street: String,
    /// Locality (city).
    pub locality: String,
    /// Region.
    pub region: String,
    /// Postal code.
    pub postal_code: String,
    /// Country.
    pub country: String,
    /// The kinds, lower case.
    pub kinds: Vec<String>,
}

/// What Croft shows and edits of a card.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Contact {
    /// `UID`.
    pub uid: Option<String>,
    /// `FN`: the name as shown.
    pub formatted_name: String,
    /// `N`.
    pub name: Name,
    /// `EMAIL`s.
    pub emails: Vec<Typed>,
    /// `TEL`s.
    pub phones: Vec<Typed>,
    /// `ADR`s.
    pub addresses: Vec<PostalAddress>,
    /// `ORG` (its first part).
    pub organization: String,
    /// `NOTE`.
    pub note: String,
    /// `BDAY`, as written.
    pub birthday: Option<String>,
    /// The card carries a `KEY` (an OpenPGP key or a link to one).
    pub has_key: bool,
}

impl Contact {
    /// The name to show: `FN`, else `N` put together, else the first email.
    pub fn display_name(&self) -> String {
        let fname = self.formatted_name.trim();
        if !fname.is_empty() {
            return fname.to_owned();
        }
        let n = &self.name;
        let parts: Vec<&str> = [&n.prefix, &n.given, &n.additional, &n.family, &n.suffix]
            .iter()
            .map(|p| p.trim())
            .filter(|p| !p.is_empty())
            .collect();
        if !parts.is_empty() {
            return parts.join(" ");
        }
        self.emails
            .first()
            .map(|e| e.value.clone())
            .unwrap_or_default()
    }

    /// The key the A to Z list sorts by: the family name, else the name shown, lower case.
    pub fn sort_key(&self) -> String {
        let family = self.name.family.trim();
        let base = if family.is_empty() {
            self.display_name()
        } else {
            format!("{family} {}", self.name.given.trim())
        };
        base.trim().to_lowercase()
    }
}

/// One content line, unfolded.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Line {
    /// The text as read (unfolded), written back as it was when the kind is not rewritten.
    raw: String,
    /// Upper-case property name, without the group.
    name: String,
    /// Parameters: upper-case name, values.
    params: Vec<(String, Vec<String>)>,
    /// The value, still escaped.
    value: String,
}

/// A card as read: its version and every line between `BEGIN` and `END`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Card {
    /// The version.
    pub version: Version,
    lines: Vec<Line>,
}

/// The property kinds [`Card::apply`] may rewrite.
const KNOWN: [&str; 9] = [
    "FN", "N", "EMAIL", "TEL", "ADR", "ORG", "NOTE", "BDAY", "UID",
];

/// Reads one card.
pub fn parse(text: &str) -> Result<Card, VcardError> {
    if text.len() > MAX_CARD {
        return Err(VcardError::TooLarge);
    }
    let unfolded = unfold(text)?;
    let mut lines = unfolded.into_iter().filter(|l| !l.trim().is_empty());
    match lines.next() {
        Some(first) if first.trim().eq_ignore_ascii_case("BEGIN:VCARD") => {}
        _ => return Err(VcardError::NotACard),
    }
    let mut body = Vec::new();
    let mut ended = false;
    for raw in lines.by_ref() {
        if raw.trim().eq_ignore_ascii_case("END:VCARD") {
            ended = true;
            break;
        }
        let Some(line) = parse_line(&raw) else {
            continue;
        };
        if line.name == "BEGIN" {
            // A nested card (AGENT in 3.0) is not read.
            return Err(VcardError::NotACard);
        }
        body.push(line);
    }
    if !ended || lines.any(|l| !l.trim().is_empty()) {
        return Err(VcardError::NotACard);
    }
    let version = match body
        .iter()
        .find(|l| l.name == "VERSION")
        .map(|l| l.value.trim())
    {
        Some("3.0") => Version::V3,
        Some("4.0") => Version::V4,
        _ => return Err(VcardError::NotACard),
    };
    body.retain(|l| l.name != "VERSION");
    Ok(Card {
        version,
        lines: body,
    })
}

/// Unfolds lines (a line starting with a space or tab continues the one before) and joins
/// quoted-printable soft breaks; LF and CRLF both end a line.
fn unfold(text: &str) -> Result<Vec<String>, VcardError> {
    let mut out: Vec<String> = Vec::new();
    let mut soft_break = false;
    for line in text.split('\n') {
        let line = line.strip_suffix('\r').unwrap_or(line);
        if let Some(rest) = line.strip_prefix([' ', '\t']).filter(|_| !out.is_empty()) {
            if let Some(last) = out.last_mut() {
                last.push_str(rest);
            }
        } else if soft_break && !out.is_empty() {
            if let Some(last) = out.last_mut() {
                last.push_str(line);
            }
        } else {
            if out.len() >= MAX_LINES {
                return Err(VcardError::TooLarge);
            }
            out.push(line.to_owned());
        }
        soft_break = out.last().is_some_and(|l| {
            l.ends_with('=') && l.to_ascii_uppercase().contains("QUOTED-PRINTABLE")
        });
        if soft_break {
            if let Some(last) = out.last_mut() {
                last.pop();
            }
        }
    }
    Ok(out)
}

/// Splits `group.NAME;PARAM=a,b;PARAM="c":value`. `None` for a line without a colon or name.
fn parse_line(raw: &str) -> Option<Line> {
    // The first colon outside double quotes ends the name and parameters.
    let mut quoted = false;
    let mut colon = None;
    for (i, c) in raw.char_indices() {
        match c {
            '"' => quoted = !quoted,
            ':' if !quoted => {
                colon = Some(i);
                break;
            }
            _ => {}
        }
    }
    let colon = colon?;
    let (head, value) = (&raw[..colon], &raw[colon + 1..]);
    let mut parts = split_unquoted(head, ';').into_iter();
    let full_name = parts.next()?;
    let name = full_name
        .rsplit('.')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_uppercase();
    if name.is_empty() || !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-') {
        return None;
    }
    let mut params: Vec<(String, Vec<String>)> = Vec::new();
    for p in parts {
        let (pname, pvalue) = match p.split_once('=') {
            Some((n, v)) => (n.trim().to_ascii_uppercase(), v),
            // vCard 2.1 style bare types (`TEL;CELL:`) are kinds.
            None => ("TYPE".to_owned(), p.as_str()),
        };
        let values = split_unquoted(pvalue, ',')
            .into_iter()
            .map(|v| v.trim().trim_matches('"').to_owned())
            .filter(|v| !v.is_empty())
            .collect();
        params.push((pname, values));
    }
    let mut line = Line {
        raw: raw.to_owned(),
        name,
        params,
        value: value.to_owned(),
    };
    if line
        .param("ENCODING")
        .iter()
        .any(|e| e.eq_ignore_ascii_case("QUOTED-PRINTABLE"))
    {
        line.value = escape(&quoted_printable(&line.value));
    }
    Some(line)
}

/// Splits at `sep` outside double quotes.
fn split_unquoted(text: &str, sep: char) -> Vec<String> {
    let mut out = vec![String::new()];
    let mut quoted = false;
    for c in text.chars() {
        if c == '"' {
            quoted = !quoted;
        }
        if c == sep && !quoted {
            out.push(String::new());
        } else if let Some(last) = out.last_mut() {
            last.push(c);
        }
    }
    out
}

/// Decodes quoted-printable into UTF-8 (lossy).
fn quoted_printable(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'=' {
            if let Some(b) = text
                .get(i + 1..i + 3)
                .and_then(|h| u8::from_str_radix(h, 16).ok())
            {
                out.push(b);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

impl Line {
    fn param(&self, name: &str) -> Vec<&str> {
        self.params
            .iter()
            .filter(|(n, _)| n == name)
            .flat_map(|(_, v)| v.iter().map(String::as_str))
            .collect()
    }

    /// The kinds (`TYPE`) in lower case, without `pref`, and whether it is preferred.
    fn kinds(&self) -> (Vec<String>, bool) {
        let mut kinds = Vec::new();
        let mut preferred = !self.param("PREF").is_empty();
        for t in self.param("TYPE") {
            let t = t.to_ascii_lowercase();
            if t == "pref" {
                preferred = true;
            } else if !t.is_empty() && !kinds.contains(&t) {
                kinds.push(t);
            }
        }
        (kinds, preferred)
    }
}

/// Undoes text escaping (`\\`, `\,`, `\;`, `\n`), capped at [`MAX_VALUE_CHARS`].
fn unescape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('n' | 'N') => out.push('\n'),
                Some(other) => out.push(other),
                None => {}
            }
        } else {
            out.push(c);
        }
    }
    out.chars()
        .filter(|c| !c.is_control() || *c == '\n')
        .take(MAX_VALUE_CHARS)
        .collect()
}

/// Escapes a text value for writing.
fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            ',' => out.push_str("\\,"),
            ';' => out.push_str("\\;"),
            '\n' => out.push_str("\\n"),
            '\r' => {}
            c if c.is_control() => {}
            c => out.push(c),
        }
    }
    out
}

/// Splits a structured value at unescaped `;`, then unescapes each part.
fn structured(value: &str) -> Vec<String> {
    let mut parts = vec![String::new()];
    let mut chars = value.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => {
                if let Some(last) = parts.last_mut() {
                    last.push('\\');
                    if let Some(n) = chars.next() {
                        last.push(n);
                    }
                }
            }
            ';' => parts.push(String::new()),
            c => {
                if let Some(last) = parts.last_mut() {
                    last.push(c);
                }
            }
        }
    }
    parts.iter().map(|p| unescape(p)).collect()
}

/// The email value: `mailto:` dropped (4.0 allows a URI), trimmed.
fn email_value(value: &str) -> String {
    let v = unescape(value);
    let v = v.trim();
    v.strip_prefix("mailto:").unwrap_or(v).trim().to_owned()
}

/// The phone value: `tel:` dropped (4.0 URIs), trimmed.
fn phone_value(value: &str) -> String {
    let v = unescape(value);
    let v = v.trim();
    v.strip_prefix("tel:").unwrap_or(v).trim().to_owned()
}

impl Card {
    /// A new 4.0 card for `contact` with `uid`.
    pub fn new(contact: &Contact, uid: &str) -> Card {
        let mut card = Card {
            version: Version::V4,
            lines: Vec::new(),
        };
        let mut c = contact.clone();
        c.uid = Some(uid.to_owned());
        card.write_kinds(&c, &KNOWN);
        card
    }

    /// What Croft shows.
    pub fn contact(&self) -> Contact {
        let mut c = Contact::default();
        for l in &self.lines {
            match l.name.as_str() {
                "UID" => c.uid = Some(unescape(&l.value).trim().to_owned()),
                "FN" if c.formatted_name.is_empty() => c.formatted_name = unescape(&l.value),
                "N" if c.name.is_empty() => {
                    let p = structured(&l.value);
                    let part = |i: usize| p.get(i).cloned().unwrap_or_default();
                    c.name = Name {
                        family: part(0),
                        given: part(1),
                        additional: part(2),
                        prefix: part(3),
                        suffix: part(4),
                    };
                }
                "EMAIL" if c.emails.len() < MAX_VALUES => {
                    let value = email_value(&l.value);
                    if !value.is_empty() {
                        let (kinds, preferred) = l.kinds();
                        c.emails.push(Typed {
                            value,
                            kinds,
                            preferred,
                        });
                    }
                }
                "TEL" if c.phones.len() < MAX_VALUES => {
                    let value = phone_value(&l.value);
                    if !value.is_empty() {
                        let (kinds, preferred) = l.kinds();
                        c.phones.push(Typed {
                            value,
                            kinds,
                            preferred,
                        });
                    }
                }
                "ADR" if c.addresses.len() < MAX_VALUES => {
                    let p = structured(&l.value);
                    let part = |i: usize| p.get(i).cloned().unwrap_or_default();
                    let street = [part(0), part(1), part(2)]
                        .into_iter()
                        .filter(|s| !s.trim().is_empty())
                        .collect::<Vec<_>>()
                        .join(", ");
                    c.addresses.push(PostalAddress {
                        street,
                        locality: part(3),
                        region: part(4),
                        postal_code: part(5),
                        country: part(6),
                        kinds: l.kinds().0,
                    });
                }
                "ORG" if c.organization.is_empty() => {
                    c.organization = structured(&l.value).into_iter().next().unwrap_or_default();
                }
                "NOTE" if c.note.is_empty() => c.note = unescape(&l.value),
                "BDAY" if c.birthday.is_none() => {
                    c.birthday = Some(unescape(&l.value).trim().to_owned())
                }
                "KEY" => c.has_key = true,
                _ => {}
            }
        }
        c
    }

    /// Writes `edited` into the card: each kind of property that differs from what the card
    /// shows now is replaced (at the place of its first line); the others, and every property
    /// Croft does not edit, keep their lines exactly.
    pub fn apply(&mut self, edited: &Contact) {
        let now = self.contact();
        let mut changed: Vec<&str> = Vec::new();
        if now.formatted_name != edited.formatted_name {
            changed.push("FN");
        }
        if now.name != edited.name {
            changed.push("N");
        }
        if now.emails != edited.emails {
            changed.push("EMAIL");
        }
        if now.phones != edited.phones {
            changed.push("TEL");
        }
        if now.addresses != edited.addresses {
            changed.push("ADR");
        }
        if now.organization != edited.organization {
            changed.push("ORG");
        }
        if now.note != edited.note {
            changed.push("NOTE");
        }
        if now.birthday != edited.birthday {
            changed.push("BDAY");
        }
        self.write_kinds(edited, &changed);
    }

    /// Replaces the lines of `kinds` with `contact`'s values, at the place of the first old line
    /// of each kind (else at the end).
    fn write_kinds(&mut self, contact: &Contact, kinds: &[&str]) {
        for kind in kinds {
            let at = self.lines.iter().position(|l| l.name == *kind);
            self.lines.retain(|l| l.name != *kind);
            let new = self.lines_for(contact, kind);
            let at = at.unwrap_or(self.lines.len()).min(self.lines.len());
            self.lines.splice(at..at, new);
        }
    }

    fn lines_for(&self, c: &Contact, kind: &str) -> Vec<Line> {
        let v4 = self.version == Version::V4;
        let typed = |name: &str, t: &Typed| {
            let mut params = String::new();
            let mut kinds = t.kinds.clone();
            if t.preferred && !v4 {
                kinds.push("pref".to_owned());
            }
            if !kinds.is_empty() {
                params.push_str(";TYPE=");
                params.push_str(
                    &kinds
                        .iter()
                        .map(|k| param_value(k))
                        .collect::<Vec<_>>()
                        .join(","),
                );
            }
            if t.preferred && v4 {
                params.push_str(";PREF=1");
            }
            make(name, &params, &escape(&t.value))
        };
        match kind {
            "UID" => c.uid.iter().map(|u| make("UID", "", &escape(u))).collect(),
            "FN" => {
                let shown = if c.formatted_name.trim().is_empty() {
                    c.display_name()
                } else {
                    c.formatted_name.clone()
                };
                vec![make("FN", "", &escape(&shown))]
            }
            "N" if !c.name.is_empty() || !v4 => {
                let n = &c.name;
                let value = [&n.family, &n.given, &n.additional, &n.prefix, &n.suffix]
                    .iter()
                    .map(|p| escape(p))
                    .collect::<Vec<_>>()
                    .join(";");
                vec![make("N", "", &value)]
            }
            "EMAIL" => c
                .emails
                .iter()
                .take(MAX_VALUES)
                .map(|e| typed("EMAIL", e))
                .collect(),
            "TEL" => c
                .phones
                .iter()
                .take(MAX_VALUES)
                .map(|t| typed("TEL", t))
                .collect(),
            "ADR" => c
                .addresses
                .iter()
                .take(MAX_VALUES)
                .map(|a| {
                    let value = [
                        "",
                        "",
                        &a.street,
                        &a.locality,
                        &a.region,
                        &a.postal_code,
                        &a.country,
                    ]
                    .iter()
                    .map(|p| escape(p))
                    .collect::<Vec<_>>()
                    .join(";");
                    let params = if a.kinds.is_empty() {
                        String::new()
                    } else {
                        format!(
                            ";TYPE={}",
                            a.kinds
                                .iter()
                                .map(|k| param_value(k))
                                .collect::<Vec<_>>()
                                .join(",")
                        )
                    };
                    make("ADR", &params, &value)
                })
                .collect(),
            "ORG" if !c.organization.trim().is_empty() => {
                vec![make("ORG", "", &escape(&c.organization))]
            }
            "NOTE" if !c.note.trim().is_empty() => vec![make("NOTE", "", &escape(&c.note))],
            "BDAY" => c
                .birthday
                .iter()
                .filter(|b| !b.trim().is_empty())
                .map(|b| make("BDAY", "", &escape(b)))
                .collect(),
            _ => Vec::new(),
        }
    }

    /// The card as text: `BEGIN`, `VERSION`, the lines (folded at 75 octets), `END`, CRLF.
    pub fn to_text(&self) -> String {
        let mut out = String::from("BEGIN:VCARD\r\n");
        out.push_str(match self.version {
            Version::V3 => "VERSION:3.0\r\n",
            Version::V4 => "VERSION:4.0\r\n",
        });
        for l in &self.lines {
            fold_into(&mut out, &l.raw);
        }
        out.push_str("END:VCARD\r\n");
        out
    }
}

/// A kind as a parameter value: letters, digits and `-` only (anything else is dropped).
fn param_value(kind: &str) -> String {
    kind.chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
        .collect()
}

fn make(name: &str, params: &str, value: &str) -> Line {
    let raw = format!("{name}{params}:{value}");
    // Parsed back so the line is exactly what a reader of the text would see.
    parse_line(&raw).unwrap_or(Line {
        raw,
        name: name.to_owned(),
        params: Vec::new(),
        value: value.to_owned(),
    })
}

/// Appends `line` folded at 75 octets (never inside a character), with CRLF.
fn fold_into(out: &mut String, line: &str) {
    let mut width = 0;
    let mut first = true;
    for c in line.chars() {
        let len = c.len_utf8();
        let limit = if first { 75 } else { 74 };
        if width + len > limit {
            out.push_str("\r\n ");
            width = 0;
            first = false;
        }
        out.push(c);
        width += len;
    }
    out.push_str("\r\n");
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    const V3: &str = "BEGIN:VCARD\r\nVERSION:3.0\r\nUID:abc-1\r\nFN:Mira Okafor\r\nN:Okafor;Mira;;Dr.;\r\n\
item1.EMAIL;TYPE=INTERNET,WORK,pref:mira@example.net\r\nitem1.X-ABLabel:Office\r\nEMAIL;TYPE=HOME:mira.home@example.org\r\n\
TEL;TYPE=CELL:+44 7700 900123\r\nADR;TYPE=WORK:;;1 High St\\, Unit 2;Fenwick;;FW1 2AB;UK\r\n\
ORG:Acme;Research\r\nNOTE:Met at the\\nconference\\; bring notes\r\nX-CUSTOM;X-PARAM=\"a:b\":keep me\r\n\
PHOTO;ENCODING=b;TYPE=JPEG:AAAA\r\nEND:VCARD\r\n";

    #[test]
    fn a_3_0_card_is_read_with_its_names_kinds_and_escapes() {
        let card = parse(V3).unwrap();
        assert_eq!(card.version, Version::V3);
        let c = card.contact();
        assert_eq!(c.uid.as_deref(), Some("abc-1"));
        assert_eq!(c.display_name(), "Mira Okafor");
        assert_eq!(c.name.family, "Okafor");
        assert_eq!(c.name.prefix, "Dr.");
        assert_eq!(c.emails.len(), 2);
        assert_eq!(c.emails[0].value, "mira@example.net");
        assert_eq!(c.emails[0].kinds, vec!["internet", "work"]);
        assert!(c.emails[0].preferred && !c.emails[1].preferred);
        assert_eq!(c.phones[0].value, "+44 7700 900123");
        assert_eq!(c.addresses[0].street, "1 High St, Unit 2");
        assert_eq!(c.addresses[0].postal_code, "FW1 2AB");
        assert_eq!(c.organization, "Acme");
        assert_eq!(c.note, "Met at the\nconference; bring notes");
        assert_eq!(c.sort_key(), "okafor mira");
    }

    #[test]
    fn folded_lines_quoted_printable_and_4_0_uris_are_read() {
        let text = "BEGIN:VCARD\nVERSION:4.0\nFN:Sam Ri\n vera\nEMAIL;PREF=1:mailto:sam@example.org\n\
TEL;VALUE=uri;TYPE=voice:tel:+1-555-0100\nNOTE;ENCODING=QUOTED-PRINTABLE:Caf=C3=A9 =\nnotes\nEND:VCARD";
        let c = parse(text).unwrap().contact();
        assert_eq!(c.formatted_name, "Sam Rivera");
        assert_eq!(c.emails[0].value, "sam@example.org");
        assert!(c.emails[0].preferred);
        assert_eq!(c.phones[0].value, "+1-555-0100");
        assert_eq!(c.note, "Café notes");
    }

    #[test]
    fn an_edit_rewrites_only_the_changed_kind_and_keeps_every_other_line() {
        let mut card = parse(V3).unwrap();
        let mut c = card.contact();
        c.phones = vec![Typed {
            value: "+44 20 7946 0000".into(),
            kinds: vec!["work".into()],
            preferred: false,
        }];
        card.apply(&c);
        let text = card.to_text();
        assert!(
            text.contains("TEL;TYPE=work:+44 20 7946 0000\r\n"),
            "{text}"
        );
        assert!(!text.contains("7700 900123"));
        // Untouched kinds keep their exact lines, groups and labels.
        for kept in [
            "item1.EMAIL;TYPE=INTERNET,WORK,pref:mira@example.net\r\n",
            "item1.X-ABLabel:Office\r\n",
            "X-CUSTOM;X-PARAM=\"a:b\":keep me\r\n",
            "PHOTO;ENCODING=b;TYPE=JPEG:AAAA\r\n",
            "NOTE:Met at the\\nconference\\; bring notes\r\n",
            "UID:abc-1\r\n",
        ] {
            assert!(text.contains(kept), "{kept} in {text}");
        }
        // And it reads back the same.
        assert_eq!(parse(&text).unwrap().contact(), c);
    }

    #[test]
    fn a_new_card_is_4_0_escaped_and_folded() {
        let c = Contact {
            formatted_name: "Nora; Lindqvist".into(),
            emails: vec![Typed {
                value: "nora@example.com".into(),
                kinds: vec!["home".into()],
                preferred: true,
            }],
            note: "x".repeat(200),
            ..Contact::default()
        };
        let card = Card::new(&c, "uid-7");
        let text = card.to_text();
        assert!(text.starts_with("BEGIN:VCARD\r\nVERSION:4.0\r\n"));
        assert!(text.contains("FN:Nora\\; Lindqvist\r\n"));
        assert!(text.contains("EMAIL;TYPE=home;PREF=1:nora@example.com\r\n"));
        assert!(text.lines().all(|l| l.len() <= 75), "{text}");
        let back = parse(&text).unwrap().contact();
        assert_eq!(back.uid.as_deref(), Some("uid-7"));
        assert_eq!(back.formatted_name, "Nora; Lindqvist");
        assert_eq!(back.note, "x".repeat(200));
    }

    #[test]
    fn hostile_or_broken_cards_are_refused_or_bounded() {
        assert_eq!(parse("hello"), Err(VcardError::NotACard));
        assert_eq!(
            parse("BEGIN:VCARD\r\nVERSION:2.1\r\nFN:x\r\nEND:VCARD"),
            Err(VcardError::NotACard)
        );
        assert_eq!(
            parse("BEGIN:VCARD\r\nVERSION:3.0\r\nFN:x\r\n"),
            Err(VcardError::NotACard),
            "no END"
        );
        assert_eq!(
            parse("BEGIN:VCARD\r\nVERSION:3.0\r\nEND:VCARD\r\nBEGIN:VCARD\r\nVERSION:3.0\r\nEND:VCARD"),
            Err(VcardError::NotACard),
            "one card only"
        );
        let big = format!(
            "BEGIN:VCARD\r\nVERSION:3.0\r\nNOTE:{}\r\nEND:VCARD",
            "x".repeat(MAX_CARD)
        );
        assert_eq!(parse(&big), Err(VcardError::TooLarge));
        let many = format!(
            "BEGIN:VCARD\r\nVERSION:3.0\r\n{}END:VCARD",
            "X-A:1\r\n".repeat(MAX_LINES)
        );
        assert_eq!(parse(&many), Err(VcardError::TooLarge));
        // Control characters never reach a value; a value is capped.
        let c = parse(&format!(
            "BEGIN:VCARD\r\nVERSION:3.0\r\nFN:a\u{7}b\u{202e}\r\nNOTE:{}\r\nEND:VCARD",
            "y".repeat(MAX_VALUE_CHARS + 5)
        ))
        .unwrap()
        .contact();
        assert_eq!(c.formatted_name, "ab\u{202e}");
        assert_eq!(c.note.chars().count(), MAX_VALUE_CHARS);
        // A kind with odd characters is not written into a parameter.
        let card = Card::new(
            &Contact {
                formatted_name: "x".into(),
                phones: vec![Typed {
                    value: "1".into(),
                    kinds: vec!["ce\"ll;X=1".into()],
                    preferred: false,
                }],
                ..Contact::default()
            },
            "u",
        );
        assert!(card.to_text().contains("TEL;TYPE=cellX1:1\r\n"));
    }
}
