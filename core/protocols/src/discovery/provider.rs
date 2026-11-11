// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The built-in table of large providers. These are matched on the domain alone, with no
//! network traffic, and the lookup stops there: Croft does not ask `autoconfig.gmail.com`
//! anything.
//!
//! Only the domains a provider issues for its own mailboxes are listed. A custom domain whose mail
//! is hosted by one of these providers (for example `you@your-company.example` on Google
//! Workspace or Microsoft 365) is not recognised here. Finding those through the domain's MX
//! record is planned for later; this module deliberately sends no MX query yet.

use super::{AuthHint, ImapServer, Provider, SmtpServer};
use crate::imap::Security;

/// Everything the table knows about one provider.
pub(crate) struct Entry {
    pub provider: Provider,
    pub imap_host: &'static str,
    pub imap_port: u16,
    pub smtp_host: &'static str,
    pub smtp_port: u16,
    pub smtp_security: Security,
    /// JMAP session URL, where the provider offers JMAP.
    pub jmap: Option<&'static str>,
    pub auth_hint: AuthHint,
}

static GMAIL: Entry = Entry {
    provider: Provider::Gmail,
    imap_host: "imap.gmail.com",
    imap_port: 993,
    smtp_host: "smtp.gmail.com",
    smtp_port: 465,
    smtp_security: Security::Tls,
    jmap: None,
    auth_hint: AuthHint::OAuth2OrAppPassword,
};

static OUTLOOK: Entry = Entry {
    provider: Provider::Outlook,
    imap_host: "outlook.office365.com",
    imap_port: 993,
    smtp_host: "smtp.office365.com",
    smtp_port: 587,
    smtp_security: Security::StartTls,
    jmap: None,
    auth_hint: AuthHint::OAuth2,
};

static ICLOUD: Entry = Entry {
    provider: Provider::ICloud,
    imap_host: "imap.mail.me.com",
    imap_port: 993,
    smtp_host: "smtp.mail.me.com",
    smtp_port: 587,
    smtp_security: Security::StartTls,
    jmap: None,
    auth_hint: AuthHint::AppPassword,
};

static FASTMAIL: Entry = Entry {
    provider: Provider::Fastmail,
    imap_host: "imap.fastmail.com",
    imap_port: 993,
    smtp_host: "smtp.fastmail.com",
    smtp_port: 465,
    smtp_security: Security::Tls,
    jmap: Some("https://api.fastmail.com/jmap/session"),
    auth_hint: AuthHint::AppPassword,
};

/// The entry for a lower-case ASCII domain, if it belongs to one of the listed providers.
pub(crate) fn lookup(domain: &str) -> Option<&'static Entry> {
    match domain {
        "gmail.com" | "googlemail.com" => Some(&GMAIL),
        "outlook.com" | "hotmail.com" | "live.com" => Some(&OUTLOOK),
        "icloud.com" | "me.com" | "mac.com" => Some(&ICLOUD),
        "fastmail.com" | "fastmail.fm" => Some(&FASTMAIL),
        _ => None,
    }
}

/// The domains whose servers are a listed provider's own: the mailbox domains of [`lookup`] plus the
/// domain the provider's mail servers sit under (`outlook.office365.com`).
const SERVER_DOMAINS: [(&str, Provider); 11] = [
    ("gmail.com", Provider::Gmail),
    ("googlemail.com", Provider::Gmail),
    ("outlook.com", Provider::Outlook),
    ("hotmail.com", Provider::Outlook),
    ("live.com", Provider::Outlook),
    ("office365.com", Provider::Outlook),
    ("icloud.com", Provider::ICloud),
    ("me.com", Provider::ICloud),
    ("mac.com", Provider::ICloud),
    ("fastmail.com", Provider::Fastmail),
    ("fastmail.fm", Provider::Fastmail),
];

/// The listed provider that owns the server `host`, if any: the host is one of the provider's
/// domains or a name under one (`imap.gmail.com`, `outlook.office365.com`). Case and a trailing dot
/// do not matter. This is the check that stops a certificate pin for a provider's own server:
/// settings that name `imap.gmail.com` are Gmail's whatever the `provider` field says, so changing
/// the port or typing the host by hand cannot turn the interceptor's certificate into a trusted one.
pub fn provider_for_host(host: &str) -> Option<Provider> {
    let host = host.trim().trim_end_matches('.').to_ascii_lowercase();
    SERVER_DOMAINS
        .iter()
        .find(|(domain, _)| {
            host == *domain
                || host
                    .strip_suffix(domain)
                    .is_some_and(|rest| rest.ends_with('.'))
        })
        .map(|(_, provider)| *provider)
}

