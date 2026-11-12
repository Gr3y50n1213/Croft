// This Source Code Form is subject to the terms of the GNU Affero General Public License,
// version 3 or later. See LICENSE-AGPL-3.0 at the repository root.

//! VAPID (RFC 8292) for endpoints bound to a Companion's key: a message is forwarded only when its
//! `Authorization: vapid t=<JWT>, k=<key>` names that key and carries an ES256 JWT for this
//! gateway's origin that has not expired (and expires within a day). A leaked endpoint is then of
//! no use to anyone without the Companion's private key.

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use ring::signature::{UnparsedPublicKey, ECDSA_P256_SHA256_FIXED};

/// The longest `Authorization` header read.
const MAX_HEADER: usize = 4096;
/// A JWT may expire at most this far ahead (RFC 8292: 24 hours), plus a minute for clocks.
const MAX_LIFETIME_S: i64 = 24 * 3600 + 60;

/// The origin (`scheme://host[:port]`, lower case) of a URL.
pub fn origin(url: &str) -> String {
    let rest = url.split_once("://").map_or(url, |(_, r)| r);
    let scheme_len = url.len() - rest.len();
    let host_end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    url[..scheme_len + host_end].to_ascii_lowercase()
}

/// Whether `authorization` is a valid VAPID authorization by `key` for `audience` at `now_s`.
pub fn verify(authorization: &str, key: &[u8; 65], audience: &str, now_s: i64) -> bool {
    if authorization.len() > MAX_HEADER {
        return false;
    }
    let Some((scheme, params)) = authorization.trim().split_once(' ') else {
        return false;
    };
    if !scheme.eq_ignore_ascii_case("vapid") {
        return false;
    }
    let (mut t, mut k) = (None, None);
    for part in params.split(',') {
        match part.trim().split_once('=') {
            Some(("t", v)) => t = Some(v.trim()),
            Some(("k", v)) => k = Some(v.trim()),
            _ => {}
        }
    }
    let (Some(t), Some(k)) = (t, k) else {
        return false;
    };
    if URL_SAFE_NO_PAD.decode(k).ok().as_deref() != Some(&key[..]) {
        return false;
    }
    let mut parts = t.split('.');
    let (Some(header), Some(claims), Some(signature), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return false;
    };
    let json = |part: &str| {
        URL_SAFE_NO_PAD
            .decode(part)
            .ok()
            .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
    };
    let (Some(h), Some(c), Ok(sig)) = (
        json(header),
        json(claims),
        URL_SAFE_NO_PAD.decode(signature),
    ) else {
        return false;
    };
    if h["alg"] != "ES256" {
        return false;
    }
    let aud_ok = c["aud"]
        .as_str()
        .is_some_and(|a| a.eq_ignore_ascii_case(audience));
    let exp_ok = c["exp"]
        .as_i64()
        .is_some_and(|e| e > now_s && e <= now_s + MAX_LIFETIME_S);
    if !aud_ok || !exp_ok {
        return false;
    }
    let signed_len = header.len() + 1 + claims.len();
    UnparsedPublicKey::new(&ECDSA_P256_SHA256_FIXED, &key[..])
        .verify(&t.as_bytes()[..signed_len], &sig)
        .is_ok()
}

/// A Companion's VAPID public key as given at registration (base64url, 65 bytes, uncompressed).
pub fn parse_key(text: &str) -> Option<[u8; 65]> {
    let raw = URL_SAFE_NO_PAD.decode(text.trim()).ok()?;
    let key: [u8; 65] = raw.try_into().ok()?;
    (key[0] == 4).then_some(key)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
pub(crate) mod tests {
    use super::*;
    use ring::rand::SystemRandom;
    use ring::signature::{EcdsaKeyPair, KeyPair, ECDSA_P256_SHA256_FIXED_SIGNING};

    /// A made-up Companion VAPID key, and a signer of JWTs with it.
    pub(crate) struct Companion {
        pair: EcdsaKeyPair,
    }

    impl Companion {
        pub(crate) fn new() -> Self {
            let rng = SystemRandom::new();
            let pkcs8 =
                EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, &rng).unwrap();
            Self {
                pair: EcdsaKeyPair::from_pkcs8(
                    &ECDSA_P256_SHA256_FIXED_SIGNING,
                    pkcs8.as_ref(),
                    &rng,
                )
                .unwrap(),
            }
        }

        pub(crate) fn public(&self) -> [u8; 65] {
            self.pair.public_key().as_ref().try_into().unwrap()
        }

        pub(crate) fn authorization(&self, aud: &str, exp: i64) -> String {
            let header = URL_SAFE_NO_PAD.encode(br#"{"typ":"JWT","alg":"ES256"}"#);
            let claims = URL_SAFE_NO_PAD.encode(
                serde_json::json!({"aud": aud, "exp": exp, "sub": "https://companion.example"})
                    .to_string(),
            );
            let signed = format!("{header}.{claims}");
            let sig = self
                .pair
                .sign(&SystemRandom::new(), signed.as_bytes())
                .unwrap();
            format!(
                "vapid t={signed}.{}, k={}",
                URL_SAFE_NO_PAD.encode(sig.as_ref()),
                URL_SAFE_NO_PAD.encode(self.public())
            )
        }
    }

    #[test]
    fn only_the_bound_companions_fresh_jwt_for_this_gateway_passes() {
        let c = Companion::new();
        let other = Companion::new();
        let now = 1_800_000_000;
        let aud = "https://push.test";
        assert!(verify(
            &c.authorization(aud, now + 3600),
            &c.public(),
            aud,
            now
        ));
        assert!(verify(
            &c.authorization("HTTPS://PUSH.TEST", now + 3600),
            &c.public(),
            aud,
            now
        ));
        // Another key, another audience, expired, too far ahead.
        assert!(!verify(
            &other.authorization(aud, now + 3600),
            &c.public(),
            aud,
            now
        ));
        assert!(!verify(
            &c.authorization("https://other.test", now + 3600),
            &c.public(),
            aud,
            now
        ));
        assert!(!verify(
            &c.authorization(aud, now - 1),
            &c.public(),
            aud,
            now
        ));
        assert!(!verify(
            &c.authorization(aud, now + 3 * 86400),
            &c.public(),
            aud,
            now
        ));
        // Another key's signature presented with this key.
        let forged = other.authorization(aud, now + 3600).replace(
            &URL_SAFE_NO_PAD.encode(other.public()),
            &URL_SAFE_NO_PAD.encode(c.public()),
        );
        assert!(!verify(&forged, &c.public(), aud, now));
        assert!(!verify("", &c.public(), aud, now));
        assert!(!verify("Bearer x", &c.public(), aud, now));
        assert!(!verify(&"v".repeat(MAX_HEADER + 1), &c.public(), aud, now));
    }

    #[test]
    fn origins_and_keys() {
        assert_eq!(
            origin("https://Push.Example:8443/gw/x?y"),
            "https://push.example:8443"
        );
        assert_eq!(origin("https://push.example"), "https://push.example");
        assert!(parse_key(&URL_SAFE_NO_PAD.encode([4u8; 65])).is_some());
        assert!(parse_key(&URL_SAFE_NO_PAD.encode([5u8; 65])).is_none());
        assert!(parse_key(&URL_SAFE_NO_PAD.encode([4u8; 64])).is_none());
    }
}
