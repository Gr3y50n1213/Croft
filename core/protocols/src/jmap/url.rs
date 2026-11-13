// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The URLs the JMAP adapter talks to: `https`, or `http` for loopback test servers only.
//!
//! Only what the adapter needs: an origin (scheme, host, port) and a path with a query. No user
//! names, no fragments, no spaces or control characters, so a [`Url`] is safe to put in a request
//! line. Relative references (as in `"apiUrl": "/jmap/"`) are resolved against the session URL.

/// An absolute `http` or `https` URL with a host name or IP address.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Url {
    /// True for `https`.
    pub secure: bool,
    /// Lower-case host name, or an IP literal (an IPv6 literal without brackets).
    pub host: String,
    /// TCP port.
    pub port: u16,
    /// Path and query, starting with `/`.
    pub path_and_query: String,
}

/// True for the names plain `http` is allowed for (resolution is checked again at connect).
pub(crate) fn is_loopback_name(host: &str) -> bool {
    let h = host.trim().trim_start_matches('[').trim_end_matches(']');
    h == "127.0.0.1" || h == "::1" || h.eq_ignore_ascii_case("localhost")
}

fn host_is_valid(host: &str) -> bool {
    if host.is_empty() || host.len() > 253 {
        return false;
    }
    if host.contains(':') {
        // An IPv6 literal.
        return host.parse::<std::net::Ipv6Addr>().is_ok();
    }
    host.split('.').all(|label| {
        !label.is_empty()
            && label.len() <= 63
            && !label.starts_with('-')
            && !label.ends_with('-')
            && label
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-')
    })
}

fn path_is_valid(path: &str) -> bool {
    path.starts_with('/') && path.bytes().all(|b| (0x21..=0x7e).contains(&b))
}

impl Url {
    /// Reads an absolute `http://` or `https://` URL. A fragment is dropped. Returns `None` for
    /// any other scheme, for a user name or password, and for anything with spaces or control
    /// characters.
    pub fn parse(text: &str) -> Option<Self> {
        let (secure, rest) = if let Some(r) = text.strip_prefix("https://") {
            (true, r)
        } else {
            (false, text.strip_prefix("http://")?)
        };
        let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
        let (authority, tail) = rest.split_at(end);
        if authority.contains('@') {
            return None;
        }
        let (host, port_text) = if let Some(inner) = authority.strip_prefix('[') {
            let (host, after) = inner.split_once(']')?;
            // Only `:port` may follow the brackets (T37 review: `[addr]x.example` named another host).
            if !after.is_empty() && !after.starts_with(':') {
                return None;
            }
            (host, after.strip_prefix(':'))
        } else {
            match authority.split_once(':') {
                Some((h, p)) => (h, Some(p)),
                None => (authority, None),
            }
        };
        let port = match port_text {
            None => {
                if secure {
                    443
                } else {
                    80
                }
            }
            Some(p) if !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()) => {
                p.parse::<u16>().ok().filter(|p| *p != 0)?
            }
            Some(_) => return None,
        };
        let host = host.to_ascii_lowercase();
        if !host_is_valid(&host) {
            return None;
        }
        let tail = tail.split('#').next().unwrap_or("");
        let path = if tail.is_empty() {
            "/".to_string()
        } else if tail.starts_with('?') {
            format!("/{tail}")
        } else {
            tail.to_string()
        };
        if !path_is_valid(&path) {
            return None;
        }
        Some(Self {
            secure,
            host,
            port,
            path_and_query: path,
        })
    }

    /// Resolves a reference the way the session resource uses them: an absolute URL, an absolute
    /// path (`/jmap/`) or a relative path.
    pub fn resolve(&self, reference: &str) -> Option<Self> {
        if reference.starts_with("https://") || reference.starts_with("http://") {
            return Self::parse(reference);
        }
        let reference = reference.split('#').next().unwrap_or("");
        // `//host/x` names another host; it is never read as a path (T38a review).
        if reference.starts_with("//") {
            return None;
        }
        let path = if reference.starts_with('/') {
            reference.to_string()
        } else {
            let base = self.path_and_query.split('?').next().unwrap_or("/");
            let dir = &base[..=base.rfind('/').unwrap_or(0)];
            format!("{dir}{reference}")
        };
        if !path_is_valid(&path) {
            return None;
        }
        Some(Self {
            path_and_query: path,
            ..self.clone()
        })
    }

    /// The same path and query on another URL's origin. The adapter uses this for the addresses
    /// the session resource names, so credentials only ever go to the origin the person set up.
    pub fn on_origin_of(&self, origin: &Self) -> Self {
        Self {
            secure: origin.secure,
            host: origin.host.clone(),
            port: origin.port,
            path_and_query: self.path_and_query.clone(),
        }
    }

    /// Same scheme, host and port.
    pub fn same_origin(&self, other: &Self) -> bool {
        self.secure == other.secure && self.host == other.host && self.port == other.port
    }

    /// The value for a `Host` header.
    pub fn host_header(&self) -> String {
        let host = if self.host.contains(':') {
            format!("[{}]", self.host)
        } else {
            self.host.clone()
        };
        let default = if self.secure { 443 } else { 80 };
        if self.port == default {
            host
        } else {
            format!("{host}:{}", self.port)
        }
    }

    /// True when the URL may be used: `https`, or `http` to a loopback host.
    pub fn is_allowed(&self) -> bool {
        self.secure || is_loopback_name(&self.host)
    }
}

