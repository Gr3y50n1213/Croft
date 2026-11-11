// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Reading an email address the way a person types it.

use super::error::DiscoveryError;
use std::fmt;

/// An email address split into its two halves. The domain is stored lower-case in its ASCII
/// (punycode) form, which is the only form that is ever put in a DNS query or a URL.
///
/// The local part (before the `@`) never leaves the device in a lookup: it is used only to fill
/// in the user name once the server is known. `Debug` shows the domain and hides the local part.
#[derive(Clone, PartialEq, Eq)]
pub struct EmailAddress {
    local: String,
    domain: String,
}

impl EmailAddress {
    /// Reads `local@domain`. Surrounding spaces are ignored. Returns
    /// [`DiscoveryError::InvalidAddress`] for anything that is not a plausible address: no `@`,
    /// an empty or over-long local part, a domain without a dot, an IP address in brackets, or a
    /// domain with characters that cannot be in a host name.
    pub fn parse(text: &str) -> Result<Self, DiscoveryError> {
        let text = text.trim();
        let (local, domain) = text
            .rsplit_once('@')
            .ok_or(DiscoveryError::InvalidAddress)?;
        if local.is_empty()
            || local.len() > 64
            || local
                .chars()
                .any(|c| c.is_control() || c.is_whitespace() || c == '@')
        {
            return Err(DiscoveryError::InvalidAddress);
        }
        let domain = normalise_domain(domain).ok_or(DiscoveryError::InvalidAddress)?;
        Ok(Self {
            local: local.to_string(),
            domain,
        })
    }

    /// The part before the `@`, exactly as typed.
    pub fn local_part(&self) -> &str {
        &self.local
    }

    /// The domain, lower-case ASCII.
    pub fn domain(&self) -> &str {
        &self.domain
    }

    /// The whole address, local part as typed and domain in its ASCII form.
    pub fn full(&self) -> String {
        format!("{}@{}", self.local, self.domain)
    }
}

impl fmt::Debug for EmailAddress {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "EmailAddress(***@{})", self.domain)
    }
}

/// Lower-cases, converts to ASCII (IDNA) and checks every label. `None` when it is not a name
/// that can be looked up.
fn normalise_domain(domain: &str) -> Option<String> {
    let domain = domain.trim().trim_end_matches('.');
    if domain.is_empty() || domain.starts_with('[') {
        return None;
    }
    let ascii = idna::domain_to_ascii(domain).ok()?;
    if !is_host_name(&ascii) || !ascii.contains('.') {
        return None;
    }
    Some(ascii)
}

/// True for a syntactically valid DNS host name: labels of 1 to 63 letters, digits and hyphens,
/// not starting or ending with a hyphen, 253 characters at most, and not ending in an all-digit
/// label (that is an IP address). Lower-case only, so values that
/// pass are already in the form the rest of the module compares.
pub(crate) fn is_host_name(name: &str) -> bool {
    if name.is_empty() || name.len() > 253 {
        return false;
    }
    // A name whose last label is all digits is an IP address (or a typo for one), not a host.
    if name
        .rsplit('.')
        .next()
        .is_some_and(|last| last.bytes().all(|b| b.is_ascii_digit()))
    {
        return false;
    }
    name.split('.').all(|label| {
        !label.is_empty()
            && label.len() <= 63
            && !label.starts_with('-')
            && !label.ends_with('-')
            && label
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_and_normalises() {
        let a = EmailAddress::parse("  Alice.Smith@Example.ORG ").unwrap();
        assert_eq!(a.local_part(), "Alice.Smith");
        assert_eq!(a.domain(), "example.org");
        assert_eq!(a.full(), "Alice.Smith@example.org");
    }

    #[test]
    fn uses_the_last_at_sign() {
        let a = EmailAddress::parse("odd@name@example.org").unwrap_err();
        assert_eq!(a, DiscoveryError::InvalidAddress); // '@' in the local part is refused
    }

    #[test]
    fn internationalised_domains_become_punycode() {
        let a = EmailAddress::parse("jo@b\u{fc}cher.example").unwrap();
        assert_eq!(a.domain(), "xn--bcher-kva.example");
    }

    #[test]
    fn rejects_things_that_are_not_addresses() {
        for bad in [
            "",
            "alice",
            "@example.org",
            "alice@",
            "alice@localhost",
            "alice@[127.0.0.1]",
            "alice@exa mple.org",
            "alice@-example.org",
            "alice@example..org",
            "a b@example.org",
            "alice@exam_ple.org",
            "alice@example.org/evil",
            "alice@10.0.0.1",
        ] {
            assert_eq!(
                EmailAddress::parse(bad).unwrap_err(),
                DiscoveryError::InvalidAddress,
                "{bad:?}"
            );
        }
        let long_local = format!("{}@example.org", "a".repeat(65));
        assert!(EmailAddress::parse(&long_local).is_err());
        let long_label = format!("a@{}.org", "b".repeat(64));
        assert!(EmailAddress::parse(&long_label).is_err());
    }

    #[test]
    fn debug_hides_the_local_part() {
        let a = EmailAddress::parse("alice@example.org").unwrap();
        let shown = format!("{a:?}");
        assert!(!shown.contains("alice"));
        assert!(shown.contains("example.org"));
    }
}
