// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Contacts (T38a): each account's CardDAV address books, kept on the device and synced both ways.
//! The CardDAV service is looked for on the account's own mail host (its `/.well-known/carddav`),
//! with the account's own credential and certificate pin, so a contact sync never sends the
//! sign-in anywhere the mail does not already go. Local edits are stored at once and sent at the
//! next sync, conditional on the card's etag; a card the server changed meanwhile keeps the
//! server's version and is marked as a conflict. OAuth accounts (Google, Microsoft) and the sample
//! mailbox have no contacts sync yet.

use std::time::Duration;

use croft_mail_core::vcard::{self, Card, Contact, Name, PostalAddress, Typed};
use croft_protocols::carddav::{CardDav, CardDavConfig, WriteOutcome};
use croft_protocols::{BackendError, Credential};
use croft_storage::{accounts, contacts as store, recipients};

use crate::core::{now_ms, Core};
use crate::credentials::{ask, Asked};
use crate::error::CoreError;
use crate::settings::{is_demo, url_host, AccountSettings};

/// The most contacts one list call returns.
const MAX_LIST: u32 = 500;

/// A phone or email with its kinds.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ContactValue {
    /// The address or number.
    pub value: String,
    /// Kinds, lower case (`home`, `work`, `cell`, ...).
    pub kinds: Vec<String>,
    /// Preferred.
    pub preferred: bool,
}

/// A postal address.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ContactAddress {
    /// Street.
    pub street: String,
    /// City.
    pub locality: String,
    /// Region.
    pub region: String,
    /// Postal code.
    pub postal_code: String,
    /// Country.
    pub country: String,
    /// Kinds, lower case.
    pub kinds: Vec<String>,
}

/// What S41 shows and S42 edits.
#[derive(Clone, PartialEq, Eq, uniffi::Record)]
pub struct ContactFields {
    /// The name as shown (`FN`); empty to build it from the parts.
    pub formatted_name: String,
    /// Given name.
    pub given_name: String,
    /// Family name.
    pub family_name: String,
    /// Emails.
    pub emails: Vec<ContactValue>,
    /// Phones.
    pub phones: Vec<ContactValue>,
    /// Postal addresses.
    pub addresses: Vec<ContactAddress>,
    /// Organisation.
    pub organization: String,
    /// Notes.
    pub note: String,
    /// Birthday as written (`1990-04-01`, `--04-01`).
    pub birthday: Option<String>,
}

impl std::fmt::Debug for ContactFields {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ContactFields")
            .field("emails", &self.emails.len())
            .finish_non_exhaustive()
    }
}

/// A contact in the list (S40).
#[derive(Clone, PartialEq, Eq, uniffi::Record)]
pub struct ContactSummary {
    /// Id.
    pub id: i64,
    /// Account.
    pub account: i64,
    /// Name shown.
    pub display_name: String,
    /// A to Z key (lower case).
    pub sort_key: String,
    /// Its first address.
    pub email: Option<String>,
}

impl std::fmt::Debug for ContactSummary {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ContactSummary")
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}

/// One contact (S41).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ContactDetail {
    /// Id.
    pub id: i64,
    /// Account.
    pub account: i64,
    /// Address book id.
    pub book: i64,
    /// Name shown.
    pub display_name: String,
    /// The fields.
    pub fields: ContactFields,
    /// The card carries a key (`KEY`).
    pub has_key: bool,
    /// A change made here was not taken: the server had changed the contact, its version is shown.
    pub conflict: bool,
    /// A change made here waits to be sent.
    pub pending: bool,
}

/// An address book.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct AddressBookInfo {
    /// Id.
    pub id: i64,
    /// Name.
    pub name: String,
}

/// One autocomplete suggestion.
#[derive(Clone, PartialEq, Eq, uniffi::Record)]
pub struct AddressSuggestion {
    /// The address.
    pub address: String,
    /// The name, when known.
    pub name: Option<String>,
    /// From the contacts (else a recent recipient).
    pub is_contact: bool,
}

impl std::fmt::Debug for AddressSuggestion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AddressSuggestion")
            .field("is_contact", &self.is_contact)
            .finish_non_exhaustive()
    }
}

/// How a contacts sync went. Fixed categories.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum ContactsSyncResult {
    /// Synced.
    Done,
    /// The server offers no CardDAV (or this account has no contacts sync): nothing to do.
    NotOffered,
    /// Offline or the server did not answer: tried again later.
    Unreachable,
    /// The server refused the sign-in.
    SignInNeeded,
}

