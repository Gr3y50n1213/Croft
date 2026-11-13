// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! CalDAV (RFC 4791; T39a): an account's calendars, through the shared DAV client
//! ([`crate::dav`]): `/.well-known/caldav`, the `calendar-home-set`, the calendars that hold
//! events (with their colour and order), `sync-collection`, `calendar-multiget`, and conditional
//! writes of `text/calendar` objects.

use crate::dav::{Dav, DavKind};
use crate::imap::Credential;
use crate::types::BackendError;

pub use crate::dav::{
    Changes, Collection as CalendarCollection, DavConfig as CalDavConfig, Fetched as FetchedObject,
    ItemRef as ObjectRef, WriteOutcome, MULTIGET_BATCH,
};

/// An account's CalDAV client.
pub struct CalDav(Dav);

impl CalDav {
    /// A client for `config` with `credential`.
    pub fn new(config: &CalDavConfig, credential: &Credential) -> Result<Self, BackendError> {
        Dav::new(DavKind::Cal, config, credential).map(CalDav)
    }

    /// Finds the calendars that hold events (an empty list when the server offers none).
    pub async fn discover(&self) -> Result<Vec<CalendarCollection>, BackendError> {
        self.0.discover().await
    }

    /// What changed in `calendar` since `token`; a full listing without one.
    pub async fn changes(
        &self,
        calendar: &str,
        token: Option<&str>,
    ) -> Result<Changes, BackendError> {
        self.0.changes(calendar, token).await
    }

    /// Fetches the objects at `paths` in `calendar`, at most [`MULTIGET_BATCH`].
    pub async fn fetch(
        &self,
        calendar: &str,
        paths: &[String],
    ) -> Result<Vec<FetchedObject>, BackendError> {
        self.0.fetch(calendar, paths).await
    }

    /// Writes the iCalendar `object` at `path` (an update with `etag`, else new).
    pub async fn put(
        &self,
        path: &str,
        object: &str,
        etag: Option<&str>,
    ) -> Result<WriteOutcome, BackendError> {
        self.0.put(path, object, etag).await
    }

    /// Deletes the object at `path` if it still has `etag`.
    pub async fn delete(&self, path: &str, etag: &str) -> Result<WriteOutcome, BackendError> {
        self.0.delete(path, etag).await
    }
}
