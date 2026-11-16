// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! OpenPGP keys (T41b): the account's own keys (their secret part only ever in the app's [`KeyVault`], the platform keystore),
//! other people's keys from Autocrypt headers, WKD and files, and for compose whether an address can be encrypted to.
//! Encrypting, signing, decrypting and verifying mail are T41c. Nothing secret is stored here, logged or put in an error.

use std::sync::Arc;

use croft_crypto::keys::{self, KeyInfo, SecretKey};
use croft_crypto::{autocrypt, wkd, CryptoError, SecretBytes};
use croft_storage::pgp::{self as store, KeySource, NewKey};
use croft_storage::{accounts, identities};
use rusqlite::Connection;
use zeroize::Zeroizing;

use crate::core::{now_ms, Core};
use crate::credentials::ProviderError;
use crate::error::CoreError;

/// Implemented by the app (T42 on Android: the Keystore). Holds the secret part of the account's own OpenPGP keys, by account
/// and fingerprint; the core never stores one. Called from Rust threads, never from a UI thread.
///
/// The bytes handed to `store_secret_key` and returned by `secret_key` are an unprotected OpenPGP secret key: the app must keep
/// them only in its encrypted, hardware-backed storage and wipe its own copies after use.
#[uniffi::export(with_foreign)]
pub trait KeyVault: Send + Sync {
    /// Keeps `secret` for the account's key `fingerprint` (replacing any).
    fn store_secret_key(
        &self,
        account_id: i64,
        fingerprint: String,
        secret: Vec<u8>,
    ) -> Result<(), ProviderError>;
    /// The secret kept for the account's key `fingerprint`, or `None`.
    fn secret_key(
        &self,
        account_id: i64,
        fingerprint: String,
    ) -> Result<Option<Vec<u8>>, ProviderError>;
    /// Forgets the secret kept for the account's key `fingerprint`.
    fn delete_secret_key(&self, account_id: i64, fingerprint: String) -> Result<(), ProviderError>;
}

/// Why an OpenPGP key call failed. Fixed categories: never key material, an address or a passphrase.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Error)]
pub enum PgpError {
    /// The app has not given the core a key vault (`set_key_vault`): nothing secret can be made or used.
    NoVault,
    /// The key vault failed.
    VaultFailed,
    /// Not a key Croft can read.
    Unreadable,
    /// The file holds more than one key.
    NotOneKey,
    /// Too large.
    TooLarge,
    /// The passphrase does not unlock the key (or is empty for a backup).
    WrongPassphrase,
    /// The key has nothing usable (expired, revoked, too weak, or no address).
    NoUsableKey,
    /// No such key in the account.
    NoSuchKey,
    /// No such account.
    NoSuchAccount,
    /// Not an address.
    BadAddress,
    /// Something else failed.
    Internal,
}

impl std::fmt::Display for PgpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Debug::fmt(self, f)
    }
}

impl std::error::Error for PgpError {}

impl From<CryptoError> for PgpError {
    fn from(e: CryptoError) -> Self {
        match e {
            CryptoError::Unreadable | CryptoError::BadHeader => PgpError::Unreadable,
            CryptoError::TooLarge => PgpError::TooLarge,
            CryptoError::NotOneKey => PgpError::NotOneKey,
            CryptoError::NoUsableKey
            | CryptoError::NoEncryptionKey(_)
            | CryptoError::CannotDecrypt => PgpError::NoUsableKey,
            CryptoError::WrongPassphrase => PgpError::WrongPassphrase,
            CryptoError::BadAddress => PgpError::BadAddress,
            CryptoError::Failed => PgpError::Internal,
        }
    }
}

impl From<CoreError> for PgpError {
    fn from(e: CoreError) -> Self {
        match e {
            CoreError::NoSuchAccount => PgpError::NoSuchAccount,
            _ => PgpError::Internal,
        }
    }
}

impl From<croft_storage::StorageError> for PgpError {
    fn from(_: croft_storage::StorageError) -> Self {
        PgpError::Internal
    }
}

