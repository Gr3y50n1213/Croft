// This Source Code Form is subject to the terms of the GNU Affero General Public License,
// version 3 or later. See LICENSE-AGPL-3.0 at the repository root.

//! OpenID Connect ID tokens (T25): the provider's key set (JWKS), the signature, and the claims.
//!
//! Only asymmetric signatures are accepted: RS256, ES256 and EdDSA (Ed25519), each checked with
//! `ring`. `none` and every HMAC algorithm are refused before any key is looked at, so a token
//! "signed" with a public key as an HMAC secret (algorithm confusion) cannot pass. A key is used
//! only for the algorithm its type allows, and only when its own `alg` (if it has one) matches.
//! A header with `crit` is refused (no extension is understood). Nothing here logs, and no error
//! carries a part of a token.

use base64::Engine;
use ring::signature::{self, RsaPublicKeyComponents, UnparsedPublicKey};
use serde_json::{Map, Value};

/// The largest ID token accepted.
pub const MAX_TOKEN: usize = 16 * 1024;
/// Clock leeway for `exp`, `iat` and `nbf`, in seconds.
pub const LEEWAY_S: i64 = 60;

/// Why a token was refused (no part of the token is kept).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JwtError {
    /// Not three base64url parts of JSON, or too big.
    Malformed,
    /// An algorithm that is not accepted (`none`, HMAC, anything else) or a `crit` header.
    Algorithm,
    /// No key in the set fits the token's key id and algorithm.
    UnknownKey,
    /// The signature does not verify.
    Signature,
    /// A claim is wrong; the word says which (`iss`, `aud`, `azp`, `exp`, `iat`, `nbf`, `nonce`,
    /// `sub`).
    Claim(&'static str),
}

impl std::fmt::Display for JwtError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Malformed => f.write_str("the ID token is malformed"),
            Self::Algorithm => f.write_str("the ID token's algorithm is not accepted"),
            Self::UnknownKey => f.write_str("no provider key fits the ID token"),
            Self::Signature => f.write_str("the ID token's signature does not verify"),
            Self::Claim(c) => write!(f, "the ID token's {c} is wrong"),
        }
    }
}

impl std::error::Error for JwtError {}

/// The accepted algorithms.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Alg {
    Rs256,
    Es256,
    EdDsa,
}

impl Alg {
    fn parse(s: &str) -> Option<Self> {
        match s {
            "RS256" => Some(Self::Rs256),
            "ES256" => Some(Self::Es256),
            "EdDSA" => Some(Self::EdDsa),
            _ => None,
        }
    }
}

/// A public key from the provider's key set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Jwk {
    kid: Option<String>,
    alg: Option<String>,
    key: KeyMaterial,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum KeyMaterial {
    Rsa {
        n: Vec<u8>,
        e: Vec<u8>,
    },
    /// Uncompressed point, `0x04 || x || y`.
    P256(Vec<u8>),
    Ed25519(Vec<u8>),
}

impl Jwk {
    /// The key's id, if it has one.
    pub fn kid(&self) -> Option<&str> {
        self.kid.as_deref()
    }

    fn fits(&self, alg: Alg) -> bool {
        let type_fits = matches!(
            (&self.key, alg),
            (KeyMaterial::Rsa { .. }, Alg::Rs256)
                | (KeyMaterial::P256(_), Alg::Es256)
                | (KeyMaterial::Ed25519(_), Alg::EdDsa)
        );
        type_fits
            && self
                .alg
                .as_deref()
                .is_none_or(|a| Alg::parse(a) == Some(alg))
    }

    fn verify(&self, message: &[u8], sig: &[u8]) -> bool {
        match &self.key {
            KeyMaterial::Rsa { n, e } => RsaPublicKeyComponents { n, e }
                .verify(&signature::RSA_PKCS1_2048_8192_SHA256, message, sig)
                .is_ok(),
            KeyMaterial::P256(point) => {
                UnparsedPublicKey::new(&signature::ECDSA_P256_SHA256_FIXED, point)
                    .verify(message, sig)
                    .is_ok()
            }
            KeyMaterial::Ed25519(x) => UnparsedPublicKey::new(&signature::ED25519, x)
                .verify(message, sig)
                .is_ok(),
        }
    }
}

