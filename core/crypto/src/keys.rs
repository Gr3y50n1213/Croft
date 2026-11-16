// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Keys: generate, read, describe, export, and which of a key's parts may be used (T41a).
//!
//! Validity is decided here, not by rPGP's `verify_bindings` (which refuses a whole key for one third-party certification it
//! cannot check): for each user ID and subkey the newest self-signature that verifies counts; a component is used only when
//! that signature is not revoked, not expired and made with a strong hash. MD5 and RIPEMD-160 are never accepted, nor SHA-1 in
//! a signature made after 2023-02-01 (the cut-off Sequoia uses after the SHA-1 chosen-prefix collisions).
//!
//! Hostile keys are bounded (T41a review): at most [`MAX_SUBKEYS`] subkeys and [`MAX_SIGS_PER_PART`] of the owner's own
//! signatures per part are looked at; DSA, ElGamal and RSA under 2048 bits are never used; a protected secret key whose
//! passphrase hashing would cost a phone too much is refused before unlocking.

use pgp::composed::{
    Deserializable, EncryptionCaps, KeyType, SecretKeyParamsBuilder, SignedPublicKey,
    SignedSecretKey, SubkeyParamsBuilder,
};
use pgp::crypto::ecc_curve::ECCCurve;
use pgp::crypto::hash::HashAlgorithm;
use pgp::crypto::public_key::PublicKeyAlgorithm;
use pgp::crypto::sym::SymmetricKeyAlgorithm;
use pgp::packet::{PublicKey, PublicSubkey, Signature, SignatureType};
use pgp::ser::Serialize;
use pgp::types::{
    KeyDetails, Password, PublicParams, S2kParams, SecretParams, StringToKey, Tag, Timestamp,
};
use rand::rngs::OsRng;
use smallvec::smallvec;
use zeroize::Zeroizing;

use crate::{CryptoError, Result, SecretBytes, MAX_KEY_BYTES};

/// SHA-1 signatures made at or after this moment (2023-02-01) are not accepted.
pub const SHA1_CUTOFF: u32 = 1_675_209_600;
/// The most subkeys looked at in one key.
pub const MAX_SUBKEYS: usize = 16;
/// The most of the owner's own signatures looked at on one part of a key (each is a public-key operation).
pub const MAX_SIGS_PER_PART: usize = 32;

/// A secret key, unprotected, in memory. `Debug` prints nothing of it (rPGP's own `Debug` prints the secret of a key of an
/// algorithm it does not know).
#[derive(Clone)]
pub struct SecretKey(pub(crate) SignedSecretKey);

impl SecretKey {
    /// The rPGP key inside, for this crate's tests only (they build hostile messages with rPGP directly).
    #[doc(hidden)]
    pub fn rpgp_for_tests(&self) -> &SignedSecretKey {
        &self.0
    }
}

impl std::fmt::Debug for SecretKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SecretKey")
    }
}

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
    // The exact size first: a growing buffer would leave unwiped copies behind (T41a review).
    let mut out = Zeroizing::new(Vec::with_capacity(key.write_len()));
    key.to_writer(&mut *out).map_err(|_| CryptoError::Failed)?;
    Ok(SecretBytes(out))
}

/// Upper-case hex without spaces (rPGP's `Display` of a fingerprint is lower-case hex).
fn hex(s: &str) -> String {
    s.chars()
        .filter(char::is_ascii_hexdigit)
        .collect::<String>()
        .to_ascii_uppercase()
}

