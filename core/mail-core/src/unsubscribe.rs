// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! One-tap unsubscribe (T37): reading `List-Unsubscribe` (RFC 2369) and choosing how to
//! unsubscribe (RFC 8058 one-click, an email, or the sender's page). Pure: nothing here contacts
//! anyone. The header comes from a message, so anyone could have written it: only the first
//! `https:` and the first `mailto:` URI are kept, each checked strictly, and a `mailto:` keeps
//! nothing but its address, subject and body.

/// The longest URI kept.
pub const MAX_URI: usize = 2048;
/// The longest subject kept from a `mailto:` URI, in characters.
pub const MAX_SUBJECT: usize = 200;
/// The longest body kept from a `mailto:` URI, in characters.
pub const MAX_BODY: usize = 2000;
/// The subject of the unsubscribe email when the URI names none.
pub const DEFAULT_SUBJECT: &str = "unsubscribe";

/// The email to send for a `mailto:` URI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mailto {
    /// The one address it goes to.
    pub to: String,
    /// The subject ([`DEFAULT_SUBJECT`] when the URI has none).
    pub subject: String,
    /// The body (often empty).
    pub body: String,
}

/// What a message's `List-Unsubscribe` offers, after checking.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Offers {
    /// The first acceptable `https:` URI.
    pub https: Option<String>,
    /// The first acceptable `mailto:` URI.
    pub mailto: Option<Mailto>,
}

/// How Croft will unsubscribe, in order of preference.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Way {
    /// RFC 8058: one `POST` of `List-Unsubscribe=One-Click` to this `https:` URI.
    OneClick(String),
    /// Send this email from the account.
    Email(Mailto),
    /// Open this `https:` page in the browser (Croft never fetches it).
    Website(String),
}

/// Reads a `List-Unsubscribe` value: the `<...>` entries in order, keeping the first acceptable
/// `https:` and `mailto:` URI. Anything else (`http:`, other schemes, text outside brackets) is
/// ignored.
pub fn parse(header: &str) -> Offers {
    let mut offers = Offers::default();
    let mut rest = header;
    while let Some(start) = rest.find('<') {
        let after = &rest[start + 1..];
        let Some(end) = after.find('>') else { break };
        let uri = after[..end].trim();
        rest = &after[end + 1..];
        if offers.https.is_none() {
            if let Some(u) = https_uri(uri) {
                offers.https = Some(u);
                continue;
            }
        }
        if offers.mailto.is_none() {
            offers.mailto = mailto_uri(uri);
        }
    }
    offers
}

/// The way to unsubscribe: one-click when there is an `https:` URI and `one_click` (the message's
/// `List-Unsubscribe-Post: List-Unsubscribe=One-Click`), else the email, else the page.
pub fn choose(offers: &Offers, one_click: bool) -> Option<Way> {
    match (&offers.https, &offers.mailto) {
        (Some(u), _) if one_click => Some(Way::OneClick(u.clone())),
        (_, Some(m)) => Some(Way::Email(m.clone())),
        (Some(u), None) => Some(Way::Website(u.clone())),
        (None, None) => None,
    }
}

/// The host of an `https:` URI that [`parse`] kept, for the confirmation ("example.com").
pub fn host_of(uri: &str) -> Option<&str> {
    let rest = strip_prefix_ignore_case(uri, "https://")?;
    let authority = &rest[..rest.find(['/', '?', '#']).unwrap_or(rest.len())];
    let host = match authority.rfind(':') {
        Some(i) if !authority.ends_with(']') => &authority[..i],
        _ => authority,
    };
    (!host.is_empty()).then_some(host)
}

fn strip_prefix_ignore_case<'a>(text: &'a str, prefix: &str) -> Option<&'a str> {
    let head = text.get(..prefix.len())?;
    head.eq_ignore_ascii_case(prefix)
        .then(|| &text[prefix.len()..])
}

/// An `https:` URI, if it is one Croft accepts: bounded, printable ASCII without spaces, no
/// credentials in its authority, a host.
fn https_uri(uri: &str) -> Option<String> {
    if uri.len() > MAX_URI || !uri.bytes().all(|b| b.is_ascii_graphic()) {
        return None;
    }
    let rest = strip_prefix_ignore_case(uri, "https://")?;
    let authority = &rest[..rest.find(['/', '?', '#']).unwrap_or(rest.len())];
    if authority.contains(['@', '\\']) {
        return None;
    }
    host_of(uri)?;
    Some(format!("https://{rest}"))
}

/// A `mailto:` URI with one plain address; only `subject` and `body` are kept from its query.
fn mailto_uri(uri: &str) -> Option<Mailto> {
    if uri.len() > MAX_URI || !uri.bytes().all(|b| b.is_ascii_graphic()) {
        return None;
    }
    let rest = strip_prefix_ignore_case(uri, "mailto:")?;
    let (addr, query) = rest.split_once('?').unwrap_or((rest, ""));
    let to = percent_decode(addr)?;
    if !plain_address(&to) {
        return None;
    }
    let mut subject = None;
    let mut body = None;
    for field in query.split('&') {
        let Some((name, value)) = field.split_once('=') else {
            continue;
        };
        if name.eq_ignore_ascii_case("subject") && subject.is_none() {
            subject = percent_decode(value).map(|s| clean(&s, false, MAX_SUBJECT));
        } else if name.eq_ignore_ascii_case("body") && body.is_none() {
            body = percent_decode(value).map(|s| clean(&s, true, MAX_BODY));
        }
    }
    Some(Mailto {
        to,
        subject: subject
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| DEFAULT_SUBJECT.to_owned()),
        body: body.unwrap_or_default(),
    })
}

