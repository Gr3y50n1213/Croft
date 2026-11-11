// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! What the core stores in `account.config_json`: server settings and the pinned certificate.
//! **Never a secret**: the password or token lives in the platform keystore and comes back
//! through the `CredentialProvider`.

use croft_protocols::demo::DEMO_ADDRESS;
use croft_protocols::discovery::{provider_for_host, Provider};
use croft_protocols::{
    clean_host, CertFingerprint, ImapConfig, Security, SmtpConfig, SmtpSecurity,
};
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
    ///
    /// **There is one pin per account, and it belongs to the IMAP server.** It is never applied to
    /// a well-known provider's server (see [`StoredSettings::imap_config`]).
    ///
    /// **The SMTP pin rule** ([`StoredSettings::smtp_config`]): the pin also applies to the SMTP
    /// server, and only when the cleaned SMTP host equals the cleaned IMAP host (`clean_host`, the
    /// function the connections use) **and** that host is not a provider's (`provider_for_host`).
    /// In every other case SMTP uses normal certificate verification. A pin the person trusted for
    /// their own IMAP server says nothing about the certificate of another host, so a pin is never
    /// carried over to a different SMTP host (the person's own `smtp.example.org` next to
    /// `imap.example.org`, say), and a provider's SMTP host (`smtp.gmail.com`) never takes one,
    /// even when a stored record was written with a pin or names the provider's host on both
    /// sides. (The cost: an account whose SMTP server is another host with a self-signed
    /// certificate cannot send until that server has a certificate that verifies.)
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
    /// Reads a stored record. A record with a `kind` key is not a server record (the sample
    /// mailbox's marker has one, see [`demo_config`]) and is refused, so no record can be read as
    /// both.
    pub(crate) fn from_json(value: &serde_json::Value) -> Result<Self, SettingsError> {
        if value.get("kind").is_some() {
            return Err(SettingsError::Unreadable);
        }
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

    /// The adapter settings. The stored pin is applied only to a server of the person's own: when
    /// the IMAP host is a well-known provider's (`provider_for_host`), the pin is ignored and the
    /// account keeps working with the provider's real certificate. A record like that is not
    /// refused (a Gmail account must keep working) and is not trusted either: `add_account` never
    /// writes such a pin, so one found here was put there by something else, and a pin on a
    /// provider's server could only ever pin an interceptor's certificate.
    pub(crate) fn imap_config(&self) -> Result<ImapConfig, SettingsError> {
        let mut config = ImapConfig::new(
            self.imap.host.clone(),
            self.imap.port,
            self.imap.security.to_security(),
            self.imap.username.clone(),
        );
        if provider_for_host(&self.imap.host).is_some() {
            return Ok(config);
        }
        if let Some(pin) = &self.pinned_fingerprint {
            let pin = CertFingerprint::from_hex(pin).map_err(|_| SettingsError::BadFingerprint)?;
            config = config.with_pinned_cert(pin);
        }
        Ok(config)
    }
}

impl StoredSettings {
    /// True when the account's pin applies to its SMTP server. See the field's note on
    /// [`StoredSettings::pinned_fingerprint`]: the same cleaned host as IMAP, and not a provider's.
    pub(crate) fn smtp_pin_applies(&self) -> bool {
        let Some(smtp) = &self.smtp else {
            return false;
        };
        clean_host(&smtp.host) == clean_host(&self.imap.host)
            && provider_for_host(&smtp.host).is_none()
            && provider_for_host(&self.imap.host).is_none()
    }

    /// The SMTP settings for sending, or `None` when the account has no SMTP server. Applies the
    /// pin only by the rule in [`StoredSettings::smtp_pin_applies`]. A pin that applies but is not
    /// valid hex is [`SettingsError::BadFingerprint`] (as for IMAP: the account does not send
    /// with a certificate check that silently became weaker).
    pub(crate) fn smtp_config(&self) -> Result<Option<SmtpConfig>, SettingsError> {
        let Some(smtp) = &self.smtp else {
            return Ok(None);
        };
        let security = match smtp.security {
            StoredSecurity::Tls => SmtpSecurity::Tls,
            StoredSecurity::StartTls => SmtpSecurity::StartTls,
            StoredSecurity::PlaintextForTestsOnly => SmtpSecurity::PlaintextForTestsOnly,
        };
        let mut config = SmtpConfig::new(
            smtp.host.clone(),
            smtp.port,
            security,
            smtp.username.clone(),
        );
        if self.smtp_pin_applies() {
            if let Some(pin) = &self.pinned_fingerprint {
                let pin =
                    CertFingerprint::from_hex(pin).map_err(|_| SettingsError::BadFingerprint)?;
                config = config.with_pinned_cert(pin);
            }
        }
        Ok(Some(config))
    }

