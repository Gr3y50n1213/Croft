// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Why finding the settings can fail, in words the UI can show as they are.
//!
//! Every message is a fixed English sentence chosen here. None of them contains the address, the
//! domain, a host name or any text the network sent back, so they are safe to log and to show.
//! The UI translates by matching on the variant; the English text is the fallback and what the
//! tests pin down.

/// One of the places Croft looks for settings, in the order it prefers them. The order of the
/// variants is the order of preference: an earlier one beats a later one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Probe {
    /// The built-in table of large providers. No network.
    ProviderTable,
    /// `https://autoconfig.<domain>/mail/config-v1.1.xml`.
    AutoconfigSubdomain,
    /// `https://<domain>/.well-known/autoconfig/mail/config-v1.1.xml`.
    AutoconfigWellKnown,
    /// RFC 6186 SRV records.
    Srv,
    /// `https://<domain>/.well-known/jmap`.
    JmapWellKnown,
}

impl Probe {
    /// All probes, in the order of preference.
    pub const ALL: [Probe; 5] = [
        Probe::ProviderTable,
        Probe::AutoconfigSubdomain,
        Probe::AutoconfigWellKnown,
        Probe::Srv,
        Probe::JmapWellKnown,
    ];
}

/// Where one probe stands, for the "Finding your server" screen (S03): waiting, running, found
/// or skipped, plus the reason when it found nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProbeState {
    /// Not started yet.
    Waiting,
    /// Asking the network.
    Running,
    /// Returned usable settings.
    Found,
    /// Looked and found nothing usable, for the given reason.
    Failed(ProbeFailure),
    /// Not needed: the domain is not in the provider table, or a better probe already won.
    Skipped,
}

/// A change in one probe's state, sent while discovery runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProbeUpdate {
    /// Which probe.
    pub probe: Probe,
    /// Its new state.
    pub state: ProbeState,
}

/// Why one probe found nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProbeFailure {
    /// The domain does not publish this: no such host, no such file or no records. The ordinary
    /// outcome for most domains.
    NotPublished,
    /// The request took too long.
    TimedOut,
    /// The network or the name service could not be reached.
    Unreachable,
    /// The server's secure-connection certificate was not accepted, so nothing was read from it.
    CertificateProblem,
    /// The answer was not in a form Croft understands (an error page, broken XML, no mail server
    /// in the file).
    Unusable,
    /// The answer was larger than Croft is willing to read.
    TooLarge,
    /// The server sent Croft somewhere it does not follow (another domain, unencrypted, or too
    /// many hops).
    BadRedirect,
    /// The domain offers only an unencrypted connection, which Croft never uses.
    OnlyUnencrypted,
    /// Something inside Croft went wrong while looking.
    Internal,
}

impl ProbeFailure {
    /// A plain-language reason for this failure, one sentence.
    pub fn message(&self) -> &'static str {
        match self {
            Self::NotPublished => "The domain does not publish settings here.",
            Self::TimedOut => "The server did not answer in time.",
            Self::Unreachable => "Croft could not reach the server or the name service.",
            Self::CertificateProblem => {
                "The server's security certificate was not accepted, so Croft did not read from it."
            }
            Self::Unusable => "The answer was not in a form Croft can use.",
            Self::TooLarge => "The answer was larger than Croft is willing to read.",
            Self::BadRedirect => {
                "The server sent Croft to another domain or to an unencrypted page, which it does not follow."
            }
            Self::OnlyUnencrypted => {
                "The domain offers only an unencrypted connection, which Croft does not use."
            }
            Self::Internal => "Something went wrong inside Croft while looking.",
        }
    }

    /// True for failures that say nothing about the domain itself (no network, timeouts), as
    /// opposed to "looked and there is nothing there".
    pub fn is_connectivity(&self) -> bool {
        matches!(self, Self::TimedOut | Self::Unreachable)
    }
}

/// Why finding the settings failed as a whole.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiscoveryError {
    /// The text is not an email address.
    InvalidAddress,
    /// Every place Croft looked was unreachable or timed out: most likely there is no network.
    Offline {
        /// What each probe reported, in the order of preference.
        reasons: Vec<(Probe, ProbeFailure)>,
    },
    /// Croft looked everywhere and the domain publishes nothing it can use. The next step is to
    /// ask the person for the server settings (S05).
    NothingFound {
        /// What each probe reported, in the order of preference.
        reasons: Vec<(Probe, ProbeFailure)>,
    },
    /// Croft could not start its name-service client (no system DNS settings could be read).
    ResolverUnavailable,
}

impl DiscoveryError {
    /// A plain-language explanation for the person, one or two sentences.
    pub fn message(&self) -> &'static str {
        match self {
            Self::InvalidAddress => "That does not look like an email address. Check it and try again.",
            Self::Offline { .. } => {
                "Croft could not reach the internet to look up your server. Check your connection and try again, or enter the server settings yourself."
            }
            Self::NothingFound { .. } => {
                "Croft could not find the settings for this address by itself. You can enter the server details yourself."
            }
            Self::ResolverUnavailable => {
                "Croft could not start its name lookup on this device. You can enter the server settings yourself."
            }
        }
    }
}

impl std::fmt::Display for DiscoveryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.message())
    }
}

impl std::error::Error for DiscoveryError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn probes_are_ordered_by_preference() {
        let mut sorted = Probe::ALL;
        sorted.sort();
        assert_eq!(sorted, Probe::ALL);
    }

    #[test]
    fn every_message_is_distinct_and_free_of_placeholders() {
        let failures = [
            ProbeFailure::NotPublished,
            ProbeFailure::TimedOut,
            ProbeFailure::Unreachable,
            ProbeFailure::CertificateProblem,
            ProbeFailure::Unusable,
            ProbeFailure::TooLarge,
            ProbeFailure::BadRedirect,
            ProbeFailure::OnlyUnencrypted,
            ProbeFailure::Internal,
        ];
        let mut seen = std::collections::HashSet::new();
        for f in failures {
            assert!(seen.insert(f.message()), "{f:?}");
            assert!(!f.message().contains('{') && !f.message().contains('%'));
        }
        let errors = [
            DiscoveryError::InvalidAddress,
            DiscoveryError::Offline { reasons: vec![] },
            DiscoveryError::NothingFound { reasons: vec![] },
            DiscoveryError::ResolverUnavailable,
        ];
        let mut seen = std::collections::HashSet::new();
        for e in &errors {
            assert!(seen.insert(e.message()));
        }
    }
}
