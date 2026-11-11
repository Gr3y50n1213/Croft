// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Reads a Mozilla-style autoconfig file (`config-v1.1.xml`).
//!
//! Pure: bytes in, settings out, no network. The input is hostile by assumption (it comes from a
//! web server Croft has no reason to trust beyond "it is the person's own domain"), so nothing
//! here can panic, expand entities, recurse without a limit or allocate more than a small
//! multiple of the input.

use super::address::{is_host_name, EmailAddress};
use super::error::ProbeFailure;
use super::{AuthHint, ImapServer, SmtpServer};
use crate::imap::Security;
use quick_xml::events::Event;
use quick_xml::{Reader, XmlVersion};
use std::net::IpAddr;

/// Largest file Croft reads.
pub(crate) const MAX_BYTES: usize = 64 * 1024;
/// Deepest element nesting accepted.
const MAX_DEPTH: usize = 16;
/// Most XML events looked at before giving up.
const MAX_EVENTS: usize = 20_000;
/// Longest text value kept for one field.
const MAX_TEXT: usize = 512;

/// The usable part of an autoconfig file.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Parsed {
    /// The best IMAP server in the file.
    pub imap: ImapServer,
    /// The best SMTP server, if the file has an acceptable one.
    pub smtp: Option<SmtpServer>,
    /// How the person signs in.
    pub auth: AuthHint,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Imap,
    Smtp,
}

