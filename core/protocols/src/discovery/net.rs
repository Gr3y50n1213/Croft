// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The two doors discovery uses to reach the network, as traits, so tests can replace them and
//! record exactly what would have been sent.
//!
//! - [`Resolver`]: DNS. SRV records, and the addresses of a host name.
//! - [`HttpFetch`]: one HTTPS `GET`, nothing else.
//!
//! The real implementations are [`HickoryResolver`](super::dns::HickoryResolver) and
//! [`HttpsFetcher`](super::http::HttpsFetcher).

use super::address::is_host_name;
use std::future::Future;
use std::net::IpAddr;
use std::pin::Pin;
use std::time::Duration;

/// A boxed future, so the traits can be used as `dyn` without extra crates.
pub type NetFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// One SRV record (RFC 2782).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SrvRecord {
    /// Lower number is tried first.
    pub priority: u16,
    /// Share of the traffic among records of the same priority.
    pub weight: u16,
    /// TCP port.
    pub port: u16,
    /// Host name, lower case, without the trailing dot. Empty means the root name `.`, which
    /// RFC 2782 defines as "this service is not available here".
    pub target: String,
}

/// Why a DNS lookup returned nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResolveError {
    /// The name or the record type does not exist. An ordinary answer, not a failure.
    NotFound,
    /// No answer in time.
    TimedOut,
    /// The name service could not be reached or gave an error.
    Failed,
}

/// DNS, as discovery needs it. Names are lower-case ASCII without a trailing dot, for example
/// `_imaps._tcp.example.org`. Implementations must treat them as fully qualified (no search
/// list), so no query ever carries a local suffix.
pub trait Resolver: Send + Sync {
    /// SRV records for a service name such as `_imaps._tcp.example.org`.
    fn lookup_srv<'a>(
        &'a self,
        name: &'a str,
    ) -> NetFuture<'a, Result<Vec<SrvRecord>, ResolveError>>;

    /// A and AAAA addresses of a host.
    fn lookup_ip<'a>(&'a self, host: &'a str) -> NetFuture<'a, Result<Vec<IpAddr>, ResolveError>>;
}

/// An `https` URL Croft is willing to request: a DNS host name (never an IP address), no user
/// name or password, and only printable ASCII in the path and query. Built only through
/// [`HttpsUrl::parse`] or [`HttpsUrl::new`], so a value of this type is safe to put in a request
/// line.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct HttpsUrl {
    host: String,
    port: u16,
    path_and_query: String,
}

impl HttpsUrl {
    /// `https://host/path` on port 443. `path` must start with `/`.
    pub fn new(host: &str, path: &str) -> Option<Self> {
        Self::with_port(host, 443, path)
    }

    /// Like [`new`](Self::new) with another port. Discovery itself only ever uses 443; this
    /// exists so tests can talk to a local server.
    pub(crate) fn with_port(host: &str, port: u16, path: &str) -> Option<Self> {
        let host = host.to_ascii_lowercase();
        if !is_host_name(&host) || port == 0 || !path.starts_with('/') {
            return None;
        }
        if !path.bytes().all(|b| (0x21..=0x7e).contains(&b)) {
            return None;
        }
        Some(Self {
            host,
            port,
            path_and_query: path.to_string(),
        })
    }

    /// Reads an absolute `https://` URL. A fragment is dropped. Refuses other schemes, user
    /// names, IP addresses, ports other than 443, and anything with spaces or control characters.
    pub fn parse(text: &str) -> Option<Self> {
        Self::parse_inner(text, false)
    }

    /// Like [`parse`](Self::parse) but accepts any port. Used only for a JMAP address that is
    /// recorded and not requested by discovery.
    pub fn parse_allowing_port(text: &str) -> Option<Self> {
        Self::parse_inner(text, true)
    }

    fn parse_inner(text: &str, any_port: bool) -> Option<Self> {
        let rest = text.strip_prefix("https://")?;
        let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
        let (authority, tail) = rest.split_at(end);
        if authority.contains('@') || authority.contains('[') {
            return None;
        }
        let (host, port) = match authority.split_once(':') {
            Some((host, "443")) => (host, 443),
            Some((host, port)) if any_port => {
                if !port.bytes().all(|b| b.is_ascii_digit()) {
                    return None;
                }
                (host, port.parse::<u16>().ok()?)
            }
            Some(_) => return None,
            None => (authority, 443),
        };
        let tail = tail.split('#').next().unwrap_or("");
        let path = if tail.is_empty() {
            "/".to_string()
        } else if tail.starts_with('?') {
            format!("/{tail}")
        } else {
            tail.to_string()
        };
        Self::with_port(host, port, &path)
    }

