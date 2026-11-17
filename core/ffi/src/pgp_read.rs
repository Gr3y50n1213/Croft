// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! T41c: OpenPGP when reading. A `multipart/encrypted` message is decrypted with the account's own keys (their secrets from
//! the app's key vault) each time it is opened, and shown from the decrypted entity: nothing decrypted is stored, indexed or
//! put in a snippet. A signature (inside the encryption, or `multipart/signed`) is checked against the keys Croft has for
//! the `From` address. Attachments inside an encrypted message have negative ids (minus their part number in the decrypted
//! entity); `save_attachment` decrypts again for them.

use croft_crypto::keys::{self, SecretKey};
use croft_crypto::mime::{self, SignatureStatus};
use croft_crypto::{CryptoError, SecretBytes};
use croft_reader::{pgp_mime, remote_allowed, PgpMime};
use croft_storage::pgp as store;
use croft_storage::reading;
use zeroize::Zeroizing;

use crate::core::Inner;
use crate::pgp::now_secs;
use crate::reading::{AttachmentInfo, MessageBody, ReadError, RemoteImagesSetting};

/// The most own keys tried on one message.
const MAX_OWN_KEYS: usize = 8;

/// Why an encrypted message could not be opened.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum DecryptProblem {
    /// The account has no own key with its secret in the app's key vault (or the app set no vault).
    NoOwnKey,
    /// It was not encrypted to any of the account's own keys.
    NotForThisKey,
    /// The encrypted data is damaged, changed, or not OpenPGP Croft reads.
    Damaged,
    /// Larger than Croft decrypts.
    TooLarge,
}

/// Whether a message came encrypted, and whether it opened (T41c).
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum MessageEncryption {
    /// Encrypted, and opened with the account's own key: what is shown was decrypted for this view only.
    Decrypted,
    /// Encrypted, and it could not be opened: the body shown is the message's outer part only.
    CannotDecrypt {
        /// Why.
        problem: DecryptProblem,
    },
}

/// A message's OpenPGP signature (T41c). Say "signed by", never "encrypted by": anyone can encrypt a signed message again.
#[derive(Clone, PartialEq, Eq, uniffi::Enum)]
pub enum MessageSignature {
    /// Signed by a key Croft has for the sender's `From` address, and valid now.
    Valid {
        /// The key that made it, upper-case hex.
        fingerprint: String,
        /// The `From` address that key is filed under (bare, lower case).
        signer: String,
    },
    /// Signed by the key this sender's own mail announced (an Autocrypt header), and valid now. Weaker than `Valid`: anyone
    /// can send mail with any `From` and announce a key for it, so this says only that the key matches the sender's earlier
    /// mail, not that the key is theirs. Never show it as verified.
    Unconfirmed {
        /// The key that made it, upper-case hex.
        fingerprint: String,
        /// The `From` address that key was announced for (bare, lower case).
        signer: String,
    },
    /// Signed, but not verified: a key Croft does not have for this sender, a changed message, a weak hash, or a time out
    /// of bounds.
    NotVerified {
        /// The key the signature names, when it names one (upper-case hex).
        issuer: Option<String>,
    },
}

impl std::fmt::Debug for MessageSignature {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Valid { fingerprint, .. } => write!(f, "Valid({fingerprint})"),
            Self::Unconfirmed { fingerprint, .. } => write!(f, "Unconfirmed({fingerprint})"),
            Self::NotVerified { issuer } => write!(f, "NotVerified({issuer:?})"),
        }
    }
}

/// The account's own secret keys from the vault (the newest first), each checked to be the key it was stored as. Empty
/// without a vault.
fn own_secrets(inner: &Inner, account: i64) -> Result<Vec<SecretKey>, ReadError> {
    let Some(vault) = inner
        .key_vault
        .lock()
        .map_err(|_| ReadError::Internal)?
        .clone()
    else {
        return Ok(Vec::new());
    };
    let rows = inner
        .handle
        .read(|db| store::own_keys(db.connection(), account))?;
    let mut out = Vec::new();
    for row in rows.into_iter().take(MAX_OWN_KEYS) {
        let Ok(Some(bytes)) = vault.secret_key(account, row.fingerprint.clone()) else {
            continue;
        };
        let secret = SecretBytes(Zeroizing::new(bytes));
        let Ok(key) = keys::load_secret(&secret) else {
            continue;
        };
        if keys::fingerprint(&keys::public_of(&key)) == row.fingerprint {
            out.push(key);
        }
    }
    Ok(out)
}

/// A key Croft has filed under the sender's address, and whether anyone but the sender's own mail vouches for it.
struct SenderKey {
    cert: croft_crypto::Certificate,
    fingerprint: String,
    /// Made or imported by the person, their own, or from the address's domain (WKD); not only an Autocrypt header.
    confirmed: bool,
}

