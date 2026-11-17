// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! OpenPGP keys (T41b): each account's public keys (its own and other people's) and its senders' Autocrypt state. Never a
//! secret: an own key's secret part is in the platform keystore only. Every statement is scoped by account.

use rusqlite::{params, Connection, OptionalExtension};

use crate::error::Result;

/// How Croft has a key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeySource {
    /// Made on this device.
    Made,
    /// Imported from a file.
    Imported,
    /// From a sender's Autocrypt header.
    Autocrypt,
    /// From the address's domain's WKD.
    Wkd,
}

impl KeySource {
    fn as_str(self) -> &'static str {
        match self {
            KeySource::Made => "made",
            KeySource::Imported => "imported",
            KeySource::Autocrypt => "autocrypt",
            KeySource::Wkd => "wkd",
        }
    }

    /// Chosen by the person (made here or imported), not discovered.
    fn chosen(self) -> bool {
        matches!(self, KeySource::Made | KeySource::Imported)
    }

    /// How much a copy from here counts: the person's choice, then a WKD (the domain's own answer), then an Autocrypt header.
    fn rank(self) -> u8 {
        match self {
            KeySource::Made | KeySource::Imported => 3,
            KeySource::Wkd => 2,
            KeySource::Autocrypt => 1,
        }
    }

    fn parse(s: &str) -> KeySource {
        match s {
            "made" => KeySource::Made,
            "imported" => KeySource::Imported,
            "autocrypt" => KeySource::Autocrypt,
            _ => KeySource::Wkd,
        }
    }
}

/// A stored key.
#[derive(Clone, PartialEq, Eq)]
pub struct KeyRow {
    /// Row id.
    pub id: i64,
    /// Fingerprint, upper-case hex.
    pub fingerprint: String,
    /// The public certificate, binary OpenPGP.
    pub public_key: Vec<u8>,
    /// Its secret part is in the keystore.
    pub own: bool,
    /// How Croft has it.
    pub source: KeySource,
    /// Created, Unix seconds.
    pub created_at: i64,
    /// Expires, Unix seconds.
    pub expires_at: Option<i64>,
    /// Revoked by its owner.
    pub revoked: bool,
    /// The addresses it is filed under.
    pub addresses: Vec<String>,
}

impl std::fmt::Debug for KeyRow {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KeyRow")
            .field("id", &self.id)
            .field("own", &self.own)
            .finish_non_exhaustive()
    }
}

/// What to store of a key.
#[derive(Debug, Clone, Copy)]
pub struct NewKey<'a> {
    /// Fingerprint, upper-case hex.
    pub fingerprint: &'a str,
    /// The public certificate.
    pub public_key: &'a [u8],
    /// Its secret part is in the keystore.
    pub own: bool,
    /// How Croft has it.
    pub source: KeySource,
    /// Created.
    pub created_at: i64,
    /// Expires.
    pub expires_at: Option<i64>,
    /// Revoked.
    pub revoked: bool,
    /// The addresses to file it under (ASCII lower-case).
    pub addresses: &'a [String],
}