    /// The host name, lower case.
    pub fn host(&self) -> &str {
        &self.host
    }

    /// The TCP port.
    pub fn port(&self) -> u16 {
        self.port
    }

    /// Path and query, starting with `/`.
    pub fn path_and_query(&self) -> &str {
        &self.path_and_query
    }

    /// The URL as text, without a default port.
    pub fn as_text(&self) -> String {
        if self.port == 443 {
            format!("https://{}{}", self.host, self.path_and_query)
        } else {
            format!("https://{}:{}{}", self.host, self.port, self.path_and_query)
        }
    }

    /// True when the host is `domain` or a subdomain of it.
    pub fn is_within(&self, domain: &str) -> bool {
        self.host == domain
            || self
                .host
                .strip_suffix(domain)
                .is_some_and(|prefix| prefix.ends_with('.'))
    }
}

/// How much one request may take and read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FetchLimits {
    /// The whole request: look up, connect, secure, send, read.
    pub timeout: Duration,
    /// Most body bytes read. A longer body is [`FetchError::TooLarge`].
    pub max_body: usize,
}

/// A response, reduced to what discovery reads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpResponse {
    /// Status code.
    pub status: u16,
    /// The `Location` header, for redirects.
    pub location: Option<String>,
    /// The body (empty for redirects).
    pub body: Vec<u8>,
}

/// Why a request got no response.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FetchError {
    /// The host name does not exist.
    NameNotFound,
    /// The name service failed or timed out.
    Dns,
    /// The server could not be reached (refused, unreachable, reset).
    Connect,
    /// The secure connection was refused: the certificate was not accepted or TLS is not offered.
    Tls,
    /// No complete answer in time.
    TimedOut,
    /// The body was longer than the limit.
    TooLarge,
    /// The reply was not valid HTTP.
    Protocol,
}

/// One HTTPS `GET`. It must not follow redirects (discovery decides which ones are followed),
/// send cookies or any header that identifies the person, and must obey the limits.
pub trait HttpFetch: Send + Sync {
    /// Requests `url` and returns the response, or why there was none.
    fn get<'a>(
        &'a self,
        url: &'a HttpsUrl,
        limits: FetchLimits,
    ) -> NetFuture<'a, Result<HttpResponse, FetchError>>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_plain_https_urls() {
        let u = HttpsUrl::parse("https://Autoconfig.Example.org/mail/config-v1.1.xml").unwrap();
        assert_eq!(u.host(), "autoconfig.example.org");
        assert_eq!(u.path_and_query(), "/mail/config-v1.1.xml");
        assert_eq!(
            u.as_text(),
            "https://autoconfig.example.org/mail/config-v1.1.xml"
        );
        let root = HttpsUrl::parse("https://example.org").unwrap();
        assert_eq!(root.path_and_query(), "/");
        let q = HttpsUrl::parse("https://example.org?x=1#frag").unwrap();
        assert_eq!(q.path_and_query(), "/?x=1");
        assert!(HttpsUrl::parse("https://example.org:443/a").is_some());
        let p =
            HttpsUrl::parse_allowing_port("https://mail.example.org:8443/jmap/session").unwrap();
        assert_eq!((p.host(), p.port()), ("mail.example.org", 8443));
        assert_eq!(p.as_text(), "https://mail.example.org:8443/jmap/session");
        assert!(HttpsUrl::parse_allowing_port("https://example.org:99999/").is_none());
        assert!(HttpsUrl::parse_allowing_port("https://example.org:80a/").is_none());
    }

    #[test]
    fn refuses_everything_that_is_not_a_plain_https_host() {
        for bad in [
            "http://example.org/",
            "ftp://example.org/",
            "//example.org/",
            "/path",
            "https://user:pw@example.org/",
            "https://example.org:8443/",
            "https://example.org:/",
            "https://127.0.0.1/",
            "https://[::1]/",
            "https://exa mple.org/",
            "https://example.org/a b",
            "https://example.org/a\r\nHost: evil",
            "https://example.org/\u{e9}",
            "https:///path",
            "https://-bad.example/",
        ] {
            assert!(HttpsUrl::parse(bad).is_none(), "{bad:?}");
        }
    }

    #[test]
    fn within_means_the_domain_or_a_subdomain() {
        let u = |h: &str| HttpsUrl::new(h, "/").unwrap();
        assert!(u("example.org").is_within("example.org"));
        assert!(u("www.example.org").is_within("example.org"));
        assert!(u("a.b.example.org").is_within("example.org"));
        assert!(!u("evilexample.org").is_within("example.org"));
        assert!(!u("example.org.evil.example").is_within("example.org"));
        assert!(!u("example.net").is_within("example.org"));
    }
}