impl From<ProviderError> for PgpError {
    fn from(_: ProviderError) -> Self {
        PgpError::VaultFailed
    }
}

/// How Croft has a key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum PgpKeySource {
    /// Made on this device.
    Made,
    /// Imported from a file.
    Imported,
    /// From the sender's Autocrypt header.
    Autocrypt,
    /// From the address's domain (WKD).
    Wkd,
}

impl From<KeySource> for PgpKeySource {
    fn from(s: KeySource) -> Self {
        match s {
            KeySource::Made => PgpKeySource::Made,
            KeySource::Imported => PgpKeySource::Imported,
            KeySource::Autocrypt => PgpKeySource::Autocrypt,
            KeySource::Wkd => PgpKeySource::Wkd,
        }
    }
}

/// A key as S54 shows it. `Debug` prints no address.
#[derive(Clone, PartialEq, Eq, uniffi::Record)]
pub struct PgpKeyInfo {
    /// The fingerprint, upper-case hex.
    pub fingerprint: String,
    /// The addresses it is filed under.
    pub addresses: Vec<String>,
    /// The account's own key (its secret is in the vault).
    pub own: bool,
    /// How Croft has it.
    pub source: PgpKeySource,
    /// Made, Unix ms.
    pub created_ms: i64,
    /// Expires, Unix ms.
    pub expires_ms: Option<i64>,
    /// Revoked by its owner.
    pub revoked: bool,
    /// Expired now.
    pub expired: bool,
    /// Croft can encrypt to it now.
    pub can_encrypt: bool,
    /// It can sign now.
    pub can_sign: bool,
    /// The primary key's algorithm, for display.
    pub algorithm: String,
}

impl std::fmt::Debug for PgpKeyInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PgpKeyInfo")
            .field("own", &self.own)
            .field("can_encrypt", &self.can_encrypt)
            .finish_non_exhaustive()
    }
}

/// Whether Croft can encrypt to one address (for compose). `Debug` prints no address.
#[derive(Clone, PartialEq, Eq, uniffi::Record)]
pub struct KeyStatus {
    /// The address, as asked.
    pub address: String,
    /// The key Croft would encrypt to, when there is one usable now that names the address.
    pub fingerprint: Option<String>,
    /// How Croft has that key.
    pub source: Option<PgpKeySource>,
    /// The sender's newest Autocrypt header said `prefer-encrypt=mutual`.
    pub prefer_mutual: bool,
}

impl std::fmt::Debug for KeyStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KeyStatus")
            .field("has_key", &self.fingerprint.is_some())
            .finish_non_exhaustive()
    }
}

/// What a WKD lookup found.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum LookupProblem {
    /// The domain publishes no key for the address.
    NotFound,
    /// The domain could not be reached: try later.
    Unreachable,
    /// The domain answered with something Croft does not take (a redirect, a key that does not name the address, too large).
    Refused,
}

/// A WKD lookup's outcome: the key stored, or why not.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct LookupResult {
    /// The key, now stored for the account.
    pub key: Option<PgpKeyInfo>,
    /// Why there is none.
    pub problem: Option<LookupProblem>,
}

fn now_secs() -> u32 {
    u32::try_from(now_ms().max(0) / 1000).unwrap_or(u32::MAX)
}

fn info_of(row: &store::KeyRow, info: &KeyInfo) -> PgpKeyInfo {
    PgpKeyInfo {
        fingerprint: row.fingerprint.clone(),
        addresses: row.addresses.clone(),
        own: row.own,
        source: row.source.into(),
        created_ms: i64::from(info.created) * 1000,
        expires_ms: info.expires.map(|e| i64::from(e) * 1000),
        revoked: info.revoked,
        expired: info.expired,
        can_encrypt: info.can_encrypt,
        can_sign: info.can_sign,
        algorithm: info.algorithm.clone(),
    }
}

