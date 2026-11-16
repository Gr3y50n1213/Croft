#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, missing_docs)]
use std::time::Instant;

use croft_crypto::mime::SignatureStatus;
use croft_crypto::{autocrypt, keys, mime, wkd};
use pgp::composed::MessageBuilder;
use pgp::crypto::hash::HashAlgorithm;
use pgp::crypto::sym::SymmetricKeyAlgorithm;
use pgp::packet::{PacketTrait, PublicKeyEncryptedSessionKey, SymEncryptedProtectedData};
use pgp::types::Password;
use rand::rngs::OsRng;

const NOW: u32 = 1_800_000_000;
const PASS: &str = "made-up passphrase";

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"))).unwrap()
}

fn encrypt_raw(plaintext: &[u8], subkey: &pgp::packet::PublicSubkey, copies: usize) -> Vec<u8> {
    let sk = SymmetricKeyAlgorithm::AES256.new_session_key(OsRng);
    let mut out = Vec::new();
    for _ in 0..copies {
        let pkesk = PublicKeyEncryptedSessionKey::from_session_key_v3(
            OsRng,
            &sk,
            SymmetricKeyAlgorithm::AES256,
            subkey,
        )
        .unwrap();
        pkesk.to_writer_with_header(&mut out).unwrap();
    }
    let seipd = SymEncryptedProtectedData::encrypt_seipdv1(
        OsRng,
        SymmetricKeyAlgorithm::AES256,
        sk.as_ref(),
        plaintext,
    )
    .unwrap();
    seipd.to_writer_with_header(&mut out).unwrap();
    out
}

fn literal(len: usize) -> Vec<u8> {
    let body_len = 2 + 4 + len;
    let mut v = vec![0xCB, 0xFF];
    v.extend_from_slice(&(u32::try_from(body_len).unwrap()).to_be_bytes());
    v.extend_from_slice(&[b'b', 0, 0, 0, 0, 0]);
    v.extend(std::iter::repeat_n(b'A', len));
    v
}

#[test]
fn rv_weak_hash_inside_encrypted_is_valid() {
    let alice = keys::read_public(&fixture("gpg-alice.pub.asc")).unwrap();
    let alice_secret = keys::read_secret(&fixture("gpg-alice.sec.asc"), PASS).unwrap();
    let bob = keys::read_public(&fixture("gpg-bob.pub.asc")).unwrap();
    let bob_secret = keys::read_secret(&fixture("gpg-bob.sec.asc"), PASS).unwrap();
    for h in [HashAlgorithm::Sha1, HashAlgorithm::Md5, HashAlgorithm::Ripemd160] {
        let mut b = MessageBuilder::from_bytes("", b"hello".to_vec())
            .seipd_v1(OsRng, SymmetricKeyAlgorithm::AES256);
        b.encrypt_to_key(OsRng, &alice.public_subkeys[0].key).unwrap();
        b.sign(&bob_secret.primary_key, Password::empty(), h);
        let bytes = match b.to_vec(OsRng) {
            Ok(v) => v,
            Err(e) => {
                println!("{h:?}: cannot build: {e}");
                continue;
            }
        };
        let d = mime::decrypt(&bytes, &[&alice_secret], &[&bob], NOW).unwrap();
        println!("{h:?}: decrypt signature = {:?}", d.signature);
    }
}

#[test]
fn rv_many_ops_hashers() {
    let alice = keys::read_public(&fixture("gpg-alice.pub.asc")).unwrap();
    let alice_secret = keys::read_secret(&fixture("gpg-alice.sec.asc"), PASS).unwrap();
    let sub = &alice.public_subkeys[0].key;
    for n in [0usize, 1000, 5000] {
        let mut p = Vec::new();
        for _ in 0..n {
            p.extend_from_slice(&[0xC4, 13, 3, 0, 10, 22, 1, 2, 3, 4, 5, 6, 7, 8, 1]);
        }
        p.extend(literal(256 * 1024));
        let msg = encrypt_raw(&p, sub, 1);
        let t = Instant::now();
        let r = mime::decrypt(&msg, &[&alice_secret], &[], NOW);
        println!(
            "ops n={n} msg={} bytes: {:?} in {:?}",
            msg.len(),
            r.map(|d| d.inner.len()),
            t.elapsed()
        );
    }
}

#[test]
fn rv_many_pkesks() {
    let bob = keys::read_public(&fixture("gpg-bob.pub.asc")).unwrap();
    let bob_secret = keys::read_secret(&fixture("gpg-bob.sec.asc"), PASS).unwrap();
    let alice = keys::read_public(&fixture("gpg-alice.pub.asc")).unwrap();
    let alice_secret = keys::read_secret(&fixture("gpg-alice.sec.asc"), PASS).unwrap();
    let plain = literal(10);
    for (name, cert, secret, n) in [
        ("bob rsa3072", &bob, &bob_secret, 300usize),
        ("alice x25519", &alice, &alice_secret, 10000usize),
    ] {
        let msg = encrypt_raw(&plain, &cert.public_subkeys[0].key, n);
        let t = Instant::now();
        let r = mime::decrypt(&msg, &[secret], &[], NOW);
        println!(
            "{name} pkesks={n} msg={} bytes: ok={} in {:?}",
            msg.len(),
            r.is_ok(),
            t.elapsed()
        );
    }
}

#[test]
fn rv_autocrypt_from_and_unicode() {
    println!("kelvin: {:?}", keys::address_of("\u{212A}im@example.org"));
    let alice = fixture("gpg-alice.pub.asc");
    let cert = keys::read_public(&alice).unwrap();
    let bin = keys::public_bytes(&cert).unwrap();
    use base64::Engine;
    let kd = base64::engine::general_purpose::STANDARD.encode(bin);
    let v = format!("addr=alice@example.org; keydata={kd}");
    for from in [
        "mallory@evil.example (<alice@example.org>)",
        "\"<alice@example.org>\" mallory@evil.example",
        "alice@example.org, mallory@evil.example",
        "Mallory <mallory@evil.example>, <alice@example.org>",
    ] {
        println!("from {from:?}: {:?}", autocrypt::parse_header(&v, from, NOW).map(|h| h.addr));
    }
}

#[test]
fn rv_wkd() {
    for a in ["x@0x7f.1", "x@0x7f.0x1", "x@0177.0x0.1", "x@a.0x7f000001", "x@1.2.3.0x4"] {
        println!("{a}: {:?}", wkd::urls(a).map(|u| u.direct));
    }
}

#[test]
fn rv_debug_secret() {
    let s = keys::read_secret(&fixture("gpg-alice.sec.asc"), PASS).unwrap();
    let d = format!("{s:?}");
    println!("debug len {} snippet {}", d.len(), &d[..d.len().min(600)]);
    let _ = SignatureStatus::Unsigned;
}
