// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Contacts (T38a): each account's address books and cards, kept on the device. A card is stored
//! whole (so an edit sent back loses nothing) with the columns the lists read. Local changes are
//! marked `pending` until the server confirms them. Every statement is scoped by account.

use croft_mail_core::vcard::{self, Contact};
use rusqlite::{params, Connection, OptionalExtension};

use crate::error::Result;

/// Pending: nothing to send.
pub const PENDING_NONE: i64 = 0;
/// Pending: made on the device, not on the server yet.
pub const PENDING_CREATE: i64 = 1;
/// Pending: changed on the device.
pub const PENDING_UPDATE: i64 = 2;
/// Pending: deleted on the device (hidden until the server confirms).
pub const PENDING_DELETE: i64 = 3;

/// An address book.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Book {
    /// Row id.
    pub id: i64,
    /// Its path on the server.
    pub path: String,
    /// Its name.
    pub name: String,
    /// The last sync token.
    pub sync_token: Option<String>,
    /// The last ctag.
    pub ctag: Option<String>,
}

/// A contact as the list shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContactRow {
    /// Row id.
    pub id: i64,
    /// Account.
    pub account_id: i64,
    /// Name shown.
    pub display_name: String,
    /// A to Z key.
    pub sort_key: String,
    /// Its first address, if any.
    pub email: Option<String>,
}

/// A whole stored contact.
#[derive(Clone, PartialEq, Eq)]
pub struct StoredContact {
    /// Row id.
    pub id: i64,
    /// Account.
    pub account_id: i64,
    /// Book row id.
    pub book_id: i64,
    /// Its path on the server (None until created there).
    pub href: Option<String>,
    /// Its etag.
    pub etag: Option<String>,
    /// The card text.
    pub card: String,
    /// What waits to be sent.
    pub pending: i64,
    /// The server had changed it; its version was kept.
    pub conflict: bool,
}

impl std::fmt::Debug for StoredContact {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StoredContact")
            .field("id", &self.id)
            .field("pending", &self.pending)
            .finish_non_exhaustive()
    }
}

/// Adds (or renames) the address book at `path`; returns its id.
pub fn upsert_book(conn: &Connection, account: i64, path: &str, name: &str) -> Result<i64> {
    Ok(conn.query_row(
        r#"INSERT INTO address_book (account_id, path, name) VALUES (?1, ?2, ?3)
           ON CONFLICT (account_id, path) DO UPDATE SET name = excluded.name
           WHERE address_book.account_id = excluded.account_id
           RETURNING id"#,
        params![account, path, name],
        |r| r.get(0),
    )?)
}