/// Stores `cert` for the account, filed under `addresses`.
fn store_cert(
    conn: &Connection,
    account: i64,
    cert: &croft_crypto::Certificate,
    own: bool,
    source: KeySource,
    addresses: &[String],
) -> Result<PgpKeyInfo, PgpError> {
    let now = now_secs();
    let info = keys::info(cert, now);
    let public = keys::public_bytes(cert)?;
    store::upsert_key(
        conn,
        account,
        &NewKey {
            fingerprint: &info.fingerprint,
            public_key: &public,
            own,
            source,
            created_at: i64::from(info.created),
            expires_at: info.expires.map(i64::from),
            revoked: info.revoked,
            addresses,
        },
        now_ms(),
    )?
    .ok_or(PgpError::NoSuchAccount)?;
    let row = store::key(conn, account, &info.fingerprint)?.ok_or(PgpError::Internal)?;
    Ok(info_of(&row, &info))
}

/// The account's own addresses, ASCII lower-case (its address and its identities').
fn own_addresses(conn: &Connection, account: i64) -> Result<Vec<String>, PgpError> {
    let acct = accounts::get(conn, account)?.ok_or(PgpError::NoSuchAccount)?;
    let mut out = vec![acct.email.to_ascii_lowercase()];
    for i in identities::list(conn, account)? {
        let a = i.address.to_ascii_lowercase();
        if !out.contains(&a) {
            out.push(a);
        }
    }
    Ok(out)
}

impl Core {
    fn vault(&self) -> Result<Arc<dyn KeyVault>, PgpError> {
        self.inner
            .key_vault
            .lock()
            .map_err(|_| PgpError::Internal)?
            .clone()
            .ok_or(PgpError::NoVault)
    }

    /// The account's own key in use: the newest own key that can sign and encrypt now, with its secret from the vault.
    /// Signing and decrypting use it (T41c).
    #[allow(dead_code)]
    pub(crate) fn own_secret(
        &self,
        account: i64,
    ) -> Result<Option<(SecretKey, croft_crypto::Certificate)>, PgpError> {
        let rows = self
            .inner
            .handle
            .read(|db| store::own_keys(db.connection(), account))?;
        let now = now_secs();
        for row in rows {
            let Ok(cert) = keys::read_public(&row.public_key) else {
                continue;
            };
            let info = keys::info(&cert, now);
            if !info.can_encrypt || !info.can_sign {
                continue;
            }
            let vault = self.vault()?;
            let Some(bytes) = vault.secret_key(account, row.fingerprint.clone())? else {
                continue;
            };
            let secret = SecretBytes(Zeroizing::new(bytes));
            let key = keys::load_secret(&secret)?;
            // The vault must give back the key it was given (never another).
            if keys::fingerprint(&keys::public_of(&key)) != row.fingerprint {
                return Err(PgpError::VaultFailed);
            }
            return Ok(Some((key, cert)));
        }
        Ok(None)
    }

    /// Takes the Autocrypt header of a read message (T41b): the sender's key is filed under the header's address when the
    /// message is newer than the last header taken from them. Messages from the account's own addresses, with more than one
    /// Autocrypt header, or whose header does not match the `From` address, give nothing. Never fails the caller.
    pub(crate) fn learn_autocrypt(
        &self,
        account: i64,
        from: Option<&str>,
        date_ms: i64,
        headers_raw: &str,
    ) {
        let Some(from) = from.and_then(keys::bare_address) else {
            return;
        };
        let _ = self.inner.handle.read(|db| {
            let c = db.connection();
            if own_addresses(c, account)
                .map_err(|_| croft_storage::StorageError::NotFound("account"))?
                .contains(&from)
            {
                return Ok::<_, croft_storage::StorageError>(());
            }
            let values = header_values(headers_raw, "autocrypt");
            let header = match values.as_slice() {
                [one] => autocrypt::parse_header(one, &from, now_secs()).ok(),
                _ => None,
            };
            let Some(h) = header else {
                store::saw_message(c, account, &from, date_ms, None)?;
                return Ok(());
            };
            let fp = keys::fingerprint(&h.cert);
            if store::saw_message(c, account, &from, date_ms, Some((&fp, h.prefer_mutual)))? {
                // Filed under the header's address only (the key may name others).
                let _ = store_cert(
                    c,
                    account,
                    &h.cert,
                    false,
                    KeySource::Autocrypt,
                    &[h.addr.clone()],
                );
            }
            Ok(())
        });
    }
}

