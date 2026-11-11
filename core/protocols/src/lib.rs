// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Protocol adapters. The UI and the sync engine only ever see [`MailBackend`]; which protocol an
//! account uses (IMAP now, JMAP next) is decided here and nowhere else.
//!
//! This crate holds the shared vocabulary ([`types`]), the trait ([`backend`]) and the IMAP
//! adapter ([`imap`]) and the SMTP sender ([`smtp`]). JMAP and DAV arrive in later tasks.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

pub mod backend;
pub mod imap;
pub mod smtp;
pub mod types;

pub use backend::{BackendFuture, MailBackend};
pub use imap::{probe_certificate, CertFingerprint, Credential, ImapBackend, ImapConfig, Security};
pub use smtp::{
    probe_certificate as probe_smtp_certificate, SendReceipt, SmtpConfig, SmtpSecurity, SmtpSender,
};
pub use types::*;
