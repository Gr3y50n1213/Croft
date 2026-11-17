//! T41b: OpenPGP keys end to end against the in-process IMAP and SMTP servers and a vault fake: an own key made and backed
//! up and imported back, nothing secret in the database, other people's keys from a file, Autocrypt in (newer only, own
//! addresses and forged senders skipped) and out, `key_status`, WKD, and no vault, no secret.

use std::collections::HashMap;
use std::sync::Mutex;

use croft_crypto::{autocrypt, keys};

use super::*;
use crate::credentials::ProviderError;
use crate::pgp::{KeyVault, LookupProblem, PgpError, PgpKeySource};

/// The platform keystore, in memory.
#[derive(Default)]
pub(super) struct Vault(pub(super) Mutex<HashMap<(i64, String), Vec<u8>>>);

impl KeyVault for Vault {
    fn store_secret_key(
        &self,
        account_id: i64,
        fingerprint: String,
        secret: Vec<u8>,
    ) -> Result<(), ProviderError> {
        self.0
            .lock()
            .unwrap()
            .insert((account_id, fingerprint), secret);
        Ok(())
    }
    fn secret_key(
        &self,
        account_id: i64,
        fingerprint: String,
    ) -> Result<Option<Vec<u8>>, ProviderError> {
        Ok(self
            .0
            .lock()
            .unwrap()
            .get(&(account_id, fingerprint))
            .cloned())
    }
    fn delete_secret_key(&self, account_id: i64, fingerprint: String) -> Result<(), ProviderError> {
        self.0.lock().unwrap().remove(&(account_id, fingerprint));
        Ok(())
    }
}

pub(super) const PASS: &str = "made-up passphrase";

pub(super) fn with_vault() -> (Mail, Arc<Vault>) {
    let m = Mail::new();
    let vault = Arc::new(Vault::default());
    m.core().set_key_vault(vault.clone());
    (m, vault)
}

pub(super) fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(format!(
        "{}/../crypto/tests/fixtures/{name}",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}

pub(super) fn rows(m: &Mail, n: usize) -> Vec<MessageRow> {
    let mut rows = Vec::new();
    wait_until("the inbox has the messages", || {
        let _ = m.core().sync_now(m.account);
        rows = m
            .core()
            .list_messages(MessageScope::UnifiedInbox, None, 50)
            .unwrap()
            .rows;
        rows.len() >= n
    });
    rows
}

fn read(m: &Mail, subject: &str, n: usize) {
    let id = rows(m, n)
        .into_iter()
        .find(|r| r.subject == subject)
        .unwrap()
        .id;
    block(m.core().load_body(id, RemoteImagesSetting::Ask)).unwrap();
}

fn mail_from(from: &str, subject: &str, date: &str, autocrypt: Option<&str>) -> String {
    let header = autocrypt
        .map(|a| format!("Autocrypt: {a}\r\n"))
        .unwrap_or_default();
    format!("From: {from}\r\nTo: alice@example.org\r\nSubject: {subject}\r\nDate: {date}\r\nMessage-ID: <{subject}@example.net>\r\n{header}\r\nHello.\r\n")
}

pub(super) fn now_secs() -> u32 {
    u32::try_from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs(),
    )
    .unwrap()
}

fn made_public(m: &Mail, fingerprint: &str) -> String {
    m.core()
        .export_public_key(m.account, fingerprint.to_owned())
        .unwrap()
}