/// The `Autocrypt:` header value for a message sent from `from` (an identity of the account), when the account has an own key
/// that names that address. `None` otherwise (and on any failure: a message is never held back for it).
pub(crate) fn autocrypt_header(conn: &Connection, account: i64, from: &str) -> Option<String> {
    let rows = store::own_keys(conn, account).ok()?;
    let now = now_secs();
    rows.iter().find_map(|row| {
        let cert = keys::read_public(&row.public_key).ok()?;
        autocrypt::header(from, &cert, false, now).ok()
    })
}

/// The unfolded values of every header called `name` (case-insensitive) in a header block, at most 4.
fn header_values(headers: &str, name: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut current: Option<String> = None;
    for line in headers.split('\n') {
        let line = line.strip_suffix('\r').unwrap_or(line);
        if line.starts_with([' ', '\t']) {
            if let Some(c) = current.as_mut() {
                c.push(' ');
                c.push_str(line.trim());
            }
            continue;
        }
        if let Some(c) = current.take() {
            out.push(c);
        }
        if line.is_empty() {
            break;
        }
        if let Some((n, v)) = line.split_once(':') {
            if n.trim().eq_ignore_ascii_case(name) {
                if out.len() >= 4 {
                    break;
                }
                current = Some(v.trim().to_owned());
            }
        }
    }
    if let Some(c) = current {
        out.push(c);
    }
    out
}

#[uniffi::export]
impl Core {
    /// Gives the core the app's key vault (the platform keystore), where the secret part of the account's own keys lives.
    pub fn set_key_vault(&self, vault: Arc<dyn KeyVault>) {
        if let Ok(mut v) = self.inner.key_vault.lock() {
            *v = Some(vault);
        }
    }

    /// The account's keys (S54, S55): its own first, then other people's, newest first.
    pub fn pgp_keys(&self, account: i64) -> Result<Vec<PgpKeyInfo>, PgpError> {
        let rows = self
            .inner
            .handle
            .read(|db| store::keys(db.connection(), account))?;
        let now = now_secs();
        Ok(rows
            .iter()
            .filter_map(|r| {
                let cert = keys::read_public(&r.public_key).ok()?;
                Some(info_of(r, &keys::info(&cert, now)))
            })
            .collect())
    }

    /// Makes a new own key for the account (its name and address as the user ID): the secret goes to the vault, the public
    /// certificate to the database.
    pub fn generate_key(&self, account: i64) -> Result<PgpKeyInfo, PgpError> {
        let vault = self.vault()?;
        let (name, address) = self.inner.handle.read(|db| {
            let c = db.connection();
            let acct = accounts::get(c, account)?.ok_or(PgpError::NoSuchAccount)?;
            let first = identities::list(c, account)?.into_iter().next();
            Ok::<_, PgpError>(match first {
                Some(i) => (i.display_name, i.address),
                None => (acct.label, acct.email),
            })
        })?;
        let address = keys::bare_address(&address).ok_or(PgpError::BadAddress)?;
        let name: String = name
            .chars()
            .filter(|c| !c.is_control() && !matches!(c, '<' | '>'))
            .take(100)
            .collect();
        let user_id = if name.trim().is_empty() {
            address.clone()
        } else {
            format!("{} <{address}>", name.trim())
        };
        let made = keys::generate(&user_id, now_secs())?;
        vault.store_secret_key(account, made.fingerprint.clone(), made.secret.0.to_vec())?;
        let cert = keys::read_public(&made.public)?;
        let stored = self.inner.handle.read(|db| {
            store_cert(
                db.connection(),
                account,
                &cert,
                true,
                KeySource::Made,
                &[address.clone()],
            )
        });
        if stored.is_err() {
            // Nothing half-made: the vault forgets the secret again.
            let _ = vault.delete_secret_key(account, made.fingerprint.clone());
        }
        stored
    }

