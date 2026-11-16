// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Keys: generate, read, describe, export, and which of a key's parts may be used (T41a).
//!
//! Validity is decided here, not by rPGP's `verify_bindings` (which refuses a whole key for one third-party certification it
//! cannot check): for each user ID and subkey the newest self-signature that verifies counts; a component is used only when
//! that signature is not revoked, not expired and made with a strong hash. MD5 and RIPEMD-160 are never accepted, nor SHA-1 in
//! a signature made after 2023-02-01 (the cut-off Sequoia uses after the SHA-1 chosen-prefix collisions).

use pgp::composed::{
    Deserializable, EncryptionCaps, KeyType, SecretKeyParamsBuilder, SignedPublicKey,
    SignedSecretKey, SubkeyParamsBuilder,
};
use pgp::crypto::ecc_curve::ECCCurve;
use pgp::crypto::hash::HashAlgorithm;
use pgp::crypto::sym::SymmetricKeyAlgorithm;
use pgp::packet::{PublicKey, PublicSubkey, Signature, SignatureType};
use pgp::ser::Serialize;
use pgp::types::{KeyDetails, Password, Tag, Timestamp};
use rand::rngs::OsRng;
use smallvec::smallvec;
use zeroize::Zeroizing;

use crate::{CryptoError, Result, SecretBytes, MAX_KEY_BYTES};

/// SHA-1 signatures made at or after this moment (2023-02-01) are not accepted.
pub const SHA1_CUTOFF: u32 = 1_675_209_600;

/// A new key: the secret key (unprotected, binary) and its public certificate (binary).
#[derive(Debug)]
pub struct Generated {
    /// The secret key, for the platform keystore only.
    pub secret: SecretBytes,
    /// The public certificate (what is shared).
    pub public: Vec<u8>,
    /// The fingerprint, upper-case hex.
    pub fingerprint: String,
}

/// Makes a new v4 key for `user_id` (`Name <address>`), created at `now` (Unix seconds): an Ed25519 primary key that certifies
/// and signs, and a Curve25519 ECDH encryption subkey; AES-256 then AES-128, SHA-512 then SHA-256, no compression; no expiry.
/// v4 with these curves is what every common OpenPGP program reads today.
pub fn generate(user_id: &str, now: u32) -> Result<Generated> {
    let uid = user_id.trim();
    if uid.is_empty() || uid.len() > 512 || uid.chars().any(char::is_control) {
        return Err(CryptoError::BadAddress);
    }
    let created = Timestamp::from_secs(now);
    let mut encrypt = SubkeyParamsBuilder::default();
    encrypt
        .key_type(KeyType::ECDH(ECCCurve::Curve25519Legacy))
        .can_sign(false)
        .can_encrypt(EncryptionCaps::All)
        .can_authenticate(false)
        .created_at(created);
    let mut params = SecretKeyParamsBuilder::default();
    params
        .key_type(KeyType::Ed25519Legacy)
        .can_certify(true)
        .can_sign(true)
        .can_encrypt(EncryptionCaps::None)
        .created_at(created)
        .preferred_symmetric_algorithms(smallvec![
            SymmetricKeyAlgorithm::AES256,
            SymmetricKeyAlgorithm::AES128
        ])
        .preferred_hash_algorithms(smallvec![HashAlgorithm::Sha512, HashAlgorithm::Sha256])
        .preferred_compression_algorithms(smallvec![])
        .primary_user_id(uid.to_owned())
        .subkeys(vec![encrypt.build().map_err(|_| CryptoError::Failed)?]);
    let secret = params
        .build()
        .map_err(|_| CryptoError::Failed)?
        .generate(OsRng)
        .map_err(|_| CryptoError::Failed)?;
    let public = SignedPublicKey::from(secret.clone());
    Ok(Generated {
        fingerprint: hex(&secret.fingerprint().to_string()),
        secret: secret_bytes(&secret)?,
        public: public.to_bytes().map_err(|_| CryptoError::Failed)?,
    })
}

fn secret_bytes(key: &SignedSecretKey) -> Result<SecretBytes> {
    let mut out = Zeroizing::new(Vec::new());
    key.to_writer(&mut *out).map_err(|_| CryptoError::Failed)?;
    Ok(SecretBytes(out))
}

/// Upper-case hex without spaces (rPGP's `Display` of a fingerprint is lower-case hex).
fn hex(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_ascii_hexdigit())
        .collect::<String>()
        .to_ascii_uppercase()
}