fn b64(s: &str) -> Option<Vec<u8>> {
    base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(s.trim_end_matches('='))
        .ok()
}

/// Reads a JWKS document. Keys of other types or curves, and keys marked for encryption, are left
/// out; a broken document gives an empty set.
pub fn parse_jwks(json: &[u8]) -> Vec<Jwk> {
    let Ok(Value::Object(doc)) = serde_json::from_slice::<Value>(json) else {
        return Vec::new();
    };
    let Some(Value::Array(keys)) = doc.get("keys") else {
        return Vec::new();
    };
    keys.iter().filter_map(jwk).collect()
}

fn jwk(v: &Value) -> Option<Jwk> {
    let s = |k: &str| v.get(k).and_then(Value::as_str);
    if s("use").is_some_and(|u| u != "sig") {
        return None;
    }
    let key = match s("kty")? {
        "RSA" => {
            let n = b64(s("n")?)?;
            // ring wants at least 2048 bits; smaller keys never verify.
            KeyMaterial::Rsa {
                n,
                e: b64(s("e")?)?,
            }
        }
        "EC" if s("crv")? == "P-256" => {
            let (x, y) = (b64(s("x")?)?, b64(s("y")?)?);
            if x.len() != 32 || y.len() != 32 {
                return None;
            }
            let mut point = vec![0x04];
            point.extend_from_slice(&x);
            point.extend_from_slice(&y);
            KeyMaterial::P256(point)
        }
        "OKP" if s("crv")? == "Ed25519" => {
            let x = b64(s("x")?)?;
            if x.len() != 32 {
                return None;
            }
            KeyMaterial::Ed25519(x)
        }
        _ => return None,
    };
    Some(Jwk {
        kid: s("kid").map(str::to_owned),
        alg: s("alg").map(str::to_owned),
        key,
    })
}

/// A token whose signature checked out, before its claims are checked.
#[derive(Debug)]
pub struct Verified {
    /// The payload's claims.
    pub claims: Map<String, Value>,
}

/// The key id a token names (to decide whether to fetch the key set again).
pub fn key_id(token: &str) -> Option<String> {
    let header = token.split('.').next()?;
    let v: Value = serde_json::from_slice(&b64(header)?).ok()?;
    v.get("kid").and_then(Value::as_str).map(str::to_owned)
}

/// Checks the token's signature with the provider's keys.
pub fn verify_signature(token: &str, keys: &[Jwk]) -> Result<Verified, JwtError> {
    if token.len() > MAX_TOKEN {
        return Err(JwtError::Malformed);
    }
    let mut parts = token.split('.');
    let (Some(h), Some(p), Some(sig), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return Err(JwtError::Malformed);
    };
    let header: Map<String, Value> = b64(h)
        .and_then(|b| serde_json::from_slice(&b).ok())
        .ok_or(JwtError::Malformed)?;
    if header.contains_key("crit") {
        return Err(JwtError::Algorithm);
    }
    let alg = header
        .get("alg")
        .and_then(Value::as_str)
        .and_then(Alg::parse)
        .ok_or(JwtError::Algorithm)?;
    let kid = header.get("kid").and_then(Value::as_str);
    let candidates: Vec<&Jwk> = keys
        .iter()
        .filter(|k| k.fits(alg) && (kid.is_none() || k.kid.as_deref() == kid))
        .collect();
    if candidates.is_empty() {
        return Err(JwtError::UnknownKey);
    }
    let sig = b64(sig).ok_or(JwtError::Malformed)?;
    let message = &token.as_bytes()[..h.len() + 1 + p.len()];
    if !candidates.iter().any(|k| k.verify(message, &sig)) {
        return Err(JwtError::Signature);
    }
    let claims: Map<String, Value> = b64(p)
        .and_then(|b| serde_json::from_slice(&b).ok())
        .ok_or(JwtError::Malformed)?;
    Ok(Verified { claims })
}