/// Stores `key` for `account`, or updates the stored copy, and returns its row id (`None` when there is no such account).
///
/// A key the person chose (their own, made or imported) is changed only by the person: a copy from an Autocrypt header or a
/// WKD changes nothing about it, not even where it is filed. A revoked certificate is never replaced (a copy without the
/// revocation would bring the key back). Once own, a key stays own; its source becomes the strongest seen (made or
/// imported, then WKD, then Autocrypt). The person's copy replaces the addresses the key is filed under; a discovered
/// copy only adds to them.
pub fn upsert_key(
    conn: &Connection,
    account: i64,
    key: &NewKey<'_>,
    now: i64,
) -> Result<Option<i64>> {
    let exists: bool = conn.query_row(
        "SELECT EXISTS (SELECT 1 FROM account WHERE id = ?1)",
        params![account],
        |r| r.get(0),
    )?;
    if !exists {
        return Ok(None);
    }
    let by_person = key.source.chosen();
    let id = match self::key(conn, account, key.fingerprint)? {
        None => conn.query_row(
            r#"INSERT INTO pgp_key (account_id, fingerprint, public_key, own, source, created_at, expires_at, revoked, updated_at)
               VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9) RETURNING id"#,
            params![
                account,
                key.fingerprint,
                key.public_key,
                key.own,
                key.source.as_str(),
                key.created_at,
                key.expires_at,
                key.revoked,
                now
            ],
            |r| r.get::<_, i64>(0),
        )?,
        Some(old) if !by_person && (old.own || old.source.chosen()) => return Ok(Some(old.id)),
        Some(old) => {
            let source = if key.source.rank() > old.source.rank() {
                key.source
            } else {
                old.source
            };
            if old.revoked {
                conn.execute(
                    "UPDATE pgp_key SET own = MAX(own, ?3), source = ?4, updated_at = ?5 WHERE account_id = ?1 AND id = ?2",
                    params![account, old.id, key.own, source.as_str(), now],
                )?;
            } else {
                conn.execute(
                    "UPDATE pgp_key SET public_key = ?3, own = MAX(own, ?4), source = ?5, expires_at = ?6, revoked = ?7,
                        updated_at = ?8 WHERE account_id = ?1 AND id = ?2",
                    params![
                        account,
                        old.id,
                        key.public_key,
                        key.own,
                        source.as_str(),
                        key.expires_at,
                        key.revoked,
                        now
                    ],
                )?;
            }
            if by_person {
                conn.execute(
                    "DELETE FROM pgp_key_address WHERE key_id = ?1 AND account_id = ?2",
                    params![old.id, account],
                )?;
            }
            old.id
        }
    };
    for a in key.addresses.iter().take(64) {
        conn.execute(
            "INSERT OR IGNORE INTO pgp_key_address (account_id, key_id, address) VALUES (?1, ?2, ?3)",
            params![account, id, a.to_ascii_lowercase()],
        )?;
    }
    Ok(Some(id))
}

fn addresses(conn: &Connection, account: i64, id: i64) -> Result<Vec<String>> {
    let mut stmt =
        conn.prepare("SELECT address FROM pgp_key_address WHERE account_id = ?1 AND key_id = ?2 ORDER BY address")?;
    let rows = stmt.query_map(params![account, id], |r| r.get(0))?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

const COLUMNS: &str = "id, fingerprint, public_key, own, source, created_at, expires_at, revoked";

fn row(r: &rusqlite::Row<'_>) -> rusqlite::Result<KeyRow> {
    let source: String = r.get(4)?;
    Ok(KeyRow {
        id: r.get(0)?,
        fingerprint: r.get(1)?,
        public_key: r.get(2)?,
        own: r.get(3)?,
        source: KeySource::parse(&source),
        created_at: r.get(5)?,
        expires_at: r.get(6)?,
        revoked: r.get(7)?,
        addresses: Vec::new(),
    })
}

fn with_addresses(conn: &Connection, account: i64, mut rows: Vec<KeyRow>) -> Result<Vec<KeyRow>> {
    for r in rows.iter_mut() {
        r.addresses = addresses(conn, account, r.id)?;
    }
    Ok(rows)
}

/// The account's keys: its own first, then the others, newest first.
pub fn keys(conn: &Connection, account: i64) -> Result<Vec<KeyRow>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {COLUMNS} FROM pgp_key WHERE account_id = ?1 ORDER BY own DESC, created_at DESC, id DESC LIMIT 500"
    ))?;
    let rows: Vec<KeyRow> = stmt
        .query_map([account], row)?
        .collect::<rusqlite::Result<_>>()?;
    with_addresses(conn, account, rows)
}