#[test]
fn an_own_key_is_made_backed_up_removed_and_imported_back_with_its_secret_only_in_the_vault() {
    let (m, vault) = with_vault();
    let made = m.core().generate_key(m.account).unwrap();
    assert!(made.own && made.can_encrypt && made.can_sign);
    assert_eq!(made.source, PgpKeySource::Made);
    assert_eq!(made.addresses, ["alice@example.org"]);
    let secret = vault
        .0
        .lock()
        .unwrap()
        .get(&(m.account, made.fingerprint.clone()))
        .cloned()
        .unwrap();
    // Nothing of the secret in the database's files (the main file and any journal beside it): no 24-byte run of the secret
    // that is not also in the public certificate (which the database keeps, signatures and all).
    let public = keys::public_bytes(
        &keys::read_public(made_public(&m, &made.fingerprint).as_bytes()).unwrap(),
    )
    .unwrap();
    let secret_only: Vec<&[u8]> = secret
        .windows(24)
        .filter(|w| !public.windows(24).any(|p| p == *w))
        .collect();
    assert!(
        !secret_only.is_empty(),
        "the secret has material of its own"
    );
    let mut files = Vec::new();
    for entry in std::fs::read_dir(m.db.path.parent().unwrap()).unwrap() {
        files.extend(std::fs::read(entry.unwrap().path()).unwrap_or_default());
    }
    assert!(
        !secret_only
            .iter()
            .any(|run| files.windows(run.len()).any(|w| w == *run)),
        "a secret key byte run is in the database"
    );
    // A backup opens only with its passphrase.
    let backup = m
        .core()
        .export_secret_key(m.account, made.fingerprint.clone(), PASS.into())
        .unwrap();
    assert!(backup.starts_with("-----BEGIN PGP PRIVATE KEY BLOCK-----"));
    assert_eq!(
        m.core()
            .export_secret_key(m.account, made.fingerprint.clone(), String::new())
            .unwrap_err(),
        PgpError::WrongPassphrase
    );
    assert!(m
        .core()
        .export_public_key(m.account, made.fingerprint.clone())
        .unwrap()
        .starts_with("-----BEGIN PGP PUBLIC KEY BLOCK-----"));
    assert!(m
        .core()
        .delete_key(m.account, made.fingerprint.clone())
        .unwrap());
    assert!(
        vault.0.lock().unwrap().is_empty(),
        "the vault forgets it too"
    );
    assert!(m.core().pgp_keys(m.account).unwrap().is_empty());
    assert_eq!(
        m.core()
            .import_key(m.account, backup.clone().into_bytes(), "wrong".into())
            .unwrap_err(),
        PgpError::WrongPassphrase
    );
    let back = m
        .core()
        .import_key(m.account, backup.into_bytes(), PASS.into())
        .unwrap();
    assert_eq!(
        (back.fingerprint.as_str(), back.own, back.source),
        (made.fingerprint.as_str(), true, PgpKeySource::Imported)
    );
    assert_eq!(vault.0.lock().unwrap().len(), 1);
}

#[test]
fn without_a_vault_nothing_secret_is_made_or_imported() {
    let m = Mail::new();
    assert_eq!(
        m.core().generate_key(m.account).unwrap_err(),
        PgpError::NoVault
    );
    assert_eq!(
        m.core()
            .import_key(m.account, fixture("gpg-alice.sec.asc"), PASS.into())
            .unwrap_err(),
        PgpError::NoVault
    );
    // A public key needs no vault.
    assert!(
        !m.core()
            .import_key(m.account, fixture("gpg-bob.pub.asc"), String::new())
            .unwrap()
            .own
    );
}

#[test]
fn a_public_key_file_becomes_a_contacts_key_and_key_status_says_who_can_be_encrypted_to() {
    let (m, _) = with_vault();
    let bob = m
        .core()
        .import_key(m.account, fixture("gpg-bob.pub.asc"), String::new())
        .unwrap();
    assert!(!bob.own);
    assert_eq!(bob.addresses, ["bob@example.net"]);
    // An expired key is kept but cannot be encrypted to.
    m.core()
        .import_key(
            m.account,
            fixture("gpg-carol-expired.pub.asc"),
            String::new(),
        )
        .unwrap();
    let status = m
        .core()
        .key_status(
            m.account,
            vec![
                "Bob@Example.NET".into(),
                "carol@example.com".into(),
                "nobody@example.org".into(),
                "not an address".into(),
            ],
        )
        .unwrap();
    assert_eq!(
        status[0].fingerprint.as_deref(),
        Some(bob.fingerprint.as_str())
    );
    assert_eq!(status[0].source, Some(PgpKeySource::Imported));
    assert!(status[1..].iter().all(|s| s.fingerprint.is_none()));
    assert_eq!(status[3].address, "not an address");
}