    /// Imports a key file (armored or binary). A secret key is unlocked with `passphrase`, its secret goes to the vault and it
    /// becomes the account's own; a public key becomes someone else's key, filed under the addresses it names.
    pub fn import_key(
        &self,
        account: i64,
        bytes: Vec<u8>,
        passphrase: String,
    ) -> Result<PgpKeyInfo, PgpError> {
        let bytes = Zeroizing::new(bytes);
        let passphrase = Zeroizing::new(passphrase);
        let looks_secret = bytes.windows(19).any(|w| w == b"PRIVATE KEY BLOCK--")
            || bytes
                .first()
                .is_some_and(|b| matches!(b, 0x95 | 0x94 | 0xc5));
        let now = now_secs();
        if looks_secret {
            let vault = self.vault()?;
            let key = keys::read_secret(&bytes, &passphrase)?;
            let cert = keys::public_of(&key);
            let info = keys::info(&cert, now);
            if !info.can_encrypt || !info.can_sign {
                return Err(PgpError::NoUsableKey);
            }
            let secret = keys::store_secret(&key)?;
            vault.store_secret_key(account, info.fingerprint.clone(), secret.0.to_vec())?;
            let stored = self.inner.handle.read(|db| {
                store_cert(
                    db.connection(),
                    account,
                    &cert,
                    true,
                    KeySource::Imported,
                    &info.addresses,
                )
            });
            if stored.is_err() {
                let _ = vault.delete_secret_key(account, info.fingerprint.clone());
            }
            return stored;
        }
        let cert = keys::read_public(&bytes)?;
        let info = keys::info(&cert, now);
        if info.addresses.is_empty() {
            return Err(PgpError::NoUsableKey);
        }
        self.inner.handle.read(|db| {
            store_cert(
                db.connection(),
                account,
                &cert,
                false,
                KeySource::Imported,
                &info.addresses,
            )
        })
    }

    /// The key's public certificate, armored (to share).
    pub fn export_public_key(&self, account: i64, fingerprint: String) -> Result<String, PgpError> {
        let row = self
            .inner
            .handle
            .read(|db| store::key(db.connection(), account, &fingerprint))?
            .ok_or(PgpError::NoSuchKey)?;
        Ok(keys::export_public(&keys::read_public(&row.public_key)?)?)
    }

    /// A backup of an own key: the secret key protected with `passphrase` (not empty), armored.
    pub fn export_secret_key(
        &self,
        account: i64,
        fingerprint: String,
        passphrase: String,
    ) -> Result<String, PgpError> {
        let passphrase = Zeroizing::new(passphrase);
        let row = self
            .inner
            .handle
            .read(|db| store::key(db.connection(), account, &fingerprint))?
            .ok_or(PgpError::NoSuchKey)?;
        if !row.own {
            return Err(PgpError::NoSuchKey);
        }
        let bytes = self
            .vault()?
            .secret_key(account, fingerprint.clone())?
            .ok_or(PgpError::NoSuchKey)?;
        let key = keys::load_secret(&SecretBytes(Zeroizing::new(bytes)))?;
        if keys::fingerprint(&keys::public_of(&key)) != fingerprint {
            return Err(PgpError::VaultFailed);
        }
        let armored = keys::export_secret(&key, &passphrase)?;
        Ok(armored.as_str().to_owned())
    }

    /// Removes a key from the account (an own key's secret from the vault too). False when there was none.
    pub fn delete_key(&self, account: i64, fingerprint: String) -> Result<bool, PgpError> {
        let row = self
            .inner
            .handle
            .read(|db| store::key(db.connection(), account, &fingerprint))?;
        let Some(row) = row else {
            return Ok(false);
        };
        if row.own {
            self.vault()?
                .delete_secret_key(account, fingerprint.clone())?;
        }
        Ok(self
            .inner
            .handle
            .read(|db| store::delete_key(db.connection(), account, &fingerprint))?)
    }

