// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Which addresses this process may connect to (T26).
//!
//! On a phone every address is allowed: the person picks their own servers, and their own network
//! is theirs to reach. The Companion is a server several people share; there, a person who picks
//! "their" mail server could aim the Companion at a service on its private network. The Companion
//! therefore sets a policy once at start, and every connection the protocols make (IMAP, SMTP,
//! JMAP and discovery) is checked against it **after** the name lookup and before connecting, so a
//! name that resolves to a forbidden address (DNS rebinding included) is refused too.

use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, RwLock};

/// Decides whether an address may be connected to.
pub type AddressPolicy = Arc<dyn Fn(IpAddr) -> bool + Send + Sync>;

static POLICY: RwLock<Option<AddressPolicy>> = RwLock::new(None);

/// Sets the policy for the whole process (the Companion at start; never the phones).
pub fn set_address_policy(policy: AddressPolicy) {
    let mut p = POLICY
        .write()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    *p = Some(policy);
}

/// Whether `ip` may be connected to under the current policy (every address without one).
pub fn allowed(ip: IpAddr) -> bool {
    let p = POLICY
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    p.as_ref().is_none_or(|f| f(ip.to_canonical()))
}

/// The addresses of a lookup that may be connected to.
pub(crate) fn filter(addrs: Vec<SocketAddr>) -> Vec<SocketAddr> {
    addrs.into_iter().filter(|a| allowed(a.ip())).collect()
}

/// The message for a server whose every address is refused (no address in it).
pub(crate) const REFUSED: &str = "this server's address is not allowed on this Companion";

/// Addresses that belong to the machine or a private network: loopback, private (RFC 1918),
/// carrier-grade NAT, link-local, unique-local, multicast, unspecified, broadcast and the
/// documentation and benchmarking ranges. A public mail server is never one of these.
pub fn is_internal(ip: IpAddr) -> bool {
    match ip.to_canonical() {
        IpAddr::V4(a) => {
            let o = a.octets();
            a.is_loopback()
                || a.is_private()
                || a.is_link_local()
                || a.is_multicast()
                || a.is_unspecified()
                || a.is_broadcast()
                || a.is_documentation()
                || o[0] == 0
                || (o[0] == 100 && (64..128).contains(&o[1]))
                || (o[0] == 198 && (o[1] == 18 || o[1] == 19))
                || o[0] >= 240
        }
        IpAddr::V6(a) => {
            let s = a.segments();
            a.is_loopback()
                || a.is_unspecified()
                || a.is_multicast()
                || (s[0] & 0xfe00) == 0xfc00
                || (s[0] & 0xffc0) == 0xfe80
                || (s[0] == 0x2001 && s[1] == 0x0db8)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap_or(IpAddr::from([0, 0, 0, 0]))
    }

    #[test]
    fn internal_addresses_are_recognised() {
        for a in [
            "127.0.0.1",
            "10.1.2.3",
            "172.16.0.1",
            "192.168.1.1",
            "169.254.169.254",
            "100.64.0.1",
            "0.0.0.0",
            "224.0.0.1",
            "255.255.255.255",
            "::1",
            "::",
            "fd00::1",
            "fe80::1",
            "::ffff:127.0.0.1",
            "::ffff:10.0.0.1",
            "198.18.0.1",
        ] {
            assert!(is_internal(ip(a)), "{a}");
        }
        for a in [
            "1.1.1.1",
            "93.184.216.34",
            "2606:4700::1111",
            "::ffff:1.1.1.1",
        ] {
            assert!(!is_internal(ip(a)), "{a}");
        }
    }
}
