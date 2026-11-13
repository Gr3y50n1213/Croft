// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! CardDAV (RFC 6352; T38a): an account's address books, through the shared DAV client
//! ([`crate::dav`]).

use crate::dav::{Dav, DavKind};
use crate::imap::Credential;
use crate::types::BackendError;

pub use crate::dav::{
    Changes, Collection as AddressBook, DavConfig as CardDavConfig, Fetched as FetchedCard,
    ItemRef as CardRef, WriteOutcome, Written, MULTIGET_BATCH,
};

/// An account's CardDAV client.
pub struct CardDav(Dav);

impl CardDav {
    /// A client for `config` with `credential`.
    pub fn new(config: &CardDavConfig, credential: &Credential) -> Result<Self, BackendError> {
        Dav::new(DavKind::Card, config, credential).map(CardDav)
    }

    /// Finds the address books (an empty list when the server offers none).
    pub async fn discover(&self) -> Result<Vec<AddressBook>, BackendError> {
        self.0.discover().await
    }

    /// What changed in `book` since `token`; a full listing without one.
    pub async fn changes(&self, book: &str, token: Option<&str>) -> Result<Changes, BackendError> {
        self.0.changes(book, token).await
    }

    /// Fetches the cards at `paths` in `book`, at most [`MULTIGET_BATCH`].
    pub async fn fetch(
        &self,
        book: &str,
        paths: &[String],
    ) -> Result<Vec<FetchedCard>, BackendError> {
        self.0.fetch(book, paths).await
    }

    /// Writes `card` at `path` (an update with `etag`, else new).
    pub async fn put(
        &self,
        path: &str,
        card: &str,
        etag: Option<&str>,
    ) -> Result<WriteOutcome, BackendError> {
        self.0.put(path, card, etag).await
    }

    /// Writes `card` at `path` like `put`, with the etag of the version written when the server gave one.
    pub async fn put_tagged(
        &self,
        path: &str,
        card: &str,
        etag: Option<&str>,
    ) -> Result<Written, BackendError> {
        self.0.put_tagged(path, card, etag).await
    }

    /// Deletes the card at `path` if it still has `etag`.
    pub async fn delete(&self, path: &str, etag: &str) -> Result<WriteOutcome, BackendError> {
        self.0.delete(path, etag).await
    }
}