/// What the Companion needs from a valid ID token.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Identity {
    /// The provider (`iss`).
    pub issuer: String,
    /// The person at that provider (`sub`).
    pub subject: String,
    /// `preferred_username`, if given (only to name a new user).
    pub preferred_username: Option<String>,
    /// The groups in the configured claim.
    pub groups: Vec<String>,
}

/// What the claims must say.
pub struct Expected<'a> {
    /// The issuer, exactly.
    pub issuer: &'a str,
    /// The Companion's client id.
    pub client_id: &'a str,
    /// The nonce sent with this sign-in.
    pub nonce: &'a str,
    /// Now, Unix seconds.
    pub now_s: i64,
    /// The claim that lists groups.
    pub groups_claim: &'a str,
}

/// Checks the claims of a token whose signature verified (OpenID Connect Core 3.1.3.7).
pub fn check_claims(v: &Verified, want: &Expected<'_>) -> Result<Identity, JwtError> {
    let c = &v.claims;
    let text = |k: &str| c.get(k).and_then(Value::as_str);
    let number = |k: &str| c.get(k).and_then(Value::as_i64);
    if text("iss") != Some(want.issuer) {
        return Err(JwtError::Claim("iss"));
    }
    let audiences: Vec<&str> = match c.get("aud") {
        Some(Value::String(a)) => vec![a.as_str()],
        Some(Value::Array(list)) => list.iter().filter_map(Value::as_str).collect(),
        _ => return Err(JwtError::Claim("aud")),
    };
    if !audiences.contains(&want.client_id) {
        return Err(JwtError::Claim("aud"));
    }
    // With several audiences, `azp` must name this client; when present, it must always.
    match text("azp") {
        Some(azp) if azp != want.client_id => return Err(JwtError::Claim("azp")),
        None if audiences.len() > 1 => return Err(JwtError::Claim("azp")),
        _ => {}
    }
    let exp = number("exp").ok_or(JwtError::Claim("exp"))?;
    if exp + LEEWAY_S <= want.now_s {
        return Err(JwtError::Claim("exp"));
    }
    let iat = number("iat").ok_or(JwtError::Claim("iat"))?;
    if iat > want.now_s + LEEWAY_S {
        return Err(JwtError::Claim("iat"));
    }
    if let Some(nbf) = c.get("nbf") {
        if nbf.as_i64().is_none_or(|n| n > want.now_s + LEEWAY_S) {
            return Err(JwtError::Claim("nbf"));
        }
    }
    let nonce = text("nonce").ok_or(JwtError::Claim("nonce"))?;
    if !crate::crypto::ct_eq(nonce.as_bytes(), want.nonce.as_bytes()) {
        return Err(JwtError::Claim("nonce"));
    }
    let subject = text("sub")
        .filter(|s| !s.is_empty() && s.len() <= 255)
        .ok_or(JwtError::Claim("sub"))?;
    let groups = match c.get(want.groups_claim) {
        Some(Value::Array(list)) => list
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect(),
        Some(Value::String(one)) => vec![one.clone()],
        _ => Vec::new(),
    };
    Ok(Identity {
        issuer: want.issuer.to_owned(),
        subject: subject.to_owned(),
        preferred_username: text("preferred_username").map(str::to_owned),
        groups,
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
pub(crate) mod tests {
    use super::*;
    use ring::rand::SystemRandom;
    use ring::signature::{EcdsaKeyPair, Ed25519KeyPair, KeyPair};
    use serde_json::json;

    /// A made-up RSA key's public half and one token it signed (the private half was thrown away).
    const RSA_N: &str = "9jNFuOcbDi2dtctdlGNHKG-QvOmkYBpemOYAAE55ecvqLjNBf2ZGZFJJvVR5v7PWX98ShK3-Ou0XG2jZsD75-GwCfXgpTV-ngGvM-IupEJuS5R0IJiXgkqx8PaOYVHaFkwBvMcyNxMsR-MmbnMjCr_h9Ko5snwQfcx4FeHKu4nivmvH9LL_P2JXqsV1st3Pm7vUSPOMMuEhcEWcKwCyGdjLmGhgQPmBDmlf7MvdRDhL_Z2wMfjpqGfWIDoyxFRcbOcjjGz5pQBO_4hLkaptE49UEaqG5STVEpSheMI1AnmHX8C7RbTK3Iy3PH10PTlsz2bWcQBQhvgzBCytz681sBw";
    const RSA_TOKEN: &str = "eyJhbGciOiJSUzI1NiIsImtpZCI6InRlc3QtcnNhIiwidHlwIjoiSldUIn0.eyJpc3MiOiJodHRwczovL2lkLmV4YW1wbGUiLCJzdWIiOiJtYWRlLXVwLXN1YmplY3QiLCJhdWQiOiJjcm9mdCIsImV4cCI6NDEwMjQ0NDgwMCwiaWF0IjoxNzAwMDAwMDAwLCJub25jZSI6Im1hZGUtdXAtbm9uY2UifQ.ZzcMGwXUi97N1npl__1IxKdqJ592PmbEOGm8CNCTDf_BhDTraz-HGHg0l9S9mUGq2_bw7CFanmFYoALS96YRV2XbMzZwSq_lI19GoyAJppHeeyyQfUp6IXOfNFqKkg-g0SGbh1X-yYBfYxiEXrQ7rGUxMm1mbKil2lELg5fY21i5bebC0EPu9rWumy79sAvSLVv1oLYhCDRLypQrWsbgB9INEGq0obaT8nLNFqz7bfsynTKI_j6uZIUkU2TTOZz_uqtvjzL5n5JT8pX0UE5TdF2HKph8lop2H6RCzLED8Ai78A4gJdTuCEXvPSkhNwYcC2c3w235dDj9IFSQ8A5tQQ";

    fn enc(b: &[u8]) -> String {
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(b)
    }

    /// A made-up provider signing key (tests): ES256 or EdDSA, generated per test.
    pub(crate) enum TestSigner {
        Es256(EcdsaKeyPair),
        Ed25519(Ed25519KeyPair),
    }

    impl TestSigner {
        pub(crate) fn es256() -> Self {
            let rng = SystemRandom::new();
            let pkcs8 =
                EcdsaKeyPair::generate_pkcs8(&signature::ECDSA_P256_SHA256_FIXED_SIGNING, &rng)
                    .unwrap();
            Self::Es256(
                EcdsaKeyPair::from_pkcs8(
                    &signature::ECDSA_P256_SHA256_FIXED_SIGNING,
                    pkcs8.as_ref(),
                    &rng,
                )
                .unwrap(),
            )
        }

        pub(crate) fn ed25519() -> Self {
            let pkcs8 = Ed25519KeyPair::generate_pkcs8(&SystemRandom::new()).unwrap();
            Self::Ed25519(Ed25519KeyPair::from_pkcs8(pkcs8.as_ref()).unwrap())
        }

        /// The public key as a JWK.
        pub(crate) fn jwk(&self, kid: &str) -> Value {
            match self {
                Self::Es256(k) => {
                    let p = k.public_key().as_ref();
                    json!({"kty": "EC", "crv": "P-256", "kid": kid, "use": "sig",
                           "x": enc(&p[1..33]), "y": enc(&p[33..65])})
                }
                Self::Ed25519(k) => {
                    json!({"kty": "OKP", "crv": "Ed25519", "kid": kid, "x": enc(k.public_key().as_ref())})
                }
            }
        }

        pub(crate) fn sign(&self, header: &Value, claims: &Value) -> String {
            let input = format!(
                "{}.{}",
                enc(header.to_string().as_bytes()),
                enc(claims.to_string().as_bytes())
            );
            let sig = match self {
                Self::Es256(k) => k
                    .sign(&SystemRandom::new(), input.as_bytes())
                    .unwrap()
                    .as_ref()
                    .to_vec(),
                Self::Ed25519(k) => k.sign(input.as_bytes()).as_ref().to_vec(),
            };
            format!("{input}.{}", enc(&sig))
        }

        pub(crate) fn alg(&self) -> &'static str {
            match self {
                Self::Es256(_) => "ES256",
                Self::Ed25519(_) => "EdDSA",
            }
        }
    }

    fn keys_of(v: &Value) -> Vec<Jwk> {
        parse_jwks(v.to_string().as_bytes())
    }

    fn want<'a>(nonce: &'a str, now_s: i64) -> Expected<'a> {
        Expected {
            issuer: "https://id.example",
            client_id: "croft",
            nonce,
            now_s,
            groups_claim: "groups",
        }
    }

    fn claims(extra: Value) -> Value {
        let mut c = json!({"iss": "https://id.example", "sub": "made-up-subject", "aud": "croft",
                           "exp": 2_000_000_100, "iat": 2_000_000_000, "nonce": "n1"});
        if let (Some(c), Some(e)) = (c.as_object_mut(), extra.as_object()) {
            for (k, v) in e {
                if v.is_null() {
                    c.remove(k);
                } else {
                    c.insert(k.clone(), v.clone());
                }
            }
        }
        c
    }

    #[test]
    fn an_rs256_token_verifies_with_its_public_key_only() {
        let keys =
            keys_of(&json!({"keys": [{"kty": "RSA", "kid": "test-rsa", "n": RSA_N, "e": "AQAB"}]}));
        let v = verify_signature(RSA_TOKEN, &keys).unwrap();
        let id = check_claims(&v, &want("made-up-nonce", 1_800_000_000)).unwrap();
        assert_eq!(id.subject, "made-up-subject");
        // One changed byte in the payload breaks it.
        let mut parts: Vec<String> = RSA_TOKEN.split('.').map(str::to_owned).collect();
        parts[1] = parts[1].replacen("e", "f", 1);
        assert_eq!(
            verify_signature(&parts.join("."), &keys).unwrap_err(),
            JwtError::Signature
        );
        // A key with another kid is not used.
        let other =
            keys_of(&json!({"keys": [{"kty": "RSA", "kid": "other", "n": RSA_N, "e": "AQAB"}]}));
        assert_eq!(
            verify_signature(RSA_TOKEN, &other).unwrap_err(),
            JwtError::UnknownKey
        );
    }

    #[test]
    fn es256_and_eddsa_tokens_verify_and_a_key_serves_only_its_own_algorithm() {
        for signer in [TestSigner::es256(), TestSigner::ed25519()] {
            let keys = keys_of(&json!({"keys": [signer.jwk("k1")]}));
            let t = signer.sign(
                &json!({"alg": signer.alg(), "kid": "k1"}),
                &claims(json!({})),
            );
            let v = verify_signature(&t, &keys).unwrap();
            assert!(check_claims(&v, &want("n1", 2_000_000_050)).is_ok());
            // The same key claimed under another algorithm is refused.
            let other = if signer.alg() == "ES256" {
                "EdDSA"
            } else {
                "ES256"
            };
            let t = signer.sign(&json!({"alg": other, "kid": "k1"}), &claims(json!({})));
            assert_eq!(
                verify_signature(&t, &keys).unwrap_err(),
                JwtError::UnknownKey
            );
        }
    }

    #[test]
    fn none_hmac_crit_and_algorithm_confusion_are_refused() {
        let keys =
            keys_of(&json!({"keys": [{"kty": "RSA", "kid": "test-rsa", "n": RSA_N, "e": "AQAB"}]}));
        let body = enc(claims(json!({})).to_string().as_bytes());
        for header in [
            json!({"alg": "none"}),
            json!({"alg": "None"}),
            json!({"alg": "HS256", "kid": "test-rsa"}),
            json!({"alg": "RS256", "kid": "test-rsa", "crit": ["exp"]}),
            json!({}),
        ] {
            let h = enc(header.to_string().as_bytes());
            assert_eq!(
                verify_signature(&format!("{h}.{body}."), &keys).unwrap_err(),
                JwtError::Algorithm,
                "{header}"
            );
        }
        // HS256 "signed" with the public key's bytes as the HMAC secret.
        let h = enc(json!({"alg": "HS256", "kid": "test-rsa"})
            .to_string()
            .as_bytes());
        let input = format!("{h}.{body}");
        let key = ring::hmac::Key::new(ring::hmac::HMAC_SHA256, RSA_N.as_bytes());
        let tag = ring::hmac::sign(&key, input.as_bytes());
        let forged = format!("{input}.{}", enc(tag.as_ref()));
        assert_eq!(
            verify_signature(&forged, &keys).unwrap_err(),
            JwtError::Algorithm
        );
        for broken in ["", "a.b", "a.b.c.d", &"x".repeat(MAX_TOKEN + 1)] {
            assert_eq!(
                verify_signature(broken, &keys).unwrap_err(),
                JwtError::Malformed
            );
        }
    }

    #[test]
    fn every_claim_rule_holds() {
        let s = TestSigner::es256();
        let keys = keys_of(&json!({"keys": [s.jwk("k")]}));
        let check = |extra: Value, nonce: &str, now: i64| {
            let t = s.sign(&json!({"alg": "ES256", "kid": "k"}), &claims(extra));
            check_claims(&verify_signature(&t, &keys).unwrap(), &want(nonce, now))
        };
        let now = 2_000_000_050;
        assert!(check(json!({}), "n1", now).is_ok());
        assert_eq!(
            check(json!({"iss": "https://id.example/"}), "n1", now),
            Err(JwtError::Claim("iss"))
        );
        assert_eq!(
            check(json!({"aud": "other"}), "n1", now),
            Err(JwtError::Claim("aud"))
        );
        assert_eq!(
            check(json!({"aud": ["croft", "other"]}), "n1", now),
            Err(JwtError::Claim("azp"))
        );
        assert!(check(
            json!({"aud": ["croft", "other"], "azp": "croft"}),
            "n1",
            now
        )
        .is_ok());
        assert_eq!(
            check(json!({"azp": "other"}), "n1", now),
            Err(JwtError::Claim("azp"))
        );
        assert_eq!(
            check(json!({}), "n1", 2_000_000_100 + LEEWAY_S),
            Err(JwtError::Claim("exp"))
        );
        assert!(
            check(json!({}), "n1", 2_000_000_100 + LEEWAY_S - 1).is_ok(),
            "within the leeway"
        );
        assert_eq!(
            check(json!({"exp": null}), "n1", now),
            Err(JwtError::Claim("exp"))
        );
        assert_eq!(
            check(json!({"iat": now + LEEWAY_S + 1}), "n1", now),
            Err(JwtError::Claim("iat"))
        );
        assert_eq!(
            check(json!({"nbf": now + LEEWAY_S + 1}), "n1", now),
            Err(JwtError::Claim("nbf"))
        );
        assert_eq!(check(json!({}), "n2", now), Err(JwtError::Claim("nonce")));
        assert_eq!(
            check(json!({"nonce": null}), "n1", now),
            Err(JwtError::Claim("nonce"))
        );
        assert_eq!(
            check(json!({"sub": ""}), "n1", now),
            Err(JwtError::Claim("sub"))
        );
        let id = check(
            json!({"groups": ["mail", "x"], "preferred_username": "ann"}),
            "n1",
            now,
        )
        .unwrap();
        assert_eq!(id.groups, ["mail", "x"]);
        assert_eq!(id.preferred_username.as_deref(), Some("ann"));
    }

    #[test]
    fn the_key_set_keeps_only_signing_keys_it_can_use() {
        let s = TestSigner::ed25519();
        let mut enc_key = s.jwk("e");
        enc_key["use"] = json!("enc");
        let keys = keys_of(&json!({"keys": [
            s.jwk("ok"), enc_key,
            {"kty": "EC", "crv": "P-384", "x": "AA", "y": "AA"},
            {"kty": "oct", "k": "c2VjcmV0"},
            {"kty": "RSA", "n": RSA_N}
        ]}));
        assert_eq!(keys.len(), 1);
        assert!(parse_jwks(b"not json").is_empty());
        assert_eq!(key_id(RSA_TOKEN).as_deref(), Some("test-rsa"));
    }
}