    /// True when the SMTP server saves the copy in Sent by itself (Gmail), so the core must not
    /// add a second one. Only Gmail does: Microsoft's, iCloud's and Fastmail's SMTP servers leave
    /// the copy to the client.
    pub(crate) fn smtp_files_sent_itself(&self) -> bool {
        self.smtp
            .as_ref()
            .is_some_and(|s| provider_for_host(&s.host) == Some(Provider::Gmail))
    }
}

/// The `kind` of the sample mailbox's settings record.
const DEMO_KIND: &str = "demo";

/// The settings record of the sample mailbox: no server, no secret, just the marker. It is not a
/// [`StoredSettings`] (it has no `imap`), so code that reads server settings refuses it.
pub(crate) fn demo_config() -> serde_json::Value {
    serde_json::json!({ "version": SETTINGS_VERSION, "kind": DEMO_KIND })
}

/// True for the account row of the sample mailbox: its settings record has exactly the shape of
/// [`demo_config`] (the keys `version` and `kind` and nothing else: no `imap`, no `jmap`), and its
/// address is the sample address. Anything else is not the sample mailbox.
pub(crate) fn is_demo(config: &serde_json::Value, email: &str) -> bool {
    let Some(map) = config.as_object() else {
        return false;
    };
    email == DEMO_ADDRESS
        && map.len() == 2
        && map.get("kind").and_then(serde_json::Value::as_str) == Some(DEMO_KIND)
        && map
            .get("version")
            .and_then(serde_json::Value::as_u64)
            .is_some_and(|v| v <= u64::from(SETTINGS_VERSION))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_demo_marker_is_not_a_server_record() {
        assert!(is_demo(&demo_config(), DEMO_ADDRESS));
        assert!(StoredSettings::from_json(&demo_config()).is_err());
        assert!(!is_demo(&sample().to_json().unwrap(), DEMO_ADDRESS));
        assert!(!is_demo(&serde_json::json!({}), DEMO_ADDRESS));
        assert!(!is_demo(&serde_json::json!("demo"), DEMO_ADDRESS));
    }

    /// The marker must have the exact shape and the sample address: a server record with a `kind`
    /// key added, or a marker on another address, is not the sample mailbox.
    #[test]
    fn only_the_exact_demo_shape_is_the_sample_mailbox() {
        let mut both = sample().to_json().unwrap();
        both["kind"] = serde_json::json!("demo");
        assert!(!is_demo(&both, DEMO_ADDRESS));
        let mut jmap = demo_config();
        jmap["jmap"] = serde_json::json!({});
        assert!(!is_demo(&jmap, DEMO_ADDRESS));
        let mut newer = demo_config();
        newer["version"] = serde_json::json!(SETTINGS_VERSION + 1);
        assert!(!is_demo(&newer, DEMO_ADDRESS));
        assert!(!is_demo(&demo_config(), "someone@example.org"));
    }

    /// A record with a `kind` key is never read as server settings, even with a valid `imap`.
    #[test]
    fn a_record_with_a_kind_is_not_server_settings() {
        let mut both = sample().to_json().unwrap();
        both["kind"] = serde_json::json!("demo");
        assert_eq!(
            StoredSettings::from_json(&both).unwrap_err(),
            SettingsError::Unreadable
        );
        both["kind"] = serde_json::json!("anything");
        assert!(StoredSettings::from_json(&both).is_err());
    }

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

    /// A stored pin on a provider's own server is ignored, whatever spelling the host has, and does
    /// not stop the account from starting. A server of the person's own keeps its pin.
    #[test]
    fn a_stored_pin_is_ignored_for_a_providers_host_and_kept_for_any_other() {
        let pin = CertFingerprint::from_bytes([9; 32]);
        for host in [
            "imap.gmail.com",
            "IMAP.Gmail.COM.",
            "[imap.gmail.com]",
            "outlook.office365.com",
            "imap.mail.me.com",
            "imap.fastmail.com",
        ] {
            let mut s = sample();
            s.imap.host = host.into();
            s.pinned_fingerprint = Some(pin.to_string());
            let config = s.imap_config().unwrap();
            assert_eq!(config.pinned_cert, None, "{host}");
            assert_eq!(config.host, host, "the host is used as stored");
            // Not even a malformed pin refuses the account: it is not looked at.
            s.pinned_fingerprint = Some("not hex".into());
            assert_eq!(s.imap_config().unwrap().pinned_cert, None, "{host}");
        }
        let mut own = sample();
        own.pinned_fingerprint = Some(pin.to_string());
        assert_eq!(own.imap_config().unwrap().pinned_cert, Some(pin));
    }

    fn with_smtp(imap_host: &str, smtp_host: &str, pin: Option<&str>) -> StoredSettings {
        let mut s = sample();
        s.imap.host = imap_host.into();
        s.smtp = Some(StoredServer {
            host: smtp_host.into(),
            port: 465,
            security: StoredSecurity::Tls,
            username: "alice@example.org".into(),
        });
        s.pinned_fingerprint = pin.map(str::to_owned);
        s
    }

    /// The SMTP pin rule: the pin applies to SMTP only for the same cleaned host as IMAP that is
    /// not a provider's; in every other case SMTP uses normal certificate verification.
    #[test]
    fn the_pin_applies_to_smtp_only_for_the_same_host_that_is_not_a_providers() {
        let pin = CertFingerprint::from_bytes([9; 32]).to_string();
        let pin = Some(pin.as_str());
        let pinned = |s: &StoredSettings| s.smtp_config().unwrap().unwrap().pinned_cert;
        let expected = Some(CertFingerprint::from_bytes([9; 32]));

        // The person's own server, one host for both: pinned, whatever the spelling.
        for (imap, smtp) in [
            ("mail.example.org", "mail.example.org"),
            ("mail.example.org", "MAIL.Example.ORG."),
            ("[mail.example.org]", " mail.example.org "),
        ] {
            let s = with_smtp(imap, smtp, pin);
            assert!(s.smtp_pin_applies(), "{imap} {smtp}");
            assert_eq!(pinned(&s), expected, "{imap} {smtp}");
        }
        // Another host, even a sibling of the same domain: normal verification.
        for (imap, smtp) in [
            ("imap.example.org", "smtp.example.org"),
            ("mail.example.org", "mail.example.net"),
            ("example.org", "mail.example.org"),
            ("127.0.0.1", "localhost"),
        ] {
            let s = with_smtp(imap, smtp, pin);
            assert!(!s.smtp_pin_applies(), "{imap} {smtp}");
            assert_eq!(pinned(&s), None, "{imap} {smtp}");
        }
        // A provider's SMTP host never takes the pin: not next to the person's own IMAP host, and
        // not when both sides name the provider (a record somebody wrote with a pin).
        for (imap, smtp) in [
            ("mail.example.org", "smtp.gmail.com"),
            ("imap.gmail.com", "smtp.gmail.com"),
            ("smtp.gmail.com", "smtp.gmail.com"),
            ("outlook.office365.com", "outlook.office365.com"),
            ("imap.fastmail.com", "SMTP.Fastmail.COM."),
            ("mail.example.org", "[smtp.office365.com]"),
            ("imap.mail.me.com", "smtp.mail.me.com"),
        ] {
            let s = with_smtp(imap, smtp, pin);
            assert!(!s.smtp_pin_applies(), "{imap} {smtp}");
            assert_eq!(pinned(&s), None, "{imap} {smtp}");
        }
        // No pin stored: nothing to apply. No SMTP server: nothing to send with.
        assert_eq!(
            pinned(&with_smtp("mail.example.org", "mail.example.org", None)),
            None
        );
        assert!(sample().smtp_config().unwrap().is_none());
        assert!(!sample().smtp_pin_applies());
    }

    /// A pin that applies but cannot be read is refused; one that does not apply is not looked at.
    #[test]
    fn a_bad_pin_refuses_smtp_only_where_it_would_apply() {
        let s = with_smtp("mail.example.org", "mail.example.org", Some("not hex"));
        assert_eq!(s.smtp_config().unwrap_err(), SettingsError::BadFingerprint);
        let s = with_smtp("imap.example.org", "smtp.example.org", Some("not hex"));
        assert!(s.smtp_config().unwrap().unwrap().pinned_cert.is_none());
    }

    #[test]
    fn the_smtp_settings_are_carried_over_as_stored() {
        let mut s = with_smtp("mail.example.org", "smtp.example.org", None);
        let smtp = s.smtp.as_mut().unwrap();
        smtp.port = 587;
        smtp.security = StoredSecurity::StartTls;
        let c = s.smtp_config().unwrap().unwrap();
        assert_eq!((c.host.as_str(), c.port), ("smtp.example.org", 587));
        assert_eq!(c.security, SmtpSecurity::StartTls);
        assert_eq!(c.username, "alice@example.org");
    }

    #[test]
    fn only_gmails_smtp_server_saves_the_sent_copy_itself() {
        assert!(with_smtp("imap.gmail.com", "smtp.gmail.com", None).smtp_files_sent_itself());
        assert!(with_smtp("x.example.org", "SMTP.GMAIL.COM.", None).smtp_files_sent_itself());
        for host in [
            "smtp.office365.com",
            "smtp.mail.me.com",
            "smtp.fastmail.com",
            "smtp.example.org",
        ] {
            assert!(
                !with_smtp("x.example.org", host, None).smtp_files_sent_itself(),
                "{host}"
            );
        }
        assert!(!sample().smtp_files_sent_itself());
    }

    #[test]
    fn a_bad_pin_is_refused() {
        let mut s = sample();
        s.pinned_fingerprint = Some("not hex".into());
        assert_eq!(s.imap_config().unwrap_err(), SettingsError::BadFingerprint);
    }
}
