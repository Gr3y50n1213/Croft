// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! PGP/MIME (RFC 3156, T41a): the bodies of `multipart/encrypted` and `multipart/signed`, and reading them back.
//!
//! - Encrypting signs (when asked) and encrypts in one OpenPGP message (SEIPDv1, AES-256), to every recipient's usable
//!   encryption keys and to the sender's own, so the Sent copy stays readable.
//! - Signing makes a detached signature over the inner part with CRLF line ends (its canonical form), without the CRLF before
//!   the boundary (RFC 3156: it belongs to the boundary).
//! - Decrypting reads at most [`MAX_MESSAGE_BYTES`] of plaintext and one level of compression; SEIPDv1 is checked in full
//!   before any plaintext is released. A message with more than [`MAX_RECIPIENT_PACKETS`] recipient packets, more than
//!   [`MAX_MATCHING_PACKETS`] for the person's keys, or more than [`MAX_SIGNATURES`] signatures is refused before the costly
//!   work (T41a review: each is a private-key operation or a pass over the whole message).
//! - A signature counts only with a strong hash (SHA-2 or SHA-3; never SHA-1, MD5 or RIPEMD-160), a creation time no later than
//!   now and not past its own expiry, by a key usable now. One that no given key verifies is
//!   [`SignatureStatus::NotVerified`]: unknown and invalid look the same to the person, and neither is ever shown as trusted.

use std::io::Read;

use pgp::composed::{
    DecryptionOptions, Deserializable, DetachedSignature, Esk, Message, MessageBuilder,
    SignedPublicKey, TheRing, VerificationResult,
};
use pgp::crypto::hash::HashAlgorithm;
use pgp::crypto::sym::SymmetricKeyAlgorithm;
use pgp::packet::Signature;
use pgp::types::{KeyDetails, Password, Seipdv1ReadMode, VerifyingKey};
use rand::rngs::OsRng;
use rand::RngCore;
use zeroize::Zeroizing;

use crate::keys::{self, EncryptionKey, SecretKey, SigningPart};
use crate::{CryptoError, Result, MAX_MESSAGE_BYTES};

/// The most recipient packets one message may carry.
pub const MAX_RECIPIENT_PACKETS: usize = 64;
/// The most recipient packets naming one of the person's keys (or the anonymous ID) that are tried.
pub const MAX_MATCHING_PACKETS: usize = 4;
/// The most signatures one message may carry.
pub const MAX_SIGNATURES: usize = 8;

/// A body to put under the message's own headers: its `Content-Type` value and the text after the blank line.
#[derive(Clone, PartialEq, Eq)]
pub struct MimeBody {
    /// The `Content-Type` header value (with its boundary).
    pub content_type: String,
    /// The body.
    pub body: String,
}

impl std::fmt::Debug for MimeBody {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "MimeBody({} bytes)", self.body.len())
    }
}

/// What a signature check found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SignatureStatus {
    /// Signed, and verified by this certificate (fingerprint, upper-case hex), valid now.
    Valid {
        /// The certificate that verified it.
        fingerprint: String,
    },
    /// Signed, but none of the given certificates verifies it acceptably (an unknown signer, a changed message, a weak hash,
    /// a time out of bounds). `issuer` is the fingerprint or key ID the signature names, when known (upper-case hex).
    NotVerified {
        /// Who the signature says it is from.
        issuer: Option<String>,
    },
    /// Not signed.
    Unsigned,
}

/// A decrypted message: the inner MIME entity (wiped when dropped) and its signature status.
pub struct Decrypted {
    /// The inner entity, as it was encrypted.
    pub inner: Zeroizing<Vec<u8>>,
    /// The signature inside the encrypted message.
    pub signature: SignatureStatus,
}

impl std::fmt::Debug for Decrypted {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Decrypted")
            .field("inner", &self.inner.len())
            .field("signature", &self.signature)
            .finish()
    }
}