/// Reads one public certificate (armored or binary). A file with no key or more than one is refused.
pub fn read_public(bytes: &[u8]) -> Result<SignedPublicKey> {
    if bytes.len() > MAX_KEY_BYTES {
        return Err(CryptoError::TooLarge);
    }
    one_block(bytes)?;
    let mut keys = if looks_armored(bytes) {
        SignedPublicKey::from_armor_many(bytes)
            .map_err(|_| CryptoError::Unreadable)?
            .0
            .collect::<Vec<_>>()
    } else {
        SignedPublicKey::from_bytes_many(bytes)
            .map_err(|_| CryptoError::Unreadable)?
            .collect::<Vec<_>>()
    };
    if keys.len() != 1 {
        return Err(if keys.is_empty() {
            CryptoError::Unreadable
        } else {
            CryptoError::NotOneKey
        });
    }
    keys.pop()
        .ok_or(CryptoError::Unreadable)?
        .map_err(|_| CryptoError::Unreadable)
}

/// Reads one secret key (armored or binary) and returns it unprotected: a key protected with a passphrase is unlocked with
/// `passphrase` (the platform keystore protects it from then on). A wrong passphrase is [`CryptoError::WrongPassphrase`].
pub fn read_secret(bytes: &[u8], passphrase: &str) -> Result<SignedSecretKey> {
    if bytes.len() > MAX_KEY_BYTES {
        return Err(CryptoError::TooLarge);
    }
    one_block(bytes)?;
    let mut keys = if looks_armored(bytes) {
        SignedSecretKey::from_armor_many(bytes)
            .map_err(|_| CryptoError::Unreadable)?
            .0
            .collect::<Vec<_>>()
    } else {
        SignedSecretKey::from_bytes_many(bytes)
            .map_err(|_| CryptoError::Unreadable)?
            .collect::<Vec<_>>()
    };
    if keys.len() != 1 {
        return Err(if keys.is_empty() {
            CryptoError::Unreadable
        } else {
            CryptoError::NotOneKey
        });
    }
    let mut key = keys
        .pop()
        .ok_or(CryptoError::Unreadable)?
        .map_err(|_| CryptoError::Unreadable)?;
    let pw = Password::from(passphrase);
    if key.primary_key.secret_params().is_encrypted() {
        key.primary_key
            .remove_password(&pw)
            .map_err(|_| CryptoError::WrongPassphrase)?;
    }
    for sub in key.secret_subkeys.iter_mut() {
        if sub.key.secret_params().is_encrypted() {
            sub.key
                .remove_password(&pw)
                .map_err(|_| CryptoError::WrongPassphrase)?;
        }
    }
    Ok(key)
}

/// Reads a secret key Croft stored itself (binary, unprotected): from the platform keystore.
pub fn load_secret(secret: &SecretBytes) -> Result<SignedSecretKey> {
    read_secret(&secret.0, "")
}

/// The secret key as Croft stores it (binary, unprotected), for the platform keystore.
pub fn store_secret(key: &SignedSecretKey) -> Result<SecretBytes> {
    secret_bytes(key)
}

/// Refuses a file with more than one armored block (rPGP reads only the first).
fn one_block(bytes: &[u8]) -> Result<()> {
    let blocks = bytes
        .windows(15)
        .filter(|w| *w == b"-----BEGIN PGP ")
        .count();
    if blocks > 1 {
        return Err(CryptoError::NotOneKey);
    }
    Ok(())
}

fn looks_armored(bytes: &[u8]) -> bool {
    let start = bytes
        .iter()
        .position(|b| !b.is_ascii_whitespace())
        .unwrap_or(0);
    bytes[start..].starts_with(b"-----BEGIN PGP")
}

/// The public certificate, armored (to share).
pub fn export_public(cert: &SignedPublicKey) -> Result<String> {
    cert.to_armored_string(Default::default())
        .map_err(|_| CryptoError::Failed)
}

/// The public certificate, binary (to store).
pub fn public_bytes(cert: &SignedPublicKey) -> Result<Vec<u8>> {
    cert.to_bytes().map_err(|_| CryptoError::Failed)
}

/// The secret key protected with `passphrase` and armored, for a backup. The passphrase must not be empty.
pub fn export_secret(key: &SignedSecretKey, passphrase: &str) -> Result<Zeroizing<String>> {
    if passphrase.is_empty() {
        return Err(CryptoError::WrongPassphrase);
    }
    let pw = Password::from(passphrase);
    let mut copy = key.clone();
    copy.primary_key
        .set_password(OsRng, &pw)
        .map_err(|_| CryptoError::Failed)?;
    for sub in copy.secret_subkeys.iter_mut() {
        sub.key
            .set_password(OsRng, &pw)
            .map_err(|_| CryptoError::Failed)?;
    }
    Ok(Zeroizing::new(
        copy.to_armored_string(Default::default())
            .map_err(|_| CryptoError::Failed)?,
    ))
}

