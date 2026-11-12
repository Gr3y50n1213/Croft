// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Whether a message may load remote images: the S49 / S57 setting "Remote images: Ask, Always
//! for known senders, Never" (default Ask), the person's "Show images" on one message, and the
//! account's list of known senders.
//!
//! | Setting | Loads remote images when |
//! | --- | --- |
//! | [`RemoteImages::Never`] | never, whatever was allowed before |
//! | [`RemoteImages::Ask`] (default) | the person tapped "Show images" on this message |
//! | [`RemoteImages::KnownSenders`] | as Ask, or the sender is on the account's known-senders list |
//!
//! "Known senders" are the addresses the person added with "Always show images from this sender"
//! (one list per account). The sender is the message's `From` address, which a forger can set: a
//! forged known sender only gets images loaded (a privacy leak, not code), which is why this is
//! offered only under its own setting.

/// The person's remote-images setting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RemoteImages {
    /// Ask for every message (the bar with "Show images"). The default.
    #[default]
    Ask,
    /// Load them automatically for known senders; ask for the others.
    KnownSenders,
    /// Never load them.
    Never,
}

/// The rule in the module documentation. `message_allowed`: the person tapped "Show images" on
/// this message. `sender_known`: the sender is on the account's known-senders list.
pub fn remote_allowed(setting: RemoteImages, message_allowed: bool, sender_known: bool) -> bool {
    match setting {
        RemoteImages::Never => false,
        RemoteImages::Ask => message_allowed,
        RemoteImages::KnownSenders => message_allowed || sender_known,
    }
}

/// An address as the known-senders list stores it: trimmed, without angle brackets, lower case.
/// `None` when it is not one plain address (no `@`, spaces or control characters, too long).
pub fn normalise_address(address: &str) -> Option<String> {
    let a = address.trim().trim_start_matches('<').trim_end_matches('>');
    let valid = a.len() <= 320
        && a.split('@').count() == 2
        && a.split('@').all(|p| !p.is_empty())
        && !a.chars().any(|c| c.is_whitespace() || c.is_control());
    valid.then(|| a.to_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_setting_decides_as_documented() {
        use RemoteImages::*;
        for (setting, message, known, expected) in [
            (Never, true, true, false),
            (Ask, false, true, false),
            (Ask, true, false, true),
            (KnownSenders, false, true, true),
            (KnownSenders, true, false, true),
            (KnownSenders, false, false, false),
        ] {
            assert_eq!(
                remote_allowed(setting, message, known),
                expected,
                "{setting:?} {message} {known}"
            );
        }
        assert_eq!(RemoteImages::default(), Ask);
    }

    #[test]
    fn addresses_are_normalised_or_refused() {
        assert_eq!(
            normalise_address(" <Ann@Example.ORG> ").as_deref(),
            Some("ann@example.org")
        );
        for bad in ["", "ann", "a@b@c", "@x", "x@", "a b@c", "a\n@c"] {
            assert_eq!(normalise_address(bad), None, "{bad:?}");
        }
    }
}