fn boundary() -> String {
    let mut seed = [0u8; 12];
    OsRng.fill_bytes(&mut seed);
    let hexed: String = seed.iter().map(|b| format!("{b:02x}")).collect();
    format!("=_croft_pgp_{hexed}")
}

/// Line ends as CRLF everywhere (RFC 3156: the canonical form signed and checked).
pub fn canonical(text: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(text.len() + text.len() / 32);
    let mut i = 0;
    while i < text.len() {
        match text[i] {
            b'\r' if text.get(i + 1) == Some(&b'\n') => {
                out.extend_from_slice(b"\r\n");
                i += 2;
            }
            b'\r' | b'\n' => {
                out.extend_from_slice(b"\r\n");
                i += 1;
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    out
}

/// Whether a message signature is acceptable at `now`: a strong hash (no SHA-1 for message signatures), a binary or text
/// signature, made no later than now and not past its own expiry.
fn acceptable(sig: &Signature, now: u32) -> bool {
    matches!(
        sig.hash_alg(),
        Some(
            HashAlgorithm::Sha224
                | HashAlgorithm::Sha256
                | HashAlgorithm::Sha384
                | HashAlgorithm::Sha512
                | HashAlgorithm::Sha3_256
                | HashAlgorithm::Sha3_512
        )
    ) && keys::live(sig, now)
}

/// The secret counterpart of a usable signing part of `key` at `now`. Boxed: rPGP implements `SigningKey` for
/// `Box<&dyn SigningKey>`, not for a bare `&dyn SigningKey`.
#[allow(clippy::redundant_allocation)]
fn signing_key(key: &SecretKey, now: u32) -> Result<Box<&dyn pgp::types::SigningKey>> {
    let public = keys::public_of(key);
    let usable = keys::usable(&public, now);
    for part in &usable.signing {
        match part {
            SigningPart::Primary(_) => {
                return Ok(Box::new(&key.0.primary_key as &dyn pgp::types::SigningKey))
            }
            SigningPart::Subkey(p) => {
                let fp = p.fingerprint();
                if let Some(s) = key
                    .0
                    .secret_subkeys
                    .iter()
                    .find(|s| s.key.fingerprint() == fp)
                {
                    return Ok(Box::new(&s.key as &dyn pgp::types::SigningKey));
                }
            }
        }
    }
    Err(CryptoError::NoUsableKey)
}

/// Encrypts `inner` (a whole MIME entity, headers and body) to `recipients` and to `own` (the sender's certificate), signed by
/// `signer` when given. Every recipient must have a usable encryption key at `now`; the first that has none is
/// [`CryptoError::NoEncryptionKey`] with its position. Returns the `multipart/encrypted` body.
pub fn encrypt(
    inner: &[u8],
    recipients: &[&SignedPublicKey],
    own: &SignedPublicKey,
    signer: Option<&SecretKey>,
    now: u32,
) -> Result<MimeBody> {
    if inner.len() > MAX_MESSAGE_BYTES {
        return Err(CryptoError::TooLarge);
    }
    if recipients.len() >= MAX_RECIPIENT_PACKETS {
        return Err(CryptoError::TooLarge);
    }
    // Every encryption key first, so nothing is built when one recipient has none.
    let mut targets: Vec<Vec<EncryptionKey<'_>>> = Vec::new();
    for (i, cert) in recipients.iter().enumerate() {
        let enc = keys::usable(cert, now).encryption;
        if enc.is_empty() {
            return Err(CryptoError::NoEncryptionKey(i));
        }
        targets.push(enc);
    }
    let own_keys = keys::usable(own, now).encryption;
    if own_keys.is_empty() {
        return Err(CryptoError::NoUsableKey);
    }
    targets.push(own_keys);
    let signing = match signer {
        Some(k) => Some(signing_key(k, now)?),
        None => None,
    };
    let mut builder = MessageBuilder::from_bytes("", canonical(inner))
        .seipd_v1(OsRng, SymmetricKeyAlgorithm::AES256);
    for keys in &targets {
        for k in keys {
            match k {
                EncryptionKey::Primary(p) => builder.encrypt_to_key(OsRng, p),
                EncryptionKey::Subkey(s) => builder.encrypt_to_key(OsRng, s),
            }
            .map_err(|_| CryptoError::Failed)?;
        }
    }
    if let Some(s) = &signing {
        builder.sign(**s, Password::empty(), HashAlgorithm::Sha512);
    }
    let armored = builder
        .to_armored_string(OsRng, Default::default())
        .map_err(|_| CryptoError::Failed)?;
    let b = boundary();
    Ok(MimeBody {
        content_type: format!("multipart/encrypted; protocol=\"application/pgp-encrypted\"; boundary=\"{b}\""),
        body: format!(
            "This is an OpenPGP/MIME encrypted message (RFC 4880 and 3156)\r\n--{b}\r\nContent-Type: application/pgp-encrypted\r\nContent-Description: PGP/MIME version identification\r\n\r\nVersion: 1\r\n\r\n--{b}\r\nContent-Type: application/octet-stream; name=\"encrypted.asc\"\r\nContent-Description: OpenPGP encrypted message\r\nContent-Disposition: inline; filename=\"encrypted.asc\"\r\n\r\n{}\r\n--{b}--\r\n",
            armored.trim_end().replace("\r\n", "\n").replace('\n', "\r\n")
        ),
    })
}

/// Signs `inner` (a whole MIME entity, already 7-bit safe) with `signer`: the `multipart/signed` body, with a detached SHA-512
/// signature over the entity's canonical form.
pub fn sign(inner: &[u8], signer: &SecretKey, now: u32) -> Result<MimeBody> {
    if inner.len() > MAX_MESSAGE_BYTES {
        return Err(CryptoError::TooLarge);
    }
    let key = signing_key(signer, now)?;
    // RFC 3156: the CRLF before the boundary belongs to the boundary, so it is not signed.
    let mut part = canonical(inner);
    if part.ends_with(b"\r\n") {
        part.truncate(part.len() - 2);
    }
    let sig = DetachedSignature::sign_binary_data(
        OsRng,
        &key,
        &Password::empty(),
        HashAlgorithm::Sha512,
        &part[..],
    )
    .map_err(|_| CryptoError::Failed)?;
    let armored = sig
        .to_armored_string(Default::default())
        .map_err(|_| CryptoError::Failed)?;
    let b = boundary();
    let mut body = Vec::with_capacity(part.len() + armored.len() + 400);
    body.extend_from_slice(
        format!("This is an OpenPGP/MIME signed message (RFC 4880 and 3156)\r\n--{b}\r\n")
            .as_bytes(),
    );
    body.extend_from_slice(&part);
    body.extend_from_slice(
        format!(
            "\r\n--{b}\r\nContent-Type: application/pgp-signature; name=\"signature.asc\"\r\nContent-Description: OpenPGP digital signature\r\nContent-Disposition: attachment; filename=\"signature.asc\"\r\n\r\n{}\r\n--{b}--\r\n",
            armored.trim_end().replace("\r\n", "\n").replace('\n', "\r\n")
        )
        .as_bytes(),
    );
    Ok(MimeBody {
        content_type: format!(
            "multipart/signed; micalg=pgp-sha512; protocol=\"application/pgp-signature\"; boundary=\"{b}\""
        ),
        body: String::from_utf8(body).map_err(|_| CryptoError::Failed)?,
    })
}

/// A key that may have made a signature.
enum Verifier {
    Primary(pgp::packet::PublicKey),
    Subkey(pgp::packet::PublicSubkey),
}

impl Verifier {
    fn as_dyn(&self) -> &dyn VerifyingKey {
        match self {
            Verifier::Primary(k) => k,
            Verifier::Subkey(k) => k,
        }
    }

    fn checks(&self, sig: &DetachedSignature, data: &[u8]) -> bool {
        match self {
            Verifier::Primary(k) => sig.verify(k, data).is_ok(),
            Verifier::Subkey(k) => sig.verify(k, data).is_ok(),
        }
    }
}

/// The verifying keys of `certs` usable at `now`, with the fingerprint of the certificate each belongs to.
fn verifying_keys(certs: &[&SignedPublicKey], now: u32) -> Vec<(Verifier, String)> {
    let mut out = Vec::new();
    for cert in certs {
        let fp = keys::fingerprint(cert);
        for part in keys::usable(cert, now).signing {
            match part {
                SigningPart::Primary(p) => out.push((Verifier::Primary(p.clone()), fp.clone())),
                SigningPart::Subkey(s) => out.push((Verifier::Subkey(s.clone()), fp.clone())),
            }
        }
    }
    out
}

/// Refuses an encrypted message with too many recipient packets, or too many naming one of `secrets` (or the anonymous ID):
/// each such packet is a private-key operation.
fn check_recipients(message: &Message<'_>, secrets: &[&SecretKey]) -> Result<()> {
    let Message::Encrypted { esk, .. } = message else {
        return Err(CryptoError::Unreadable);
    };
    if esk.len() > MAX_RECIPIENT_PACKETS {
        return Err(CryptoError::TooLarge);
    }
    let mine: Vec<pgp::types::KeyId> = secrets
        .iter()
        .flat_map(|k| {
            std::iter::once(k.0.primary_key.legacy_key_id())
                .chain(k.0.secret_subkeys.iter().map(|s| s.key.legacy_key_id()))
        })
        .collect();
    let matching = esk
        .iter()
        .filter(|e| match e {
            Esk::PublicKeyEncryptedSessionKey(p) => {
                p.id().is_ok_and(|id| id.is_wildcard() || mine.contains(id))
            }
            Esk::SymKeyEncryptedSessionKey(_) => false,
        })
        .count();
    if matching > MAX_MATCHING_PACKETS {
        return Err(CryptoError::TooLarge);
    }
    Ok(())
}

/// Appends `data` to `buf`, never leaving an unwiped copy behind when the buffer grows.
fn push_wiped(buf: &mut Zeroizing<Vec<u8>>, data: &[u8]) {
    if buf.capacity() - buf.len() < data.len() {
        let mut bigger = Zeroizing::new(Vec::with_capacity(
            (buf.len() + data.len()).max(buf.capacity() * 2),
        ));
        bigger.extend_from_slice(buf);
        // The old buffer is wiped as it drops.
        *buf = bigger;
    }
    buf.extend_from_slice(data);
}

/// Decrypts the OpenPGP message in a `multipart/encrypted` body's second part (`data`, armored or binary) with one of
/// `secrets`, and checks a signature inside it against `certs` at `now`.
pub fn decrypt(
    data: &[u8],
    secrets: &[&SecretKey],
    certs: &[&SignedPublicKey],
    now: u32,
) -> Result<Decrypted> {
    if data.len() > MAX_MESSAGE_BYTES {
        return Err(CryptoError::TooLarge);
    }
    if secrets.is_empty() {
        return Err(CryptoError::CannotDecrypt);
    }
    let start = data
        .iter()
        .position(|b| !b.is_ascii_whitespace())
        .unwrap_or(0);
    let message = if data[start..].starts_with(b"-----BEGIN PGP MESSAGE") {
        Message::from_armor(&data[start..])
            .map_err(|_| CryptoError::Unreadable)?
            .0
    } else {
        Message::from_bytes(data).map_err(|_| CryptoError::Unreadable)?
    };
    check_recipients(&message, secrets)?;
    let empty = Password::empty();
    let ring = TheRing {
        secret_keys: secrets.iter().map(|k| &k.0).collect(),
        key_passwords: secrets.iter().map(|_| &empty).collect(),
        // GnuPG 2.4 sends its own OCB format to keys that announce it; SEIPDv1 is checked in full before any plaintext is
        // released, up to Croft's own message cap.
        decrypt_options: DecryptionOptions::new()
            .enable_gnupg_aead()
            .set_seipdv1_read_mode(Seipdv1ReadMode::CheckFirst {
                max_message_size: MAX_MESSAGE_BYTES,
            }),
        ..Default::default()
    };
    let (mut plain, _) = message
        .decrypt_the_ring(ring, true)
        .map_err(|_| CryptoError::CannotDecrypt)?;
    if plain.is_compressed() {
        plain = plain.decompress().map_err(|_| CryptoError::Unreadable)?;
        if plain.is_compressed() {
            // One level of compression only (a nested one could be a decompression bomb).
            return Err(CryptoError::Unreadable);
        }
    }
    if plain.is_encrypted() {
        return Err(CryptoError::Unreadable);
    }
    // Every signature is a pass over the whole message while reading it (T41a review): a handful at most.
    if let Message::Signed { reader, .. } = &plain {
        if reader.num_signatures() > MAX_SIGNATURES {
            return Err(CryptoError::TooLarge);
        }
    }
    // Read at most the cap (compressed data could expand without bound), into a buffer wiped as it grows.
    let mut inner = Zeroizing::new(Vec::new());
    let mut buf = Zeroizing::new(vec![0u8; 64 * 1024]);
    loop {
        let n = plain.read(&mut buf).map_err(|_| CryptoError::Unreadable)?;
        if n == 0 {
            break;
        }
        if inner.len() + n > MAX_MESSAGE_BYTES {
            return Err(CryptoError::TooLarge);
        }
        push_wiped(&mut inner, &buf[..n]);
    }
    let signature = if plain.is_signed() {
        let candidates = verifying_keys(certs, now);
        let refs: Vec<&dyn VerifyingKey> = candidates.iter().map(|(k, _)| k.as_dyn()).collect();
        let results = plain.verify_nested(&refs).unwrap_or_default();
        match results
            .iter()
            .zip(candidates.iter())
            .find(|(r, _)| match r {
                VerificationResult::Valid(sig) => acceptable(sig, now),
                VerificationResult::Invalid => false,
            }) {
            Some((_, (_, fp))) => SignatureStatus::Valid {
                fingerprint: fp.clone(),
            },
            None => SignatureStatus::NotVerified { issuer: None },
        }
    } else {
        SignatureStatus::Unsigned
    };
    Ok(Decrypted { inner, signature })
}

/// Checks the detached `signature` (armored or binary) of a `multipart/signed` message over its first part (`signed_part`,
/// exactly as it stands between the boundaries) against `certs` at `now`.
pub fn verify(
    signed_part: &[u8],
    signature: &[u8],
    certs: &[&SignedPublicKey],
    now: u32,
) -> Result<SignatureStatus> {
    if signed_part.len() > MAX_MESSAGE_BYTES || signature.len() > crate::MAX_KEY_BYTES {
        return Err(CryptoError::TooLarge);
    }
    let start = signature
        .iter()
        .position(|b| !b.is_ascii_whitespace())
        .unwrap_or(0);
    let sig = if signature[start..].starts_with(b"-----BEGIN PGP SIGNATURE") {
        DetachedSignature::from_armor_single(&signature[start..])
            .map_err(|_| CryptoError::Unreadable)?
            .0
    } else {
        DetachedSignature::from_bytes(signature).map_err(|_| CryptoError::Unreadable)?
    };
    if !acceptable(&sig.signature, now) {
        return Ok(SignatureStatus::NotVerified {
            issuer: issuer(&sig),
        });
    }
    let part = canonical(signed_part);
    for (key, fp) in verifying_keys(certs, now) {
        if key.checks(&sig, &part) {
            return Ok(SignatureStatus::Valid { fingerprint: fp });
        }
    }
    Ok(SignatureStatus::NotVerified {
        issuer: issuer(&sig),
    })
}

fn issuer(sig: &DetachedSignature) -> Option<String> {
    let s = &sig.signature;
    s.issuer_fingerprint()
        .first()
        .map(|f| f.to_string())
        .or_else(|| s.issuer_key_id().first().map(|k| k.to_string()))
        .map(|t| {
            t.chars()
                .filter(char::is_ascii_hexdigit)
                .collect::<String>()
                .to_ascii_uppercase()
        })
}
