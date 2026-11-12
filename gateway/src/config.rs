// This Source Code Form is subject to the terms of the GNU Affero General Public License,
// version 3 or later. See LICENSE-AGPL-3.0 at the repository root.

//! The gateway's settings: an optional TOML file (`CROFT_GATEWAY_CONFIG`), then the environment
//! (`CROFT_GATEWAY_*`), which wins. The key and the service account are files, never values here.

use std::net::IpAddr;

use serde::Deserialize;

/// The gateway's settings.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct Config {
    /// Where to listen (plain HTTP: TLS is the reverse proxy's job).
    pub listen: String,
    /// The gateway's public `https` URL, without a trailing slash (endpoints start with it).
    pub public_url: String,
    /// The file holding the gateway's key (64 hexadecimal characters).
    pub key_file: String,
    /// The Google service account key file (JSON) of the app's Firebase project.
    pub service_account_file: String,
    /// FCM's address: Google's own (plain http on this machine only, for tests).
    pub fcm_base: String,
    /// Reverse proxies whose `X-Forwarded-For` is believed: addresses or ranges (`172.16.0.0/12`).
    pub trusted_proxies: Vec<String>,
    /// Messages an hour to one device (on average; [`Config::push_burst`] at once).
    pub push_per_hour: u32,
    /// Messages at once to one device.
    pub push_burst: u32,
    /// Messages an hour from one client address (a Companion sends for all its users).
    pub push_per_address_per_hour: u32,
    /// Messages an hour to FCM in all.
    pub total_per_hour: u32,
    /// Registrations an hour from one client address.
    pub register_per_hour: u32,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            listen: "0.0.0.0:8490".into(),
            public_url: String::new(),
            key_file: "/keys/gateway.key".into(),
            service_account_file: "/keys/service-account.json".into(),
            fcm_base: crate::fcm::FCM_BASE.into(),
            trusted_proxies: Vec::new(),
            push_per_hour: 60,
            push_burst: 10,
            push_per_address_per_hour: 36_000,
            total_per_hour: 720_000,
            register_per_hour: 30,
        }
    }
}

/// Why the settings could not be used.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigError(pub String);

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ConfigError {}

/// An address range (`10.0.0.0/8`, `fd00::/8`; a single address is a whole-length range).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Network {
    base: IpAddr,
    prefix: u8,
}

impl Network {
    /// Parses `address` or `address/prefix`.
    pub fn parse(text: &str) -> Option<Self> {
        let (addr, prefix) = match text.trim().split_once('/') {
            Some((a, p)) => (a, Some(p.parse::<u8>().ok()?)),
            None => (text.trim(), None),
        };
        let base = addr.parse::<IpAddr>().ok()?.to_canonical();
        let max = if base.is_ipv4() { 32 } else { 128 };
        let prefix = prefix.unwrap_or(max);
        (prefix <= max).then_some(Self { base, prefix })
    }

    /// Whether `ip` is in the range.
    pub fn contains(&self, ip: IpAddr) -> bool {
        let bits = |ip: IpAddr| match ip {
            IpAddr::V4(a) => (u128::from(u32::from(a)), 32),
            IpAddr::V6(a) => (u128::from(a), 128),
        };
        let (ip, ip_len) = bits(ip.to_canonical());
        let (base, len) = bits(self.base);
        if ip_len != len {
            return false;
        }
        let shift = u32::from(len - self.prefix);
        shift >= 128 || (ip >> shift) == (base >> shift)
    }
}

