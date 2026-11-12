// This Source Code Form is subject to the terms of the GNU Affero General Public License,
// version 3 or later. See LICENSE-AGPL-3.0 at the repository root.

//! Web Push messages (T28): encryption per RFC 8291 (`aes128gcm`, RFC 8188) and VAPID per RFC 8292,
//! with `ring`. Browsers and UnifiedPush distributors both take these messages.
//!
//! Each message gets a new ephemeral P-256 key and a random salt; the subscription's public key
//! (`p256dh`) and authentication secret (`auth`) come from the client. The VAPID key is the
//! Companion's own (ES256), kept sealed with the master key.

use base64::Engine;
use ring::aead::{self, Aad, LessSafeKey, Nonce, UnboundKey};
use ring::agreement::{self, EphemeralPrivateKey, UnparsedPublicKey};
use ring::hkdf;
use ring::rand::{SecureRandom, SystemRandom};
use ring::signature::{EcdsaKeyPair, KeyPair, ECDSA_P256_SHA256_FIXED_SIGNING};
use zeroize::Zeroizing;

/// The record size written in the header (one record carries a ping).
const RECORD_SIZE: u32 = 4096;

/// Why a message could not be made.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PushError {
    /// The subscription's key or secret is not usable.
    BadSubscription,
    /// No randomness, or the crypto failed.
    Crypto,
}

fn b64(bytes: &[u8]) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

/// Base64url without padding (the form subscriptions use for their keys).
pub fn b64_public(bytes: &[u8]) -> String {
    b64(bytes)
}

/// Decodes base64url (padded or not), as subscriptions give their keys.
pub fn unb64(text: &str) -> Option<Vec<u8>> {
    base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(text.trim_end_matches('='))
        .ok()
}

struct Len(usize);

impl hkdf::KeyType for Len {
    fn len(&self) -> usize {
        self.0
    }
}

fn hkdf(salt: &[u8], ikm: &[u8], info: &[u8], len: usize) -> Result<Zeroizing<Vec<u8>>, PushError> {
    let prk = hkdf::Salt::new(hkdf::HKDF_SHA256, salt).extract(ikm);
    let infos = [info];
    let okm = prk
        .expand(&infos, Len(len))
        .map_err(|_| PushError::Crypto)?;
    let mut out = Zeroizing::new(vec![0u8; len]);
    okm.fill(&mut out).map_err(|_| PushError::Crypto)?;
    Ok(out)
}

/// RFC 8291 section 3.4 and RFC 8188, given the ECDH secret: the whole message body (header and the
/// one record). Separate from the key agreement so the RFC's example can be checked.
pub fn encrypt_with(
    ecdh_secret: &[u8],
    as_public: &[u8],
    ua_public: &[u8],
    auth_secret: &[u8],
    salt: &[u8; 16],
    plaintext: &[u8],
) -> Result<Vec<u8>, PushError> {
    if as_public.len() != 65 || ua_public.len() != 65 || auth_secret.len() != 16 {
        return Err(PushError::BadSubscription);
    }
    if plaintext.len() + 1 + 16 > RECORD_SIZE as usize {
        return Err(PushError::Crypto);
    }
    let mut key_info = b"WebPush: info\0".to_vec();
    key_info.extend_from_slice(ua_public);
    key_info.extend_from_slice(as_public);
    let ikm = hkdf(auth_secret, ecdh_secret, &key_info, 32)?;
    let cek = hkdf(salt, &ikm, b"Content-Encoding: aes128gcm\0", 16)?;
    let nonce = hkdf(salt, &ikm, b"Content-Encoding: nonce\0", 12)?;
    let key =
        LessSafeKey::new(UnboundKey::new(&aead::AES_128_GCM, &cek).map_err(|_| PushError::Crypto)?);
    let nonce = Nonce::try_assume_unique_for_key(&nonce).map_err(|_| PushError::Crypto)?;
    // The last (and only) record ends with the delimiter 2.
    let mut record = plaintext.to_vec();
    record.push(2);
    key.seal_in_place_append_tag(nonce, Aad::empty(), &mut record)
        .map_err(|_| PushError::Crypto)?;
    let mut out = Vec::with_capacity(16 + 4 + 1 + 65 + record.len());
    out.extend_from_slice(salt);
    out.extend_from_slice(&RECORD_SIZE.to_be_bytes());
    out.push(65);
    out.extend_from_slice(as_public);
    out.extend_from_slice(&record);
    Ok(out)
}

/// Encrypts `plaintext` for a subscription (`p256dh` and `auth` decoded): a new ephemeral key and
/// salt each time.
pub fn encrypt(
    ua_public: &[u8],
    auth_secret: &[u8],
    plaintext: &[u8],
) -> Result<Vec<u8>, PushError> {
    let rng = SystemRandom::new();
    let private = EphemeralPrivateKey::generate(&agreement::ECDH_P256, &rng)
        .map_err(|_| PushError::Crypto)?;
    let as_public = private
        .compute_public_key()
        .map_err(|_| PushError::Crypto)?;
    let mut salt = [0u8; 16];
    rng.fill(&mut salt).map_err(|_| PushError::Crypto)?;
    let peer = UnparsedPublicKey::new(&agreement::ECDH_P256, ua_public);
    let as_public = as_public.as_ref().to_vec();
    agreement::agree_ephemeral(private, &peer, |secret| {
        encrypt_with(secret, &as_public, ua_public, auth_secret, &salt, plaintext)
    })
    .map_err(|_| PushError::BadSubscription)?
}