/// What a sync did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Record)]
pub struct ContactsSyncOutcome {
    /// How it went.
    pub result: ContactsSyncResult,
    /// Changes made here that the server had overtaken (the server's versions were kept).
    pub conflicts: u32,
}

fn fields_of(c: &Contact) -> ContactFields {
    let value = |t: &Typed| ContactValue {
        value: t.value.clone(),
        kinds: t.kinds.clone(),
        preferred: t.preferred,
    };
    ContactFields {
        formatted_name: c.formatted_name.clone(),
        given_name: c.name.given.clone(),
        family_name: c.name.family.clone(),
        emails: c.emails.iter().map(value).collect(),
        phones: c.phones.iter().map(value).collect(),
        addresses: c
            .addresses
            .iter()
            .map(|a| ContactAddress {
                street: a.street.clone(),
                locality: a.locality.clone(),
                region: a.region.clone(),
                postal_code: a.postal_code.clone(),
                country: a.country.clone(),
                kinds: a.kinds.clone(),
            })
            .collect(),
        organization: c.organization.clone(),
        note: c.note.clone(),
        birthday: c.birthday.clone(),
    }
}

/// `fields` written over `base` (the parts S42 does not show stay as they were).
fn contact_of(fields: &ContactFields, base: &Contact) -> Contact {
    let typed = |v: &ContactValue| Typed {
        value: v.value.trim().to_owned(),
        kinds: v
            .kinds
            .iter()
            .map(|k| k.trim().to_lowercase())
            .filter(|k| !k.is_empty())
            .collect(),
        preferred: v.preferred,
    };
    let mut c = base.clone();
    c.formatted_name = fields.formatted_name.trim().to_owned();
    c.name = Name {
        given: fields.given_name.trim().to_owned(),
        family: fields.family_name.trim().to_owned(),
        ..base.name.clone()
    };
    c.emails = fields
        .emails
        .iter()
        .map(typed)
        .filter(|t| !t.value.is_empty())
        .collect();
    c.phones = fields
        .phones
        .iter()
        .map(typed)
        .filter(|t| !t.value.is_empty())
        .collect();
    c.addresses = fields
        .addresses
        .iter()
        .map(|a| PostalAddress {
            street: a.street.trim().to_owned(),
            locality: a.locality.trim().to_owned(),
            region: a.region.trim().to_owned(),
            postal_code: a.postal_code.trim().to_owned(),
            country: a.country.trim().to_owned(),
            kinds: a.kinds.clone(),
        })
        .collect();
    c.organization = fields.organization.trim().to_owned();
    c.note = fields.note.clone();
    c.birthday = fields.birthday.clone().filter(|b| !b.trim().is_empty());
    c
}

/// A new card's id: random, so it never says anything about the person.
fn new_uid() -> Result<String, CoreError> {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).map_err(|_| CoreError::Internal)?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

impl Core {
    /// The account's CardDAV client, or `None` when it has no contacts sync (the sample mailbox,
    /// an OAuth account, unreadable settings). Errors only for "ask again later".
    async fn carddav(&self, account: i64) -> Result<Option<CardDav>, CoreError> {
        let row = self
            .inner
            .handle
            .read(|db| accounts::get(db.connection(), account))?
            .ok_or(CoreError::NoSuchAccount)?;
        if is_demo(&row.config, &row.email) {
            return Ok(None);
        }
        let Ok(settings) = AccountSettings::from_json(&row.config) else {
            return Ok(None);
        };
        let pin = settings.applied_pin();
        let (origin, username) = match &settings {
            AccountSettings::Imap(s) => {
                let host = s.imap.host.trim().to_owned();
                let origin = if host.contains(':') {
                    format!("https://[{host}]")
                } else {
                    format!("https://{host}")
                };
                (origin, s.imap.username.clone())
            }
            AccountSettings::Jmap(s) => {
                let host = url_host(&s.jmap.session_url);
                let origin = s
                    .jmap
                    .session_url
                    .find("://")
                    .and_then(|i| s.jmap.session_url[i + 3..].find('/').map(|j| i + 3 + j))
                    .map_or_else(
                        || s.jmap.session_url.clone(),
                        |end| s.jmap.session_url[..end].to_owned(),
                    );
                if host.is_empty() {
                    return Ok(None);
                }
                (origin, s.jmap.username.clone())
            }
        };
        let origin = self
            .inner
            .carddav_urls
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(&account)
            .cloned()
            .unwrap_or(origin);
        let provider = self.inner.provider.clone();
        // The app's callback may take a moment: not on a runtime worker (this runs on the core's
        // runtime, from `sync_contacts`).
        let asked = tokio::task::spawn_blocking(move || ask(&*provider, account))
            .await
            .unwrap_or(Asked::NotReady);
        let credential: Credential = match asked {
            Asked::Credential(c) => c,
            // OAuth providers' contacts need their own scopes (OPEN-ITEMS).
            Asked::OAuth(_) => return Ok(None),
            Asked::NoCredential => return Err(CoreError::NoCredential),
            Asked::NotReady => return Err(CoreError::Internal),
        };
        let config = CardDavConfig {
            base_url: origin,
            username,
            pinned_cert: pin,
            timeout: Duration::from_secs(20),
            deadline: Duration::from_secs(60),
        };
        Ok(CardDav::new(&config, &credential).ok())
    }

