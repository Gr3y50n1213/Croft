// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! What the core stores in `account.config_json`: server settings and the pinned certificate.
//! **Never a secret**: the password or token lives in the platform keystore and comes back
//! through the `CredentialProvider`.

use croft_protocols::{CertFingerprint, ImapConfig, Security};
use serde::{Deserialize, Serialize};

/// The version this build writes. A record with a higher version was written by a newer app and
/// is not used (the account shows as an error instead of being misread).
pub(crate) const SETTINGS_VERSION: u32 = 1;

/// How a connection is secured, as stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum StoredSecurity {
    Tls,
    StartTls,
    /// Only the tests build it; `ImapBackend::new` refuses it for any host but loopback.
    PlaintextForTestsOnly,
}

impl StoredSecurity {
    pub(crate) fn to_security(self) -> Security {
        match self {
            Self::Tls => Security::Tls,
            Self::StartTls => Security::StartTls,
            Self::PlaintextForTestsOnly => Security::PlaintextForTestsOnly,
        }
    }
}

/// One server.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct StoredServer {
    pub(crate) host: String,
    pub(crate) port: u16,
    pub(crate) security: StoredSecurity,
    pub(crate) username: String,
}

/// The settings record of one account.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct StoredSettings {
    /// [`SETTINGS_VERSION`].
    pub(crate) version: u32,
    /// The IMAP server.
    pub(crate) imap: StoredServer,
    /// The SMTP server, kept for the sending task.
    #[serde(default)]
    pub(crate) smtp: Option<StoredServer>,
    /// SHA-256 of the one certificate to trust (upper-case hex with colons), when the person
    /// trusted a self-signed one.
    #[serde(default)]
    pub(crate) pinned_fingerprint: Option<String>,
}

/// Why stored settings cannot be used.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SettingsError {
    /// Not a settings record, or written by a newer version.
    Unreadable,
    /// The pinned fingerprint is malformed.
    BadFingerprint,
}

impl StoredSettings {
    /// Reads a stored record.
    pub(crate) fn from_json(value: &serde_json::Value) -> Result<Self, SettingsError> {
        let s: Self =
            serde_json::from_value(value.clone()).map_err(|_| SettingsError::Unreadable)?;
        if s.version > SETTINGS_VERSION {
            return Err(SettingsError::Unreadable);
        }
        Ok(s)
    }

    /// The JSON to store.
    pub(crate) fn to_json(&self) -> Result<serde_json::Value, SettingsError> {
        serde_json::to_value(self).map_err(|_| SettingsError::Unreadable)
    }

    /// The adapter settings.
    pub(crate) fn imap_config(&self) -> Result<ImapConfig, SettingsError> {
        let mut config = ImapConfig::new(
            self.imap.host.clone(),
            self.imap.port,
            self.imap.security.to_security(),
            self.imap.username.clone(),
        );
        if let Some(pin) = &self.pinned_fingerprint {
            let pin = CertFingerprint::from_hex(pin).map_err(|_| SettingsError::BadFingerprint)?;
            config = config.with_pinned_cert(pin);
        }
        Ok(config)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> StoredSettings {
        StoredSettings {
            version: SETTINGS_VERSION,
            imap: StoredServer {
                host: "imap.example.org".into(),
                port: 993,
                security: StoredSecurity::Tls,
                username: "alice@example.org".into(),
            },
            smtp: None,
            pinned_fingerprint: None,
        }
    }

    #[test]
    fn round_trips_and_has_no_secret_field() {
        let s = sample();
        let json = s.to_json().unwrap();
        assert_eq!(StoredSettings::from_json(&json).unwrap(), s);
        let keys: Vec<_> = json.as_object().unwrap().keys().cloned().collect();
        assert!(keys
            .iter()
            .all(|k| !k.contains("pass") && !k.contains("secret")));
    }

    #[test]
    fn a_newer_version_is_refused() {
        let mut json = sample().to_json().unwrap();
        json["version"] = serde_json::json!(SETTINGS_VERSION + 1);
        assert_eq!(
            StoredSettings::from_json(&json).unwrap_err(),
            SettingsError::Unreadable
        );
        assert!(StoredSettings::from_json(&serde_json::json!({})).is_err());
    }

    #[test]
    fn a_bad_pin_is_refused() {
        let mut s = sample();
        s.pinned_fingerprint = Some("not hex".into());
        assert_eq!(s.imap_config().unwrap_err(), SettingsError::BadFingerprint);
    }
}