/// What a key is, as Croft shows and uses it.
#[derive(Clone, PartialEq, Eq)]
pub struct KeyInfo {
    /// The fingerprint, upper-case hex.
    pub fingerprint: String,
    /// The addresses of its valid user IDs, lower-case, in order.
    pub addresses: Vec<String>,
    /// When the primary key was made, Unix seconds.
    pub created: u32,
    /// When it expires, if it does.
    pub expires: Option<u32>,
    /// Revoked by its owner.
    pub revoked: bool,
    /// Expired at `now`.
    pub expired: bool,
    /// A valid key can encrypt to it now.
    pub can_encrypt: bool,
    /// A valid key can sign now.
    pub can_sign: bool,
    /// The primary key's algorithm, for display (`EdDSA`, `RSA 3072`, ...).
    pub algorithm: String,
}

impl std::fmt::Debug for KeyInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KeyInfo")
            .field("revoked", &self.revoked)
            .field("expired", &self.expired)
            .field("can_encrypt", &self.can_encrypt)
            .finish_non_exhaustive()
    }
}

/// Whether a self-signature's hash is acceptable.
fn strong(sig: &Signature) -> bool {
    match sig.hash_alg() {
        Some(HashAlgorithm::Md5 | HashAlgorithm::Ripemd160 | HashAlgorithm::None) | None => false,
        Some(HashAlgorithm::Sha1) => sig.created().is_some_and(|t| t.as_secs() < SHA1_CUTOFF),
        Some(_) => true,
    }
}

fn created(sig: &Signature) -> u32 {
    sig.created().map_or(0, Timestamp::as_secs)
}

/// The primary key's state at `now`: the newest valid self-signature on a user ID (or a direct-key signature), and whether
/// a valid key revocation exists.
struct PrimaryState<'a> {
    binding: Option<&'a Signature>,
    revoked: bool,
    addresses: Vec<String>,
}

fn primary_state<'a>(cert: &'a SignedPublicKey, now: u32) -> PrimaryState<'a> {
    let key = &cert.primary_key;
    let revoked = cert.details.revocation_signatures.iter().any(|s| {
        s.typ() == Some(SignatureType::KeyRevocation)
            && created(s) <= now
            && s.verify_key(key).is_ok()
    });
    let mut binding: Option<&Signature> = None;
    let mut addresses = Vec::new();
    for user in &cert.details.users {
        // The newest self-signature on this user ID that verifies: a certification makes it valid, a revocation not.
        let newest = user
            .signatures
            .iter()
            .filter(|s| created(s) <= now && strong(s))
            .filter(|s| {
                matches!(
                    s.typ(),
                    Some(
                        SignatureType::CertGeneric
                            | SignatureType::CertPersona
                            | SignatureType::CertCasual
                            | SignatureType::CertPositive
                            | SignatureType::CertRevocation
                    )
                )
            })
            .filter(|s| s.verify_certification(key, Tag::UserId, &user.id).is_ok())
            .max_by_key(|s| created(s));
        let Some(sig) = newest else {
            continue;
        };
        if sig.typ() == Some(SignatureType::CertRevocation) {
            continue;
        }
        if let Some(addr) = user.id.as_str().and_then(address_of) {
            if !addresses.contains(&addr) {
                addresses.push(addr);
            }
        }
        if binding.is_none_or(|b| {
            sig.is_primary() && !b.is_primary()
                || created(sig) > created(b) && sig.is_primary() == b.is_primary()
        }) {
            binding = Some(sig);
        }
    }
    if binding.is_none() {
        binding = cert
            .details
            .direct_signatures
            .iter()
            .filter(|s| {
                s.typ() == Some(SignatureType::Key)
                    && created(s) <= now
                    && strong(s)
                    && s.verify_key(key).is_ok()
            })
            .max_by_key(|s| created(s));
    }
    PrimaryState {
        binding,
        revoked,
        addresses,
    }
}

/// The address in a user ID (`Name <a@b>` or a bare address), lower-case.
pub fn address_of(user_id: &str) -> Option<String> {
    let s = user_id.trim();
    let inner = match (s.rfind('<'), s.rfind('>')) {
        (Some(a), Some(b)) if a < b => &s[a + 1..b],
        _ => s,
    };
    let inner = inner.trim();
    let (local, domain) = inner.rsplit_once('@')?;
    if local.is_empty()
        || domain.is_empty()
        || inner.chars().any(|c| c.is_whitespace() || c.is_control())
        || inner.len() > 320
    {
        return None;
    }
    Some(inner.to_lowercase())
}

