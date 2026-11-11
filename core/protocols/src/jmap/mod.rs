// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The JMAP implementation of [`MailBackend`](crate::MailBackend) (RFC 8620 core, RFC 8621 mail),
//! so a Stalwart or Fastmail account behaves like an IMAP one. Nothing outside `croft-protocols`
//! learns which protocol an account uses.
//!
//! - [`JmapBackend`] is the mail backend, [`JmapSubmitter`] sends mail (the JMAP counterpart of
//!   `SmtpSender`), [`JmapConfig`] holds the settings, [`probe_certificate`] reads a server's
//!   certificate fingerprint for account setup, and [`JmapIdStore`] is the persistent id map the
//!   adapter needs (with [`MemoryIdStore`] for tests; `croft-storage` has the SQLite one).
//!
//! # Identity: JMAP ids become UIDs
//!
//! The trait names a message by (folder, `u32` UID, folder UIDVALIDITY). JMAP ids are strings and
//! stay the same when a message moves; a message can also be in several mailboxes. The adapter
//! keeps a map per account (see [`JmapIdStore`]): for each (mailbox id, email id) a UID that increases per
//! mailbox and is never reused, and for each mailbox a random UIDVALIDITY. A move gives the message
//! a new UID in the destination (reported in `MoveOutcome::dest_uids`) and removes its entry in the
//! source, exactly like IMAP's `COPYUID`. If the map for a mailbox is lost or inconsistent the
//! mailbox gets a new UIDVALIDITY and the engine starts the folder over. The engine only ever sees
//! UIDs.
//!
//! # Mapping
//!
//! | Trait | JMAP |
//! | --- | --- |
//! | `list_folders` | `Mailbox/get`; the path is the names from the top joined with `/`; roles become `SpecialUse` (a mailbox with no role is guessed from its name, like IMAP); a mailbox the person may not read is not selectable |
//! | `folder_status` | UIDVALIDITY and next UID from the map, `exists` = `totalEmails`, `highest_modseq` = a stable number derived from the account's `Email` state, so the engine's "has anything changed" check works |
//! | `snapshot` | `Email/query` (oldest first, so new UIDs follow arrival order) and `Email/get` keywords; later calls in the same process use `Email/changes` and fetch only what changed, and fall back to the full read on `cannotCalculateChanges` (or a state the server no longer knows) |
//! | `fetch_headers` | `Email/get` with the header properties, newest first |
//! | `find_by_message_id` | `Email/query` with the `header` filter, checked against the real Message-ID; if the server does not index that header, a scan of the newest 2000 messages |
//! | `set_flag` | `Email/set` keyword patch (`$seen`, `$flagged`) |
//! | `move_to` | `Email/set` patch of `mailboxIds`, only for messages still in the source mailbox |
//! | `delete_forever` | `Email/set` destroy; a message that is in other mailboxes too is only taken out of this one; `notFound` counts as done |
//! | `capabilities` | move and UIDPLUS-style new UIDs: yes; IDLE: not yet; `condstore`: yes, in the sense of the `highest_modseq` above |
//!
//! `Email/changes` is used within a running adapter. The flags it works from are kept in memory, so
//! after a restart the first snapshot of each folder is a full read; the persisted `Email` state
//! string in the id map records how far the map was reconciled.
//!
//! Spam does not set `$junk` or `$notjunk` yet: the trait has no flag change for them (neither
//! does the IMAP adapter); a move into the Junk mailbox is only a move.
//!
//! # Safety rules
//!
//! - TLS 1.2 and newer, certificate validation always on; a self-signed server is accepted only by
//!   the exact fingerprint ([`JmapConfig::pinned_cert`]). Plain `http` only for `localhost`.
//! - The credential (Basic with the password, Bearer with an OAuth token) goes only to the origin
//!   of the session URL. The `apiUrl` and `uploadUrl` the server names are used for their path
//!   only, and redirects to another origin are refused.
//! - Errors are fixed strings; no token, address, server text or message content reaches one. See
//!   `error.rs` for the classes.
//! - Responses are bounded in size and time; a hostile server cannot make the adapter read forever.
//!
//! # Why our own HTTP client
//!
//! The `jmap-client` crate (0.4.3, Stalwart's) was checked first. It builds on `reqwest` and
//! offers one TLS switch, `danger_accept_invalid_certs`: no custom verifier, so no pinning by
//! fingerprint, and the crate chooses its own TLS provider (aws-lc-rs by default) and follows
//! redirects. Croft's rule is the same policy as IMAP and SMTP, so the adapter has a small HTTP/1.1
//! client on `tokio-rustls` (see `http.rs`) and reads answers with `serde_json`.

mod api;
mod backend;
mod config;
mod error;
mod http;
mod idmap;
mod mapping;
mod probe;
mod session;
mod submit;
mod tls;
mod url;

pub use backend::JmapBackend;
pub use config::{JmapConfig, DEFAULT_DEADLINE, DEFAULT_MAX_RESPONSE_BYTES};
pub use idmap::{fresh_uid_validity, IdStoreError, JmapIdStore, MemoryIdStore};
pub use probe::probe_certificate;
pub use submit::JmapSubmitter;
