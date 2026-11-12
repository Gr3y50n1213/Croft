// This Source Code Form is subject to the terms of the GNU Affero General Public License,
// version 3 or later. See LICENSE-AGPL-3.0 at the repository root.

//! The Croft Companion (phase 2): a self-hosted, multi-user server that will serve the web app,
//! push and server-side search. T23 is its foundation: settings, per-user envelope encryption under
//! a master key kept outside the database, storage, the command line and the HTTP service. T24 adds
//! sign-in: passkeys, TOTP with recovery codes, tokens, rate limits and the admin's user management.

pub mod api;
pub mod auth;
pub mod ceremony;
pub mod cli;
pub mod config;
pub mod crypto;
pub mod enrol_page;
pub mod http;
pub mod jwt;
pub mod limits;
pub mod store;
pub mod totp;