/// Whether a key with this creation time and binding is expired at `now`.
fn expired(created_at: u32, binding: &Signature, now: u32) -> (Option<u32>, bool) {
    match binding
        .key_expiration_time()
        .map(|d| d.as_secs())
        .filter(|d| *d > 0)
    {
        Some(d) => {
            let until = created_at.saturating_add(d);
            (Some(until), now >= until)
        }
        None => (None, false),
    }
}

/// A subkey's valid binding at `now` (the newest that verifies, with a strong hash, and the back-signature when it signs),
/// or `None` when it has none or is revoked.
fn subkey_binding<'a>(
    primary: &PublicKey,
    key: &PublicSubkey,
    sigs: &'a [Signature],
    now: u32,
) -> Option<&'a Signature> {
    let revoked = sigs.iter().any(|s| {
        s.typ() == Some(SignatureType::SubkeyRevocation)
            && created(s) <= now
            && s.verify_subkey_binding(primary, key).is_ok()
    });
    if revoked {
        return None;
    }
    sigs.iter()
        .filter(|s| s.typ() == Some(SignatureType::SubkeyBinding) && created(s) <= now && strong(s))
        .filter(|s| s.verify_subkey_binding(primary, key).is_ok())
        .filter(|s| {
            !s.key_flags().sign()
                || s.embedded_signature().is_some_and(|b| {
                    strong(b) && b.verify_primary_key_binding(key, primary).is_ok()
                })
        })
        .max_by_key(|s| created(s))
}

/// The parts of a key usable at `now`.
pub struct Usable<'a> {
    /// The encryption keys (subkeys, or the primary key when it is flagged to encrypt).
    pub encryption: Vec<EncryptionKey<'a>>,
    /// The signing keys.
    pub signing: Vec<SigningPart<'a>>,
}

/// A key to encrypt to.
pub enum EncryptionKey<'a> {
    /// The primary key.
    Primary(&'a PublicKey),
    /// A subkey.
    Subkey(&'a PublicSubkey),
}

/// A key a signature may come from.
pub enum SigningPart<'a> {
    /// The primary key.
    Primary(&'a PublicKey),
    /// A subkey.
    Subkey(&'a PublicSubkey),
}

/// What of `cert` may be used at `now`: nothing when the primary key is revoked, expired or has no valid self-signature.
pub fn usable(cert: &SignedPublicKey, now: u32) -> Usable<'_> {
    let mut out = Usable {
        encryption: vec![],
        signing: vec![],
    };
    let state = primary_state(cert, now);
    let Some(binding) = state.binding else {
        return out;
    };
    if state.revoked || expired(cert.primary_key.created_at().as_secs(), binding, now).1 {
        return out;
    }
    let flags = binding.key_flags();
    if (flags.encrypt_comms() || flags.encrypt_storage())
        && cert.primary_key.algorithm().can_encrypt()
    {
        out.encryption
            .push(EncryptionKey::Primary(&cert.primary_key));
    }
    if flags.sign() && cert.primary_key.algorithm().can_sign() {
        out.signing.push(SigningPart::Primary(&cert.primary_key));
    }
    for sub in &cert.public_subkeys {
        let Some(b) = subkey_binding(&cert.primary_key, &sub.key, &sub.signatures, now) else {
            continue;
        };
        if expired(sub.key.created_at().as_secs(), b, now).1 {
            continue;
        }
        let f = b.key_flags();
        if (f.encrypt_comms() || f.encrypt_storage()) && sub.key.algorithm().can_encrypt() {
            out.encryption.push(EncryptionKey::Subkey(&sub.key));
        }
        if f.sign() && sub.key.algorithm().can_sign() {
            out.signing.push(SigningPart::Subkey(&sub.key));
        }
    }
    out
}

/// Describes `cert` at `now` (Unix seconds).
pub fn info(cert: &SignedPublicKey, now: u32) -> KeyInfo {
    let state = primary_state(cert, now);
    let created_at = cert.primary_key.created_at().as_secs();
    let (expires, is_expired) = state
        .binding
        .map_or((None, false), |b| expired(created_at, b, now));
    let use_now = usable(cert, now);
    KeyInfo {
        fingerprint: hex(&cert.fingerprint().to_string()),
        addresses: state.addresses,
        created: created_at,
        expires,
        revoked: state.revoked,
        expired: is_expired,
        can_encrypt: !use_now.encryption.is_empty(),
        can_sign: !use_now.signing.is_empty(),
        algorithm: format!("{:?}", cert.primary_key.algorithm()),
    }
}

/// The public certificate of a secret key.
pub fn public_of(key: &SignedSecretKey) -> SignedPublicKey {
    key.to_public_key()
}