/// The fingerprint of a certificate, upper-case hex.
pub fn fingerprint(cert: &SignedPublicKey) -> String {
    hex(&cert.fingerprint().to_string())
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

/// Whether a passphrase's hashing would take more memory or time than a phone should spend: Argon2 above 128 MiB or 8 passes
/// (iterated SHA hashing is bounded by its format, about 65 MB per part, and the parts are capped).
fn too_costly(params: &S2kParams) -> bool {
    let s2k = match params {
        S2kParams::Aead { s2k, .. }
        | S2kParams::Cfb { s2k, .. }
        | S2kParams::MalleableCfb { s2k, .. } => s2k,
        S2kParams::Unprotected | S2kParams::LegacyCfb { .. } => return false,
    };
    matches!(s2k, StringToKey::Argon2 { t, m_enc, .. } if *t > 8 || *m_enc > 17)
}

/// Reads one secret key (armored or binary) and returns it unprotected: a key protected with a passphrase is unlocked with
/// `passphrase` (the platform keystore protects it from then on). A wrong passphrase is [`CryptoError::WrongPassphrase`].
pub fn read_secret(bytes: &[u8], passphrase: &str) -> Result<SecretKey> {
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
    // Refused before any unlocking: too many parts, or passphrase hashing too costly for a phone (T41a review).
    if key.secret_subkeys.len() > MAX_SUBKEYS || key.public_subkeys.len() > MAX_SUBKEYS {
        return Err(CryptoError::Unreadable);
    }
    let costly = std::iter::once(key.primary_key.secret_params())
        .chain(key.secret_subkeys.iter().map(|s| s.key.secret_params()))
        .any(|p| match p {
            SecretParams::Encrypted(e) => too_costly(e.string_to_key_params()),
            SecretParams::Plain(_) => false,
        });
    if costly {
        return Err(CryptoError::Unreadable);
    }
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
    Ok(SecretKey(key))
}

/// Reads a secret key Croft stored itself (binary, unprotected): from the platform keystore.
pub fn load_secret(secret: &SecretBytes) -> Result<SecretKey> {
    read_secret(&secret.0, "")
}

/// The secret key as Croft stores it (binary, unprotected), for the platform keystore.
pub fn store_secret(key: &SecretKey) -> Result<SecretBytes> {
    secret_bytes(&key.0)
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
pub fn export_secret(key: &SecretKey, passphrase: &str) -> Result<Zeroizing<String>> {
    if passphrase.is_empty() {
        return Err(CryptoError::WrongPassphrase);
    }
    let pw = Password::from(passphrase);
    let mut copy = key.0.clone();
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
    /// The addresses of its valid user IDs, ASCII lower-case, in order.
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
    /// The primary key's algorithm, for display (`EdDSALegacy`, `RSA`, ...).
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

/// Whether a signature counts at `now`: it has a creation time no later than `now`, and its own expiry (if any) has not
/// passed (T41a review).
pub(crate) fn live(sig: &Signature, now: u32) -> bool {
    let Some(c) = sig.created().map(Timestamp::as_secs) else {
        return false;
    };
    c <= now
        && sig
            .signature_expiration_time()
            .map(|d| d.as_secs())
            .filter(|d| *d > 0)
            .is_none_or(|d| c.saturating_add(d) > now)
}

fn created(sig: &Signature) -> u32 {
    sig.created().map_or(0, Timestamp::as_secs)
}

/// The signatures that may be the key owner's own: those naming `primary` as issuer (or naming none), live at `now`, newest
/// first, at most [`MAX_SIGS_PER_PART`] (T41a review: each is a public-key operation).
fn own_sigs<'a>(sigs: &'a [Signature], primary: &PublicKey, now: u32) -> Vec<&'a Signature> {
    let fp = primary.fingerprint();
    let id = primary.legacy_key_id();
    let mut out: Vec<&Signature> = sigs
        .iter()
        .filter(|s| live(s, now))
        .filter(|s| {
            let fps = s.issuer_fingerprint();
            let ids = s.issuer_key_id();
            (fps.is_empty() && ids.is_empty())
                || fps.iter().any(|f| **f == fp)
                || ids.iter().any(|k| **k == id)
        })
        .collect();
    out.sort_by_key(|s| std::cmp::Reverse(created(s)));
    out.truncate(MAX_SIGS_PER_PART);
    out
}

/// Whether a key's algorithm and size are strong enough to use (T41a review): no DSA or ElGamal, RSA of 2048 bits or more.
fn strong_key(params: &PublicParams, alg: PublicKeyAlgorithm) -> bool {
    match alg {
        PublicKeyAlgorithm::DSA
        | PublicKeyAlgorithm::Elgamal
        | PublicKeyAlgorithm::ElgamalEncrypt
        | PublicKeyAlgorithm::DiffieHellman => false,
        PublicKeyAlgorithm::RSA | PublicKeyAlgorithm::RSAEncrypt | PublicKeyAlgorithm::RSASign => {
            // The first two bytes of an MPI are its length in bits: here, the modulus's.
            params
                .to_bytes()
                .ok()
                .and_then(|b| b.get(..2).map(|h| u16::from_be_bytes([h[0], h[1]])))
                .is_some_and(|bits| bits >= 2048)
        }
        _ => true,
    }
}

/// The primary key's state at `now`: the newest valid self-signature on a user ID (or a direct-key signature), whether a valid
/// key revocation exists, and the addresses of the valid user IDs.
struct PrimaryState<'a> {
    binding: Option<&'a Signature>,
    revoked: bool,
    addresses: Vec<String>,
}