impl Config {
    /// The settings from `toml` (a file's text, if any) and then `env` (name, value pairs).
    pub fn load(
        toml: Option<&str>,
        env: impl IntoIterator<Item = (String, String)>,
    ) -> Result<Self, ConfigError> {
        let mut c: Config = match toml {
            Some(text) => toml::from_str(text)
                .map_err(|e| ConfigError(format!("the configuration file: {}", e.message())))?,
            None => Config::default(),
        };
        for (name, value) in env {
            let number = |v: &str| {
                v.parse::<u32>()
                    .map_err(|_| ConfigError(format!("{name} is not a number")))
            };
            match name.as_str() {
                "CROFT_GATEWAY_LISTEN" => c.listen = value,
                "CROFT_GATEWAY_PUBLIC_URL" => c.public_url = value,
                "CROFT_GATEWAY_KEY_FILE" => c.key_file = value,
                "CROFT_GATEWAY_SERVICE_ACCOUNT_FILE" => c.service_account_file = value,
                "CROFT_GATEWAY_FCM_BASE" => c.fcm_base = value,
                "CROFT_GATEWAY_TRUSTED_PROXIES" => {
                    c.trusted_proxies = value
                        .split(',')
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .map(str::to_owned)
                        .collect();
                }
                "CROFT_GATEWAY_PUSH_PER_HOUR" => c.push_per_hour = number(&value)?,
                "CROFT_GATEWAY_PUSH_BURST" => c.push_burst = number(&value)?,
                "CROFT_GATEWAY_PUSH_PER_ADDRESS_PER_HOUR" => {
                    c.push_per_address_per_hour = number(&value)?;
                }
                "CROFT_GATEWAY_TOTAL_PER_HOUR" => c.total_per_hour = number(&value)?,
                "CROFT_GATEWAY_REGISTER_PER_HOUR" => c.register_per_hour = number(&value)?,
                _ => {}
            }
        }
        c.public_url = c.public_url.trim_end_matches('/').to_owned();
        let public_ok = c.public_url.starts_with("https://")
            || croft_protocols::oauth::fetch::is_loopback_http(&c.public_url);
        if !public_ok || c.public_url.contains(['?', '#', '@']) {
            return Err(ConfigError(
                "public_url (CROFT_GATEWAY_PUBLIC_URL) must be the gateway's https address".into(),
            ));
        }
        // The access token can send to every user of the app: it goes to Google's FCM only.
        c.fcm_base = c.fcm_base.trim_end_matches('/').to_owned();
        let loopback = croft_protocols::oauth::fetch::is_loopback_http(&c.fcm_base)
            && crate::vapid::origin(&c.fcm_base) == c.fcm_base;
        if c.fcm_base != crate::fcm::FCM_BASE && !loopback {
            return Err(ConfigError(format!(
                "fcm_base must be {}",
                crate::fcm::FCM_BASE
            )));
        }
        if c.proxies().is_none() {
            return Err(ConfigError(
                "trusted_proxies: each entry must be an address or a range like 172.16.0.0/12"
                    .into(),
            ));
        }
        if [
            c.push_per_hour,
            c.push_burst,
            c.push_per_address_per_hour,
            c.total_per_hour,
            c.register_per_hour,
        ]
        .contains(&0)
        {
            return Err(ConfigError("the limits must be above zero".into()));
        }
        Ok(c)
    }

    /// The trusted proxies as ranges (`None` if one does not parse).
    pub fn proxies(&self) -> Option<Vec<Network>> {
        self.trusted_proxies
            .iter()
            .map(|t| Network::parse(t))
            .collect()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn env(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(a, b)| ((*a).to_owned(), (*b).to_owned()))
            .collect()
    }

    fn with_url(more: &[(&str, &str)]) -> Result<Config, ConfigError> {
        let mut e = env(&[("CROFT_GATEWAY_PUBLIC_URL", "https://push.example")]);
        e.extend(env(more));
        Config::load(None, e)
    }

    #[test]
    fn the_environment_wins_and_the_addresses_are_checked() {
        let c = Config::load(
            Some("public_url = \"https://push.example/\"\npush_per_hour = 5\n"),
            env(&[
                ("CROFT_GATEWAY_PUSH_BURST", "3"),
                (
                    "CROFT_GATEWAY_TRUSTED_PROXIES",
                    "10.0.0.1, 172.16.0.0/12,fd00::/8",
                ),
            ]),
        )
        .unwrap();
        assert_eq!(c.public_url, "https://push.example");
        assert_eq!((c.push_per_hour, c.push_burst), (5, 3));
        let p = c.proxies().unwrap();
        let ip = |s: &str| s.parse::<IpAddr>().unwrap();
        assert!(p[0].contains(ip("10.0.0.1")) && !p[0].contains(ip("10.0.0.2")));
        assert!(p[1].contains(ip("172.20.0.1")) && p[1].contains(ip("::ffff:172.31.255.255")));
        assert!(!p[1].contains(ip("172.32.0.1")));
        assert!(p[2].contains(ip("fd12::1")) && !p[2].contains(ip("fe80::1")));
        assert!(Config::load(None, env(&[])).is_err(), "no public URL");
        assert!(Config::load(
            None,
            env(&[("CROFT_GATEWAY_PUBLIC_URL", "http://push.example")])
        )
        .is_err());
        assert!(Config::load(Some("unknown = 1"), env(&[])).is_err());
        assert!(with_url(&[("CROFT_GATEWAY_PUSH_PER_HOUR", "0")]).is_err());
        assert!(with_url(&[("CROFT_GATEWAY_TRUSTED_PROXIES", "10.0.0.0/33")]).is_err());
        assert!(with_url(&[("CROFT_GATEWAY_TRUSTED_PROXIES", "proxy")]).is_err());
    }

    #[test]
    fn fcm_is_googles_own_address() {
        assert!(with_url(&[]).is_ok());
        assert!(with_url(&[("CROFT_GATEWAY_FCM_BASE", "https://fcm.googleapis.com/")]).is_ok());
        for bad in [
            "https://fcm.googleapis.com/v1",
            "https://fcm.example",
            "https://fcm.googleapis.com.example",
            "http://fcm.googleapis.com",
            "http://127.0.0.1:9/path",
        ] {
            assert!(
                with_url(&[("CROFT_GATEWAY_FCM_BASE", bad)]).is_err(),
                "{bad}"
            );
        }
        assert!(with_url(&[("CROFT_GATEWAY_FCM_BASE", "http://127.0.0.1:9")]).is_ok());
    }
}
