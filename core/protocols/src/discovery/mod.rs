// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Account discovery: from an email address to server settings that work (screens S02 to S04).
//!
//! # Two phases
//!
//! 1. **Find** ([`Discoverer::discover`]): looks for the settings. No password exists yet and
//!    nothing secret is sent. Only the person's own domain and, for a handful of well-known
//!    providers, the provider's own hosts are involved (see "What leaves the device").
//! 2. **Verify** ([`verify`]): signs in over IMAP with the settings that were found and the
//!    password the person typed, using the same adapter the rest of Croft uses. The outcome is
//!    either [`Verified`] or a [`VerifyError`] whose variant says what the screen should do:
//!    show "wrong password", offer to trust a self-signed certificate once, explain that the
//!    provider needs an app password, and so on.
//!
//! # Where it looks, and which answer wins
//!
//! In this order of preference ([`Probe`]):
//!
//! 1. the built-in provider table (Gmail, Outlook, iCloud, Fastmail);
//! 2. autoconfig XML at `https://autoconfig.<domain>/mail/config-v1.1.xml`;
//! 3. autoconfig XML at `https://<domain>/.well-known/autoconfig/mail/config-v1.1.xml`;
//! 4. SRV records (RFC 6186, and `_jmap._tcp` from RFC 8620);
//! 5. `https://<domain>/.well-known/jmap`.
//!
//! If the domain is in the provider table, that answer is used at once and nothing else is asked.
//! Otherwise the four network probes start **together**. "The first match wins" means the first
//! in the order above, not the first to answer: Croft waits for each probe in turn and takes the
//! first one that found usable settings, so a fast SRV answer never overrides a slower autoconfig
//! file, and an earlier probe that fails never holds up a later one that already succeeded. The
//! cost is that a slow earlier probe delays the result by at most its own timeout. When a probe
//! wins, the ones after it are cancelled and reported as skipped. If none wins, the error lists
//! why each failed in plain language ([`DiscoveryError`], [`ProbeFailure`]).
//!
//! # What leaves the device
//!
//! Discovery never asks Mozilla's ISPDB or any other third-party database, and never sends the
//! local part of the address (the part before the `@`) anywhere, including in autoconfig URLs
//! (Thunderbird's `?emailaddress=` is not used).
//!
//! - HTTPS, port 443 only, to `autoconfig.<domain>` and `<domain>`; a redirect is followed only
//!   to an `https` address on the same domain or a subdomain of it, at most 3 times. The JMAP
//!   probe follows none (the redirect target is the answer, and is only recorded).
//! - DNS queries for exactly `_imaps._tcp.<domain>`, `_imap._tcp.<domain>`,
//!   `_submissions._tcp.<domain>`, `_submission._tcp.<domain>`, `_jmap._tcp.<domain>`, and the
//!   addresses of the hosts the HTTPS requests go to. No MX query is made.
//! - Responses are capped at 64 KiB; each request is capped at 10 seconds and the whole search at
//!   20 seconds.
//! - **Verification** connects to the IMAP host that the settings name and sends the password
//!   there, over TLS. That host is the one the person's own domain (or the provider table)
//!   published, so it is "the person's own server" by definition, even when it has another
//!   name, as with a domain hosted by a mail provider. Nothing else is contacted.
//!
//! The network is reached through the [`Resolver`] and [`HttpFetch`] traits, so tests substitute
//! recording fakes (the `testing` module, behind the `test-fakes` cargo feature so release builds
//! do not carry it) and check exactly what would have been asked.

mod address;
mod autoconfig;
mod dns;
mod error;
mod find;
mod http;
mod net;
mod provider;
mod srv;
#[cfg(any(test, feature = "test-fakes"))]
pub mod testing;
mod verify;

pub use address::EmailAddress;
pub use dns::HickoryResolver;
pub use error::{DiscoveryError, Probe, ProbeFailure, ProbeState, ProbeUpdate};
pub use find::{Discoverer, Limits};
pub use http::HttpsFetcher;
pub use net::{
    FetchError, FetchLimits, HttpFetch, HttpResponse, HttpsUrl, NetFuture, ResolveError, Resolver,
    SrvRecord,
};
pub use verify::{verify, Verified, VerifyError, VerifyOptions};