/// Percent-encodes everything except the unreserved characters, for a value placed in a path.
pub(crate) fn percent_encode(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for b in text.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_hosts_ports_and_paths() {
        let u = Url::parse("https://Mail.Example.org/.well-known/jmap").unwrap();
        assert!(u.secure && u.host == "mail.example.org" && u.port == 443);
        assert_eq!(u.path_and_query, "/.well-known/jmap");
        let u = Url::parse("http://127.0.0.1:18080").unwrap();
        assert!(!u.secure && u.port == 18080 && u.path_and_query == "/");
        let u = Url::parse("https://[::1]:8443/x?y=1#frag").unwrap();
        assert_eq!((u.host.as_str(), u.port), ("::1", 8443));
        assert_eq!(u.path_and_query, "/x?y=1");
        assert_eq!(u.host_header(), "[::1]:8443");
    }

    #[test]
    fn refuses_unsafe_urls() {
        for bad in [
            "ftp://x.org/",
            "https://user:pw@x.org/",
            "https://x.org:0/",
            "https://x.org:99999/",
            "https://x.org/a b",
            "https://x.org/\r\nHost: evil",
            // T37 review: nothing but `:port` after an IPv6 literal.
            "https://[2606:4700::1111]x.bank.example:443/u",
            "https://[::1]evil/",
            "https://-x.org/",
            "https://x..org/",
            "https:///path",
            "mail.example.org",
        ] {
            assert!(Url::parse(bad).is_none(), "{bad}");
        }
    }

    #[test]
    fn resolves_references_against_the_session_url() {
        let base = Url::parse("https://mail.example.org/jmap/session").unwrap();
        assert_eq!(base.resolve("/jmap/").unwrap().path_and_query, "/jmap/");
        assert_eq!(base.resolve("api").unwrap().path_and_query, "/jmap/api");
        let abs = base.resolve("https://api.example.org/x").unwrap();
        assert_eq!(abs.host, "api.example.org");
        assert!(base.resolve("/a b").is_none());
        let moved = abs.on_origin_of(&base);
        assert!(moved.same_origin(&base) && moved.path_and_query == "/x");
    }

    #[test]
    fn plain_http_is_for_loopback_only() {
        assert!(Url::parse("http://127.0.0.1:1/").unwrap().is_allowed());
        assert!(Url::parse("http://localhost/").unwrap().is_allowed());
        assert!(!Url::parse("http://mail.example.org/").unwrap().is_allowed());
        assert!(Url::parse("https://mail.example.org/")
            .unwrap()
            .is_allowed());
    }

    #[test]
    fn percent_encoding_keeps_unreserved_characters() {
        assert_eq!(percent_encode("abc-1_2.3~"), "abc-1_2.3~");
        assert_eq!(percent_encode("a b/c"), "a%20b%2Fc");
    }
}
