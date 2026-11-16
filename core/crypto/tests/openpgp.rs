// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! T41a: keys, PGP/MIME, Autocrypt and WKD, with keys Croft makes and with GnuPG's (tests/fixtures, made by
//! tools/make_fixtures.sh with made-up people and the made-up passphrase "made-up passphrase").
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use croft_crypto::mime::SignatureStatus;
use croft_crypto::{autocrypt, keys, mime, wkd, CryptoError};

/// 2027-01-15: after the fixtures were made (they do not expire, but Carol's did in 2020).
const NOW: u32 = 1_800_000_000;
const PASS: &str = "made-up passphrase";

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(format!(
        "{}/tests/fixtures/{name}",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}

fn made(
    name: &str,
) -> (
    pgp::composed::SignedSecretKey,
    pgp::composed::SignedPublicKey,
) {
    let g = keys::generate(
        &format!("{name} Example <{}@example.org>", name.to_lowercase()),
        NOW - 100,
    )
    .unwrap();
    let secret = keys::load_secret(&g.secret).unwrap();
    let public = keys::read_public(&g.public).unwrap();
    assert_eq!(keys::info(&public, NOW).fingerprint, g.fingerprint);
    (secret, public)
}

#[test]
fn a_new_key_reads_back_exports_and_imports_with_its_passphrase() {
    let g = keys::generate("Ann Example <Ann@Example.org>", NOW - 10).unwrap();
    assert!(
        !format!("{:?}", g.secret).contains(&format!("{:?}", &g.secret.0[..4])),
        "Debug shows no key bytes"
    );
    let cert = keys::read_public(&g.public).unwrap();
    let info = keys::info(&cert, NOW);
    assert_eq!(info.addresses, ["ann@example.org"]);
    assert!(
        info.can_encrypt
            && info.can_sign
            && !info.expired
            && !info.revoked
            && info.expires.is_none()
    );
    assert_eq!(info.fingerprint.len(), 40);
    // The armored public key reads back the same.
    let armored = keys::export_public(&cert).unwrap();
    assert!(armored.starts_with("-----BEGIN PGP PUBLIC KEY BLOCK-----"));
    assert_eq!(
        keys::info(&keys::read_public(armored.as_bytes()).unwrap(), NOW).fingerprint,
        info.fingerprint
    );
    // A backup is protected with the passphrase, and only that passphrase opens it.
    let secret = keys::load_secret(&g.secret).unwrap();
    let backup = keys::export_secret(&secret, PASS).unwrap();
    assert!(backup.starts_with("-----BEGIN PGP PRIVATE KEY BLOCK-----"));
    assert_eq!(
        keys::read_secret(backup.as_bytes(), "wrong").unwrap_err(),
        CryptoError::WrongPassphrase
    );
    let back = keys::read_secret(backup.as_bytes(), PASS).unwrap();
    assert_eq!(
        keys::info(&keys::public_of(&back), NOW).fingerprint,
        info.fingerprint
    );
    assert_eq!(
        keys::export_secret(&secret, "").unwrap_err(),
        CryptoError::WrongPassphrase
    );
    // A user ID with a line break is refused.
    assert!(keys::generate("Eve\r\nBcc: x <eve@example.org>", NOW).is_err());
}

#[test]
fn gnupg_keys_are_read_and_expired_or_revoked_ones_are_not_used() {
    let alice = keys::read_public(&fixture("gpg-alice.pub.asc")).unwrap();
    let a = keys::info(&alice, NOW);
    assert_eq!(a.addresses, ["alice@example.org"]);
    assert!(a.can_encrypt && a.can_sign);
    let bob = keys::info(
        &keys::read_public(&fixture("gpg-bob.pub.asc")).unwrap(),
        NOW,
    );
    assert!(bob.can_encrypt && bob.can_sign, "RSA keys work");
    let carol = keys::info(
        &keys::read_public(&fixture("gpg-carol-expired.pub.asc")).unwrap(),
        NOW,
    );
    assert!(carol.expired && !carol.can_encrypt && !carol.can_sign);
    assert!(carol.expires.is_some());
    let dave = keys::info(
        &keys::read_public(&fixture("gpg-dave-revoked.pub.asc")).unwrap(),
        NOW,
    );
    assert!(dave.revoked && !dave.can_encrypt && !dave.can_sign);
    // A protected secret key opens with its passphrase only.
    assert_eq!(
        keys::read_secret(&fixture("gpg-alice.sec.asc"), "nope").unwrap_err(),
        CryptoError::WrongPassphrase
    );
    keys::read_secret(&fixture("gpg-alice.sec.asc"), PASS).unwrap();
    // Two keys in one file, garbage, and an oversized file are refused.
    let mut two = fixture("gpg-alice.pub.asc");
    two.extend(fixture("gpg-bob.pub.asc"));
    assert_eq!(keys::read_public(&two).unwrap_err(), CryptoError::NotOneKey);
    assert_eq!(
        keys::read_public(b"not a key").unwrap_err(),
        CryptoError::Unreadable
    );
    assert_eq!(
        keys::read_public(&vec![b'A'; croft_crypto::MAX_KEY_BYTES + 1]).unwrap_err(),
        CryptoError::TooLarge
    );
}

#[test]
fn encrypted_and_signed_mail_reads_back_for_every_recipient_and_the_sender() {
    let (sam_secret, sam) = made("Sam");
    let (ann_secret, ann) = made("Ann");
    let alice = keys::read_public(&fixture("gpg-alice.pub.asc")).unwrap();
    let alice_secret = keys::read_secret(&fixture("gpg-alice.sec.asc"), PASS).unwrap();
    let inner = b"Content-Type: text/plain; charset=utf-8\nSubject: secret\n\nLunch at noon.\n";
    let body = mime::encrypt(inner, &[&ann, &alice], &sam, Some(&sam_secret), NOW).unwrap();
    assert!(body
        .content_type
        .starts_with("multipart/encrypted; protocol=\"application/pgp-encrypted\""));
    assert!(body.body.contains("Version: 1") && body.body.contains("-----BEGIN PGP MESSAGE-----"));
    assert!(!body.body.contains("Lunch"));
    let start = body.body.find("-----BEGIN PGP MESSAGE-----").unwrap();
    let end =
        body.body.find("-----END PGP MESSAGE-----").unwrap() + "-----END PGP MESSAGE-----".len();
    let armored = &body.body.as_bytes()[start..end];
    let sam_fp = keys::info(&sam, NOW).fingerprint;
    for secret in [&ann_secret, &alice_secret, &sam_secret] {
        let d = mime::decrypt(armored, &[secret], &[&sam], NOW).unwrap();
        assert_eq!(&d.inner[..], &mime::canonical(inner)[..]);
        assert_eq!(
            d.signature,
            SignatureStatus::Valid {
                fingerprint: sam_fp.clone()
            }
        );
    }
    // Without the signer's key the signature is not verified (never shown as trusted).
    let d = mime::decrypt(armored, &[&ann_secret], &[&alice], NOW).unwrap();
    assert_eq!(d.signature, SignatureStatus::NotVerified { issuer: None });
    // Someone else's key cannot read it.
    let (eve_secret, _) = made("Eve");
    assert_eq!(
        mime::decrypt(armored, &[&eve_secret], &[&sam], NOW).unwrap_err(),
        CryptoError::CannotDecrypt
    );
    // A changed byte in the ciphertext is never read as something else.
    let mut tampered = armored.to_vec();
    // In the encrypted data (the last packet): well before the checksum and the END line.
    let mid = tampered.len() - 120;
    tampered[mid] = if tampered[mid] == b'A' { b'B' } else { b'A' };
    assert!(mime::decrypt(&tampered, &[&ann_secret], &[&sam], NOW).is_err());
    // Unsigned: said so.
    let plain = mime::encrypt(inner, &[&ann], &sam, None, NOW).unwrap();
    let s = plain.body.find("-----BEGIN PGP MESSAGE-----").unwrap();
    let e = plain.body.find("-----END PGP MESSAGE-----").unwrap() + 25;
    assert_eq!(
        mime::decrypt(&plain.body.as_bytes()[s..e], &[&ann_secret], &[&sam], NOW)
            .unwrap()
            .signature,
        SignatureStatus::Unsigned
    );
}

#[test]
fn a_recipient_without_a_usable_key_is_named_and_nothing_is_sent() {
    let (sam_secret, sam) = made("Sam");
    let (_, ann) = made("Ann");
    let carol = keys::read_public(&fixture("gpg-carol-expired.pub.asc")).unwrap();
    let dave = keys::read_public(&fixture("gpg-dave-revoked.pub.asc")).unwrap();
    assert_eq!(
        mime::encrypt(b"x", &[&ann, &carol], &sam, Some(&sam_secret), NOW).unwrap_err(),
        CryptoError::NoEncryptionKey(1)
    );
    assert_eq!(
        mime::encrypt(b"x", &[&dave], &sam, None, NOW).unwrap_err(),
        CryptoError::NoEncryptionKey(0)
    );
}

#[test]
fn gnupg_mail_decrypts_and_its_signature_is_checked() {
    let alice_secret = keys::read_secret(&fixture("gpg-alice.sec.asc"), PASS).unwrap();
    let bob = keys::read_public(&fixture("gpg-bob.pub.asc")).unwrap();
    let d = mime::decrypt(
        &fixture("gpg-bob-to-alice.asc"),
        &[&alice_secret],
        &[&bob],
        NOW,
    )
    .unwrap();
    assert!(String::from_utf8_lossy(&d.inner).contains("Hello Alice, from Bob."));
    assert_eq!(
        d.signature,
        SignatureStatus::Valid {
            fingerprint: keys::info(&bob, NOW).fingerprint
        }
    );
    let unknown =
        mime::decrypt(&fixture("gpg-bob-to-alice.asc"), &[&alice_secret], &[], NOW).unwrap();
    assert_eq!(
        unknown.signature,
        SignatureStatus::NotVerified { issuer: None }
    );
}

#[test]
fn signed_mail_is_verified_over_canonical_line_ends_and_a_change_is_caught() {
    let (sam_secret, sam) = made("Sam");
    let inner = b"Content-Type: text/plain; charset=utf-8\n\nSigned words.\n";
    let body = mime::sign(inner, &sam_secret, NOW).unwrap();
    assert!(body.content_type.starts_with(
        "multipart/signed; micalg=pgp-sha512; protocol=\"application/pgp-signature\""
    ));
    let b = body
        .content_type
        .split("boundary=\"")
        .nth(1)
        .unwrap()
        .trim_end_matches('"');
    let first = body.body.split(&format!("--{b}\r\n")).nth(1).unwrap();
    // The CRLF before the next boundary belongs to the boundary (RFC 3156), as every reader takes it.
    let part = first.strip_suffix("\r\n").unwrap_or(first);
    let sig_start = body.body.find("-----BEGIN PGP SIGNATURE-----").unwrap();
    let sig_end = body.body.find("-----END PGP SIGNATURE-----").unwrap() + 27;
    let sig = &body.body.as_bytes()[sig_start..sig_end];
    let fp = keys::info(&sam, NOW).fingerprint;
    assert_eq!(
        mime::verify(part.as_bytes(), sig, &[&sam], NOW).unwrap(),
        SignatureStatus::Valid {
            fingerprint: fp.clone()
        }
    );
    // The same part with bare LF line ends still verifies (the canonical form is checked).
    assert_eq!(
        mime::verify(part.replace("\r\n", "\n").as_bytes(), sig, &[&sam], NOW).unwrap(),
        SignatureStatus::Valid {
            fingerprint: fp.clone()
        }
    );
    let changed = part.replace("Signed", "Forged");
    let SignatureStatus::NotVerified { issuer } =
        mime::verify(changed.as_bytes(), sig, &[&sam], NOW).unwrap()
    else {
        panic!("a changed part must not verify")
    };
    assert!(
        issuer.is_some_and(|i| fp.ends_with(&i) || i == fp),
        "names who it claims to be from"
    );
    // GnuPG's detached signature.
    let alice = keys::read_public(&fixture("gpg-alice.pub.asc")).unwrap();
    assert_eq!(
        mime::verify(
            &fixture("gpg-signed-part.txt"),
            &fixture("gpg-signed-part.sig.asc"),
            &[&alice],
            NOW
        )
        .unwrap(),
        SignatureStatus::Valid {
            fingerprint: keys::info(&alice, NOW).fingerprint
        }
    );
}

#[test]
fn autocrypt_headers_round_trip_and_odd_ones_are_refused() {
    let (_, ann) = made("Ann");
    let value = autocrypt::header("ann@example.org", &ann, true, NOW).unwrap();
    assert!(value.starts_with("addr=ann@example.org; prefer-encrypt=mutual; keydata="));
    let h = autocrypt::parse_header(&value, "Ann Example <ANN@example.org>", NOW).unwrap();
    assert!(h.prefer_mutual);
    assert_eq!(
        keys::info(&h.cert, NOW).fingerprint,
        keys::info(&ann, NOW).fingerprint
    );
    assert_eq!(
        autocrypt::parse_header(&value, "mallory@example.com", NOW).unwrap_err(),
        CryptoError::BadHeader
    );
    let critical = value.replace(
        "prefer-encrypt=mutual;",
        "prefer-encrypt=mutual; secret-sauce=1;",
    );
    assert_eq!(
        autocrypt::parse_header(&critical, "ann@example.org", NOW).unwrap_err(),
        CryptoError::BadHeader
    );
    let optional = value.replace("prefer-encrypt=mutual;", "prefer-encrypt=mutual; _extra=1;");
    assert!(autocrypt::parse_header(&optional, "ann@example.org", NOW).is_ok());
    assert_eq!(
        autocrypt::parse_header("addr=ann@example.org", "ann@example.org", NOW).unwrap_err(),
        CryptoError::BadHeader
    );
    // An expired key is not taken.
    let carol = keys::read_public(&fixture("gpg-carol-expired.pub.asc")).unwrap();
    let carol_b64 = base64::Engine::encode(
        &base64::engine::general_purpose::STANDARD,
        keys::public_bytes(&carol).unwrap(),
    );
    assert_eq!(
        autocrypt::parse_header(
            &format!("addr=carol@example.com; keydata={carol_b64}"),
            "carol@example.com",
            NOW
        )
        .unwrap_err(),
        CryptoError::NoUsableKey
    );
}

#[test]
fn wkd_addresses_follow_the_draft() {
    let u = wkd::urls("Joe.Doe@Example.ORG").unwrap();
    assert_eq!(u.advanced, "https://openpgpkey.example.org/.well-known/openpgpkey/example.org/hu/iy9q119eutrkn8s1mk4r39qejnbu3n5q?l=Joe.Doe");
    assert_eq!(
        u.direct,
        "https://example.org/.well-known/openpgpkey/hu/iy9q119eutrkn8s1mk4r39qejnbu3n5q?l=Joe.Doe"
    );
    for bad in [
        "joe@1.2.3.4",
        "a/b@example.org",
        "joe@localhost",
        "joe@exa_mple.org",
        "@example.org",
        "joe",
        "joe@-x.org",
    ] {
        assert_eq!(
            wkd::urls(bad).unwrap_err(),
            CryptoError::BadAddress,
            "{bad}"
        );
    }
}

/// Writes a message Croft encrypted and signed to Alice, and a part Croft signed, into `CROFT_INTEROP_OUT` (when set), for
/// `tools/check_with_gpg.sh` to read with GnuPG. Does nothing otherwise.
#[test]
fn writes_samples_for_gnupg_when_asked() {
    let Ok(dir) = std::env::var("CROFT_INTEROP_OUT") else {
        return;
    };
    // The real time: rPGP dates signatures with it, and GnuPG refuses a key newer than its signature.
    let now = u32::try_from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs(),
    )
    .unwrap();
    let g = keys::generate("Sam Example <sam@example.org>", now - 60).unwrap();
    let (sam_secret, sam) = (
        keys::load_secret(&g.secret).unwrap(),
        keys::read_public(&g.public).unwrap(),
    );
    let alice = keys::read_public(&fixture("gpg-alice.pub.asc")).unwrap();
    let inner = b"Content-Type: text/plain; charset=utf-8\r\n\r\nHello Alice, from Croft.\r\n";
    let enc = mime::encrypt(inner, &[&alice], &sam, Some(&sam_secret), now).unwrap();
    let s = enc.body.find("-----BEGIN PGP MESSAGE-----").unwrap();
    let e = enc.body.find("-----END PGP MESSAGE-----").unwrap() + 25;
    std::fs::write(format!("{dir}/croft-to-alice.asc"), &enc.body[s..e]).unwrap();
    std::fs::write(
        format!("{dir}/croft-sam.pub.asc"),
        keys::export_public(&sam).unwrap(),
    )
    .unwrap();
    let signed = mime::sign(inner, &sam_secret, now).unwrap();
    let b = signed
        .content_type
        .split("boundary=\"")
        .nth(1)
        .unwrap()
        .trim_end_matches('"')
        .to_owned();
    let first = signed.body.split(&format!("--{b}\r\n")).nth(1).unwrap();
    std::fs::write(
        format!("{dir}/croft-signed-part.txt"),
        first.strip_suffix("\r\n").unwrap(),
    )
    .unwrap();
    let ss = signed.body.find("-----BEGIN PGP SIGNATURE-----").unwrap();
    let se = signed.body.find("-----END PGP SIGNATURE-----").unwrap() + 27;
    std::fs::write(
        format!("{dir}/croft-signed-part.sig.asc"),
        &signed.body[ss..se],
    )
    .unwrap();
}
