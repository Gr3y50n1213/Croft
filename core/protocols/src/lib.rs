//! Protocol adapters. The UI and the sync engine only ever see [`MailBackend`]; which protocol an
//! account uses (IMAP now, JMAP next) is decided here and nowhere else.
//!
//! The adapters themselves (`async-imap`, `jmap-client`, SMTP, DAV) arrive once crates.io is
//! reachable. This crate currently holds the shared vocabulary and the trait they implement.

pub mod backend;
pub mod types;

pub use backend::{BackendFuture, MailBackend};
pub use types::*;