    /// Sends the account's waiting changes, then brings in the server's (see the module docs).
    async fn sync_contacts_now(&self, account: i64) -> Result<ContactsSyncOutcome, CoreError> {
        let outcome = |result, conflicts| ContactsSyncOutcome { result, conflicts };
        let dav = match self.carddav(account).await {
            Ok(Some(d)) => d,
            Ok(None) => return Ok(outcome(ContactsSyncResult::NotOffered, 0)),
            Err(CoreError::NoCredential) => {
                return Ok(outcome(ContactsSyncResult::SignInNeeded, 0))
            }
            Err(e) => return Err(e),
        };
        let net = |e: BackendError| match e {
            BackendError::Auth(_) => ContactsSyncResult::SignInNeeded,
            BackendError::Transient(_) => ContactsSyncResult::Unreachable,
            BackendError::Permanent(_) => ContactsSyncResult::NotOffered,
        };
        let books = match dav.discover().await {
            Ok(b) => b,
            Err(e) => return Ok(outcome(net(e), 0)),
        };
        if books.is_empty() {
            return Ok(outcome(ContactsSyncResult::NotOffered, 0));
        }
        let keep: Vec<String> = books.iter().map(|b| b.path.clone()).collect();
        self.inner.handle.read(|db| {
            let c = db.connection();
            for b in &books {
                store::upsert_book(c, account, &b.path, &b.name)?;
            }
            store::remove_books_not_in(c, account, &keep)
        })?;

        // Out: what was changed here.
        let mut conflicts = 0u32;
        let waiting = self
            .inner
            .handle
            .read(|db| store::pending(db.connection(), account))?;
        for (contact, book_path) in waiting {
            let result = match contact.pending {
                store::PENDING_CREATE => {
                    let uid = vcard::parse(&contact.card)
                        .ok()
                        .and_then(|c| c.contact().uid)
                        .filter(|u| u.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-'))
                        .map_or_else(new_uid, Ok)?;
                    let href = format!("{}/{uid}.vcf", book_path.trim_end_matches('/'));
                    dav.put(&href, &contact.card, None).await.map(|o| (o, href))
                }
                store::PENDING_UPDATE => match (&contact.href, &contact.etag) {
                    (Some(href), etag) => {
                        let etag = match etag {
                            Some(e) => Some(e.clone()),
                            // Created here and not listed since: its current etag first.
                            None => dav
                                .changes(&book_path, None)
                                .await
                                .ok()
                                .and_then(|ch| ch.changed.into_iter().find(|r| &r.path == href))
                                .map(|r| r.etag),
                        };
                        match etag {
                            Some(e) => dav
                                .put(href, &contact.card, Some(&e))
                                .await
                                .map(|o| (o, href.clone())),
                            None => Ok((WriteOutcome::Gone, href.clone())),
                        }
                    }
                    (None, _) => continue,
                },
                store::PENDING_DELETE => match (&contact.href, &contact.etag) {
                    (Some(href), Some(etag)) => {
                        dav.delete(href, etag).await.map(|o| (o, href.clone()))
                    }
                    (Some(href), None) => Ok((WriteOutcome::Done, href.clone())),
                    (None, _) => Ok((WriteOutcome::Done, String::new())),
                },
                _ => continue,
            };
            match result {
                Ok((WriteOutcome::Done, href)) => {
                    self.inner
                        .handle
                        .read(|db| store::confirm(db.connection(), account, contact.id, &href))?;
                }
                Ok((WriteOutcome::Conflict, _)) => {
                    conflicts += 1;
                    self.inner
                        .handle
                        .read(|db| store::mark_conflict(db.connection(), account, contact.id))?;
                }
                Ok((WriteOutcome::Gone, href)) => {
                    // Removed on the server: a delete is done, an update is dropped with it.
                    self.inner.handle.read(|db| {
                        store::confirm(db.connection(), account, contact.id, &href)?;
                        db.connection().execute(
                            "DELETE FROM contact WHERE account_id = ?1 AND id = ?2",
                            rusqlite::params![account, contact.id],
                        )?;
                        Ok::<_, croft_storage::StorageError>(())
                    })?;
                }
                Err(e) => return Ok(outcome(net(e), conflicts)),
            }
        }

        // In: what changed on the server.
        let stored_books = self
            .inner
            .handle
            .read(|db| store::books(db.connection(), account))?;
        for book in stored_books {
            let changes = match dav.changes(&book.path, book.sync_token.as_deref()).await {
                Ok(c) => c,
                Err(e) => return Ok(outcome(net(e), conflicts)),
            };
            let known: std::collections::HashMap<String, String> = self
                .inner
                .handle
                .read(|db| store::server_cards(db.connection(), account, book.id))?
                .into_iter()
                .collect();
            let wanted: Vec<String> = changes
                .changed
                .iter()
                .filter(|r| known.get(&r.path) != Some(&r.etag))
                .map(|r| r.path.clone())
                .collect();
            let fetched = match dav.fetch(&book.path, &wanted).await {
                Ok(f) => f,
                Err(e) => return Ok(outcome(net(e), conflicts)),
            };
            let now = now_ms();
            self.inner.handle.read(|db| {
                let c = db.connection();
                for card in &fetched {
                    store::store_from_server(
                        c, account, book.id, &card.path, &card.etag, &card.text, now,
                    )?;
                }
                for gone in &changes.removed {
                    store::remove_from_server(c, account, book.id, gone)?;
                }
                if changes.full {
                    let listed: std::collections::HashSet<&str> =
                        changes.changed.iter().map(|r| r.path.as_str()).collect();
                    for (href, _) in known.iter().filter(|(h, _)| !listed.contains(h.as_str())) {
                        store::remove_from_server(c, account, book.id, href)?;
                    }
                }
                store::set_book_sync(
                    c,
                    account,
                    book.id,
                    changes.sync_token.as_deref(),
                    None,
                    now,
                )
            })?;
        }
        Ok(outcome(ContactsSyncResult::Done, conflicts))
    }
}

#[uniffi::export]
impl Core {
    /// Syncs the contacts of `account` now: sends the changes made here, then brings in the
    /// server's. One at a time; the screens are told through the usual change notices.
    pub async fn sync_contacts(
        self: std::sync::Arc<Self>,
        account: i64,
    ) -> Result<ContactsSyncOutcome, CoreError> {
        let core = self.clone();
        self.spawn(
            async move {
                let _one = core.inner.contact_syncs.lock().await;
                core.sync_contacts_now(account).await
            },
            false,
        )
        .await?
    }

