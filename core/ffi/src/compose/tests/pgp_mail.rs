//! T41c: OpenPGP mail end to end against the in-process IMAP and SMTP servers and a vault fake: sending encrypted and signed
//! (and never plain instead), signed only, the server's copy of an encrypted draft; reading an encrypted message (shown,
//! never stored), its signature, an attachment inside it, one encrypted for someone else, and a signed one.

use croft_crypto::keys;
use croft_crypto::mime::{self, SignatureStatus};
use croft_reader::{pgp_mime, PgpMime};

use super::pgp::{fixture, now_secs, rows, with_vault, PASS};
use super::*;
use crate::pgp::{EncryptionCheck, PgpKeyInfo};
use crate::pgp_read::{DecryptProblem, MessageEncryption, MessageSignature};

/// Alice's own key made, Bob's public key imported; returns (Alice's key, Bob's key).
fn keys_for_both(m: &Mail) -> (PgpKeyInfo, PgpKeyInfo) {
    let alice = m.core().generate_key(m.account).unwrap();
    let bob = m
        .core()
        .import_key(m.account, fixture("gpg-bob.pub.asc"), String::new())
        .unwrap();
    (alice, bob)
}

fn alice_cert(m: &Mail, alice: &PgpKeyInfo) -> croft_crypto::Certificate {
    keys::read_public(
        m.core()
            .export_public_key(m.account, alice.fingerprint.clone())
            .unwrap()
            .as_bytes(),
    )
    .unwrap()
}

fn locked(to: &[&str], subject: &str, body: &str, encrypt: bool, sign: bool) -> DraftContent {
    DraftContent {
        encrypt,
        sign,
        ..content(to, subject, body)
    }
}

/// Every byte the database left on disk (its file and any journal beside it).
fn database_bytes(m: &Mail) -> Vec<u8> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(m.db.path.parent().unwrap()).unwrap() {
        out.extend(std::fs::read(entry.unwrap().path()).unwrap_or_default());
    }
    out
}

fn contains(haystack: &[u8], needle: &str) -> bool {
    haystack
        .windows(needle.len())
        .any(|w| w == needle.as_bytes())
}

#[test]
fn an_encrypted_signed_message_goes_out_readable_only_by_its_recipient_and_the_sender() {
    let (m, _) = with_vault();
    let (alice, bob) = keys_for_both(&m);
    let words = "The secret words are 7781 aubergine.";
    let id = m
        .core()
        .save_draft(
            m.account,
            None,
            locked(&["bob@example.net"], "Locked", words, true, true),
        )
        .unwrap()
        .id;
    assert_eq!(
        m.core().encryption_check(m.account, id).unwrap(),
        EncryptionCheck {
            own_key: true,
            missing: vec![],
            has_bcc: false
        }
    );
    m.core().send(m.account, id).unwrap();
    wait_until("sent", || m.accepted() == 1);
    let data = m.smtp.accepted()[0].data.clone();
    let text = String::from_utf8_lossy(&data);
    assert!(!text.contains("aubergine"), "the body is not in the clear");
    assert!(text.contains("Subject: Locked"));
    assert!(text.contains("Autocrypt: "));
    let Some(PgpMime::Encrypted { data: sealed }) = pgp_mime(&data).unwrap() else {
        panic!("not PGP/MIME encrypted");
    };
    let bob_secret = keys::read_secret(&fixture("gpg-bob.sec.asc"), PASS).unwrap();
    let opened = mime::decrypt(
        &sealed,
        &[&bob_secret],
        &[&alice_cert(&m, &alice)],
        now_secs(),
    )
    .unwrap();
    assert!(contains(&opened.inner, "aubergine"));
    assert_eq!(
        opened.signature,
        SignatureStatus::Valid {
            fingerprint: alice.fingerprint.clone()
        }
    );
    assert!(!bob.own);
}