fn primary_state(cert: &SignedPublicKey, now: u32) -> PrimaryState<'_> {
    let key = &cert.primary_key;
    let revoked = own_sigs(&cert.details.revocation_signatures, key, now)
        .into_iter()
        .any(|s| s.typ() == Some(SignatureType::KeyRevocation) && s.verify_key(key).is_ok());
    let mut binding: Option<&Signature> = None;
    let mut addresses = Vec::new();
    for user in cert.details.users.iter().take(MAX_SIGS_PER_PART) {
        // The newest self-signature on this user ID that verifies: a certification makes it valid, a revocation not.
        let newest = own_sigs(&user.signatures, key, now)
            .into_iter()
            .filter(|s| strong(s))
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
            .find(|s| s.verify_certification(key, Tag::UserId, &user.id).is_ok());
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
        // The primary user ID's binding wins; otherwise the newest.
        let better = match binding {
            None => true,
            Some(b) => {
                (sig.is_primary() && !b.is_primary())
                    || (sig.is_primary() == b.is_primary() && created(sig) > created(b))
            }
        };
        if better {
            binding = Some(sig);
        }
    }
    if binding.is_none() {
        binding = own_sigs(&cert.details.direct_signatures, key, now)
            .into_iter()
            .find(|s| {
                s.typ() == Some(SignatureType::Key) && strong(s) && s.verify_key(key).is_ok()
            });
    }
    PrimaryState {
        binding,
        revoked,
        addresses,
    }
}

/// The address in a user ID (`Name <a@b>` or a bare address), ASCII lower-case. For showing which addresses a key names; an
/// address that must match a sender is checked with [`bare_address`].
pub fn address_of(user_id: &str) -> Option<String> {
    let s = user_id.trim();
    let inner = match (s.rfind('<'), s.rfind('>')) {
        (Some(a), Some(b)) if a < b => &s[a + 1..b],
        _ => s,
    };
    bare_address(inner)
}

/// A bare address (`a@b`, nothing else: no name, brackets, comment, list or space), ASCII lower-cased (never Unicode case
/// folding, which turns the Kelvin sign into `k`); `None` otherwise.
pub fn bare_address(address: &str) -> Option<String> {
    let a = address.trim();
    if a.is_empty()
        || a.len() > 320
        || a.chars().any(|c| {
            c.is_whitespace()
                || c.is_control()
                || matches!(c, '<' | '>' | '(' | ')' | ',' | '"' | ';' | '\\')
        })
        || a.matches('@').count() != 1
    {
        return None;
    }
    let (local, domain) = a.split_once('@')?;
    if local.is_empty() || domain.is_empty() {
        return None;
    }
    Some(a.to_ascii_lowercase())
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

/// A subkey's valid binding at `now` (the newest that verifies, with a strong hash, and the back-signature when it signs), or
/// `None` when it has none or is revoked.
fn subkey_binding<'a>(
    primary: &PublicKey,
    key: &PublicSubkey,
    sigs: &'a [Signature],
    now: u32,
) -> Option<&'a Signature> {
    let own = own_sigs(sigs, primary, now);
    let revoked = own.iter().any(|s| {
        s.typ() == Some(SignatureType::SubkeyRevocation)
            && s.verify_subkey_binding(primary, key).is_ok()
    });
    if revoked {
        return None;
    }
    own.into_iter()
        .filter(|s| s.typ() == Some(SignatureType::SubkeyBinding) && strong(s))
        .filter(|s| {
            !s.key_flags().sign()
                || s.embedded_signature().is_some_and(|b| {
                    strong(b) && live(b, now) && b.verify_primary_key_binding(key, primary).is_ok()
                })
        })
        .find(|s| s.verify_subkey_binding(primary, key).is_ok())
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

/// What of `cert` may be used at `now`: nothing when the primary key is revoked, expired, too weak or has no valid
/// self-signature.
pub fn usable(cert: &SignedPublicKey, now: u32) -> Usable<'_> {
    usable_with(cert, &primary_state(cert, now), now)
}

fn usable_with<'a>(cert: &'a SignedPublicKey, state: &PrimaryState<'_>, now: u32) -> Usable<'a> {
    let mut out = Usable {
        encryption: vec![],
        signing: vec![],
    };
    let Some(binding) = state.binding else {
        return out;
    };
    if state.revoked
        || expired(cert.primary_key.created_at().as_secs(), binding, now).1
        || !strong_key(
            cert.primary_key.public_params(),
            cert.primary_key.algorithm(),
        )
    {
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
    for sub in cert.public_subkeys.iter().take(MAX_SUBKEYS) {
        if !strong_key(sub.key.public_params(), sub.key.algorithm()) {
            continue;
        }
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
    let use_now = usable_with(cert, &state, now);
    KeyInfo {
        fingerprint: fingerprint(cert),
        can_encrypt: !use_now.encryption.is_empty(),
        can_sign: !use_now.signing.is_empty(),
        addresses: state.addresses,
        created: created_at,
        expires,
        revoked: state.revoked,
        expired: is_expired,
        algorithm: format!("{:?}", cert.primary_key.algorithm()),
    }
}

/// The public certificate of a secret key.
pub fn public_of(key: &SecretKey) -> SignedPublicKey {
    key.0.to_public_key()
}