/// The account's address books.
pub fn books(conn: &Connection, account: i64) -> Result<Vec<Book>> {
    let mut stmt = conn.prepare(
        "SELECT id, path, name, sync_token, ctag FROM address_book WHERE account_id = ?1 ORDER BY name, id",
    )?;
    let rows = stmt.query_map([account], |r| {
        Ok(Book {
            id: r.get(0)?,
            path: r.get(1)?,
            name: r.get(2)?,
            sync_token: r.get(3)?,
            ctag: r.get(4)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// Removes the account's books whose paths are not in `keep` (gone from the server), with their
/// contacts.
pub fn remove_books_not_in(conn: &Connection, account: i64, keep: &[String]) -> Result<()> {
    for b in books(conn, account)? {
        if !keep.contains(&b.path) {
            conn.execute(
                "DELETE FROM address_book WHERE account_id = ?1 AND id = ?2",
                params![account, b.id],
            )?;
        }
    }
    Ok(())
}

/// Records a book's sync state after a sync.
pub fn set_book_sync(
    conn: &Connection,
    account: i64,
    book: i64,
    token: Option<&str>,
    ctag: Option<&str>,
    now_ms: i64,
) -> Result<()> {
    conn.execute(
        "UPDATE address_book SET sync_token = ?3, ctag = ?4, synced_at = ?5 WHERE account_id = ?1 AND id = ?2",
        params![account, book, token, ctag, now_ms],
    )?;
    Ok(())
}

/// The cards of a book known on the server: path and etag.
pub fn server_cards(conn: &Connection, account: i64, book: i64) -> Result<Vec<(String, String)>> {
    let mut stmt = conn.prepare(
        "SELECT href, COALESCE(etag, '') FROM contact
          WHERE account_id = ?1 AND book_id = ?2 AND href IS NOT NULL",
    )?;
    let rows = stmt.query_map(params![account, book], |r| Ok((r.get(0)?, r.get(1)?)))?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// Stores a card fetched from the server at `href`. A card that cannot be read is skipped
/// (`Ok(None)`); a card with a change of the device's still waiting is not overwritten.
pub fn store_from_server(
    conn: &Connection,
    account: i64,
    book: i64,
    href: &str,
    etag: &str,
    text: &str,
    now_ms: i64,
) -> Result<Option<i64>> {
    let Ok(card) = vcard::parse(text) else {
        return Ok(None);
    };
    let contact = card.contact();
    let existing: Option<(i64, i64)> = conn
        .query_row(
            "SELECT id, pending FROM contact WHERE account_id = ?1 AND book_id = ?2 AND href = ?3",
            params![account, book, href],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    let id = match existing {
        Some((id, pending)) if pending != PENDING_NONE => return Ok(Some(id)),
        Some((id, _)) => {
            conn.execute(
                r#"UPDATE contact SET etag = ?3, uid = ?4, display_name = ?5, sort_key = ?6, card = ?7,
                                      conflict = 0, updated_at = ?8
                    WHERE account_id = ?1 AND id = ?2"#,
                params![
                    account,
                    id,
                    etag,
                    uid_of(&contact, href),
                    contact.display_name(),
                    contact.sort_key(),
                    text,
                    now_ms
                ],
            )?;
            id
        }
        None => {
            let inserted: Option<i64> = conn
                .query_row(
            r#"INSERT INTO contact (account_id, book_id, href, etag, uid, display_name, sort_key, card, updated_at)
               SELECT ?1, b.id, ?3, ?4, ?5, ?6, ?7, ?8, ?9 FROM address_book b
                WHERE b.account_id = ?1 AND b.id = ?2
               RETURNING id"#,
            params![
                account,
                book,
                href,
                etag,
                uid_of(&contact, href),
                contact.display_name(),
                contact.sort_key(),
                text,
                now_ms
            ],
            |r| r.get(0),
                )
                .optional()?;
            // Not a book of this account: nothing is stored.
            match inserted {
                Some(id) => id,
                None => return Ok(None),
            }
        }
    };
    set_emails(conn, account, id, &contact)?;
    Ok(Some(id))
}

fn uid_of(contact: &Contact, href: &str) -> String {
    contact
        .uid
        .clone()
        .filter(|u| !u.is_empty())
        .unwrap_or_else(|| href.to_owned())
}

fn set_emails(conn: &Connection, account: i64, id: i64, contact: &Contact) -> Result<()> {
    conn.execute(
        "DELETE FROM contact_email WHERE account_id = ?1 AND contact_id = ?2",
        params![account, id],
    )?;
    for e in &contact.emails {
        conn.execute(
            "INSERT OR IGNORE INTO contact_email (contact_id, account_id, address) VALUES (?2, ?1, ?3)",
            params![account, id, e.value.trim().to_lowercase()],
        )?;
    }
    Ok(())
}

/// Removes the card at `href` (gone from the server), unless a change of the device's waits.
pub fn remove_from_server(conn: &Connection, account: i64, book: i64, href: &str) -> Result<()> {
    conn.execute(
        "DELETE FROM contact WHERE account_id = ?1 AND book_id = ?2 AND href = ?3 AND pending = 0",
        params![account, book, href],
    )?;
    Ok(())
}

/// A contact made on the device in `book`: stored at once, sent at the next sync.
pub fn create_local(
    conn: &Connection,
    account: i64,
    book: i64,
    card: &str,
    now_ms: i64,
) -> Result<i64> {
    let parsed = vcard::parse(card).map_err(|_| crate::error::StorageError::NotFound("card"))?;
    let contact = parsed.contact();
    let uid = contact.uid.clone().unwrap_or_default();
    let id: i64 = conn
        .query_row(
            r#"INSERT INTO contact (account_id, book_id, href, etag, uid, display_name, sort_key, card, pending, updated_at)
               SELECT ?1, b.id, NULL, NULL, ?3, ?4, ?5, ?6, 1, ?7 FROM address_book b
                WHERE b.account_id = ?1 AND b.id = ?2
               RETURNING id"#,
            params![account, book, uid, contact.display_name(), contact.sort_key(), card, now_ms],
            |r| r.get(0),
        )
        .optional()?
        .ok_or(crate::error::StorageError::NotFound("address book"))?;
    set_emails(conn, account, id, &contact)?;
    Ok(id)
}

/// A contact changed on the device: its new card, sent at the next sync. False when there is no
/// such contact in the account.
pub fn update_local(
    conn: &Connection,
    account: i64,
    id: i64,
    card: &str,
    now_ms: i64,
) -> Result<bool> {
    let parsed = vcard::parse(card).map_err(|_| crate::error::StorageError::NotFound("card"))?;
    let contact = parsed.contact();
    let n = conn.execute(
        r#"UPDATE contact SET card = ?3, display_name = ?4, sort_key = ?5, updated_at = ?6, conflict = 0,
                              pending = CASE pending WHEN 1 THEN 1 ELSE 2 END
            WHERE account_id = ?1 AND id = ?2 AND pending <> 3"#,
        params![account, id, card, contact.display_name(), contact.sort_key(), now_ms],
    )?;
    if n > 0 {
        set_emails(conn, account, id, &contact)?;
    }
    Ok(n > 0)
}

/// A contact deleted on the device: one never sent goes at once; another is hidden until the server
/// confirms. False when there is no such contact in the account.
pub fn delete_local(conn: &Connection, account: i64, id: i64) -> Result<bool> {
    let gone = conn.execute(
        "DELETE FROM contact WHERE account_id = ?1 AND id = ?2 AND pending = 1",
        params![account, id],
    )?;
    if gone > 0 {
        return Ok(true);
    }
    Ok(conn.execute(
        "UPDATE contact SET pending = 3 WHERE account_id = ?1 AND id = ?2",
        params![account, id],
    )? > 0)
}

/// The contacts with a change waiting, oldest first, with their book's path.
pub fn pending(conn: &Connection, account: i64) -> Result<Vec<(StoredContact, String)>> {
    let mut stmt = conn.prepare(
        r#"SELECT c.id, c.account_id, c.book_id, c.href, c.etag, c.card, c.pending, c.conflict, b.path
             FROM contact c JOIN address_book b ON b.id = c.book_id AND b.account_id = c.account_id
            WHERE c.account_id = ?1 AND c.pending <> 0
            ORDER BY c.updated_at, c.id"#,
    )?;
    let rows = stmt.query_map([account], |r| Ok((stored(r)?, r.get(8)?)))?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

fn stored(r: &rusqlite::Row<'_>) -> rusqlite::Result<StoredContact> {
    Ok(StoredContact {
        id: r.get(0)?,
        account_id: r.get(1)?,
        book_id: r.get(2)?,
        href: r.get(3)?,
        etag: r.get(4)?,
        card: r.get(5)?,
        pending: r.get(6)?,
        conflict: r.get(7)?,
    })
}

/// The server took the change of contact `id`: a delete removes the row; a create or update keeps
/// it, at `href`, with no etag until the next listing.
pub fn confirm(conn: &Connection, account: i64, id: i64, href: &str) -> Result<()> {
    conn.execute(
        "DELETE FROM contact WHERE account_id = ?1 AND id = ?2 AND pending = 3",
        params![account, id],
    )?;
    conn.execute(
        "UPDATE contact SET pending = 0, href = ?3, etag = NULL WHERE account_id = ?1 AND id = ?2",
        params![account, id, href],
    )?;
    Ok(())
}

/// The server had changed contact `id`: the device's change is dropped (the next sync brings the
/// server's version) and the contact is marked so the screen can say so.
pub fn mark_conflict(conn: &Connection, account: i64, id: i64) -> Result<()> {
    conn.execute(
        "UPDATE contact SET pending = 0, conflict = 1, etag = NULL WHERE account_id = ?1 AND id = ?2",
        params![account, id],
    )?;
    Ok(())
}

/// One contact of the account.
pub fn get(conn: &Connection, account: i64, id: i64) -> Result<Option<StoredContact>> {
    Ok(conn
        .query_row(
            "SELECT id, account_id, book_id, href, etag, card, pending, conflict FROM contact
              WHERE account_id = ?1 AND id = ?2 AND pending <> 3",
            params![account, id],
            stored,
        )
        .optional()?)
}

/// The account's contacts A to Z whose name or address contains `query` (all for an empty one),
/// at most `limit` after `offset`.
pub fn list(
    conn: &Connection,
    account: i64,
    query: &str,
    limit: u32,
    offset: u32,
) -> Result<Vec<ContactRow>> {
    let q = like_escape(&query.trim().to_lowercase());
    let mut stmt = conn.prepare(
        r#"SELECT c.id, c.account_id, c.display_name, c.sort_key,
                  (SELECT min(address) FROM contact_email e WHERE e.account_id = c.account_id AND e.contact_id = c.id)
             FROM contact c
            WHERE c.account_id = ?1 AND c.pending <> 3
              AND (?2 = '' OR lower(c.display_name) LIKE '%' || ?2 || '%' ESCAPE '\'
                   OR EXISTS (SELECT 1 FROM contact_email e WHERE e.account_id = c.account_id
                                AND e.contact_id = c.id AND e.address LIKE '%' || ?2 || '%' ESCAPE '\'))
            ORDER BY c.sort_key, c.id LIMIT ?3 OFFSET ?4"#,
    )?;
    let rows = stmt.query_map(params![account, q, limit.min(500), offset], |r| {
        Ok(ContactRow {
            id: r.get(0)?,
            account_id: r.get(1)?,
            display_name: r.get(2)?,
            sort_key: r.get(3)?,
            email: r.get(4)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// Addresses of the account's contacts whose address, or the contact's name (or a word of it),
/// starts with `prefix`, A to Z, at most `limit`: (address, name).
pub fn suggest(
    conn: &Connection,
    account: i64,
    prefix: &str,
    limit: u32,
) -> Result<Vec<(String, String)>> {
    let p = like_escape(&prefix.trim().to_lowercase());
    if p.is_empty() {
        return Ok(Vec::new());
    }
    let mut stmt = conn.prepare(
        r#"SELECT e.address, c.display_name FROM contact_email e
             JOIN contact c ON c.id = e.contact_id AND c.account_id = e.account_id
            WHERE e.account_id = ?1 AND c.pending <> 3
              AND (e.address LIKE ?2 || '%' ESCAPE '\'
                   OR lower(c.display_name) LIKE ?2 || '%' ESCAPE '\'
                   OR lower(c.display_name) LIKE '% ' || ?2 || '%' ESCAPE '\')
            ORDER BY c.sort_key, e.address LIMIT ?3"#,
    )?;
    let rows = stmt.query_map(params![account, p, limit.min(50)], |r| {
        Ok((r.get(0)?, r.get(1)?))
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// Whether `address` is one of the account's contacts.
pub fn is_contact(conn: &Connection, account: i64, address: &str) -> Result<bool> {
    Ok(conn
        .query_row(
            r#"SELECT 1 FROM contact_email e JOIN contact c ON c.id = e.contact_id AND c.account_id = e.account_id
                WHERE e.account_id = ?1 AND e.address = ?2 AND c.pending <> 3"#,
            params![account, address.trim().to_lowercase()],
            |_| Ok(()),
        )
        .optional()?
        .is_some())
}

fn like_escape(text: &str) -> String {
    text.replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::accounts::{self, NewAccount, Protocol};
    use crate::Db;

    fn account(c: &Connection, n: i64) -> i64 {
        accounts::insert(
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

    fn card(uid: &str, name: &str, email: &str) -> String {
        format!("BEGIN:VCARD\r\nVERSION:4.0\r\nUID:{uid}\r\nFN:{name}\r\nEMAIL:{email}\r\nEND:VCARD\r\n")
    }

    #[test]
    fn server_cards_are_stored_listed_searched_and_scoped_per_account() {
        let db = Db::open_in_memory().unwrap();
        let c = db.connection();
        let (a1, a2) = (account(c, 1), account(c, 2));
        let b1 = upsert_book(c, a1, "/b/", "Contacts").unwrap();
        let b2 = upsert_book(c, a2, "/b/", "Theirs").unwrap();
        assert_eq!(upsert_book(c, a1, "/b/", "Renamed").unwrap(), b1);
        store_from_server(
            c,
            a1,
            b1,
            "/b/1.vcf",
            "\"e1\"",
            &card("1", "Zoe Adams", "ZOE@x.example"),
            1,
        )
        .unwrap();
        store_from_server(
            c,
            a1,
            b1,
            "/b/2.vcf",
            "\"e2\"",
            &card("2", "Ann Brown", "ann@y.example"),
            1,
        )
        .unwrap();
        assert_eq!(
            store_from_server(c, a1, b1, "/b/3.vcf", "\"e3\"", "not a card", 1).unwrap(),
            None
        );
        assert_eq!(
            store_from_server(
                c,
                a1,
                b2,
                "/b/4.vcf",
                "\"e4\"",
                &card("4", "X", "x@z.example"),
                1
            )
            .unwrap(),
            None,
            "not that account's book"
        );
        store_from_server(
            c,
            a2,
            b2,
            "/b/1.vcf",
            "\"e9\"",
            &card("9", "Other", "zoe@x.example"),
            1,
        )
        .unwrap();

        let names: Vec<String> = list(c, a1, "", 50, 0)
            .unwrap()
            .into_iter()
            .map(|r| r.display_name)
            .collect();
        assert_eq!(
            names,
            ["Ann Brown", "Zoe Adams"],
            "A to Z (no N in these cards: by the name shown)"
        );
        assert_eq!(list(c, a1, "brown", 50, 0).unwrap().len(), 1);
        assert_eq!(
            list(c, a1, "x.example", 50, 0).unwrap()[0].email.as_deref(),
            Some("zoe@x.example")
        );
        assert_eq!(list(c, a1, "50%", 50, 0).unwrap().len(), 0);
        assert_eq!(
            suggest(c, a1, "zo", 10).unwrap(),
            [("zoe@x.example".to_owned(), "Zoe Adams".to_owned())]
        );
        assert_eq!(
            suggest(c, a1, "brow", 10).unwrap().len(),
            1,
            "a word of the name"
        );
        assert!(is_contact(c, a1, "Zoe@X.example").unwrap());
        assert!(!is_contact(c, a1, "other@x.example").unwrap());
        assert_eq!(list(c, a2, "", 50, 0).unwrap().len(), 1);

        remove_from_server(c, a1, b1, "/b/1.vcf").unwrap();
        assert_eq!(list(c, a1, "", 50, 0).unwrap().len(), 1);
        remove_books_not_in(c, a1, &[]).unwrap();
        assert!(list(c, a1, "", 50, 0).unwrap().is_empty());
        assert_eq!(
            list(c, a2, "", 50, 0).unwrap().len(),
            1,
            "the other account keeps its own"
        );
    }

    #[test]
    fn local_changes_wait_for_the_server_and_a_sync_does_not_undo_them() {
        let db = Db::open_in_memory().unwrap();
        let c = db.connection();
        let a = account(c, 1);
        let b = upsert_book(c, a, "/b/", "Contacts").unwrap();
        let new = create_local(c, a, b, &card("n", "New One", "new@x.example"), 5).unwrap();
        assert_eq!(pending(c, a).unwrap()[0].0.pending, PENDING_CREATE);
        assert!(update_local(c, a, new, &card("n", "New Two", "new@x.example"), 6).unwrap());
        assert_eq!(
            pending(c, a).unwrap()[0].0.pending,
            PENDING_CREATE,
            "still a create"
        );
        confirm(c, a, new, "/b/n.vcf").unwrap();
        assert!(pending(c, a).unwrap().is_empty());
        let got = get(c, a, new).unwrap().unwrap();
        assert_eq!(got.href.as_deref(), Some("/b/n.vcf"));

        assert!(update_local(c, a, new, &card("n", "Edited", "new@x.example"), 7).unwrap());
        // A sync that has not seen the edit leaves it alone.
        store_from_server(
            c,
            a,
            b,
            "/b/n.vcf",
            "\"e5\"",
            &card("n", "Server", "new@x.example"),
            8,
        )
        .unwrap();
        assert!(get(c, a, new).unwrap().unwrap().card.contains("FN:Edited"));
        mark_conflict(c, a, new).unwrap();
        assert!(get(c, a, new).unwrap().unwrap().conflict);
        store_from_server(
            c,
            a,
            b,
            "/b/n.vcf",
            "\"e6\"",
            &card("n", "Server", "new@x.example"),
            9,
        )
        .unwrap();
        let back = get(c, a, new).unwrap().unwrap();
        assert!(back.card.contains("FN:Server") && !back.conflict);

        assert!(delete_local(c, a, new).unwrap());
        assert!(
            get(c, a, new).unwrap().is_none(),
            "hidden while the delete waits"
        );
        assert!(list(c, a, "", 50, 0).unwrap().is_empty());
        assert_eq!(pending(c, a).unwrap()[0].0.pending, PENDING_DELETE);
        confirm(c, a, new, "/b/n.vcf").unwrap();
        assert!(pending(c, a).unwrap().is_empty());

        // A contact never sent is simply gone.
        let draft = create_local(c, a, b, &card("d", "Draft", "d@x.example"), 10).unwrap();
        assert!(delete_local(c, a, draft).unwrap());
        assert!(pending(c, a).unwrap().is_empty());
        assert!(!update_local(c, a + 1, new, &card("n", "X", "x@x.example"), 11).unwrap());
    }
}