#[test]
fn encryption_that_cannot_be_done_is_refused_and_nothing_is_sent_plain() {
    let (m, vault) = with_vault();
    m.core()
        .import_key(m.account, fixture("gpg-bob.pub.asc"), String::new())
        .unwrap();
    let draft = |c: DraftContent| m.core().save_draft(m.account, None, c).unwrap().id;
    // No own key yet.
    let id = draft(locked(&["bob@example.net"], "No key", "x", true, false));
    assert!(!m.core().encryption_check(m.account, id).unwrap().own_key);
    assert_eq!(
        m.core().send(m.account, id).unwrap_err(),
        ComposeError::NoOwnKey
    );
    let signed = draft(locked(
        &["bob@example.net"],
        "No key to sign",
        "x",
        false,
        true,
    ));
    assert_eq!(
        m.core().send(m.account, signed).unwrap_err(),
        ComposeError::NoOwnKey
    );
    m.core().generate_key(m.account).unwrap();
    // A recipient without a key.
    let id = draft(locked(
        &["bob@example.net", "carol@example.com"],
        "Carol has none",
        "x",
        true,
        true,
    ));
    let check = m.core().encryption_check(m.account, id).unwrap();
    assert_eq!(
        (check.own_key, check.missing, check.has_bcc),
        (true, vec!["carol@example.com".to_owned()], false)
    );
    assert_eq!(
        m.core().send(m.account, id).unwrap_err(),
        ComposeError::NoRecipientKey
    );
    // Bcc.
    let mut with_bcc = locked(&["bob@example.net"], "Bcc", "x", true, false);
    with_bcc.bcc = vec![FfiAddress {
        name: None,
        address: "bob@example.net".into(),
    }];
    let id = draft(with_bcc);
    assert!(m.core().encryption_check(m.account, id).unwrap().has_bcc);
    assert_eq!(
        m.core().send(m.account, id).unwrap_err(),
        ComposeError::EncryptionWithBcc
    );
    // The own key's secret gone from the vault: the check says so, as sending would find.
    vault.0.lock().unwrap().clear();
    let id = draft(locked(
        &["bob@example.net"],
        "Secret gone",
        "x",
        true,
        false,
    ));
    assert!(!m.core().encryption_check(m.account, id).unwrap().own_key);
    assert_eq!(
        m.core().send(m.account, id).unwrap_err(),
        ComposeError::NoOwnKey
    );
    assert!(m.core().list_outbox(m.account).unwrap().is_empty());
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(m.accepted(), 0, "nothing went out");
}

#[test]
fn a_signed_message_goes_out_readable_with_a_signature_that_verifies() {
    let (m, _) = with_vault();
    let (alice, _) = keys_for_both(&m);
    let id = m
        .core()
        .save_draft(
            m.account,
            None,
            locked(
                &["carol@example.com"],
                "Signed",
                "Plain words, signed.\nFrom here on, a signature:\n-- \nAlice",
                false,
                true,
            ),
        )
        .unwrap()
        .id;
    m.core().send(m.account, id).unwrap();
    wait_until("sent", || m.accepted() == 1);
    let data = m.smtp.accepted()[0].data.clone();
    assert!(contains(&data, "Plain words, signed."));
    let Some(PgpMime::Signed {
        signed_part,
        signature,
    }) = pgp_mime(&data).unwrap()
    else {
        panic!("not PGP/MIME signed");
    };
    assert_eq!(
        mime::verify(
            &signed_part,
            &signature,
            &[&alice_cert(&m, &alice)],
            now_secs()
        )
        .unwrap(),
        SignatureStatus::Valid {
            fingerprint: alice.fingerprint.clone()
        }
    );
}

#[test]
fn the_servers_copy_of_an_encrypted_draft_is_encrypted_or_not_made() {
    let (m, _) = with_vault();
    let words = "Draft words 4412 pomegranate.";
    // First plain: its copy goes to the server as it is.
    let id = m
        .core()
        .save_draft(
            m.account,
            None,
            locked(&["bob@example.net"], "Draft", words, false, false),
        )
        .unwrap()
        .id;
    assert_eq!(
        block(m.core().sync_draft_to_server(m.account, id, true)).unwrap(),
        DraftSyncOutcome::Saved
    );
    assert_eq!(m.imap.messages("Drafts").len(), 1);
    // Then to be encrypted: without an own key, the plain copy comes off the server and nothing replaces it.
    m.core()
        .save_draft(
            m.account,
            Some(id),
            locked(&["bob@example.net"], "Draft", words, true, true),
        )
        .unwrap();
    assert_eq!(
        block(m.core().sync_draft_to_server(m.account, id, true)).unwrap(),
        DraftSyncOutcome::EncryptedKeptOnDevice
    );
    assert!(m.imap.messages("Drafts").is_empty());
    let alice = m.core().generate_key(m.account).unwrap();
    assert_eq!(
        block(m.core().sync_draft_to_server(m.account, id, true)).unwrap(),
        DraftSyncOutcome::Saved
    );
    let copies = m.imap.messages("Drafts");
    assert_eq!(copies.len(), 1);
    let raw = copies[0].raw.as_bytes();
    assert!(!contains(raw, "pomegranate"));
    let Some(PgpMime::Encrypted { data }) = pgp_mime(raw).unwrap() else {
        panic!("the copy is not encrypted");
    };
    // Only the sender's own key opens it.
    let bob_secret = keys::read_secret(&fixture("gpg-bob.sec.asc"), PASS).unwrap();
    assert!(mime::decrypt(&data, &[&bob_secret], &[], now_secs()).is_err());
    let _ = alice;
}