/// One address `local@domain`: no spaces, brackets, commas, quotes or further `@`, a dotted or
/// plain domain of letters, digits, `-` and `.`.
fn plain_address(text: &str) -> bool {
    let Some((local, domain)) = text.split_once('@') else {
        return false;
    };
    let local_ok = !local.is_empty()
        && local.len() <= 64
        && local
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "!#$%&'*+-/=?^_`{|}~.".contains(c));
    let domain_ok = !domain.is_empty()
        && domain.len() <= 253
        && !domain.starts_with(['.', '-'])
        && !domain.ends_with(['.', '-'])
        && domain
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.');
    local_ok && domain_ok
}

/// `%XX` decoding into UTF-8 (lossy); `None` for a broken escape.
fn percent_decode(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = text.get(i + 1..i + 3)?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    Some(String::from_utf8_lossy(&out).into_owned())
}

/// Removes control characters (keeping line breaks, as `\n`, when `lines`), and keeps at most
/// `max` characters.
fn clean(text: &str, lines: bool, max: usize) -> String {
    text.replace("\r\n", "\n")
        .chars()
        .filter(|c| !c.is_control() || (lines && *c == '\n'))
        .take(max)
        .collect()
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn the_first_https_and_mailto_are_kept_in_order_and_other_schemes_ignored() {
        let o = parse(
            "<http://plain.example/u>, <ftp://x.example>, <https://list.example/u?id=7>, \
             <https://second.example/u>, <mailto:leave@list.example?subject=bye>",
        );
        assert_eq!(o.https.as_deref(), Some("https://list.example/u?id=7"));
        assert_eq!(
            o.mailto,
            Some(Mailto {
                to: "leave@list.example".into(),
                subject: "bye".into(),
                body: String::new()
            })
        );
        assert_eq!(parse("https://no.brackets.example/u"), Offers::default());
        assert_eq!(parse("<https://unclosed.example/u"), Offers::default());
    }

    #[test]
    fn the_choice_prefers_one_click_then_email_then_the_page() {
        let both = parse("<mailto:a@b.example>, <https://c.example/u>");
        assert_eq!(
            choose(&both, true),
            Some(Way::OneClick("https://c.example/u".into()))
        );
        assert!(matches!(choose(&both, false), Some(Way::Email(_))));
        let page = parse("<https://c.example/u>");
        assert_eq!(
            choose(&page, false),
            Some(Way::Website("https://c.example/u".into()))
        );
        let mail = parse("<mailto:a@b.example>");
        assert!(
            matches!(choose(&mail, true), Some(Way::Email(_))),
            "one-click needs https"
        );
        assert_eq!(choose(&Offers::default(), true), None);
    }

    #[test]
    fn unsafe_https_uris_are_refused() {
        for bad in [
            "<https://user:pass@evil.example/u>",
            "<https://evil.example@good.example/u>",
            "<https:///nohost>",
            "<https://sp ace.example/u>",
            "<https://tab\t.example/u>",
            "<https://back\\slash.example/u>",
            "<https://caf\u{e9}.example/u>",
        ] {
            assert_eq!(parse(bad).https, None, "{bad}");
        }
        let long = format!("<https://a.example/{}>", "x".repeat(MAX_URI));
        assert_eq!(parse(&long).https, None);
        assert_eq!(
            parse("<HTTPS://Mixed.Example:8443/u>").https.as_deref(),
            Some("https://Mixed.Example:8443/u")
        );
        assert_eq!(
            host_of("https://Mixed.Example:8443/u"),
            Some("Mixed.Example")
        );
        assert_eq!(host_of("https://[2001:db8::1]/u"), Some("[2001:db8::1]"));
    }

    #[test]
    fn a_mailto_keeps_one_address_a_clean_subject_and_body_and_nothing_else() {
        let m = parse(
            "<mailto:leave%40list.example?subject=Un%0Dsub%01&body=line1%0D%0Aline2%00&cc=boss@x.example&bcc=b@x.example>",
        )
        .mailto
        .unwrap();
        assert_eq!(m.to, "leave@list.example");
        assert_eq!(m.subject, "Unsub");
        assert_eq!(m.body, "line1\nline2");
        for bad in [
            "<mailto:a@b.example,c@d.example>",
            "<mailto:Name%20%3Ca@b.example%3E>",
            "<mailto:nobody>",
            "<mailto:a@b@c.example>",
            "<mailto:a@-b.example>",
            "<mailto:a%ZZ@b.example>",
        ] {
            assert_eq!(parse(bad).mailto, None, "{bad}");
        }
        // A broken escape in the subject drops only the subject.
        let m = parse("<mailto:a@b.example?subject=%ZZ>").mailto.unwrap();
        assert_eq!(m.subject, DEFAULT_SUBJECT);
        let long = format!("<mailto:a@b.example?body={}>", "x".repeat(MAX_BODY + 10));
        assert!(long.len() < MAX_URI);
        assert_eq!(parse(&long).mailto.unwrap().body.len(), MAX_BODY);
    }
}