    /// For compose: whether Croft can encrypt to each address now (a usable key filed under it), and whether its sender asked
    /// for encryption.
    pub fn key_status(
        &self,
        account: i64,
        addresses: Vec<String>,
    ) -> Result<Vec<KeyStatus>, PgpError> {
        let now = now_secs();
        self.inner.handle.read(|db| {
            let c = db.connection();
            let mut out = Vec::with_capacity(addresses.len().min(500));
            for asked in addresses.into_iter().take(500) {
                let bare = keys::bare_address(&asked);
                let (mut fingerprint, mut source, mut prefer) = (None, None, false);
                if let Some(a) = bare.as_deref() {
                    for row in store::keys_for(c, account, a)? {
                        let Ok(cert) = keys::read_public(&row.public_key) else {
                            continue;
                        };
                        if keys::info(&cert, now).can_encrypt {
                            fingerprint = Some(row.fingerprint.clone());
                            source = Some(row.source.into());
                            break;
                        }
                    }
                    prefer = store::peer(c, account, a)?
                        .is_some_and(|p| p.prefer_mutual && p.fingerprint == fingerprint);
                }
                out.push(KeyStatus {
                    address: asked,
                    fingerprint,
                    source,
                    prefer_mutual: prefer,
                });
            }
            Ok::<_, PgpError>(out)
        })
    }

    /// Looks `address` up in its domain's Web Key Directory (the advanced address, then the direct one): `https` to public
    /// addresses only, no redirect, at most 256 KiB. A key found must name the address and be usable; it is stored filed under
    /// that address.
    pub async fn lookup_key(
        self: Arc<Self>,
        account: i64,
        address: String,
    ) -> Result<LookupResult, PgpError> {
        let core = self.clone();
        self.spawn(
            async move { core.lookup_now(account, &address).await },
            false,
        )
        .await
        .map_err(|_| PgpError::Internal)?
    }
}

/// Tests: fetch WKD keys from plain http on this machine (the in-process fake), at this base instead of the domain.
#[cfg(test)]
pub(crate) static WKD_BASE_FOR_TESTS: std::sync::Mutex<Option<String>> =
    std::sync::Mutex::new(None);

impl Core {
    async fn lookup_now(&self, account: i64, address: &str) -> Result<LookupResult, PgpError> {
        let bare = keys::bare_address(address).ok_or(PgpError::BadAddress)?;
        let urls = wkd::urls(&bare)?;
        let none = |p| {
            Ok(LookupResult {
                key: None,
                problem: Some(p),
            })
        };
        let mut found: Option<Vec<u8>> = None;
        let mut unreachable = false;
        for url in [&urls.advanced, &urls.direct] {
            #[cfg(test)]
            let got = match WKD_BASE_FOR_TESTS.lock().ok().and_then(|b| b.clone()) {
                Some(base) => {
                    let path = url
                        .split_once("/.well-known/")
                        .map(|(_, p)| p)
                        .unwrap_or_default();
                    croft_protocols::oauth::fetch::get_wkd_key_from_loopback_for_tests(&format!(
                        "{base}/.well-known/{path}"
                    ))
                    .await
                }
                None => croft_protocols::oauth::fetch::get_wkd_key(url).await,
            };
            #[cfg(not(test))]
            let got = croft_protocols::oauth::fetch::get_wkd_key(url).await;
            match got {
                Ok(Some(bytes)) => {
                    found = Some(bytes);
                    break;
                }
                Ok(None) => {}
                Err(croft_protocols::oauth::fetch::FetchError::Unreachable) => unreachable = true,
                Err(_) => {}
            }
        }
        let Some(bytes) = found else {
            return none(if unreachable {
                LookupProblem::Unreachable
            } else {
                LookupProblem::NotFound
            });
        };
        let Ok(cert) = keys::read_public(&bytes) else {
            return none(LookupProblem::Refused);
        };
        let info = keys::info(&cert, now_secs());
        if !info.can_encrypt || !info.addresses.contains(&bare) {
            return none(LookupProblem::Refused);
        }
        let stored = self.inner.handle.read(|db| {
            store_cert(
                db.connection(),
                account,
                &cert,
                false,
                KeySource::Wkd,
                &[bare.clone()],
            )
        })?;
        Ok(LookupResult {
            key: Some(stored),
            problem: None,
        })
    }
}
