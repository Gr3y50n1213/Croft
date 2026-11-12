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
    /// FCM's address.
    pub fcm_base: String,
    /// Reverse proxies whose `X-Forwarded-For` is believed.
    pub trusted_proxies: Vec<IpAddr>,
    /// Messages an hour to one device (on average; [`Config::push_burst`] at once).
    pub push_per_hour: u32,
    /// Messages at once to one device.
    pub push_burst: u32,
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
                        .map(|s| {
                            s.parse::<IpAddr>().map_err(|_| {
                                ConfigError("CROFT_GATEWAY_TRUSTED_PROXIES: not an address".into())
                            })
                        })
                        .collect::<Result<_, _>>()?;
                }
                "CROFT_GATEWAY_PUSH_PER_HOUR" => c.push_per_hour = number(&value)?,
                "CROFT_GATEWAY_PUSH_BURST" => c.push_burst = number(&value)?,
                "CROFT_GATEWAY_REGISTER_PER_HOUR" => c.register_per_hour = number(&value)?,
                _ => {}
            }
        }
        c.public_url = c.public_url.trim_end_matches('/').to_owned();
        c.trusted_proxies = c
            .trusted_proxies
            .iter()
            .map(|ip| ip.to_canonical())
            .collect();
        let public_ok = c.public_url.starts_with("https://")
            || croft_protocols::oauth::fetch::is_loopback_http(&c.public_url);
        if !public_ok || c.public_url.contains(['?', '#']) {
            return Err(ConfigError(
                "public_url (CROFT_GATEWAY_PUBLIC_URL) must be the gateway's https address".into(),
            ));
        }
        if !(c.fcm_base.starts_with("https://")
            || croft_protocols::oauth::fetch::is_loopback_http(&c.fcm_base))
        {
            return Err(ConfigError("fcm_base must be an https address".into()));
        }
        if c.push_per_hour == 0 || c.push_burst == 0 || c.register_per_hour == 0 {
            return Err(ConfigError("the limits must be above zero".into()));
        }
        Ok(c)
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

    #[test]
    fn the_environment_wins_and_the_public_url_is_checked() {
        let c = Config::load(
            Some("public_url = \"https://push.example/\"\npush_per_hour = 5\n"),
            env(&[
                ("CROFT_GATEWAY_PUSH_BURST", "3"),
                ("CROFT_GATEWAY_TRUSTED_PROXIES", "10.0.0.1, ::ffff:10.0.0.2"),
            ]),
        )
        .unwrap();
        assert_eq!(c.public_url, "https://push.example");
        assert_eq!((c.push_per_hour, c.push_burst), (5, 3));
        assert_eq!(
            c.trusted_proxies,
            vec![
                "10.0.0.1".parse::<IpAddr>().unwrap(),
                "10.0.0.2".parse::<IpAddr>().unwrap()
            ]
        );
        assert!(Config::load(None, env(&[])).is_err(), "no public URL");
        assert!(Config::load(
            None,
            env(&[("CROFT_GATEWAY_PUBLIC_URL", "http://push.example")])
        )
        .is_err());
        assert!(Config::load(Some("unknown = 1"), env(&[])).is_err());
        assert!(Config::load(
            None,
            env(&[
                ("CROFT_GATEWAY_PUBLIC_URL", "https://push.example"),
                ("CROFT_GATEWAY_PUSH_PER_HOUR", "0")
            ])
        )
        .is_err());
    }
}