#[test]
fn autocrypt_is_learnt_from_read_mail_newer_only_and_never_from_own_or_forged_senders() {
    let (m, _) = with_vault();
    let sam = keys::generate("Sam <sam@example.net>", now_secs() - 60).unwrap();
    let sam_cert = keys::read_public(&sam.public).unwrap();
    let header = autocrypt::header("sam@example.net", &sam_cert, true, now_secs()).unwrap();
    let other = keys::generate("Sam two <sam@example.net>", now_secs() - 60).unwrap();
    let other_header = autocrypt::header(
        "sam@example.net",
        &keys::read_public(&other.public).unwrap(),
        false,
        now_secs(),
    )
    .unwrap();
    // "Newer" is by arrival (the server's INTERNALDATE, which the sender cannot choose): the fake server dates each message a
    // second after the one before, so "old" goes in first whatever its Date header says.
    m.imap.add_message(
        "INBOX",
        &mail_from(
            "Sam <sam@example.net>",
            "old",
            "Mon, 02 Nov 2026 09:00:00 +0000",
            Some(&other_header),
        ),
    );
    m.imap.add_message(
        "INBOX",
        &mail_from(
            "Sam <sam@example.net>",
            "new",
            "Mon, 16 Nov 2026 09:00:00 +0000",
            Some(&header),
        ),
    );
    // Mallory writes Sam's address in the header: refused (it is not the From address).
    m.imap.add_message(
        "INBOX",
        &mail_from(
            "Mallory <mallory@example.com>",
            "forged",
            "Tue, 17 Nov 2026 09:00:00 +0000",
            Some(&other_header),
        ),
    );
    // The account's own address: never taken.
    m.imap.add_message(
        "INBOX",
        &mail_from(
            "Alice <alice@example.org>",
            "mine",
            "Tue, 17 Nov 2026 09:00:00 +0000",
            Some(&other_header.replace("sam@example.net", "alice@example.org")),
        ),
    );
    read(&m, "old", 4);
    assert_eq!(
        m.core().pgp_keys(m.account).unwrap()[0].fingerprint,
        other.fingerprint
    );
    for s in ["new", "forged", "mine"] {
        read(&m, s, 4);
    }
    // Reading the older one again changes nothing.
    read(&m, "old", 4);
    let keys_now = m.core().pgp_keys(m.account).unwrap();
    assert_eq!(
        keys_now.len(),
        1,
        "only Sam's newest header; the key it replaced is gone"
    );
    assert_eq!(
        (keys_now[0].fingerprint.as_str(), keys_now[0].source),
        (sam.fingerprint.as_str(), PgpKeySource::Autocrypt)
    );
    assert_eq!(keys_now[0].addresses, ["sam@example.net"]);
    let status = m
        .core()
        .key_status(
            m.account,
            vec!["sam@example.net".into(), "mallory@example.com".into()],
        )
        .unwrap();
    assert_eq!(
        status[0].fingerprint.as_deref(),
        Some(sam.fingerprint.as_str())
    );
    assert!(status[0].prefer_mutual);
    assert!(status[1].fingerprint.is_none());
}

#[test]
fn every_sent_message_carries_the_accounts_autocrypt_header() {
    let (m, _) = with_vault();
    let id = m.draft("Before a key");
    m.core().send(m.account, id).unwrap();
    wait_until("sent", || m.accepted() == 1);
    assert!(!String::from_utf8_lossy(&m.smtp.accepted()[0].data).contains("Autocrypt:"));
    let made = m.core().generate_key(m.account).unwrap();
    let id = m.draft("With a key");
    m.core().send(m.account, id).unwrap();
    wait_until("sent", || m.accepted() == 2);
    let data = String::from_utf8_lossy(&m.smtp.accepted()[1].data).into_owned();
    let start = data.find("Autocrypt: ").expect("the header is there");
    // The header ends at the first line break not followed by a space (its folded lines start with one).
    let rest = &data[start..];
    let end = rest
        .match_indices("\r\n")
        .find(|(i, _)| !rest[i + 2..].starts_with(' '))
        .map_or(rest.len(), |(i, _)| i);
    let value = rest["Autocrypt: ".len()..end].replace("\r\n ", " ");
    let h = autocrypt::parse_header(&value, "alice@example.org", now_secs()).unwrap();
    assert_eq!(keys::fingerprint(&h.cert), made.fingerprint);
}