/// The Companion's VAPID key.
pub struct Vapid {
    key: EcdsaKeyPair,
}

impl std::fmt::Debug for Vapid {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Vapid(***)")
    }
}

impl Vapid {
    /// A new key, as PKCS#8 (to seal and keep).
    pub fn generate_pkcs8() -> Result<Zeroizing<Vec<u8>>, PushError> {
        EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, &SystemRandom::new())
            .map(|d| Zeroizing::new(d.as_ref().to_vec()))
            .map_err(|_| PushError::Crypto)
    }

    /// The key from its PKCS#8 form.
    pub fn from_pkcs8(pkcs8: &[u8]) -> Result<Self, PushError> {
        EcdsaKeyPair::from_pkcs8(
            &ECDSA_P256_SHA256_FIXED_SIGNING,
            pkcs8,
            &SystemRandom::new(),
        )
        .map(|key| Self { key })
        .map_err(|_| PushError::Crypto)
    }

    /// The public key, base64url (a browser's `applicationServerKey`).
    pub fn public_key(&self) -> String {
        b64(self.key.public_key().as_ref())
    }

    /// The `Authorization` header for a message to `endpoint` (RFC 8292): an ES256 JWT for the
    /// endpoint's origin, valid 12 hours, naming the Companion (`sub`).
    pub fn authorization(
        &self,
        endpoint: &str,
        subject: &str,
        now_s: i64,
    ) -> Result<String, PushError> {
        let origin = url::Url::parse(endpoint)
            .map_err(|_| PushError::BadSubscription)?
            .origin()
            .ascii_serialization();
        let header = b64(br#"{"typ":"JWT","alg":"ES256"}"#);
        let claims = serde_json::json!({ "aud": origin, "exp": now_s + 12 * 3600, "sub": subject });
        let input = format!("{header}.{}", b64(claims.to_string().as_bytes()));
        let sig = self
            .key
            .sign(&SystemRandom::new(), input.as_bytes())
            .map_err(|_| PushError::Crypto)?;
        Ok(format!(
            "vapid t={input}.{}, k={}",
            b64(sig.as_ref()),
            self.public_key()
        ))
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
pub(crate) mod tests {
    use super::*;

    /// A receiver's side given the ECDH secret (tests).
    pub(crate) fn decrypt_with(
        secret: &[u8],
        ua_public: &[u8],
        auth: &[u8],
        body: &[u8],
    ) -> Vec<u8> {
        let salt: [u8; 16] = body[..16].try_into().unwrap();
        let as_public = &body[21..86];
        let mut record = body[86..].to_vec();
        let mut key_info = b"WebPush: info\0".to_vec();
        key_info.extend_from_slice(ua_public);
        key_info.extend_from_slice(as_public);
        let ikm = hkdf(auth, secret, &key_info, 32).unwrap();
        let cek = hkdf(&salt, &ikm, b"Content-Encoding: aes128gcm\0", 16).unwrap();
        let nonce = hkdf(&salt, &ikm, b"Content-Encoding: nonce\0", 12).unwrap();
        let key = LessSafeKey::new(UnboundKey::new(&aead::AES_128_GCM, &cek).unwrap());
        let plain = key
            .open_in_place(
                Nonce::try_assume_unique_for_key(&nonce).unwrap(),
                Aad::empty(),
                &mut record,
            )
            .unwrap();
        assert_eq!(plain.last(), Some(&2));
        plain[..plain.len() - 1].to_vec()
    }

    /// A receiver's side (tests): decrypts what [`encrypt`] made, as a browser would.
    pub(crate) fn decrypt(ua_private: EphemeralPrivateKey, auth: &[u8], body: &[u8]) -> Vec<u8> {
        let ua_public = ua_private.compute_public_key().unwrap().as_ref().to_vec();
        let salt: [u8; 16] = body[..16].try_into().unwrap();
        assert_eq!(&body[16..20], &RECORD_SIZE.to_be_bytes());
        assert_eq!(body[20], 65);
        let as_public = body[21..86].to_vec();
        let mut record = body[86..].to_vec();
        let peer = UnparsedPublicKey::new(&agreement::ECDH_P256, as_public.clone());
        agreement::agree_ephemeral(ua_private, &peer, |secret| {
            let mut key_info = b"WebPush: info\0".to_vec();
            key_info.extend_from_slice(&ua_public);
            key_info.extend_from_slice(&as_public);
            let ikm = hkdf(auth, secret, &key_info, 32).unwrap();
            let cek = hkdf(&salt, &ikm, b"Content-Encoding: aes128gcm\0", 16).unwrap();
            let nonce = hkdf(&salt, &ikm, b"Content-Encoding: nonce\0", 12).unwrap();
            let key = LessSafeKey::new(UnboundKey::new(&aead::AES_128_GCM, &cek).unwrap());
            let plain = key
                .open_in_place(
                    Nonce::try_assume_unique_for_key(&nonce).unwrap(),
                    Aad::empty(),
                    &mut record,
                )
                .unwrap();
            assert_eq!(plain.last(), Some(&2));
            plain[..plain.len() - 1].to_vec()
        })
        .unwrap()
    }

    /// RFC 8291 Appendix A.
    #[test]
    fn the_rfc_8291_example_is_reproduced() {
        let as_private = unb64("yfWPiYE-n46HLnH0KqZOF1fJJU3MYrct3AELtAQ-oRw").unwrap();
        let as_public = unb64("BP4z9KsN6nGRTbVYI_c7VJSPQTBtkgcy27mlmlMoZIIgDll6e3vCYLocInmYWAmS6TlzAC8wEqKK6PBru3jl7A8").unwrap();
        let ua_public = unb64("BCVxsr7N_eNgVRqvHtD0zTZsEc6-VV-JvLexhqUzORcxaOzi6-AYWXvTBHm4bjyPjs7Vd8pZGH6SRpkNtoIAiw4").unwrap();
        let auth = unb64("BTBZMqHH6r4Tts7J_aSIgg").unwrap();
        let salt: [u8; 16] = unb64("DGv6ra1nlYgDCS1FRnbzlw").unwrap().try_into().unwrap();
        let expected = "DGv6ra1nlYgDCS1FRnbzlwAAEABBBP4z9KsN6nGRTbVYI_c7VJSPQTBtkgcy27mlmlMoZIIgDll6e3vCYLocInmYWAmS6TlzAC8wEqKK6PBru3jl7A_yl95bQpu6cVPTpK4Mqgkf1CXztLVBSt2Ks3oZwbuwXPXLWyouBWLVWGNWQexSgSxsj_Qulcy4a-fN";
        let secret = {
            let sk = p256::SecretKey::from_slice(&as_private).unwrap();
            let pk = p256::PublicKey::from_sec1_bytes(&ua_public).unwrap();
            p256::ecdh::diffie_hellman(sk.to_nonzero_scalar(), pk.as_affine())
                .raw_secret_bytes()
                .to_vec()
        };
        let body = encrypt_with(
            &secret,
            &as_public,
            &ua_public,
            &auth,
            &salt,
            b"When I grow up, I want to be a watermelon",
        )
        .unwrap();
        assert_eq!(b64(&body), expected);
    }

    #[test]
    fn a_message_decrypts_with_the_subscriptions_key_only() {
        let rng = SystemRandom::new();
        let ua = EphemeralPrivateKey::generate(&agreement::ECDH_P256, &rng).unwrap();
        let ua_public = ua.compute_public_key().unwrap().as_ref().to_vec();
        let auth = [7u8; 16];
        let body = encrypt(&ua_public, &auth, br#"{"v":1,"type":"changed"}"#).unwrap();
        let again = encrypt(&ua_public, &auth, br#"{"v":1,"type":"changed"}"#).unwrap();
        assert_ne!(body, again, "a new key and salt each time");
        assert_eq!(decrypt(ua, &auth, &body), br#"{"v":1,"type":"changed"}"#);
        assert_eq!(
            encrypt(&[4u8; 10], &auth, b"x"),
            Err(PushError::BadSubscription)
        );
    }

    #[test]
    fn the_vapid_jwt_verifies_with_the_published_key() {
        let vapid = Vapid::from_pkcs8(&Vapid::generate_pkcs8().unwrap()).unwrap();
        let auth = vapid
            .authorization(
                "https://push.example.net:8443/wpush/v2/abc?x=1",
                "https://companion.test",
                1_800_000_000,
            )
            .unwrap();
        let (t, k) = auth
            .strip_prefix("vapid t=")
            .unwrap()
            .split_once(", k=")
            .unwrap();
        assert_eq!(k, vapid.public_key());
        let mut parts = t.split('.');
        let (h, c, s) = (
            parts.next().unwrap(),
            parts.next().unwrap(),
            parts.next().unwrap(),
        );
        let claims: serde_json::Value = serde_json::from_slice(&unb64(c).unwrap()).unwrap();
        assert_eq!(claims["aud"], "https://push.example.net:8443");
        assert_eq!(claims["exp"], 1_800_000_000 + 12 * 3600);
        assert_eq!(claims["sub"], "https://companion.test");
        let key = ring::signature::UnparsedPublicKey::new(
            &ring::signature::ECDSA_P256_SHA256_FIXED,
            unb64(k).unwrap(),
        );
        key.verify(format!("{h}.{c}").as_bytes(), &unb64(s).unwrap())
            .unwrap();
    }
}