/// The keys a signature from `from` (bare) is checked against: those the person chose or a WKD gave (confirmed), and the
/// sender's current Autocrypt key (not confirmed). An Autocrypt key the sender has since replaced does not count.
fn sender_keys(
    inner: &Inner,
    account: i64,
    from: Option<&str>,
) -> Result<Vec<SenderKey>, ReadError> {
    let Some(from) = from else {
        return Ok(Vec::new());
    };
    let (rows, current) = inner.handle.read(|db| {
        let c = db.connection();
        Ok::<_, ReadError>((
            store::keys_for(c, account, from)?,
            store::peer(c, account, from)?.and_then(|p| p.fingerprint),
        ))
    })?;
    Ok(rows
        .into_iter()
        .filter_map(|r| {
            let confirmed = r.own || r.source != store::KeySource::Autocrypt;
            if !confirmed && current.as_deref() != Some(r.fingerprint.as_str()) {
                return None;
            }
            Some(SenderKey {
                cert: keys::read_public(&r.public_key).ok()?,
                fingerprint: r.fingerprint,
                confirmed,
            })
        })
        .collect())
}

fn signature_of(
    status: SignatureStatus,
    from: Option<&str>,
    keys: &[SenderKey],
) -> Option<MessageSignature> {
    match status {
        SignatureStatus::Unsigned => None,
        SignatureStatus::Valid { fingerprint } => {
            let key = keys.iter().find(|k| k.fingerprint == fingerprint);
            Some(match (from, key) {
                (Some(f), Some(k)) if k.confirmed => MessageSignature::Valid {
                    fingerprint,
                    signer: f.to_owned(),
                },
                (Some(f), Some(_)) => MessageSignature::Unconfirmed {
                    fingerprint,
                    signer: f.to_owned(),
                },
                _ => MessageSignature::NotVerified {
                    issuer: Some(fingerprint),
                },
            })
        }
        SignatureStatus::NotVerified { issuer } => Some(MessageSignature::NotVerified { issuer }),
    }
}

fn problem_of(e: CryptoError) -> DecryptProblem {
    match e {
        CryptoError::CannotDecrypt => DecryptProblem::NotForThisKey,
        CryptoError::TooLarge => DecryptProblem::TooLarge,
        _ => DecryptProblem::Damaged,
    }
}

/// The decrypted entity to show and its signature, or why there is none.
struct Opened {
    entity: Zeroizing<Vec<u8>>,
    signature: Option<MessageSignature>,
}

/// Decrypts `data` with the account's own keys and checks the signature inside it (or, when the decrypted entity is itself
/// `multipart/signed`, that signature over its first part, which is then the entity shown).
fn open(
    inner: &Inner,
    account: i64,
    from: Option<&str>,
    data: &[u8],
) -> Result<Result<Opened, DecryptProblem>, ReadError> {
    let secrets = own_secrets(inner, account)?;
    if secrets.is_empty() {
        return Ok(Err(DecryptProblem::NoOwnKey));
    }
    let sender = sender_keys(inner, account, from)?;
    let now = now_secs();
    let secret_refs: Vec<&SecretKey> = secrets.iter().collect();
    let cert_refs: Vec<&croft_crypto::Certificate> = sender.iter().map(|k| &k.cert).collect();
    let decrypted = match mime::decrypt(data, &secret_refs, &cert_refs, now) {
        Ok(d) => d,
        Err(e) => return Ok(Err(problem_of(e))),
    };
    let mut signature = signature_of(decrypted.signature, from, &sender);
    let mut entity = decrypted.inner;
    if let Ok(Some(PgpMime::Signed {
        signed_part,
        signature: sig,
    })) = pgp_mime(&entity)
    {
        if signature.is_none() {
            signature = mime::verify(&signed_part, &sig, &cert_refs, now)
                .ok()
                .and_then(|s| signature_of(s, from, &sender));
        }
        entity = Zeroizing::new(signed_part);
    }
    Ok(Ok(Opened { entity, signature }))
}