/// The account's key with this fingerprint.
pub fn key(conn: &Connection, account: i64, fingerprint: &str) -> Result<Option<KeyRow>> {
    let found = conn
        .query_row(
            &format!("SELECT {COLUMNS} FROM pgp_key WHERE account_id = ?1 AND fingerprint = ?2"),
            params![account, fingerprint],
            row,
        )
        .optional()?;
    Ok(match found {
        Some(k) => with_addresses(conn, account, vec![k])?.pop(),
        None => None,
    })
}

/// The account's keys filed under `address` (ASCII lower-cased here): own first, then the person's imports, then WKD, then
/// Autocrypt; newest first within each.
pub fn keys_for(conn: &Connection, account: i64, address: &str) -> Result<Vec<KeyRow>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {} FROM pgp_key k JOIN pgp_key_address a ON a.key_id = k.id AND a.account_id = k.account_id
          WHERE k.account_id = ?1 AND a.address = ?2
          ORDER BY k.own DESC, CASE k.source WHEN 'made' THEN 3 WHEN 'imported' THEN 3 WHEN 'wkd' THEN 2 ELSE 1 END DESC,
                   k.created_at DESC, k.id DESC LIMIT 20",
        COLUMNS.split(", ").map(|c| format!("k.{c}")).collect::<Vec<_>>().join(", ")
    ))?;
    let rows: Vec<KeyRow> = stmt
        .query_map(params![account, address.to_ascii_lowercase()], row)?
        .collect::<rusqlite::Result<_>>()?;
    with_addresses(conn, account, rows)
}

/// The account's own keys (secret in the keystore), newest first.
pub fn own_keys(conn: &Connection, account: i64) -> Result<Vec<KeyRow>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {COLUMNS} FROM pgp_key WHERE account_id = ?1 AND own = 1 ORDER BY created_at DESC, id DESC LIMIT 20"
    ))?;
    let rows: Vec<KeyRow> = stmt
        .query_map([account], row)?
        .collect::<rusqlite::Result<_>>()?;
    with_addresses(conn, account, rows)
}

/// Removes the account's key with this fingerprint (and its addresses). False when there is none.
pub fn delete_key(conn: &Connection, account: i64, fingerprint: &str) -> Result<bool> {
    Ok(conn.execute(
        "DELETE FROM pgp_key WHERE account_id = ?1 AND fingerprint = ?2",
        params![account, fingerprint],
    )? > 0)
}

/// Removes a key Croft had only from an Autocrypt header once no sender's newest header names it any more (the sender replaced
/// it). A key made, imported or found in a Web Key Directory stays. True when it went.
pub fn drop_replaced_autocrypt_key(
    conn: &Connection,
    account: i64,
    fingerprint: &str,
) -> Result<bool> {
    Ok(conn.execute(
        "DELETE FROM pgp_key WHERE account_id = ?1 AND fingerprint = ?2 AND own = 0 AND source = 'autocrypt'
           AND NOT EXISTS (SELECT 1 FROM autocrypt_peer p WHERE p.account_id = ?1 AND p.fingerprint = ?2)",
        params![account, fingerprint],
    )? > 0)
}

/// What the account knows of a sender's Autocrypt headers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Peer {
    /// The fingerprint their newest header named.
    pub fingerprint: Option<String>,
    /// That header said `prefer-encrypt=mutual`.
    pub prefer_mutual: bool,
    /// The date of the message with that header.
    pub autocrypt_at: Option<i64>,
    /// The date of the newest message from them.
    pub last_seen: i64,
}

/// The account's Autocrypt state for `address` (ASCII lower-cased here).
pub fn peer(conn: &Connection, account: i64, address: &str) -> Result<Option<Peer>> {
    Ok(conn
        .query_row(
            "SELECT fingerprint, prefer_mutual, autocrypt_at, last_seen FROM autocrypt_peer WHERE account_id = ?1 AND address = ?2",
            params![account, address.to_ascii_lowercase()],
            |r| {
                Ok(Peer {
                    fingerprint: r.get(0)?,
                    prefer_mutual: r.get(1)?,
                    autocrypt_at: r.get(2)?,
                    last_seen: r.get(3)?,
                })
            },
        )
        .optional()?)
}