/// A message from Bob encrypted (and signed) to `to`, with a text part and one attached file.
fn sealed_from_bob(subject: &str, to: &[&croft_crypto::Certificate]) -> String {
    let bob_secret = keys::read_secret(&fixture("gpg-bob.sec.asc"), PASS).unwrap();
    let bob_cert = keys::read_public(&fixture("gpg-bob.pub.asc")).unwrap();
    let entity = "Content-Type: multipart/mixed; boundary=\"m\"\r\n\r\n--m\r\nContent-Type: text/plain; charset=utf-8\r\n\r\n\
                  Meet at the old mill, 7781.\r\n--m\r\nContent-Type: text/plain; name=\"plan.txt\"\r\n\
                  Content-Disposition: attachment; filename=\"plan.txt\"\r\n\r\nthe plan is secret\r\n--m--\r\n";
    let body = mime::encrypt(
        entity.as_bytes(),
        to,
        &bob_cert,
        Some(&bob_secret),
        now_secs(),
    )
    .unwrap();
    format!(
        "From: Bob <bob@example.net>\r\nTo: alice@example.org\r\nSubject: {subject}\r\nDate: Mon, 16 Nov 2026 09:00:00 +0000\r\n\
         Message-ID: <{subject}@example.net>\r\nMIME-Version: 1.0\r\nContent-Type: {}\r\n\r\n{}",
        body.content_type, body.body
    )
}

#[test]
fn an_encrypted_message_opens_for_this_view_only_with_its_signer_and_attachment() {
    let (m, _) = with_vault();
    let (alice, bob) = keys_for_both(&m);
    let mine = alice_cert(&m, &alice);
    let bob_cert = keys::read_public(&fixture("gpg-bob.pub.asc")).unwrap();
    m.imap
        .add_message("INBOX", &sealed_from_bob("Sealed", &[&mine]));
    // Encrypted for someone else only (Bob's own key): Alice cannot open it.
    m.imap
        .add_message("INBOX", &sealed_from_bob("Not for Alice", &[&bob_cert]));
    let all = rows(&m, 2);
    let id_of = |s: &str| all.iter().find(|r| r.subject == s).unwrap().id;
    let body = block(
        m.core()
            .load_body(id_of("Sealed"), RemoteImagesSetting::Ask),
    )
    .unwrap();
    assert_eq!(body.encryption, Some(MessageEncryption::Decrypted));
    assert!(body.text.as_deref().unwrap().contains("old mill"));
    assert_eq!(
        body.signature,
        Some(MessageSignature::Valid {
            fingerprint: bob.fingerprint.clone(),
            signer: "bob@example.net".into()
        })
    );
    let file = body
        .attachments
        .iter()
        .find(|a| a.file_name == "plan.txt")
        .expect("the attachment inside");
    assert!(file.id < 0);
    let dir = tempfile::tempdir().unwrap();
    let path = block(m.core().save_attachment(
        id_of("Sealed"),
        file.id,
        dir.path().display().to_string(),
    ))
    .unwrap();
    assert_eq!(
        std::fs::read_to_string(path).unwrap().trim_end(),
        "the plan is secret"
    );
    // Nothing decrypted was kept: not in the database, not in the list's preview.
    let disk = database_bytes(&m);
    assert!(!contains(&disk, "old mill"));
    assert!(!contains(&disk, "the plan is secret"));
    assert!(rows(&m, 2).iter().all(|r| !r.snippet.contains("old mill")));
    let other = block(
        m.core()
            .load_body(id_of("Not for Alice"), RemoteImagesSetting::Ask),
    )
    .unwrap();
    assert_eq!(
        other.encryption,
        Some(MessageEncryption::CannotDecrypt {
            problem: DecryptProblem::NotForThisKey
        })
    );
    assert!(other.signature.is_none());
}