    /// The address books of `account`.
    pub fn address_books(&self, account: i64) -> Result<Vec<AddressBookInfo>, CoreError> {
        Ok(self
            .inner
            .handle
            .read(|db| store::books(db.connection(), account))?
            .into_iter()
            .map(|b| AddressBookInfo {
                id: b.id,
                name: b.name,
            })
            .collect())
    }

    /// Contacts of `account` (or of every account), A to Z, whose name or address contains
    /// `query` (all for an empty one): at most `limit` (500 at most) after `offset`.
    pub fn list_contacts(
        &self,
        account: Option<i64>,
        query: String,
        limit: u32,
        offset: u32,
    ) -> Result<Vec<ContactSummary>, CoreError> {
        let limit = limit.min(MAX_LIST);
        self.inner.handle.read(|db| {
            let c = db.connection();
            let ids: Vec<i64> = match account {
                Some(a) => vec![a],
                None => accounts::list(c)?.into_iter().map(|a| a.id).collect(),
            };
            let mut out = Vec::new();
            for a in ids {
                out.extend(store::list(c, a, &query, limit.saturating_add(offset), 0)?);
            }
            out.sort_by(|x, y| x.sort_key.cmp(&y.sort_key).then(x.id.cmp(&y.id)));
            Ok::<_, CoreError>(
                out.into_iter()
                    .skip(offset as usize)
                    .take(limit as usize)
                    .map(|r| ContactSummary {
                        id: r.id,
                        account: r.account_id,
                        display_name: r.display_name,
                        sort_key: r.sort_key,
                        email: r.email,
                    })
                    .collect(),
            )
        })
    }

