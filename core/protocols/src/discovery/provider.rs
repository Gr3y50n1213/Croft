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
use crate::host::clean_host;
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

/// The mailbox domains of the listed providers and the entry each one uses. [`lookup`] reads this
/// table, and a test checks that [`provider_for_host`] recognises every domain and every host in it.
static LISTED: [(&str, &Entry); 10] = [
    ("gmail.com", &GMAIL),
    ("googlemail.com", &GMAIL),
    ("outlook.com", &OUTLOOK),
    ("hotmail.com", &OUTLOOK),
    ("live.com", &OUTLOOK),
    ("icloud.com", &ICLOUD),
    ("me.com", &ICLOUD),
    ("mac.com", &ICLOUD),
    ("fastmail.com", &FASTMAIL),
    ("fastmail.fm", &FASTMAIL),
];

/// The entry for a lower-case ASCII domain, if it belongs to one of the listed providers.
pub(crate) fn lookup(domain: &str) -> Option<&'static Entry> {
    LISTED
        .iter()
        .find(|(listed, _)| *listed == domain)
        .map(|(_, entry)| *entry)
}

/// The domains whose servers are a listed provider's own: the mailbox domains of [`LISTED`] plus
/// the domains the providers' mail servers sit under (`outlook.office365.com`, `outlook.office.com`,
/// `messagingengine.com`, which Fastmail's servers are also reachable under). A host equal to one of
/// these, or a name under one, is that provider's.
///
/// The Android app keeps a copy of this list for its sign-in screens (`ProviderHosts.kt`); a test
/// in this crate fails when the two differ.
const SERVER_DOMAINS: [(&str, Provider); 13] = [
    ("gmail.com", Provider::Gmail),
    ("googlemail.com", Provider::Gmail),
    ("outlook.com", Provider::Outlook),
    ("hotmail.com", Provider::Outlook),
    ("live.com", Provider::Outlook),
    ("office365.com", Provider::Outlook),
    ("outlook.office.com", Provider::Outlook),
    ("icloud.com", Provider::ICloud),
    ("me.com", Provider::ICloud),
    ("mac.com", Provider::ICloud),
    ("fastmail.com", Provider::Fastmail),
    ("fastmail.fm", Provider::Fastmail),
    ("messagingengine.com", Provider::Fastmail),
];

/// The listed provider that owns the server `host`, if any: the host is one of the provider's
/// domains or a name under one (`imap.gmail.com`, `outlook.office365.com`). The host is cleaned up
/// first with the same function the IMAP connection uses (surrounding spaces, the brackets of
/// `[imap.gmail.com]`, a trailing dot and case do not matter). This is the check that stops a certificate pin for a provider's own server:
/// settings that name `imap.gmail.com` are Gmail's whatever the `provider` field says, so changing
/// the port or typing the host by hand cannot turn the interceptor's certificate into a trusted one.
pub fn provider_for_host(host: &str) -> Option<Provider> {
    let host = clean_host(host);
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
    fn brackets_a_trailing_dot_spaces_and_case_do_not_hide_a_providers_host() {
        // The same spellings the IMAP connection cleans up (`crate::host::clean_host`): what it would
        // connect to as Gmail's server must be recognised as Gmail's here, or a pin could be
        // accepted for it.
        for host in [
            "[imap.gmail.com]",
            "imap.gmail.com.",
            "[imap.gmail.com.]",
            "[IMAP.Gmail.COM]",
            " imap.gmail.com ",
            "[gmail.com]",
        ] {
            assert_eq!(provider_for_host(host), Some(Provider::Gmail), "{host:?}");
        }
        assert_eq!(
            provider_for_host("[outlook.office365.com.]"),
            Some(Provider::Outlook)
        );
        // A name that only looks like one is still somebody else's.
        for other in ["[imap.gmail.com.evil.example]", "[notgmail.com]", "[::1]"] {
            assert_eq!(provider_for_host(other), None, "{other:?}");
        }
    }

    #[test]
    fn the_providers_other_server_domains_are_recognised() {
        for (host, provider) in [
            ("outlook.office.com", Provider::Outlook),
            ("imap.outlook.office.com", Provider::Outlook),
            ("imap.messagingengine.com", Provider::Fastmail),
            ("messagingengine.com", Provider::Fastmail),
        ] {
            assert_eq!(provider_for_host(host), Some(provider), "{host}");
        }
        for other in [
            "office.com",
            "evilmessagingengine.com",
            "messagingengine.org",
        ] {
            assert_eq!(provider_for_host(other), None, "{other}");
        }
    }

    /// The lists must agree: everything `lookup` and the table's entries name (the mailbox domains,
    /// the IMAP and SMTP hosts, the JMAP host) is a provider's own server for `provider_for_host`,
    /// and says the same provider. A domain added to the table and not to `SERVER_DOMAINS` would
    /// otherwise be a provider whose servers can take a certificate pin.
    #[test]
    fn everything_the_table_knows_is_matched_by_provider_for_host() {
        for (domain, entry) in LISTED.iter() {
            assert_eq!(lookup(domain).map(|e| e.provider), Some(entry.provider));
            let hosts = [
                (*domain).to_string(),
                format!("imap.{domain}"),
                entry.imap_host.to_string(),
                entry.smtp_host.to_string(),
            ];
            for host in hosts {
                assert_eq!(
                    provider_for_host(&host),
                    Some(entry.provider),
                    "{host} (from {domain})"
                );
            }
            if let Some(url) = entry.jmap {
                let host = url
                    .strip_prefix("https://")
                    .and_then(|rest| rest.split('/').next())
                    .unwrap_or_default();
                assert_eq!(provider_for_host(host), Some(entry.provider), "{url}");
            }
        }
        // And the other way round: the server list holds every mailbox domain of the table.
        for (domain, entry) in LISTED.iter() {
            assert!(
                SERVER_DOMAINS
                    .iter()
                    .any(|(d, p)| d == domain && *p == entry.provider),
                "{domain} is not in SERVER_DOMAINS"
            );
        }
    }

    /// The Kotlin copy (`ProviderHosts.kt`) is read as text: it must hold every entry of
    /// `SERVER_DOMAINS` with the provider's name, and nothing else, so the sign-in screens and the
    /// core cannot disagree about which hosts are a provider's.
    #[test]
    fn the_kotlin_copy_of_the_server_domains_is_the_same_list() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(
            "../../android/app/src/main/kotlin/com/example/croft/mail/onboarding/ProviderHosts.kt",
        );
        let kotlin = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("cannot read ProviderHosts.kt at {}: {e}", path.display()));
        for (domain, provider) in SERVER_DOMAINS {
            let name = match provider {
                Provider::Gmail => "GMAIL",
                Provider::Outlook => "OUTLOOK",
                Provider::ICloud => "I_CLOUD",
                Provider::Fastmail => "FASTMAIL",
            };
            let entry = format!("\"{domain}\" to KnownProvider.{name}");
            assert!(
                kotlin.contains(&entry),
                "ProviderHosts.kt has no `{entry}` (keep it in step with SERVER_DOMAINS)"
            );
        }
        // No entry in Kotlin that the core does not have.
        assert_eq!(
            kotlin.matches("\" to KnownProvider.").count(),
            SERVER_DOMAINS.len(),
            "ProviderHosts.kt has entries that are not in SERVER_DOMAINS"
        );
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