/// `load_body`'s OpenPGP step (T41c): an encrypted message is shown decrypted (nothing of it stored), a signature is
/// checked. Any other message comes back unchanged.
pub(crate) fn open_body(
    inner: &Inner,
    account: i64,
    from: Option<&str>,
    remote_images: RemoteImagesSetting,
    mut body: MessageBody,
) -> Result<MessageBody, ReadError> {
    let id = body.message_id;
    let Some(raw) = inner
        .handle
        .read(|db| reading::raw(db.connection(), account, id))?
    else {
        return Ok(body);
    };
    let raw = Zeroizing::new(raw);
    let pgp = match pgp_mime(&raw) {
        Ok(Some(p)) => p,
        Ok(None) => return Ok(body),
        Err(e) => {
            // Encrypted, but past what the reader takes: say so, never show it as a plain message.
            if says_encrypted(&raw) {
                body.encryption = Some(MessageEncryption::CannotDecrypt {
                    problem: match e {
                        croft_reader::ParseError::TooLarge { .. } => DecryptProblem::TooLarge,
                        _ => DecryptProblem::Damaged,
                    },
                });
            }
            return Ok(body);
        }
    };
    let from = from.and_then(keys::bare_address);
    match pgp {
        PgpMime::Signed {
            signed_part,
            signature,
        } => {
            let sender = sender_keys(inner, account, from.as_deref())?;
            let refs: Vec<&croft_crypto::Certificate> = sender.iter().map(|k| &k.cert).collect();
            body.signature = Some(
                mime::verify(&signed_part, &signature, &refs, now_secs())
                    .ok()
                    .and_then(|s| signature_of(s, from.as_deref(), &sender))
                    .unwrap_or(MessageSignature::NotVerified { issuer: None }),
            );
        }
        PgpMime::Encrypted { data } => match open(inner, account, from.as_deref(), &data)? {
            Err(problem) => body.encryption = Some(MessageEncryption::CannotDecrypt { problem }),
            Ok(opened) => {
                let message_allowed = inner
                    .handle
                    .read(|db| reading::remote_allowed(db.connection(), account, id))?;
                let allowed =
                    remote_allowed(remote_images.into(), message_allowed, body.sender_known);
                let Ok(r) = croft_reader::render(&opened.entity, allowed) else {
                    body.encryption = Some(MessageEncryption::CannotDecrypt {
                        problem: DecryptProblem::Damaged,
                    });
                    return Ok(body);
                };
                let html = (!r.too_complex && !r.html_pending)
                    .then_some(r.html)
                    .flatten();
                body.remote_allowed = allowed && html.is_some();
                body.text = r.text;
                body.html_sanitised = html;
                body.blocked_remote_count = r.blocked_remote;
                body.trackers_removed = r.trackers_removed;
                body.truncated = r.truncated || r.too_complex;
                body.attachments = r
                    .attachments
                    .into_iter()
                    .filter_map(|a| {
                        let part: i64 = a.part_id.parse().ok()?;
                        Some(AttachmentInfo {
                            id: -part,
                            file_name: a.file_name,
                            mime_type: a.mime_type,
                            size: a.size,
                            content_id: a.content_id,
                            shown_inline: a.shown_inline,
                            blocked: a.executable,
                        })
                    })
                    .collect();
                body.encryption = Some(MessageEncryption::Decrypted);
                body.signature = opened.signature;
            }
        },
    }
    Ok(body)
}

/// Whether the message's own `Content-Type` (in its first 64 KiB of headers) is `multipart/encrypted`.
fn says_encrypted(raw: &[u8]) -> bool {
    let head = &raw[..raw.len().min(64 * 1024)];
    let text = String::from_utf8_lossy(head);
    croft_mail_core::compose::header_value(&text, "Content-Type").is_some_and(|v| {
        v.trim_start()
            .to_ascii_lowercase()
            .starts_with("multipart/encrypted")
    })
}

/// One attachment inside an encrypted message (`part`: its part number in the decrypted entity), decrypted again: its safe
/// name, type and bytes. Programs are refused.
pub(crate) fn decrypted_attachment(
    inner: &Inner,
    account: i64,
    message_id: i64,
    from: Option<&str>,
    part: i64,
) -> Result<(String, String, Zeroizing<Vec<u8>>), ReadError> {
    let raw = Zeroizing::new(
        inner
            .handle
            .read(|db| reading::raw(db.connection(), account, message_id))?
            .ok_or(ReadError::NoSuchAttachment)?,
    );
    let Ok(Some(PgpMime::Encrypted { data })) = pgp_mime(&raw) else {
        return Err(ReadError::NoSuchAttachment);
    };
    let from = from.and_then(keys::bare_address);
    let opened =
        open(inner, account, from.as_deref(), &data)?.map_err(|_| ReadError::Unreadable)?;
    let part_id = part.to_string();
    let rendered =
        croft_reader::render(&opened.entity, false).map_err(|_| ReadError::Unreadable)?;
    let a = rendered
        .attachments
        .into_iter()
        .find(|a| a.part_id == part_id)
        .ok_or(ReadError::NoSuchAttachment)?;
    if a.executable {
        return Err(ReadError::AttachmentBlocked);
    }
    let bytes = croft_reader::part_content(&opened.entity, &part_id)
        .map_err(|_| ReadError::Unreadable)?
        .ok_or(ReadError::NoSuchAttachment)?;
    Ok((a.file_name, a.mime_type, Zeroizing::new(bytes)))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::says_encrypted;

    #[test]
    fn an_encrypted_message_is_told_by_its_own_content_type_only() {
        let head = |ct: &str| {
            format!("From: a@example.org\r\nContent-Type: {ct}\r\n\r\nbody\r\n").into_bytes()
        };
        assert!(says_encrypted(&head(
            "multipart/encrypted; protocol=\"application/pgp-encrypted\"; boundary=b"
        )));
        assert!(says_encrypted(&head("  Multipart/Encrypted; boundary=b")));
        assert!(!says_encrypted(&head("multipart/mixed; boundary=b")));
        // A header in the body is not the message's.
        assert!(!says_encrypted(
            b"From: a@example.org\r\n\r\nContent-Type: multipart/encrypted\r\n"
        ));
    }
}