/// A key directory on this machine: answers the direct WKD path with `key`, 404 otherwise.
fn wkd_server(path: String, key: Vec<u8>) -> String {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = block(tokio::net::TcpListener::bind("127.0.0.1:0")).unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async move {
            let listener = tokio::net::TcpListener::from_std(listener.into_std().unwrap()).unwrap();
            while let Ok((mut s, _)) = listener.accept().await {
                let mut buf = vec![0u8; 8192];
                let n = s.read(&mut buf).await.unwrap_or(0);
                let head = String::from_utf8_lossy(&buf[..n]).into_owned();
                let asked = head.split_whitespace().nth(1).unwrap_or("").to_owned();
                let answer = if asked == path {
                    let mut a = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n", key.len())
                        .into_bytes();
                    a.extend_from_slice(&key);
                    a
                } else {
                    b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n".to_vec()
                };
                let _ = s.write_all(&answer).await;
            }
        });
    });
    format!("http://127.0.0.1:{port}")
}

#[test]
fn wkd_finds_a_key_that_names_the_address_and_refuses_one_that_does_not() {
    let (m, _) = with_vault();
    let joe = keys::generate("Joe <Joe.Doe@example.org>", now_secs() - 60).unwrap();
    let urls = croft_crypto::wkd::urls("Joe.Doe@example.org").unwrap();
    let direct = format!(
        "/{}",
        urls.direct
            .split_once("/.well-known/")
            .map(|(_, p)| format!(".well-known/{p}"))
            .unwrap()
    );
    let base = wkd_server(direct.clone(), joe.public.clone());
    *crate::pgp::WKD_BASE_FOR_TESTS.lock().unwrap() = Some(base);
    let found = block(
        m.core()
            .clone()
            .lookup_key(m.account, "Joe.Doe@example.org".into()),
    )
    .unwrap();
    let key = found.key.unwrap();
    assert_eq!(
        (key.fingerprint.as_str(), key.source),
        (joe.fingerprint.as_str(), PgpKeySource::Wkd)
    );
    assert_eq!(key.addresses, ["joe.doe@example.org"]);
    // Nobody published: not found.
    let none = block(
        m.core()
            .clone()
            .lookup_key(m.account, "nobody@example.org".into()),
    )
    .unwrap();
    assert_eq!(none.problem, Some(LookupProblem::NotFound));
    // A key that does not name the address it was found for: refused.
    let eve = keys::generate("Eve <eve@example.org>", now_secs() - 60).unwrap();
    let urls = croft_crypto::wkd::urls("ann@example.org").unwrap();
    let path = format!(
        "/{}",
        urls.direct
            .split_once("/.well-known/")
            .map(|(_, p)| format!(".well-known/{p}"))
            .unwrap()
    );
    *crate::pgp::WKD_BASE_FOR_TESTS.lock().unwrap() = Some(wkd_server(path, eve.public));
    let wrong = block(
        m.core()
            .clone()
            .lookup_key(m.account, "ann@example.org".into()),
    )
    .unwrap();
    assert_eq!(wrong.problem, Some(LookupProblem::Refused));
    assert_eq!(
        block(m.core().clone().lookup_key(m.account, "joe@0x7f.1".into())).unwrap_err(),
        PgpError::BadAddress
    );
    *crate::pgp::WKD_BASE_FOR_TESTS.lock().unwrap() = None;
}

/// The folder id of `path`, once the first folder sync has stored it.
fn folder(m: &Mail, path: &str) -> i64 {
    let mut id = None;
    wait_until("the folder is known", || {
        id = m
            .core()
            .inner
            .handle
            .read(|db| croft_storage::folders::find_by_path(db.connection(), m.account, path))
            .unwrap()
            .map(|f| f.id);
        id.is_some()
    });
    id.unwrap()
}

/// An Autocrypt header block naming `addr`, with `cert`'s minimal key.
fn header_block(addr: &str, cert: &croft_crypto::Certificate, mutual: bool) -> String {
    let named = keys::info(cert, now_secs()).addresses[0].clone();
    let value = autocrypt::header(&named, cert, mutual, now_secs())
        .unwrap()
        .replace(&format!("addr={named}"), &format!("addr={addr}"));
    format!("Autocrypt: {value}\r\n")
}

fn now_ms_here() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64
}