    /// One contact of `account`, or `None`.
    pub fn get_contact(&self, account: i64, id: i64) -> Result<Option<ContactDetail>, CoreError> {
        let Some(s) = self
            .inner
            .handle
            .read(|db| store::get(db.connection(), account, id))?
        else {
            return Ok(None);
        };
        let contact = vcard::parse(&s.card)
            .map(|c| c.contact())
            .unwrap_or_default();
        Ok(Some(ContactDetail {
            id: s.id,
            account: s.account_id,
            book: s.book_id,
            display_name: contact.display_name(),
            fields: fields_of(&contact),
            has_key: contact.has_key,
            conflict: s.conflict,
            pending: s.pending != store::PENDING_NONE,
        }))
    }

    /// Saves S42: a new contact in `book` (when `id` is `None`), or changes contact `id`. Stored
    /// at once, sent at the next sync. Returns the contact's id.
    pub fn save_contact(
        &self,
        account: i64,
        id: Option<i64>,
        book: i64,
        fields: ContactFields,
    ) -> Result<i64, CoreError> {
        let now = now_ms();
        match id {
            None => {
                let contact = contact_of(&fields, &Contact::default());
                let card = Card::new(&contact, &new_uid()?).to_text();
                self.inner
                    .handle
                    .read(|db| store::create_local(db.connection(), account, book, &card, now))
                    .map_err(|_| CoreError::NoSuchAccount)
            }
            Some(id) => {
                let stored = self
                    .inner
                    .handle
                    .read(|db| store::get(db.connection(), account, id))?
                    .ok_or(CoreError::NoSuchMessage)?;
                let mut card = vcard::parse(&stored.card).map_err(|_| CoreError::Internal)?;
                let edited = contact_of(&fields, &card.contact());
                card.apply(&edited);
                let text = card.to_text();
                let done = self
                    .inner
                    .handle
                    .read(|db| store::update_local(db.connection(), account, id, &text, now))?;
                if done {
                    Ok(id)
                } else {
                    Err(CoreError::NoSuchMessage)
                }
            }
        }
    }

    /// Deletes contact `id` of `account` (sent at the next sync).
    pub fn delete_contact(&self, account: i64, id: i64) -> Result<bool, CoreError> {
        Ok(self
            .inner
            .handle
            .read(|db| store::delete_local(db.connection(), account, id))?)
    }

    /// Compose's suggestions for `prefix`: the contacts first, then recent recipients not among
    /// them, at most `limit` (50 at most). `account` limits them to one account.
    pub fn autocomplete(
        &self,
        account: Option<i64>,
        prefix: String,
        limit: u32,
    ) -> Result<Vec<AddressSuggestion>, CoreError> {
        let limit = limit.min(50) as usize;
        self.inner.handle.read(|db| {
            let c = db.connection();
            let ids: Vec<i64> = match account {
                Some(a) => vec![a],
                None => accounts::list(c)?.into_iter().map(|a| a.id).collect(),
            };
            let mut out: Vec<AddressSuggestion> = Vec::new();
            let mut seen = std::collections::HashSet::new();
            for &a in &ids {
                for (address, name) in store::suggest(c, a, &prefix, limit as u32)? {
                    if seen.insert(address.to_lowercase()) {
                        out.push(AddressSuggestion {
                            address,
                            name: Some(name).filter(|n| !n.is_empty()),
                            is_contact: true,
                        });
                    }
                }
            }
            for &a in &ids {
                for s in recipients::suggest(c, a, &prefix, limit as u32)? {
                    if seen.insert(s.address.to_lowercase()) {
                        out.push(AddressSuggestion {
                            address: s.address,
                            name: s.name,
                            is_contact: false,
                        });
                    }
                }
            }
            out.truncate(limit);
            Ok::<_, CoreError>(out)
        })
    }

    /// Whether `address` is one of `account`'s contacts.
    pub fn is_contact(&self, account: i64, address: String) -> Result<bool, CoreError> {
        Ok(self
            .inner
            .handle
            .read(|db| store::is_contact(db.connection(), account, &address))?)
    }
}

impl Core {
    /// Tests: look for CardDAV at `url` for `account` instead of the mail host.
    #[cfg(test)]
    pub(crate) fn set_carddav_url_for_tests(&self, account: i64, url: &str) {
        self.inner
            .carddav_urls
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(account, url.to_owned());
    }
}