#[derive(Default)]
struct Raw {
    host: String,
    port: String,
    socket: String,
    username: String,
    auth: Vec<String>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Field {
    Host,
    Port,
    Socket,
    User,
    Auth,
}

/// Why one `<incomingServer>` or `<outgoingServer>` block was not usable.
enum Reject {
    Unencrypted,
    Other,
}

struct Candidate {
    host: String,
    port: u16,
    security: Security,
    username: String,
    auth: Vec<String>,
}

/// Reads the file for `address`, filling `%EMAILADDRESS%`, `%EMAILLOCALPART%` and `%EMAILDOMAIN%`
/// into user names, and `%EMAILDOMAIN%` only into host names (a server entry whose host name
/// contains the local part or the whole address is ignored, so the local part never reaches DNS). Servers with `socketType` `plain` are ignored; SSL beats
/// STARTTLS. Returns [`ProbeFailure::OnlyUnencrypted`] when the file offers IMAP only without
/// encryption and [`ProbeFailure::Unusable`] for anything else wrong with the file.
pub(crate) fn parse(xml: &[u8], address: &EmailAddress) -> Result<Parsed, ProbeFailure> {
    if xml.len() > MAX_BYTES {
        return Err(ProbeFailure::TooLarge);
    }
    let text = std::str::from_utf8(xml).map_err(|_| ProbeFailure::Unusable)?;
    let blocks = read_blocks(text)?;

    let mut imap: Vec<Candidate> = Vec::new();
    let mut smtp: Vec<Candidate> = Vec::new();
    let mut imap_unencrypted = false;
    for (kind, raw) in blocks {
        let default_ports = match kind {
            Kind::Imap => (993, 143),
            Kind::Smtp => (465, 587),
        };
        match candidate(raw, address, default_ports) {
            Ok(c) => match kind {
                Kind::Imap => imap.push(c),
                Kind::Smtp => smtp.push(c),
            },
            Err(Reject::Unencrypted) if kind == Kind::Imap => imap_unencrypted = true,
            Err(_) => {}
        }
    }
    let best_imap = best(imap).ok_or(if imap_unencrypted {
        ProbeFailure::OnlyUnencrypted
    } else {
        ProbeFailure::Unusable
    })?;
    let auth = auth_hint(&best_imap.auth);
    let best_smtp = best(smtp);
    Ok(Parsed {
        imap: ImapServer {
            host: best_imap.host,
            port: best_imap.port,
            security: best_imap.security,
            username: best_imap.username,
        },
        smtp: best_smtp.map(|c| SmtpServer {
            host: c.host,
            port: c.port,
            security: c.security,
            username: c.username,
        }),
        auth,
    })
}

/// SSL beats STARTTLS; among equals the file's order decides.
fn best(list: Vec<Candidate>) -> Option<Candidate> {
    let mut chosen: Option<Candidate> = None;
    for c in list {
        let better = match &chosen {
            None => true,
            Some(cur) => cur.security == Security::StartTls && c.security == Security::Tls,
        };
        if better {
            chosen = Some(c);
        }
    }
    chosen
}

/// Password wins when the file offers it, because that is what Croft can verify today.
fn auth_hint(offered: &[String]) -> AuthHint {
    let is = |s: &str, names: &[&str]| names.iter().any(|n| s.eq_ignore_ascii_case(n));
    let password = offered.iter().any(|a| {
        is(
            a,
            &[
                "password-cleartext",
                "plain",
                "password-encrypted",
                "secure",
                "login",
            ],
        )
    });
    let oauth = offered.iter().any(|a| is(a, &["oauth2", "oauth"]));
    if oauth && !password {
        AuthHint::OAuth2
    } else {
        AuthHint::Password
    }
}

/// Walks the XML and returns each IMAP and SMTP server block as raw text fields.
fn read_blocks(text: &str) -> Result<Vec<(Kind, Raw)>, ProbeFailure> {
    let mut reader = Reader::from_str(text);

    let mut depth = 0usize;
    let mut events = 0usize;
    // The server block being read, with the depth of its own element.
    let mut current: Option<(Kind, Raw, usize)> = None;
    // The field being read inside it, and its text so far.
    let mut field: Option<(Field, String)> = None;
    let mut out = Vec::new();
    let mut saw_root = false;

    loop {
        events += 1;
        if events > MAX_EVENTS {
            return Err(ProbeFailure::Unusable);
        }
        match reader.read_event().map_err(unusable)? {
            Event::Start(e) => {
                depth += 1;
                if depth > MAX_DEPTH {
                    return Err(ProbeFailure::Unusable);
                }
                saw_root = true;
                let name = local_lower(e.local_name().as_ref());
                match &current {
                    None => {
                        let kind = match name.as_str() {
                            "incomingserver" => Some((Kind::Imap, "imap")),
                            "outgoingserver" => Some((Kind::Smtp, "smtp")),
                            _ => None,
                        };
                        if let Some((kind, wanted)) = kind {
                            let ty = e
                                .try_get_attribute("type")
                                .map_err(unusable)?
                                .map(|a| {
                                    a.normalized_value(XmlVersion::Implicit1_0)
                                        .map(|v| v.into_owned())
                                })
                                .transpose()
                                .map_err(unusable)?;
                            if ty.is_some_and(|t| t.trim().eq_ignore_ascii_case(wanted)) {
                                current = Some((kind, Raw::default(), depth));
                            }
                        }
                    }
                    Some((_, _, server_depth)) if depth == server_depth + 1 => {
                        field = match name.as_str() {
                            "hostname" => Some((Field::Host, String::new())),
                            "port" => Some((Field::Port, String::new())),
                            "sockettype" => Some((Field::Socket, String::new())),
                            "username" => Some((Field::User, String::new())),
                            "authentication" => Some((Field::Auth, String::new())),
                            _ => None,
                        };
                    }
                    Some(_) => {}
                }
            }
            Event::Empty(_) => {
                // An empty element has no text and no children, so it carries nothing we read.
                saw_root = true;
                if depth + 1 > MAX_DEPTH {
                    return Err(ProbeFailure::Unusable);
                }
            }
            Event::End(_) => {
                // Only the end of the field's own element closes it, not that of a child inside.
                if let Some((_, raw, server_depth)) = current.as_mut() {
                    if depth == *server_depth + 1 {
                        if let Some((f, value)) = field.take() {
                            let value = value.trim().to_string();
                            match f {
                                Field::Host => raw.host = value,
                                Field::Port => raw.port = value,
                                Field::Socket => raw.socket = value,
                                Field::User => raw.username = value,
                                Field::Auth => raw.auth.push(value),
                            }
                        }
                    }
                }
                if matches!(&current, Some((_, _, d)) if *d == depth) {
                    if let Some((kind, raw, _)) = current.take() {
                        out.push((kind, raw));
                    }
                }
                depth = depth.saturating_sub(1);
            }
            Event::Text(t) => {
                if let Some((_, buf)) = field.as_mut() {
                    push_text(buf, &t.xml10_content())?;
                }
            }
            Event::CData(c) => {
                if let Some((_, buf)) = field.as_mut() {
                    push_text(buf, &c.xml10_content())?;
                }
            }
            Event::GeneralRef(r) => {
                if let Some((_, buf)) = field.as_mut() {
                    let ch = if r.is_char_ref() {
                        r.resolve_char_ref()
                            .map_err(unusable)?
                            .ok_or(ProbeFailure::Unusable)?
                    } else {
                        match r.xml10_content().as_ref() {
                            "amp" => '&',
                            "lt" => '<',
                            "gt" => '>',
                            "quot" => '"',
                            "apos" => '\'',
                            // Any other entity would have to come from a DOCTYPE, which is refused.
                            _ => return Err(ProbeFailure::Unusable),
                        }
                    };
                    let mut tmp = [0u8; 4];
                    push_text(buf, ch.encode_utf8(&mut tmp))?;
                }
            }
            Event::DocType(_) => return Err(ProbeFailure::Unusable),
            Event::Decl(d) => {
                if let Some(Ok(enc)) = d.encoding() {
                    let enc = enc.trim();
                    if !(enc.eq_ignore_ascii_case("utf-8") || enc.eq_ignore_ascii_case("us-ascii"))
                    {
                        return Err(ProbeFailure::Unusable);
                    }
                }
            }
            Event::PI(_) | Event::Comment(_) => {}
            Event::Eof => break,
        }
    }
    if depth != 0 || !saw_root {
        return Err(ProbeFailure::Unusable);
    }
    Ok(out)
}

/// Any XML error means the file cannot be used; the details are not kept.
fn unusable<E>(_: E) -> ProbeFailure {
    ProbeFailure::Unusable
}

fn push_text(buf: &mut String, more: &str) -> Result<(), ProbeFailure> {
    if buf.len() + more.len() > MAX_TEXT {
        return Err(ProbeFailure::Unusable);
    }
    buf.push_str(more);
    Ok(())
}

fn local_lower(name: &str) -> String {
    name.to_ascii_lowercase()
}

fn candidate(
    raw: Raw,
    address: &EmailAddress,
    (default_tls_port, default_starttls_port): (u16, u16),
) -> Result<Candidate, Reject> {
    let security = match raw.socket.to_ascii_lowercase().as_str() {
        "ssl" | "tls" => Security::Tls,
        "starttls" => Security::StartTls,
        "plain" | "none" => return Err(Reject::Unencrypted),
        _ => return Err(Reject::Other),
    };
    // A host name is looked up in DNS, so it must never carry the part of the address before the
    // `@`: a file saying `%EMAILLOCALPART%.evil.example` would send the local part to whoever runs
    // that name server. Only the domain is allowed in a host name.
    let host_template = raw.host.to_ascii_uppercase();
    if host_template.contains("%EMAILLOCALPART%") || host_template.contains("%EMAILADDRESS%") {
        return Err(Reject::Other);
    }
    let host = substitute(&raw.host, address).ok_or(Reject::Other)?;
    let host = host.trim().to_ascii_lowercase();
    if !(is_host_name(&host) || host.parse::<IpAddr>().is_ok()) {
        return Err(Reject::Other);
    }
    let port = if raw.port.is_empty() {
        match security {
            Security::Tls => default_tls_port,
            _ => default_starttls_port,
        }
    } else {
        match raw.port.parse::<u16>() {
            Ok(p) if p != 0 => p,
            _ => return Err(Reject::Other),
        }
    };
    let username = if raw.username.is_empty() {
        address.full()
    } else {
        substitute(&raw.username, address).ok_or(Reject::Other)?
    };
    if username.is_empty() || username.chars().any(char::is_control) {
        return Err(Reject::Other);
    }
    Ok(Candidate {
        host,
        port,
        security,
        username,
        auth: raw.auth,
    })
}

/// Fills in the three placeholders in one pass (so a value that itself contains `%...%` is not
/// expanded twice). A `%` that does not start a known placeholder makes the value unusable.
fn substitute(template: &str, address: &EmailAddress) -> Option<String> {
    let mut out = String::with_capacity(template.len() + 16);
    let mut rest = template;
    while let Some(i) = rest.find('%') {
        out.push_str(&rest[..i]);
        let after = &rest[i..];
        let (value, len) = if starts_with_ci(after, "%EMAILADDRESS%") {
            (address.full(), "%EMAILADDRESS%".len())
        } else if starts_with_ci(after, "%EMAILLOCALPART%") {
            (address.local_part().to_string(), "%EMAILLOCALPART%".len())
        } else if starts_with_ci(after, "%EMAILDOMAIN%") {
            (address.domain().to_string(), "%EMAILDOMAIN%".len())
        } else {
            return None;
        };
        out.push_str(&value);
        rest = &after[len..];
    }
    out.push_str(rest);
    Some(out)
}

fn starts_with_ci(text: &str, prefix: &str) -> bool {
    text.get(..prefix.len())
        .is_some_and(|t| t.eq_ignore_ascii_case(prefix))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SELFHOSTED: &str = include_str!("fixtures/autoconfig/selfhosted.xml");
    const LOCALPART: &str = include_str!("fixtures/autoconfig/localpart.xml");
    const PLAIN_ONLY: &str = include_str!("fixtures/autoconfig/plain_only.xml");
    const OAUTH: &str = include_str!("fixtures/autoconfig/oauth.xml");
    const NO_MAIL: &str = include_str!("fixtures/autoconfig/no_mail_servers.xml");
    const TRUNCATED: &str = include_str!("fixtures/autoconfig/truncated.xml");
    const BOMB: &str = include_str!("fixtures/autoconfig/entity_bomb.xml");
    const HTML: &str = include_str!("fixtures/autoconfig/html_page.html");
    const BAD_VALUES: &str = include_str!("fixtures/autoconfig/bad_values.xml");

    fn addr(s: &str) -> EmailAddress {
        EmailAddress::parse(s).unwrap()
    }

    #[test]
    fn selfhosted_prefers_ssl_and_fills_the_address() {
        let p = parse(SELFHOSTED.as_bytes(), &addr("Ann@example.org")).unwrap();
        assert_eq!(p.imap.host, "mail.example.org");
        assert_eq!((p.imap.port, p.imap.security), (993, Security::Tls));
        assert_eq!(p.imap.username, "Ann@example.org");
        let smtp = p.smtp.unwrap();
        assert_eq!((smtp.port, smtp.security), (465, Security::Tls));
        assert_eq!(p.auth, AuthHint::Password);
    }

    #[test]
    fn placeholders_work_in_host_and_user_name() {
        let p = parse(LOCALPART.as_bytes(), &addr("ann@hosted.example")).unwrap();
        assert_eq!(p.imap.host, "imap.hosted.example");
        assert_eq!(p.imap.username, "ann");
        let smtp = p.smtp.unwrap();
        assert_eq!(smtp.host, "smtp.hosted.example");
        assert_eq!((smtp.port, smtp.security), (587, Security::StartTls));
        assert_eq!(smtp.username, "ann");
    }

    #[test]
    fn a_host_name_may_not_contain_the_local_part_or_the_address() {
        let a = addr("alice@example.org");
        for host in [
            "%EMAILLOCALPART%.evil.example",
            "%emaillocalpart%.evil.example",
            "imap.%EMAILADDRESS%",
            "x-%EmailAddress%",
        ] {
            let xml = format!(
                "<c><incomingServer type=\"imap\"><hostname>{host}</hostname><socketType>SSL</socketType></incomingServer></c>"
            );
            assert_eq!(
                parse(xml.as_bytes(), &a).unwrap_err(),
                ProbeFailure::Unusable,
                "{host}"
            );
        }
        // The domain is still fine, in a host name and next to a local-part user name.
        let xml = "<c><incomingServer type=\"imap\"><hostname>imap.%EMAILDOMAIN%</hostname><socketType>SSL</socketType><username>%EMAILLOCALPART%</username></incomingServer></c>";
        let p = parse(xml.as_bytes(), &a).unwrap();
        assert_eq!(
            (p.imap.host.as_str(), p.imap.username.as_str()),
            ("imap.example.org", "alice")
        );
    }

    #[test]
    fn a_bad_server_entry_does_not_hide_a_good_one() {
        let a = addr("alice@example.org");
        let xml = "<c><incomingServer type=\"imap\"><hostname>%EMAILLOCALPART%.evil.example</hostname><socketType>SSL</socketType></incomingServer><incomingServer type=\"imap\"><hostname>mail.example.org</hostname><socketType>STARTTLS</socketType></incomingServer></c>";
        let p = parse(xml.as_bytes(), &a).unwrap();
        assert_eq!(p.imap.host, "mail.example.org");
    }

    #[test]
    fn a_placeholder_inside_the_local_part_is_not_expanded_twice() {
        let a = addr("%EMAILDOMAIN%@hosted.example");
        let p = parse(LOCALPART.as_bytes(), &a).unwrap();
        assert_eq!(p.imap.username, "%EMAILDOMAIN%");
    }

    #[test]
    fn plain_text_servers_are_refused_with_their_own_reason() {
        let err = parse(PLAIN_ONLY.as_bytes(), &addr("a@old.example")).unwrap_err();
        assert_eq!(err, ProbeFailure::OnlyUnencrypted);
    }

    #[test]
    fn oauth_only_files_say_so() {
        let p = parse(OAUTH.as_bytes(), &addr("a@oauth.example")).unwrap();
        assert_eq!(p.auth, AuthHint::OAuth2);
        assert!(p.smtp.is_none());
    }

    #[test]
    fn a_file_without_imap_is_unusable() {
        assert_eq!(
            parse(NO_MAIL.as_bytes(), &addr("a@pop.example")).unwrap_err(),
            ProbeFailure::Unusable
        );
    }

    #[test]
    fn hostile_and_broken_input_is_an_error_never_a_panic() {
        let a = addr("a@example.org");
        for (name, input) in [
            ("truncated", TRUNCATED),
            ("entity bomb", BOMB),
            ("html page", HTML),
            ("bad values", BAD_VALUES),
            ("empty", ""),
            ("text", "just some words"),
            ("lone angle", "<"),
            ("unclosed", "<clientConfig><emailProvider>"),
            ("mismatched", "<a><b></a></b>"),
            ("end only", "</a>"),
            ("bad entity", "<a><incomingServer type=\"imap\"><hostname>&bogus;</hostname></incomingServer></a>"),
            ("nul", "<a>\u{0}</a>"),
            ("wrong encoding", "<?xml version=\"1.0\" encoding=\"UTF-16\"?><a/>"),
        ] {
            assert!(parse(input.as_bytes(), &a).is_err(), "{name}");
        }
        assert_eq!(
            parse(&[0xff, 0xfe, 0x00, 0x3c], &a).unwrap_err(),
            ProbeFailure::Unusable
        );
    }

    #[test]
    fn deep_nesting_and_huge_input_are_cut_off() {
        let a = addr("a@example.org");
        let deep = format!("{}{}", "<x>".repeat(5000), "</x>".repeat(5000));
        assert!(parse(deep.as_bytes(), &a).is_err());
        let big = vec![b' '; MAX_BYTES + 1];
        assert_eq!(parse(&big, &a).unwrap_err(), ProbeFailure::TooLarge);
        let many = format!("<a>{}</a>", "<b/>".repeat(MAX_EVENTS));
        assert!(parse(many.as_bytes(), &a).is_err());
        let long_value = format!(
            "<a><incomingServer type=\"imap\"><hostname>{}</hostname></incomingServer></a>",
            "h".repeat(MAX_TEXT + 1)
        );
        assert!(parse(long_value.as_bytes(), &a).is_err());
    }

    #[test]
    fn bad_hosts_ports_and_socket_types_are_skipped_not_trusted() {
        let err = parse(BAD_VALUES.as_bytes(), &addr("a@bad.example")).unwrap_err();
        assert_eq!(err, ProbeFailure::Unusable);
    }

    #[test]
    fn a_good_server_after_bad_ones_is_still_found() {
        let xml = format!(
            "{}{}",
            "<c><incomingServer type=\"imap\"><hostname>a b</hostname><socketType>SSL</socketType></incomingServer>",
            "<incomingServer type=\"IMAP\"><hostname>Good.Example</hostname><socketType>ssl</socketType></incomingServer></c>"
        );
        let p = parse(xml.as_bytes(), &addr("a@example.org")).unwrap();
        assert_eq!(p.imap.host, "good.example");
        assert_eq!(p.imap.port, 993);
        assert_eq!(p.imap.username, "a@example.org");
    }

    #[test]
    fn char_references_and_cdata_are_read() {
        let xml = "<c><incomingServer type=\"imap\"><hostname><![CDATA[mail.example.org]]></hostname><socketType>S&#83;L</socketType><port>9&#x39;3</port></incomingServer></c>";
        let p = parse(xml.as_bytes(), &addr("a@example.org")).unwrap();
        assert_eq!(
            (p.imap.host.as_str(), p.imap.port),
            ("mail.example.org", 993)
        );
    }

    #[test]
    fn ip_literal_hosts_are_allowed() {
        let xml = "<c><incomingServer type=\"imap\"><hostname>127.0.0.1</hostname><port>3993</port><socketType>SSL</socketType></incomingServer></c>";
        let p = parse(xml.as_bytes(), &addr("a@example.org")).unwrap();
        assert_eq!(p.imap.host, "127.0.0.1");
    }
}