#[test]
fn a_header_cannot_take_over_a_key_the_person_chose_nor_file_it_under_another_sender() {
    let (m, _) = with_vault();
    let inbox = folder(&m, "INBOX");
    let made = m.core().generate_key(m.account).unwrap();
    let bob = m
        .core()
        .import_key(m.account, fixture("gpg-bob.pub.asc"), String::new())
        .unwrap();
    let own_cert = keys::read_public(
        m.core()
            .export_public_key(m.account, made.fingerprint.clone())
            .unwrap()
            .as_bytes(),
    )
    .unwrap();
    let bob_cert = keys::read_public(&fixture("gpg-bob.pub.asc")).unwrap();
    // Mallory sends the account's own key, then Bob's, as Mallory's Autocrypt key.
    for cert in [&own_cert, &bob_cert] {
        m.core().learn_autocrypt(
            m.account,
            inbox,
            Some("mallory@example.com"),
            now_ms_here(),
            &header_block("mallory@example.com", cert, true),
        );
    }
    let keys_now = m.core().pgp_keys(m.account).unwrap();
    assert_eq!(keys_now.len(), 2);
    let own = keys_now.iter().find(|k| k.own).unwrap();
    assert_eq!(
        (own.source, own.addresses.clone(), own.can_sign),
        (
            PgpKeySource::Made,
            vec!["alice@example.org".to_owned()],
            true
        )
    );
    let theirs = keys_now.iter().find(|k| !k.own).unwrap();
    assert_eq!(
        (
            theirs.fingerprint.as_str(),
            theirs.source,
            theirs.addresses.clone()
        ),
        (
            bob.fingerprint.as_str(),
            PgpKeySource::Imported,
            vec!["bob@example.net".to_owned()]
        )
    );
    let status = m
        .core()
        .key_status(
            m.account,
            vec!["mallory@example.com".into(), "bob@example.net".into()],
        )
        .unwrap();
    assert!(
        status[0].fingerprint.is_none(),
        "nothing filed under Mallory"
    );
    assert_eq!(
        status[1].fingerprint.as_deref(),
        Some(bob.fingerprint.as_str())
    );
    // The account still announces its own key.
    let header = m
        .core()
        .inner
        .handle
        .read(|db| {
            Ok::<_, PgpError>(crate::pgp::autocrypt_header(
                db.connection(),
                m.account,
                "alice@example.org",
            ))
        })
        .unwrap();
    assert!(header.is_some());
}

#[test]
fn autocrypt_from_a_future_date_or_junk_pins_nothing_and_a_later_plain_mail_stops_asking() {
    let (m, _) = with_vault();
    let (inbox, junk) = (folder(&m, "INBOX"), folder(&m, "Junk"));
    let first = keys::generate("Sam <sam@example.net>", now_secs() - 60).unwrap();
    let second = keys::generate("Sam <sam@example.net>", now_secs() - 60).unwrap();
    let cert = |k: &keys::Generated| keys::read_public(&k.public).unwrap();
    let learn = |folder: i64, date: i64, headers: &str| {
        m.core()
            .learn_autocrypt(m.account, folder, Some("sam@example.net"), date, headers);
    };
    // In Junk: nothing at all, not even the date.
    learn(
        junk,
        now_ms_here(),
        &header_block("sam@example.net", &cert(&first), true),
    );
    assert!(m.core().pgp_keys(m.account).unwrap().is_empty());
    // Dated in 2099: taken as dated now, so the next real header still replaces it.
    learn(
        inbox,
        4_070_908_800_000,
        &header_block("sam@example.net", &cert(&first), false),
    );
    std::thread::sleep(std::time::Duration::from_millis(20));
    learn(
        inbox,
        now_ms_here(),
        &header_block("sam@example.net", &cert(&second), true),
    );
    let status = |m: &Mail| {
        m.core()
            .key_status(m.account, vec!["sam@example.net".into()])
            .unwrap()
            .remove(0)
    };
    let s = status(&m);
    assert_eq!(s.fingerprint.as_deref(), Some(second.fingerprint.as_str()));
    assert!(s.prefer_mutual);
    // A later message without the header: the key stays, encryption is no longer asked for.
    std::thread::sleep(std::time::Duration::from_millis(20));
    learn(inbox, now_ms_here(), "Subject: plain\r\n");
    let s = status(&m);
    assert_eq!(s.fingerprint.as_deref(), Some(second.fingerprint.as_str()));
    assert!(!s.prefer_mutual);
}
