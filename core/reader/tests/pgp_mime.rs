// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! `pgp_mime`: the pieces of PGP/MIME messages (RFC 3156) and nothing from anything else.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use croft_reader::{pgp_mime, PgpMime};

const HEAD: &str =
    "From: a@example.org\r\nTo: b@example.org\r\nSubject: x\r\nMIME-Version: 1.0\r\n";

#[test]
fn an_encrypted_message_gives_its_openpgp_data() {
    let raw = format!(
        "{HEAD}Content-Type: multipart/encrypted; protocol=\"application/pgp-encrypted\"; boundary=\"b\"\r\n\r\n\
         preamble\r\n--b\r\nContent-Type: application/pgp-encrypted\r\n\r\nVersion: 1\r\n\r\n--b\r\n\
         Content-Type: application/octet-stream\r\n\r\n-----BEGIN PGP MESSAGE-----\r\nabc\r\n-----END PGP MESSAGE-----\r\n--b--\r\n"
    );
    let Some(PgpMime::Encrypted { data }) = pgp_mime(raw.as_bytes()).unwrap() else {
        panic!("not read as encrypted");
    };
    let text = String::from_utf8(data).unwrap();
    assert!(text.starts_with("-----BEGIN PGP MESSAGE-----"));
    assert!(text.trim_end().ends_with("-----END PGP MESSAGE-----"));
}

#[test]
fn a_signed_message_gives_the_part_exactly_as_sent_and_its_signature() {
    let part = "Content-Type: text/plain; charset=utf-8\r\nContent-Transfer-Encoding: 7bit\r\n\r\nSigned text.\r\n";
    let raw = format!(
        "{HEAD}Content-Type: multipart/signed; micalg=pgp-sha512; protocol=\"application/pgp-signature\"; boundary=\"s\"\r\n\r\n\
         --s\r\n{part}\r\n--s\r\nContent-Type: application/pgp-signature; name=\"signature.asc\"\r\n\r\n\
         -----BEGIN PGP SIGNATURE-----\r\nxyz\r\n-----END PGP SIGNATURE-----\r\n--s--\r\n"
    );
    let Some(PgpMime::Signed {
        signed_part,
        signature,
    }) = pgp_mime(raw.as_bytes()).unwrap()
    else {
        panic!("not read as signed");
    };
    // Up to, not including, the line break before the boundary (RFC 3156 section 5).
    assert_eq!(String::from_utf8(signed_part).unwrap(), part);
    assert!(String::from_utf8(signature)
        .unwrap()
        .starts_with("-----BEGIN PGP SIGNATURE-----"));
}

#[test]
fn other_messages_and_wrong_protocols_are_not_pgp_mime() {
    for content_type in [
        "text/plain",
        "multipart/mixed; boundary=\"b\"",
        "multipart/encrypted; protocol=\"application/pkcs7-mime\"; boundary=\"b\"",
        "multipart/signed; protocol=\"application/pkcs7-signature\"; boundary=\"b\"",
    ] {
        let raw = format!(
            "{HEAD}Content-Type: {content_type}\r\n\r\n--b\r\nContent-Type: text/plain\r\n\r\none\r\n--b\r\n\
             Content-Type: text/plain\r\n\r\ntwo\r\n--b--\r\n"
        );
        assert!(
            pgp_mime(raw.as_bytes()).unwrap().is_none(),
            "{content_type}"
        );
    }
    // Signed, but the second part is not a signature.
    let raw = format!(
        "{HEAD}Content-Type: multipart/signed; protocol=\"application/pgp-signature\"; boundary=\"s\"\r\n\r\n\
         --s\r\nContent-Type: text/plain\r\n\r\none\r\n--s\r\nContent-Type: text/plain\r\n\r\ntwo\r\n--s--\r\n"
    );
    assert!(pgp_mime(raw.as_bytes()).unwrap().is_none());
}
