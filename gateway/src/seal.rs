// This Source Code Form is subject to the terms of the GNU Affero General Public License,
// version 3 or later. See LICENSE-AGPL-3.0 at the repository root.

//! The gateway's key and the endpoints it makes. An endpoint's last part is the device's FCM token
//! sealed with the key (AES-256-GCM, a random nonce, a fixed context): the gateway stores nothing,
//! only it can open an endpoint, and nobody can make one for a token that was not registered here.

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use ring::aead::{Aad, LessSafeKey, Nonce, UnboundKey, AES_256_GCM, NONCE_LEN};
use ring::rand::{SecureRandom, SystemRandom};
use zeroize::Zeroizing;

/// What an endpoint's seal is bound to (a key used for anything else cannot open it).
const CONTEXT: &[u8] = b"croft-gateway/endpoint/v1";
/// The longest sealed part opened (a token of [`MAX_TOKEN`] characters, sealed and encoded).
pub const MAX_SEALED: usize = 5600;
/// The longest FCM registration token taken.
pub const MAX_TOKEN: usize = 4096;

/// Why the key could not be made or read. Carries nothing about the key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyError {
    /// Not 64 hexadecimal characters.
    Malformed,
    /// No random bytes.
    Random,
}

impl std::fmt::Display for KeyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Malformed => "the key file must hold 64 hexadecimal characters",
            Self::Random => "no random bytes",
        })
    }
}

impl std::error::Error for KeyError {}

/// The gateway's key.
pub struct GatewayKey {
    key: LessSafeKey,
    rng: SystemRandom,
}

impl std::fmt::Debug for GatewayKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("GatewayKey")
    }
}

impl GatewayKey {
    /// A new random key, as the 64 hexadecimal characters a key file holds.
    pub fn generate_hex() -> Result<Zeroizing<String>, KeyError> {
        let mut bytes = Zeroizing::new([0u8; 32]);
        SystemRandom::new()
            .fill(&mut bytes[..])
            .map_err(|_| KeyError::Random)?;
        let mut out = Zeroizing::new(String::with_capacity(64));
        for b in bytes.iter() {
            out.push_str(&format!("{b:02x}"));
        }
        Ok(out)
    }

    /// The key from a key file's text (64 hexadecimal characters, surrounding white space ignored).
    pub fn from_hex(text: &str) -> Result<Self, KeyError> {
        let text = text.trim();
        if text.len() != 64 || !text.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(KeyError::Malformed);
        }
        let mut bytes = Zeroizing::new([0u8; 32]);
        for (i, b) in bytes.iter_mut().enumerate() {
            *b =
                u8::from_str_radix(&text[2 * i..2 * i + 2], 16).map_err(|_| KeyError::Malformed)?;
        }
        let key = UnboundKey::new(&AES_256_GCM, &bytes[..]).map_err(|_| KeyError::Malformed)?;
        Ok(Self {
            key: LessSafeKey::new(key),
            rng: SystemRandom::new(),
        })
    }

    /// The sealed form of a token (base64url, no padding).
    pub fn seal(&self, token: &str) -> Result<String, KeyError> {
        let mut nonce = [0u8; NONCE_LEN];
        self.rng.fill(&mut nonce).map_err(|_| KeyError::Random)?;
        let mut data = token.as_bytes().to_vec();
        self.key
            .seal_in_place_append_tag(
                Nonce::assume_unique_for_key(nonce),
                Aad::from(CONTEXT),
                &mut data,
            )
            .map_err(|_| KeyError::Random)?;
        let mut out = nonce.to_vec();
        out.extend_from_slice(&data);
        Ok(URL_SAFE_NO_PAD.encode(out))
    }

    /// The token a sealed part holds, when this key sealed it and it is a token.
    pub fn open(&self, sealed: &str) -> Option<Zeroizing<String>> {
        if sealed.len() > MAX_SEALED {
            return None;
        }
        let raw = Zeroizing::new(URL_SAFE_NO_PAD.decode(sealed).ok()?);
        if raw.len() < NONCE_LEN {
            return None;
        }
        let (nonce, sealed) = raw.split_at(NONCE_LEN);
        let nonce = Nonce::try_assume_unique_for_key(nonce).ok()?;
        let mut data = Zeroizing::new(sealed.to_vec());
        let plain = self
            .key
            .open_in_place(nonce, Aad::from(CONTEXT), &mut data[..])
            .ok()?;
        let token = Zeroizing::new(String::from_utf8(plain.to_vec()).ok()?);
        token_ok(&token).then_some(token)
    }
}

/// Whether `token` looks like an FCM registration token: 1 to [`MAX_TOKEN`] characters of
/// `[A-Za-z0-9:_-]`.
pub fn token_ok(token: &str) -> bool {
    !token.is_empty()
        && token.len() <= MAX_TOKEN
        && token
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b':' | b'_' | b'-'))
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn a_sealed_token_opens_with_its_key_only_and_not_when_altered() {
        let key = GatewayKey::from_hex(&GatewayKey::generate_hex().unwrap()).unwrap();
        let other = GatewayKey::from_hex(&GatewayKey::generate_hex().unwrap()).unwrap();
        let token = "made-up-fcm-token:APA91b_x-1";
        let sealed = key.seal(token).unwrap();
        assert!(!sealed.contains("made-up"));
        assert_ne!(sealed, key.seal(token).unwrap(), "a fresh nonce each time");
        assert_eq!(key.open(&sealed).unwrap().as_str(), token);
        assert!(other.open(&sealed).is_none());
        let mut altered = sealed.into_bytes();
        let last = altered.len() - 3;
        altered[last] = if altered[last] == b'A' { b'B' } else { b'A' };
        assert!(key.open(&String::from_utf8(altered).unwrap()).is_none());
        assert!(key.open("").is_none());
        assert!(key.open("not base64 !").is_none());
        assert!(key.open(&"A".repeat(MAX_SEALED + 1)).is_none());
        // A sealed value that is not a token is not one.
        assert!(key.open(&key.seal("has space").unwrap()).is_none());
    }

    #[test]
    fn keys_and_tokens_are_checked() {
        assert_eq!(
            GatewayKey::from_hex("abc").unwrap_err(),
            KeyError::Malformed
        );
        assert!(GatewayKey::from_hex(&"g".repeat(64)).is_err());
        assert!(GatewayKey::from_hex(&format!(" {}\n", "0f".repeat(32))).is_ok());
        assert!(token_ok("abc:DEF_12-3"));
        assert!(!token_ok(""));
        assert!(!token_ok("a b"));
        assert!(!token_ok("a/b"));
        assert!(!token_ok(&"a".repeat(MAX_TOKEN + 1)));
        assert!(token_ok(&"a".repeat(MAX_TOKEN)));
    }
}