use crate::imap::{ImapConfig, Security};
use std::fmt;

/// A large provider Croft knows by domain (see the provider table).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Provider {
    /// Gmail and Google Mail.
    Gmail,
    /// Outlook.com, Hotmail and Live.
    Outlook,
    /// iCloud Mail.
    ICloud,
    /// Fastmail.
    Fastmail,
}

/// Where the settings came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Source {
    /// The built-in provider table.
    ProviderTable,
    /// An autoconfig XML file on the person's domain.
    Autoconfig,
    /// SRV records on the person's domain.
    Srv,
    /// `/.well-known/jmap` on the person's domain.
    JmapWellKnown,
}

/// What kind of secret the server wants, as far as discovery can tell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthHint {
    /// An ordinary account password.
    Password,
    /// An app-specific password made in the provider's account settings.
    AppPassword,
    /// Sign-in through the provider's own page (OAuth 2).
    OAuth2,
    /// OAuth 2 sign-in, or an app password as the fallback (Gmail).
    OAuth2OrAppPassword,
}

/// An IMAP server found by discovery.
#[derive(Clone, PartialEq, Eq)]
pub struct ImapServer {
    /// Host name or IP address.
    pub host: String,
    /// TCP port.
    pub port: u16,
    /// Implicit TLS or STARTTLS; discovery never produces an unencrypted choice.
    pub security: Security,
    /// The login name, normally the whole address.
    pub username: String,
}

impl ImapServer {
    /// Adapter settings for this server (no pinned certificate yet).
    pub fn to_config(&self) -> ImapConfig {
        ImapConfig::new(
            self.host.clone(),
            self.port,
            self.security,
            self.username.clone(),
        )
    }
}

impl fmt::Debug for ImapServer {
    /// The login name is usually the address, so it is not printed.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ImapServer")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("security", &self.security)
            .field("username", &"***")
            .finish()
    }
}

/// An SMTP submission server found by discovery. Croft does not send mail yet; the settings are
/// kept for the task that does.
#[derive(Clone, PartialEq, Eq)]
pub struct SmtpServer {
    /// Host name or IP address.
    pub host: String,
    /// TCP port (465 for implicit TLS, 587 for STARTTLS, usually).
    pub port: u16,
    /// Implicit TLS or STARTTLS.
    pub security: Security,
    /// The login name.
    pub username: String,
}

impl fmt::Debug for SmtpServer {
    /// The login name is usually the address, so it is not printed.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SmtpServer")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("security", &self.security)
            .field("username", &"***")
            .finish()
    }
}

/// Settings found for an address. At least one of `imap` and `jmap` is set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveredSettings {
    /// The IMAP server. `None` only when the domain offers JMAP and nothing else Croft can use
    /// yet; [`verify`] then fails with [`VerifyError::JmapOnly`].
    pub imap: Option<ImapServer>,
    /// The SMTP submission server, when one was found.
    pub smtp: Option<SmtpServer>,
    /// A JMAP URL: a session resource, or `/.well-known/jmap` on the host. Resolved by the JMAP
    /// adapter (a later task).
    ///
    /// **The host in this URL is not checked against the person's domain.** A `/.well-known/jmap`
    /// redirect (RFC 8620 section 2.2) may point at any `https` host, and discovery only records
    /// it. The JMAP adapter must validate it the way a discovered IMAP host is treated: it is
    /// trusted only because the person's own domain published it, so show the host at setup, send
    /// no credential on a later redirect to another host, and apply the same certificate rules
    /// (no trust-on-first-use for a provider-table domain).
    pub jmap: Option<String>,
    /// Which probe produced these settings.
    pub source: Source,
    /// The provider, when the address is in the built-in table.
    pub provider: Option<Provider>,
    /// What kind of secret to ask for.
    pub auth_hint: AuthHint,
}
