// This Source Code Form is subject to the terms of the GNU Affero General Public License,
// version 3 or later. See LICENSE-AGPL-3.0 at the repository root.

//! Signing a phone in (T34a). A phone signs in through the browser (RFC 8252): the Croft app opens
//! the Companion's `/app/sign-in` page in a Custom Tab with a PKCE challenge and its redirect, the
//! person signs in there, and the Companion sends the browser to the app's redirect with a one-time
//! code; the app redeems it with its verifier for its own tokens. The redirect is a private-use URI
//! scheme named after the app (`<applicationId>:/companion`): Android verifies `https` App Links
//! only for domains the app names when it is built, and a self-hosted Companion's domain is chosen
//! later. Another app can claim the same scheme; PKCE makes a code useless to it.

use std::sync::{Mutex, PoisonError};

use zeroize::Zeroizing;

use crate::ceremony::{Ceremonies, Owner};
use crate::store::StoreError;

/// The Croft app's redirect schemes: its application ids (the release builds, and the preview
/// build's `.preview`). PLACEHOLDERS, with the app's own (`android/gradle.properties`,
/// `croft.applicationId`): change them together (docs/OPEN-ITEMS.md).
pub const APP_SCHEMES: &[&str] = &["com.example.croft", "com.example.croft.preview"];

/// The path every app redirect has.
pub const APP_REDIRECT_PATH: &str = ":/companion";

/// How long a hand-over code lasts.
pub const APP_HANDOVER_TTL_MS: i64 = 60 * 1000;

/// Whether `url` is exactly one of the Croft app's redirects (no query, no fragment, no variation).
pub fn is_app_redirect(url: &str) -> bool {
    APP_SCHEMES
        .iter()
        .any(|scheme| url.strip_prefix(scheme) == Some(APP_REDIRECT_PATH))
}

/// A PKCE S256 challenge: 43 base64url characters.
pub fn challenge_ok(c: &str) -> bool {
    c.len() == 43
        && c.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

struct Handover {
    challenge: String,
    user_id: i64,
}

/// Hand-over codes for apps, waiting to be redeemed: once each, within
/// [`APP_HANDOVER_TTL_MS`], only with the verifier of the challenge they were made for.
pub struct AppHandovers(Mutex<Ceremonies<Handover>>);

impl Default for AppHandovers {
    fn default() -> Self {
        Self(Mutex::new(Ceremonies::with_ttl(APP_HANDOVER_TTL_MS)))
    }
}

impl std::fmt::Debug for AppHandovers {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("AppHandovers")
    }
}

impl AppHandovers {
    /// A code that signs `user_id` in for the app holding the verifier of `challenge`.
    pub fn put(
        &self,
        user_id: i64,
        challenge: &str,
        now: i64,
    ) -> Result<Zeroizing<String>, StoreError> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner).put(
            Owner::User(user_id),
            Handover {
                challenge: challenge.to_owned(),
                user_id,
            },
            now,
        )
    }

    /// Redeems a code (spent whatever the verifier): the user, if the verifier matches.
    pub fn redeem(&self, code: &str, verifier: &str, now: i64) -> Option<i64> {
        let h = self
            .0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take(code, now)?;
        crate::crypto::ct_eq(
            crate::oidc::s256(verifier).as_bytes(),
            h.challenge.as_bytes(),
        )
        .then_some(h.user_id)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn only_the_apps_own_redirects_are_accepted() {
        assert!(is_app_redirect("com.example.croft:/companion"));
        assert!(is_app_redirect("com.example.croft.preview:/companion"));
        for bad in [
            "com.example.croft:/companion?x=1",
            "com.example.croft:/companion#x",
            "com.example.croft:/companion/",
            "com.example.croft://companion",
            "com.example.croft:/Companion",
            "COM.EXAMPLE.CROFT:/companion",
            "com.example.crofts:/companion",
            "com.example.croft.evil:/companion",
            "evil.com.example.croft:/companion",
            "https://companion.test/companion",
            "",
        ] {
            assert!(!is_app_redirect(bad), "{bad}");
        }
    }

    #[test]
    fn a_code_is_spent_once_and_only_with_its_verifier_in_time() {
        let h = AppHandovers::default();
        let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        let challenge = crate::oidc::s256(verifier);
        assert!(challenge_ok(&challenge));
        let code = h.put(7, &challenge, 0).unwrap();
        assert_eq!(h.redeem(&code, verifier, 1), Some(7));
        assert_eq!(h.redeem(&code, verifier, 2), None, "once");
        // A wrong verifier spends it too.
        let code = h.put(7, &challenge, 0).unwrap();
        assert_eq!(
            h.redeem(&code, "wrong-verifier-wrong-verifier-wrong-verif", 1),
            None
        );
        assert_eq!(h.redeem(&code, verifier, 1), None);
        // Too late.
        let code = h.put(7, &challenge, 0).unwrap();
        assert_eq!(h.redeem(&code, verifier, APP_HANDOVER_TTL_MS), None);
        assert_eq!(h.redeem("not-a-code", verifier, 1), None);
    }
}
