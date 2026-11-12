// This Source Code Form is subject to the terms of the GNU Affero General Public License,
// version 3 or later. See LICENSE-AGPL-3.0 at the repository root.

//! Envelope encryption (T23).
//!
//! - The **master key** (32 random bytes) lives in a file the operator keeps outside the database.
//! - Each user has a random 32-byte **data key**, stored only wrapped: AES-256-GCM under the master
//!   key, with the user id in the associated data.
//! - Every secret the Companion keeps for a user (a later task's mail password, token or cached
//!   mail) is **sealed** with AES-256-GCM under that user's data key, with a context string and the
//!   user id in the associated data, so a value moved to another user or another field never opens.
//!
//! The sealed form is `version (1 byte) || nonce (12) || ciphertext || tag (16)`. Keys are wiped
//! from memory when dropped. Nothing here prints a key or a plaintext.

use ring::aead::{Aad, LessSafeKey, Nonce, UnboundKey, AES_256_GCM, NONCE_LEN};
use ring::rand::{SecureRandom, SystemRandom};
use zeroize::Zeroizing;

/// The sealed form's version.
const VERSION: u8 = 1;

/// Why a key or a sealed value could not be used. No variant carries key material.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CryptoError {
    /// No random numbers.
    Random,
    /// The value does not open with this key and context (wrong key, wrong user, wrong field, or
    /// damaged).
    Open,
    /// A key is not 32 bytes.
    BadKey,
}

impl std::fmt::Display for CryptoError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Random => "no random numbers available",
            Self::Open => "the value does not open with this key",
            Self::BadKey => "a key is not 32 bytes",
        })
    }
}

impl std::error::Error for CryptoError {}

/// A 32-byte key, wiped when dropped. `Debug` prints nothing of it.
#[derive(Clone)]
pub struct Key(Zeroizing<[u8; 32]>);

impl std::fmt::Debug for Key {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Key(***)")
    }
}

impl Key {
    /// A new random key.
    pub fn generate() -> Result<Self, CryptoError> {
        let mut k = Zeroizing::new([0u8; 32]);
        SystemRandom::new()
            .fill(&mut k[..])
            .map_err(|_| CryptoError::Random)?;
        Ok(Self(k))
    }

    /// A key from exactly 32 bytes.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, CryptoError> {
        let arr: [u8; 32] = bytes.try_into().map_err(|_| CryptoError::BadKey)?;
        Ok(Self(Zeroizing::new(arr)))
    }

    /// The key as lower-case hex (for the master key file).
    pub fn to_hex(&self) -> Zeroizing<String> {
        let mut s = Zeroizing::new(String::with_capacity(64));
        for b in self.0.iter() {
            s.push_str(&format!("{b:02x}"));
        }
        s
    }

    /// A key from 64 hex digits (surrounding whitespace ignored).
    pub fn from_hex(text: &str) -> Result<Self, CryptoError> {
        let t = text.trim();
        // Exactly 64 hex digits: `from_str_radix` alone would also take a sign (`+1`).
        if t.len() != 64 || !t.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(CryptoError::BadKey);
        }
        let mut k = Zeroizing::new([0u8; 32]);
        for (i, byte) in k.iter_mut().enumerate() {
            *byte = u8::from_str_radix(t.get(i * 2..i * 2 + 2).ok_or(CryptoError::BadKey)?, 16)
                .map_err(|_| CryptoError::BadKey)?;
        }
        Ok(Self(k))
    }

    fn aead(&self) -> Result<LessSafeKey, CryptoError> {
        let unbound =
            UnboundKey::new(&AES_256_GCM, &self.0[..]).map_err(|_| CryptoError::BadKey)?;
        Ok(LessSafeKey::new(unbound))
    }

    /// Seals `plaintext` with `aad` (bound to the result: the same `aad` is needed to open it).
    pub fn seal(&self, aad: &[u8], plaintext: &[u8]) -> Result<Vec<u8>, CryptoError> {
        let mut nonce = [0u8; NONCE_LEN];
        SystemRandom::new()
            .fill(&mut nonce)
            .map_err(|_| CryptoError::Random)?;
        let mut buf = Zeroizing::new(plaintext.to_vec());
        self.aead()?
            .seal_in_place_append_tag(
                Nonce::assume_unique_for_key(nonce),
                Aad::from(aad),
                &mut *buf,
            )
            .map_err(|_| CryptoError::Open)?;
        let mut out = Vec::with_capacity(1 + NONCE_LEN + buf.len());
        out.push(VERSION);
        out.extend_from_slice(&nonce);
        out.extend_from_slice(&buf);
        Ok(out)
    }

    /// Opens what [`Key::seal`] made with the same `aad`.
    pub fn open(&self, aad: &[u8], sealed: &[u8]) -> Result<Zeroizing<Vec<u8>>, CryptoError> {
        if sealed.len() < 1 + NONCE_LEN + 16 || sealed[0] != VERSION {
            return Err(CryptoError::Open);
        }
        let nonce: [u8; NONCE_LEN] = sealed[1..1 + NONCE_LEN]
            .try_into()
            .map_err(|_| CryptoError::Open)?;
        let mut buf = Zeroizing::new(sealed[1 + NONCE_LEN..].to_vec());
        let plain_len = self
            .aead()?
            .open_in_place(
                Nonce::assume_unique_for_key(nonce),
                Aad::from(aad),
                &mut buf,
            )
            .map_err(|_| CryptoError::Open)?
            .len();
        buf.truncate(plain_len);
        Ok(buf)
    }
}

/// The associated data of a user's wrapped data key.
fn data_key_aad(user_id: i64) -> Vec<u8> {
    format!("croft-companion/data-key/v1/user/{user_id}").into_bytes()
}