#[test]
fn a_signed_message_is_checked_against_the_senders_keys() {
    let (m, _) = with_vault();
    let bob_secret = keys::read_secret(&fixture("gpg-bob.sec.asc"), PASS).unwrap();
    let part = "Content-Type: text/plain; charset=utf-8\r\n\r\nSigned by Bob.\r\n";
    let signed = mime::sign(part.as_bytes(), &bob_secret, now_secs()).unwrap();
    let raw = |subject: &str, from: &str| {
        format!(
            "From: {from}\r\nTo: alice@example.org\r\nSubject: {subject}\r\nDate: Mon, 16 Nov 2026 09:00:00 +0000\r\n\
             Message-ID: <{subject}@example.net>\r\nMIME-Version: 1.0\r\nContent-Type: {}\r\n\r\n{}",
            signed.content_type, signed.body
        )
    };
    m.imap
        .add_message("INBOX", &raw("From Bob", "Bob <bob@example.net>"));
    // The same signature under someone else's From: Croft has no key for that sender, so it is not verified.
    m.imap.add_message(
        "INBOX",
        &raw("From Mallory", "Mallory <mallory@example.com>"),
    );
    let bob = m
        .core()
        .import_key(m.account, fixture("gpg-bob.pub.asc"), String::new())
        .unwrap();
    let all = rows(&m, 2);
    let load = |s: &str| {
        let id = all.iter().find(|r| r.subject == s).unwrap().id;
        block(m.core().load_body(id, RemoteImagesSetting::Ask)).unwrap()
    };
    let from_bob = load("From Bob");
    assert!(from_bob.encryption.is_none());
    assert!(from_bob.text.as_deref().unwrap().contains("Signed by Bob."));
    assert_eq!(
        from_bob.signature,
        Some(MessageSignature::Valid {
            fingerprint: bob.fingerprint.clone(),
            signer: "bob@example.net".into()
        })
    );
    assert!(matches!(
        load("From Mallory").signature,
        Some(MessageSignature::NotVerified { .. })
    ));
}

#[test]
fn a_key_announced_only_in_mail_never_makes_a_signature_valid() {
    let (m, _) = with_vault();
    // Mallory writes as Bob, announces a key of her own for Bob's address, and signs with it.
    let made = keys::generate("Bob <bob@example.net>", now_secs() - 60).unwrap();
    let mallory = keys::load_secret(&made.secret).unwrap();
    let cert = keys::read_public(&made.public).unwrap();
    let header =
        croft_crypto::autocrypt::header("bob@example.net", &cert, false, now_secs()).unwrap();
    let part = "Content-Type: text/plain; charset=utf-8\r\n\r\nPay the invoice today.\r\n";
    let signed = mime::sign(part.as_bytes(), &mallory, now_secs()).unwrap();
    m.imap.add_message(
        "INBOX",
        &format!(
            "From: Bob <bob@example.net>\r\nTo: alice@example.org\r\nSubject: Invoice\r\nDate: Mon, 16 Nov 2026 09:00:00 +0000\r\n\
             Message-ID: <invoice@example.net>\r\nAutocrypt: {header}\r\nMIME-Version: 1.0\r\nContent-Type: {}\r\n\r\n{}",
            signed.content_type, signed.body
        ),
    );
    let id = rows(&m, 1)[0].id;
    let open = || {
        block(m.core().load_body(id, RemoteImagesSetting::Ask))
            .unwrap()
            .signature
    };
    // First open: the key is not known yet. It is learnt from the header as the message is read.
    assert!(matches!(open(), Some(MessageSignature::NotVerified { .. })));
    // Later opens: signed by the key the mail announced, never "valid".
    assert_eq!(
        open(),
        Some(MessageSignature::Unconfirmed {
            fingerprint: made.fingerprint.clone(),
            signer: "bob@example.net".into()
        })
    );
    // Bob's real key imported by the person: Mallory's signature is still only "unconfirmed".
    m.core()
        .import_key(m.account, fixture("gpg-bob.pub.asc"), String::new())
        .unwrap();
    assert!(matches!(open(), Some(MessageSignature::Unconfirmed { .. })));
}