impl Entry {
    /// IMAP settings for this provider. The user name is the whole address, which every listed
    /// provider accepts.
    pub(crate) fn imap(&self, username: String) -> ImapServer {
        ImapServer {
            host: self.imap_host.to_string(),
            port: self.imap_port,
            security: Security::Tls,
            username,
        }
    }

    /// SMTP settings for this provider.
    pub(crate) fn smtp(&self, username: String) -> SmtpServer {
        SmtpServer {
            host: self.smtp_host.to_string(),
            port: self.smtp_port,
            security: self.smtp_security,
            username,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_listed_domain_maps_to_its_provider() {
        let cases = [
            ("gmail.com", Provider::Gmail),
            ("googlemail.com", Provider::Gmail),
            ("outlook.com", Provider::Outlook),
            ("hotmail.com", Provider::Outlook),
            ("live.com", Provider::Outlook),
            ("icloud.com", Provider::ICloud),
            ("me.com", Provider::ICloud),
            ("mac.com", Provider::ICloud),
            ("fastmail.com", Provider::Fastmail),
            ("fastmail.fm", Provider::Fastmail),
        ];
        for (domain, provider) in cases {
            assert_eq!(
                lookup(domain).map(|e| e.provider),
                Some(provider),
                "{domain}"
            );
        }
        for other in [
            "example.org",
            "gmail.com.evil.example",
            "notgmail.com",
            "mail.gmail.com",
        ] {
            assert!(lookup(other).is_none(), "{other}");
        }
    }

    #[test]
    fn a_server_host_of_a_listed_provider_is_recognised_whatever_its_case() {
        for (host, provider) in [
            ("imap.gmail.com", Provider::Gmail),
            ("IMAP.Gmail.COM.", Provider::Gmail),
            ("gmail.com", Provider::Gmail),
            ("imap.googlemail.com", Provider::Gmail),
            ("outlook.office365.com", Provider::Outlook),
            ("smtp.office365.com", Provider::Outlook),
            ("imap.mail.me.com", Provider::ICloud),
            ("imap.fastmail.com", Provider::Fastmail),
        ] {
            assert_eq!(provider_for_host(host), Some(provider), "{host}");
        }
        // Every host the table itself uses is a provider's.
        for domain in ["gmail.com", "outlook.com", "icloud.com", "fastmail.com"] {
            let e = lookup(domain).unwrap();
            assert_eq!(provider_for_host(e.imap_host), Some(e.provider));
            assert_eq!(provider_for_host(e.smtp_host), Some(e.provider));
        }
        for other in [
            "mail.example.org",
            "gmail.com.evil.example",
            "notgmail.com",
            "imap.gmail.com.evil.example",
            "acme.com",
            "127.0.0.1",
            "",
        ] {
            assert_eq!(provider_for_host(other), None, "{other}");
        }
    }

    #[test]
    fn hosts_ports_and_security_match_the_providers_documentation() {
        let g = lookup("gmail.com").unwrap();
        assert_eq!((g.imap_host, g.imap_port), ("imap.gmail.com", 993));
        assert_eq!(
            (g.smtp_host, g.smtp_port, g.smtp_security),
            ("smtp.gmail.com", 465, Security::Tls)
        );
        let o = lookup("outlook.com").unwrap();
        assert_eq!((o.imap_host, o.imap_port), ("outlook.office365.com", 993));
        assert_eq!(
            (o.smtp_host, o.smtp_port, o.smtp_security),
            ("smtp.office365.com", 587, Security::StartTls)
        );
        let i = lookup("icloud.com").unwrap();
        assert_eq!((i.imap_host, i.imap_port), ("imap.mail.me.com", 993));
        assert_eq!(
            (i.smtp_host, i.smtp_port, i.smtp_security),
            ("smtp.mail.me.com", 587, Security::StartTls)
        );
        let f = lookup("fastmail.com").unwrap();
        assert_eq!((f.imap_host, f.imap_port), ("imap.fastmail.com", 993));
        assert_eq!(
            (f.smtp_host, f.smtp_port, f.smtp_security),
            ("smtp.fastmail.com", 465, Security::Tls)
        );
        assert_eq!(f.jmap, Some("https://api.fastmail.com/jmap/session"));
    }
}