/// The associated data of a value sealed for a user under a context (`"mail-password"`, ...).
fn value_aad(user_id: i64, context: &str) -> Vec<u8> {
    format!("croft-companion/value/v1/{context}/user/{user_id}").into_bytes()
}

/// A new data key for `user_id`, and its wrapped form (to store).
pub fn new_data_key(master: &Key, user_id: i64) -> Result<(Key, Vec<u8>), CryptoError> {
    let key = Key::generate()?;
    let wrapped = master.seal(&data_key_aad(user_id), &key.0[..])?;
    Ok((key, wrapped))
}

/// Unwraps `user_id`'s data key.
pub fn unwrap_data_key(master: &Key, user_id: i64, wrapped: &[u8]) -> Result<Key, CryptoError> {
    let raw = master.open(&data_key_aad(user_id), wrapped)?;
    Key::from_bytes(&raw)
}

/// Re-wraps `user_id`'s data key under another master key (rotation).
pub fn rewrap_data_key(
    old: &Key,
    new: &Key,
    user_id: i64,
    wrapped: &[u8],
) -> Result<Vec<u8>, CryptoError> {
    let key = unwrap_data_key(old, user_id, wrapped)?;
    new.seal(&data_key_aad(user_id), &key.0[..])
}

/// Seals a secret for `user_id` under `context` with the user's data key.
pub fn seal_value(
    data_key: &Key,
    user_id: i64,
    context: &str,
    plaintext: &[u8],
) -> Result<Vec<u8>, CryptoError> {
    data_key.seal(&value_aad(user_id, context), plaintext)
}

/// Opens what [`seal_value`] made, for the same user and context only.
pub fn open_value(
    data_key: &Key,
    user_id: i64,
    context: &str,
    sealed: &[u8],
) -> Result<Zeroizing<Vec<u8>>, CryptoError> {
    data_key.open(&value_aad(user_id, context), sealed)
}

/// Compares two secrets in time that depends only on their lengths (never on where they differ).
pub fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let diff = a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y));
    std::hint::black_box(diff) == 0
}

/// The associated data of the master key check value (see the store).
pub(crate) const MASTER_CHECK_AAD: &[u8] = b"croft-companion/master-check/v1";

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn ct_eq_compares_whole_values() {
        assert!(ct_eq(b"", b""));
        assert!(ct_eq(b"abc", b"abc"));
        assert!(!ct_eq(b"abc", b"abd"));
        assert!(!ct_eq(b"abc", b"xbc"));
        assert!(!ct_eq(b"abc", b"abcd"));
    }

    #[test]
    fn a_sealed_value_opens_only_for_its_user_and_context() {
        let master = Key::generate().unwrap();
        let (k7, wrapped7) = new_data_key(&master, 7).unwrap();
        let sealed = seal_value(&k7, 7, "mail-password", b"made-up-secret").unwrap();
        assert_eq!(
            &**open_value(&k7, 7, "mail-password", &sealed).unwrap(),
            b"made-up-secret"
        );
        assert_eq!(
            open_value(&k7, 8, "mail-password", &sealed).unwrap_err(),
            CryptoError::Open
        );
        assert_eq!(
            open_value(&k7, 7, "oauth-token", &sealed).unwrap_err(),
            CryptoError::Open
        );
        // The unwrapped key is the same key.
        let again = unwrap_data_key(&master, 7, &wrapped7).unwrap();
        assert_eq!(
            &**open_value(&again, 7, "mail-password", &sealed).unwrap(),
            b"made-up-secret"
        );
        // A wrapped key moved to another user does not unwrap.
        assert_eq!(
            unwrap_data_key(&master, 8, &wrapped7).unwrap_err(),
            CryptoError::Open
        );
    }

    #[test]
    fn another_master_key_opens_nothing_and_rotation_keeps_every_value() {
        let master = Key::generate().unwrap();
        let (k, wrapped) = new_data_key(&master, 1).unwrap();
        let sealed = seal_value(&k, 1, "c", b"v").unwrap();
        let other = Key::generate().unwrap();
        assert_eq!(
            unwrap_data_key(&other, 1, &wrapped).unwrap_err(),
            CryptoError::Open
        );
        let rewrapped = rewrap_data_key(&master, &other, 1, &wrapped).unwrap();
        let k2 = unwrap_data_key(&other, 1, &rewrapped).unwrap();
        assert_eq!(&**open_value(&k2, 1, "c", &sealed).unwrap(), b"v");
        assert_eq!(
            unwrap_data_key(&master, 1, &rewrapped).unwrap_err(),
            CryptoError::Open
        );
    }

    #[test]
    fn damaged_or_foreign_input_never_opens_and_keys_never_print() {
        let k = Key::generate().unwrap();
        let mut sealed = seal_value(&k, 1, "c", b"v").unwrap();
        let last = sealed.len() - 1;
        sealed[last] ^= 1;
        assert!(open_value(&k, 1, "c", &sealed).is_err());
        assert!(open_value(&k, 1, "c", b"short").is_err());
        assert!(open_value(&k, 1, "c", &[9u8; 40]).is_err());
        assert_eq!(format!("{k:?}"), "Key(***)");
        let hex = k.to_hex();
        assert_eq!(Key::from_hex(&hex).unwrap().to_hex(), hex);
        assert!(Key::from_hex("zz").is_err());
        // Two seals of the same value differ (random nonce).
        assert_ne!(
            seal_value(&k, 1, "c", b"v").unwrap(),
            seal_value(&k, 1, "c", b"v").unwrap()
        );
    }
}
