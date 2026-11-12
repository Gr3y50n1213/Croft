// This Source Code Form is subject to the terms of the GNU Affero General Public License,
// version 3 or later. See LICENSE-AGPL-3.0 at the repository root.

//! The Croft push gateway (T29): a stateless relay from Web Push (what every Companion sends, T28)
//! to Google's FCM (what the Play build of the app can receive). A Companion cannot send through
//! FCM to the official app; only its publisher can. The gateway forwards the Companion's message,
//! already encrypted for the device, without opening it: it learns that some device is woken at
//! some time, never anything about mail or people (`docs/PLAN.md`, "Review decisions").

pub mod config;
pub mod fcm;
pub mod limits;
pub mod seal;
pub mod server;
pub mod vapid;

#[cfg(test)]
mod tests;

#[cfg(test)]
mod lock_tests {
    #[test]
    fn the_core_patch_is_in_force() {
        let lock = include_str!("../Cargo.lock");
        let entries: Vec<&str> = lock
            .split("[[package]]")
            .filter(|p| p.contains("name = \"async-imap\""))
            .collect();
        assert!(!entries.is_empty(), "async-imap is not in the lock file");
        for entry in entries {
            assert!(!entry.contains("source ="), "a stock async-imap is built");
        }
    }
}
