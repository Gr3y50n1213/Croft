// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Server host names as people and settings files write them.
//!
//! One function cleans a host up, and the two places that must agree about what a host *is* both
//! call it: the IMAP connection (which name it looks up and checks the certificate for) and the
//! provider check ([`provider_for_host`](crate::discovery::provider_for_host), which decides
//! whether a certificate pin is allowed). If they cleaned a host differently, a spelling one of
//! them did not recognise (`[imap.gmail.com]`, `imap.gmail.com.`) could connect to a provider's
//! server and still pass as someone else's.

/// The host as it is looked up and compared: surrounding spaces removed, the brackets of an IPv6
/// literal (`[::1]`) and a trailing dot (`imap.gmail.com.`) dropped, ASCII letters in lower case.
///
/// DNS names are not case sensitive, so the lower-case form can be used for the lookup, for the
/// certificate's name check and for comparing against a list.
pub fn clean_host(host: &str) -> String {
    host.trim()
        .trim_start_matches('[')
        // `]` and `.` in any order: `[host.]` and `[host].` are both `host`.
        .trim_end_matches([']', '.'])
        .to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::clean_host;

    #[test]
    fn spaces_brackets_a_trailing_dot_and_case_are_cleaned_up() {
        for (written, clean) in [
            ("imap.gmail.com", "imap.gmail.com"),
            ("  IMAP.Gmail.COM  ", "imap.gmail.com"),
            ("imap.gmail.com.", "imap.gmail.com"),
            ("[imap.gmail.com]", "imap.gmail.com"),
            ("[imap.gmail.com.]", "imap.gmail.com"),
            ("[imap.gmail.com].", "imap.gmail.com"),
            (" [IMAP.gmail.com.] ", "imap.gmail.com"),
            ("[::1]", "::1"),
            ("::1", "::1"),
            ("127.0.0.1", "127.0.0.1"),
            ("", ""),
        ] {
            assert_eq!(clean_host(written), clean, "{written:?}");
        }
    }
}
