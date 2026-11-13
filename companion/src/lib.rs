// This Source Code Form is subject to the terms of the GNU Affero General Public License,
// version 3 or later. See LICENSE-AGPL-3.0 at the repository root.

//! The Croft Companion (phase 2): a self-hosted, multi-user server that will serve the web app,
//! push and server-side search. T23 is its foundation: settings, per-user envelope encryption under
//! a master key kept outside the database, storage, the command line and the HTTP service. T24 adds
//! sign-in: passkeys, TOTP with recovery codes, tokens, rate limits and the admin's user management.

pub mod api;
pub mod app_signin;
pub mod auth;
pub mod ceremony;
pub mod cli;
pub mod config;
pub mod crypto;
pub mod enrol_page;
pub mod http;
pub mod jwt;
pub mod limits;
pub mod mail;
pub mod mail_store;
pub mod oidc;
pub mod oidc_store;
pub mod push;
pub mod push_store;
pub mod store;
pub mod totp;
pub mod webapp;
pub mod webpush;

#[cfg(test)]
mod tests {
    /// The core's patched crates must be the ones built (the stock async-imap logs protocol lines,
    /// which can hold passwords; html5ever and ammonia carry the reader's work budget): a patched
    /// crate has no `source` in the lock file.
    #[test]
    fn the_core_patches_are_in_force() {
        let lock = include_str!("../Cargo.lock");
        for name in ["async-imap", "html5ever", "ammonia"] {
            let entries: Vec<&str> = lock
                .split("[[package]]")
                .filter(|p| p.contains(&format!("name = \"{name}\"")))
                .collect();
            assert!(!entries.is_empty(), "{name} is not in the lock file");
            for entry in entries {
                assert!(!entry.contains("source ="), "a stock {name} is built");
            }
        }
    }
}
