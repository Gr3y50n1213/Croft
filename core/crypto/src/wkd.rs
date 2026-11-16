// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! WKD, the Web Key Directory (draft-koch-openpgp-webkey-service, T41a): where a domain publishes its people's keys. Only the
//! addresses are computed here; the fetch (public addresses only, bounded) belongs to the core's network code (T41b).

use sha1::{Digest, Sha1};

use crate::{CryptoError, Result};

/// z-base-32's alphabet.
const ZBASE32: &[u8; 32] = b"ybndrfg8ejkmcpqxot1uwisza345h769";

/// The two places a WKD key may be: the advanced method first, then the direct one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WkdUrls {
    /// `https://openpgpkey.<domain>/.well-known/openpgpkey/<domain>/hu/<hash>?l=<local>`.
    pub advanced: String,
    /// `https://<domain>/.well-known/openpgpkey/hu/<hash>?l=<local>`.
    pub direct: String,
}

/// z-base-32 of `data` (RFC 6189), as WKD uses it.
pub fn zbase32(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len() * 8 / 5 + 1);
    let mut buffer: u32 = 0;
    let mut bits = 0;
    for &b in data {
        buffer = (buffer << 8) | u32::from(b);
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            out.push(char::from(ZBASE32[((buffer >> bits) & 31) as usize]));
        }
    }
    if bits > 0 {
        out.push(char::from(ZBASE32[((buffer << (5 - bits)) & 31) as usize]));
    }
    out
}

/// The WKD addresses for `address`. Only an ordinary address is taken: an ASCII local part without `/`, `?`, `#` or `%`, and a
/// domain name of letters, digits, `-` and `.` (no IP address, no port).
pub fn urls(address: &str) -> Result<WkdUrls> {
    let a = address.trim();
    let (local, domain) = a.rsplit_once('@').ok_or(CryptoError::BadAddress)?;
    let domain = domain.to_ascii_lowercase();
    let local_ok = !local.is_empty()
        && local.len() <= 64
        && local.bytes().all(|b| {
            b.is_ascii_graphic() && !matches!(b, b'/' | b'?' | b'#' | b'%' | b'@' | b'\\' | b'"')
        });
    let domain_ok = !domain.is_empty()
        && domain.len() <= 253
        && domain.contains('.')
        && domain.split('.').all(|l| {
            !l.is_empty()
                && l.len() <= 63
                && !l.starts_with('-')
                && !l.ends_with('-')
                && l.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
        })
        // A real top-level label (letters, or punycode): no IP address, whatever its spelling (`0x7f.1`, `0177.0.0.1`).
        && domain.rsplit('.').next().is_some_and(|tld| {
            tld.len() >= 2 && (tld.bytes().all(|b| b.is_ascii_alphabetic()) || tld.starts_with("xn--"))
        });
    if !local_ok || !domain_ok {
        return Err(CryptoError::BadAddress);
    }
    let hash = zbase32(&Sha1::digest(local.to_ascii_lowercase().as_bytes()));
    // `l=` carries the local part as written; the characters taken above need no escaping but `+` and `&`.
    let l: String = local
        .bytes()
        .map(|b| match b {
            b'+' | b'&' | b'=' => format!("%{b:02X}"),
            _ => char::from(b).to_string(),
        })
        .collect();
    Ok(WkdUrls {
        advanced: format!(
            "https://openpgpkey.{domain}/.well-known/openpgpkey/{domain}/hu/{hash}?l={l}"
        ),
        direct: format!("https://{domain}/.well-known/openpgpkey/hu/{hash}?l={l}"),
    })
}