/// A message from `address` dated `date` was read. With `header` (fingerprint, `prefer-encrypt=mutual`), its Autocrypt
/// header is taken when the message is newer than the one the stored header came from; without, only `last_seen` moves.
/// Returns whether the header was taken.
pub fn saw_message(
    conn: &Connection,
    account: i64,
    address: &str,
    date: i64,
    header: Option<(&str, bool)>,
) -> Result<bool> {
    let address = address.to_ascii_lowercase();
    let current = peer(conn, account, &address)?;
    let newer_header = header.is_some()
        && current
            .as_ref()
            .is_none_or(|p| p.autocrypt_at.is_none_or(|t| date > t));
    let last_seen = current.as_ref().map_or(date, |p| p.last_seen.max(date));
    let (fingerprint, prefer, at) = match (newer_header, header, &current) {
        (true, Some((fp, mutual)), _) => (Some(fp.to_owned()), mutual, Some(date)),
        (_, _, Some(p)) => (p.fingerprint.clone(), p.prefer_mutual, p.autocrypt_at),
        _ => (None, false, None),
    };
    conn.execute(
        r#"INSERT INTO autocrypt_peer (account_id, address, fingerprint, prefer_mutual, autocrypt_at, last_seen)
           SELECT ?1, ?2, ?3, ?4, ?5, ?6 WHERE EXISTS (SELECT 1 FROM account WHERE id = ?1)
           ON CONFLICT (account_id, address) DO UPDATE SET fingerprint = excluded.fingerprint,
               prefer_mutual = excluded.prefer_mutual, autocrypt_at = excluded.autocrypt_at, last_seen = excluded.last_seen"#,
        params![account, address, fingerprint, prefer, at, last_seen],
    )?;
    Ok(newer_header)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

    use super::*;
    use crate::accounts::{insert, NewAccount, Protocol};
    use crate::Db;

    fn account(c: &Connection, n: i64) -> i64 {
        insert(
            c,
            &NewAccount {
                label: format!("A{n}"),
                email: format!("a{n}@example.org"),
                protocol: Protocol::Imap,
                provider: "generic".into(),
                config: "{}".into(),
                color: None,
                position: n,
            },
            0,
        )
        .unwrap()
    }

    fn new_key<'a>(fp: &'a str, own: bool, source: KeySource, addrs: &'a [String]) -> NewKey<'a> {
        NewKey {
            fingerprint: fp,
            public_key: b"public certificate bytes",
            own,
            source,
            created_at: 10,
            expires_at: None,
            revoked: false,
            addresses: addrs,
        }
    }

    #[test]
    fn keys_are_kept_per_account_once_per_fingerprint_and_filed_by_address() {
        let db = Db::open_in_memory().unwrap();
        let c = db.connection();
        let (a, b) = (account(c, 1), account(c, 2));
        let sam = vec!["Sam@Example.org".to_owned()];
        upsert_key(c, a, &new_key("AA", false, KeySource::Autocrypt, &sam), 1)
            .unwrap()
            .unwrap();
        // The same key again, from a WKD: one row, the stronger source.
        upsert_key(c, a, &new_key("AA", false, KeySource::Wkd, &sam), 2)
            .unwrap()
            .unwrap();
        assert_eq!(keys(c, a).unwrap().len(), 1);
        assert_eq!(keys(c, a).unwrap()[0].source, KeySource::Wkd);
        // A later Autocrypt copy does not lower it.
        upsert_key(c, a, &new_key("AA", false, KeySource::Autocrypt, &sam), 3).unwrap();
        assert_eq!(keys(c, a).unwrap()[0].source, KeySource::Wkd);
        assert_eq!(
            keys_for(c, a, "SAM@example.ORG").unwrap()[0].fingerprint,
            "AA"
        );
        assert!(
            keys_for(c, b, "sam@example.org").unwrap().is_empty(),
            "another account's keys stay apart"
        );
        // An own key stays own and keeps its source.
        let me = vec!["a1@example.org".to_owned()];
        upsert_key(c, a, &new_key("BB", true, KeySource::Made, &me), 3).unwrap();
        upsert_key(c, a, &new_key("BB", false, KeySource::Autocrypt, &me), 4).unwrap();
        let own = own_keys(c, a).unwrap();
        assert_eq!((own.len(), own[0].source), (1, KeySource::Made));
        assert_eq!(keys(c, a).unwrap()[0].fingerprint, "BB", "own first");
        assert!(
            upsert_key(c, 99, &new_key("CC", false, KeySource::Wkd, &me), 1)
                .unwrap()
                .is_none()
        );
        assert!(!delete_key(c, b, "AA").unwrap());
        assert!(delete_key(c, a, "AA").unwrap());
        assert!(keys_for(c, a, "sam@example.org").unwrap().is_empty());
    }

    #[test]
    fn only_the_person_changes_a_key_they_chose_and_a_revoked_certificate_is_never_replaced() {
        let db = Db::open_in_memory().unwrap();
        let c = db.connection();
        let a = account(c, 1);
        let (bob, mallory) = (
            vec!["bob@example.org".to_owned()],
            vec!["mallory@example.com".to_owned()],
        );
        let imported = NewKey {
            public_key: b"bob's full certificate",
            ..new_key("BB", false, KeySource::Imported, &bob)
        };
        upsert_key(c, a, &imported, 1).unwrap();
        // Discovered copies (Mallory's header with Bob's key, a WKD answer) change nothing about it.
        for source in [KeySource::Autocrypt, KeySource::Wkd] {
            let copy = NewKey {
                public_key: b"a minimal copy",
                ..new_key("BB", false, source, &mallory)
            };
            upsert_key(c, a, &copy, 2).unwrap();
        }
        let k = key(c, a, "BB").unwrap().unwrap();
        assert_eq!(
            (k.public_key.as_slice(), k.source, k.addresses.clone()),
            (
                &b"bob's full certificate"[..],
                KeySource::Imported,
                bob.clone()
            )
        );
        assert!(keys_for(c, a, "mallory@example.com").unwrap().is_empty());
        // A discovered key adds addresses as senders use it; it never loses one.
        let (sam, samuel) = (
            vec!["sam@example.org".to_owned()],
            vec!["samuel@example.org".to_owned()],
        );
        upsert_key(c, a, &new_key("SS", false, KeySource::Autocrypt, &sam), 1).unwrap();
        upsert_key(
            c,
            a,
            &new_key("SS", false, KeySource::Autocrypt, &samuel),
            2,
        )
        .unwrap();
        assert_eq!(
            keys_for(c, a, "sam@example.org").unwrap()[0].fingerprint,
            "SS"
        );
        assert_eq!(
            keys_for(c, a, "samuel@example.org").unwrap()[0].fingerprint,
            "SS"
        );
        // Revoked: a later copy without the revocation, even the person's, does not bring it back.
        let revoked = NewKey {
            public_key: b"with its revocation",
            revoked: true,
            ..new_key("RR", false, KeySource::Wkd, &bob)
        };
        upsert_key(c, a, &revoked, 1).unwrap();
        for source in [KeySource::Autocrypt, KeySource::Imported] {
            let older = NewKey {
                public_key: b"without it",
                ..new_key("RR", false, source, &bob)
            };
            upsert_key(c, a, &older, 2).unwrap();
        }
        let k = key(c, a, "RR").unwrap().unwrap();
        assert!(k.revoked);
        assert_eq!(k.public_key, b"with its revocation");
    }

    #[test]
    fn a_key_the_person_imported_comes_before_a_newer_discovered_one() {
        let db = Db::open_in_memory().unwrap();
        let c = db.connection();
        let a = account(c, 1);
        let bob = vec!["bob@example.org".to_owned()];
        upsert_key(c, a, &new_key("OLD", false, KeySource::Imported, &bob), 1).unwrap();
        let newer = |fp, source| NewKey {
            created_at: 99,
            ..new_key(fp, false, source, &bob)
        };
        upsert_key(c, a, &newer("AC", KeySource::Autocrypt), 2).unwrap();
        upsert_key(c, a, &newer("WK", KeySource::Wkd), 3).unwrap();
        let order: Vec<String> = keys_for(c, a, "bob@example.org")
            .unwrap()
            .into_iter()
            .map(|k| k.fingerprint)
            .collect();
        assert_eq!(order, ["OLD", "WK", "AC"]);
    }

    #[test]
    fn a_replaced_autocrypt_key_goes_only_when_no_sender_names_it_and_it_came_from_autocrypt() {
        let db = Db::open_in_memory().unwrap();
        let c = db.connection();
        let a = account(c, 1);
        let (sam, kim) = (
            vec!["sam@example.org".to_string()],
            vec!["kim@example.org".to_string()],
        );
        upsert_key(c, a, &new_key("AA", false, KeySource::Autocrypt, &sam), 1).unwrap();
        upsert_key(c, a, &new_key("CC", false, KeySource::Wkd, &sam), 1).unwrap();
        saw_message(c, a, "sam@example.org", 100, Some(("AA", false))).unwrap();
        saw_message(c, a, "kim@example.org", 100, Some(("AA", false))).unwrap();
        upsert_key(c, a, &new_key("AA", false, KeySource::Autocrypt, &kim), 1).unwrap();
        // Sam moves on; Kim's newest header still names AA, so it stays.
        saw_message(c, a, "sam@example.org", 200, Some(("BB", false))).unwrap();
        assert!(!drop_replaced_autocrypt_key(c, a, "AA").unwrap());
        saw_message(c, a, "kim@example.org", 200, Some(("BB", false))).unwrap();
        assert!(drop_replaced_autocrypt_key(c, a, "AA").unwrap());
        assert!(key(c, a, "AA").unwrap().is_none());
        // A key from a Web Key Directory is never dropped this way.
        assert!(!drop_replaced_autocrypt_key(c, a, "CC").unwrap());
        assert!(key(c, a, "CC").unwrap().is_some());
    }

    #[test]
    fn autocrypt_takes_only_a_newer_header_and_remembers_the_last_message() {
        let db = Db::open_in_memory().unwrap();
        let c = db.connection();
        let a = account(c, 1);
        assert!(saw_message(c, a, "sam@example.org", 100, Some(("AA", true))).unwrap());
        // An older message's header does not replace it.
        assert!(!saw_message(c, a, "sam@example.org", 50, Some(("EVIL", false))).unwrap());
        let p = peer(c, a, "SAM@example.org").unwrap().unwrap();
        assert_eq!(
            (
                p.fingerprint.as_deref(),
                p.prefer_mutual,
                p.autocrypt_at,
                p.last_seen
            ),
            (Some("AA"), true, Some(100), 100)
        );
        // A newer message without a header moves only last_seen.
        assert!(!saw_message(c, a, "sam@example.org", 200, None).unwrap());
        let p = peer(c, a, "sam@example.org").unwrap().unwrap();
        assert_eq!(
            (p.fingerprint.as_deref(), p.autocrypt_at, p.last_seen),
            (Some("AA"), Some(100), 200)
        );
        // A newer header replaces it.
        assert!(saw_message(c, a, "sam@example.org", 300, Some(("BB", false))).unwrap());
        assert_eq!(
            peer(c, a, "sam@example.org")
                .unwrap()
                .unwrap()
                .fingerprint
                .as_deref(),
            Some("BB")
        );
    }
}
