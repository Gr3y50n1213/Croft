// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Protocol adapters. The UI and the sync engine only ever see [`MailBackend`]; which protocol an
//! account uses (IMAP now, JMAP next) is decided here and nowhere else.
//!
//! This crate holds the shared vocabulary ([`types`]), the trait ([`backend`]) and the IMAP
//! adapter ([`imap`]), the JMAP adapter ([`jmap`]) and the SMTP sender ([`smtp`]). DAV arrives in a later task.
//! [`demo`] is the built-in sample mailbox, which never touches a network.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

pub mod backend;
pub mod caldav;
pub mod carddav;
pub mod companion;
pub mod dav;
pub mod demo;
pub mod discovery;
pub mod drafts;
mod host;
pub mod imap;
pub mod jmap;
pub mod lines;
pub mod netpolicy;
pub mod oauth;
pub mod smtp;
pub mod submit;
pub mod types;

pub use backend::{BackendFuture, MailBackend};
pub use demo::{DemoBackend, DemoFolderState, DemoSubmitter};
pub use host::clean_host;
pub use imap::{
    check_certificate, probe_certificate, CertFingerprint, CertificateProblem, Credential,
    ImapBackend, ImapConfig, Security,
};
pub use jmap::{
    check_certificate as check_jmap_certificate, probe_certificate as probe_jmap_certificate,
    IdStoreError, JmapBackend, JmapConfig, JmapIdStore, JmapSubmitter, MemoryIdStore,
};
pub use smtp::{
    probe_certificate as probe_smtp_certificate, SendReceipt, SmtpConfig, SmtpSecurity, SmtpSender,
};
pub use submit::{MailSubmitter, SmtpImapSubmitter};
pub use types::*;
