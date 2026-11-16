// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Autocrypt Level 1 headers (T41a): a sender's key, carried in every message they send. What to remember from it (and when
//! to prefer encryption) is T41b's; here the header is only read and written.

use base64::Engine;
use pgp::composed::{SignedKeyDetails, SignedPublicKey};
use pgp::types::Tag;

use crate::keys::{self, EncryptionKey};
use crate::{CryptoError, Result};

/// The most key data an Autocrypt header may carry, decoded.
pub const MAX_KEYDATA: usize = 64 * 1024;

/// A sender's Autocrypt header.
#[derive(Clone)]
pub struct AutocryptHeader {
    /// The sender's address, lower-case (the `From` address it was checked against).
    pub addr: String,
    /// `prefer-encrypt=mutual`.
    pub prefer_mutual: bool,
    /// The key.
    pub cert: SignedPublicKey,
}

impl std::fmt::Debug for AutocryptHeader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AutocryptHeader")
            .field("prefer_mutual", &self.prefer_mutual)
            .finish_non_exhaustive()
    }
}

/// Reads an `Autocrypt:` header value of a message from `from`, at `now`. Refused ([`CryptoError::BadHeader`]) when `addr` is
/// not `from`, when `addr` or `keydata` is missing or given twice, or when an attribute Croft does not know is not marked
/// optional (`_name`); [`CryptoError::NoUsableKey`] when the key cannot be encrypted to.
pub fn parse_header(value: &str, from: &str, now: u32) -> Result<AutocryptHeader> {
    if value.len() > MAX_KEYDATA * 2 {
        return Err(CryptoError::TooLarge);
    }
    let mut addr: Option<String> = None;
    let mut prefer_mutual = false;
    let mut keydata: Option<String> = None;
    for attr in value.split(';') {
        let attr = attr.trim();
        if attr.is_empty() {
            continue;
        }
        let (name, val) = attr.split_once('=').ok_or(CryptoError::BadHeader)?;
        let name = name.trim().to_ascii_lowercase();
        let val = val.trim();
        match name.as_str() {
            "addr" => {
                // A bare address only, compared in ASCII case only (T41a review).
                let bare = keys::bare_address(val).ok_or(CryptoError::BadHeader)?;
                if addr.replace(bare).is_some() {
                    return Err(CryptoError::BadHeader);
                }
            }
            "prefer-encrypt" => prefer_mutual = val.eq_ignore_ascii_case("mutual"),
            "keydata" => {
                if keydata
                    .replace(val.chars().filter(|c| !c.is_whitespace()).collect())
                    .is_some()
                {
                    return Err(CryptoError::BadHeader);
                }
            }
            n if n.starts_with('_') => {}
            _ => return Err(CryptoError::BadHeader),
        }
    }
    let addr = addr.ok_or(CryptoError::BadHeader)?;
    // `from` must be the one mailbox of the message's `From`, as the core's mail parser read it: a bare address. Anything else
    // (a name, a comment, a list) is refused, so a crafted header can never match another address (T41a review).
    let from = keys::bare_address(from).ok_or(CryptoError::BadHeader)?;
    if addr != from {
        return Err(CryptoError::BadHeader);
    }
    let data = keydata.ok_or(CryptoError::BadHeader)?;
    if data.len() > MAX_KEYDATA * 4 / 3 + 4 {
        return Err(CryptoError::TooLarge);
    }
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(data.as_bytes())
        .map_err(|_| CryptoError::BadHeader)?;
    if bytes.len() > MAX_KEYDATA {
        return Err(CryptoError::TooLarge);
    }
    let cert = keys::read_public(&bytes)?;
    if keys::usable(&cert, now).encryption.is_empty() {
        return Err(CryptoError::NoUsableKey);
    }
    Ok(AutocryptHeader {
        addr,
        prefer_mutual,
        cert,
    })
}

/// The `Autocrypt:` header value for `addr` with `cert` at `now`: the minimal key (the primary key, the user IDs for `addr`
/// with their self-signatures, the usable encryption subkeys), base64 with spaces every 76 characters so it can be folded.
pub fn header(addr: &str, cert: &SignedPublicKey, prefer_mutual: bool, now: u32) -> Result<String> {
    let addr = keys::address_of(addr).ok_or(CryptoError::BadAddress)?;
    let usable = keys::usable(cert, now);
    if usable.encryption.is_empty() {
        return Err(CryptoError::NoUsableKey);
    }
    let primary = &cert.primary_key;
    let users: Vec<_> = cert
        .details
        .users
        .iter()
        .filter(|u| u.id.as_str().and_then(keys::address_of).as_deref() == Some(addr.as_str()))
        .map(|u| {
            let mut u = u.clone();
            // Only the owner's own signatures (no third-party certifications).
            u.signatures
                .retain(|s| s.verify_certification(primary, Tag::UserId, &u.id).is_ok());
            u
        })
        .filter(|u| !u.signatures.is_empty())
        .collect();
    if users.is_empty() {
        return Err(CryptoError::BadAddress);
    }
    let enc_fps: Vec<_> = usable
        .encryption
        .iter()
        .filter_map(|k| match k {
            EncryptionKey::Subkey(s) => Some(pgp::types::KeyDetails::fingerprint(*s)),
            EncryptionKey::Primary(_) => None,
        })
        .collect();
    // Only signatures that verify are carried (T41a review): the subkeys' bindings, and the owner's own direct-key and revocation
    // signatures.
    let subkeys: Vec<_> = cert
        .public_subkeys
        .iter()
        .filter(|s| enc_fps.contains(&pgp::types::KeyDetails::fingerprint(&s.key)))
        .map(|s| {
            let mut s = s.clone();
            s.signatures
                .retain(|sig| sig.verify_subkey_binding(primary, &s.key).is_ok());
            s
        })
        .collect();
    let own = |sigs: &[pgp::packet::Signature]| -> Vec<pgp::packet::Signature> {
        sigs.iter()
            .filter(|s| s.verify_key(primary).is_ok())
            .cloned()
            .collect()
    };
    let minimal = SignedPublicKey::new(
        primary.clone(),
        SignedKeyDetails::new(
            own(&cert.details.revocation_signatures),
            own(&cert.details.direct_signatures),
            users,
            vec![],
        ),
        subkeys,
    );
    let bytes = keys::public_bytes(&minimal)?;
    let b64 = base64::engine::general_purpose::STANDARD.encode(bytes);
    let folded: Vec<&str> = b64
        .as_bytes()
        .chunks(76)
        .map(|c| std::str::from_utf8(c).unwrap_or_default())
        .collect();
    let prefer = if prefer_mutual {
        " prefer-encrypt=mutual;"
    } else {
        ""
    };
    Ok(format!("addr={addr};{prefer} keydata={}", folded.join(" ")))
}
