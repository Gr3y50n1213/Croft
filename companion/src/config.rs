// This Source Code Form is subject to the terms of the GNU Affero General Public License,
// version 3 or later. See LICENSE-AGPL-3.0 at the repository root.

//! The Companion's settings (T23): a TOML file, each field overridable by an environment
//! variable, and the master key file. Unknown keys are an error; a missing or unsafe master key
//! is an error with a plain message, never a fallback.

use std::io::Write;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::crypto::Key;

/// What went wrong with the settings or the master key file. No variant carries a key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigError {
    /// The settings file could not be read or parsed (the reason is safe to show).
    Unreadable(String),
    /// A setting has a value that cannot be used.
    Invalid(&'static str),
    /// The master key file is missing.
    NoMasterKey(PathBuf),
    /// The master key file can be read by other users of the machine.
    MasterKeyExposed(PathBuf),
    /// The master key file does not hold a key.
    MasterKeyDamaged(PathBuf),
    /// Writing a file failed.
    Write(PathBuf),
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unreadable(why) => write!(f, "the settings file cannot be used: {why}"),
            Self::Invalid(what) => write!(f, "a setting cannot be used: {what}"),
            Self::NoMasterKey(p) => write!(
                f,
                "no master key at {} (run `croft-companion init`, or restore the key from your backup)",
                p.display()
            ),
            Self::MasterKeyExposed(p) => write!(
                f,
                "the master key file {} can be read by other users; run `chmod 600` on it",
                p.display()
            ),
            Self::MasterKeyDamaged(p) => {
                write!(f, "the master key file {} does not hold a key", p.display())
            }
            Self::Write(p) => write!(f, "could not write {}", p.display()),
        }
    }
}

impl std::error::Error for ConfigError {}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct FileConfig {
    listen: Option<String>,
    public_url: Option<String>,
    data_dir: Option<PathBuf>,
    master_key_file: Option<PathBuf>,
}

/// The settings in force.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    /// Where the HTTP server listens (plain HTTP; TLS is the reverse proxy's job).
    pub listen: String,
    /// The address people use, `https://...` (plain `http` only for localhost).
    pub public_url: String,
    /// Where the database lives.
    pub data_dir: PathBuf,
    /// The master key file.
    pub master_key_file: PathBuf,
}

impl Config {
    /// The database file.
    pub fn database(&self) -> PathBuf {
        self.data_dir.join("companion.db")
    }

    /// Reads `text` (the settings file, may be empty) and applies `env` (name to value) on top.
    pub fn parse(text: &str, env: &dyn Fn(&str) -> Option<String>) -> Result<Self, ConfigError> {
        let file: FileConfig =
            toml::from_str(text).map_err(|e| ConfigError::Unreadable(e.message().to_owned()))?;
        let listen = env("CROFT_COMPANION_LISTEN")
            .or(file.listen)
            .unwrap_or_else(|| "127.0.0.1:8480".to_owned());
        let public_url = env("CROFT_COMPANION_PUBLIC_URL")
            .or(file.public_url)
            .ok_or(ConfigError::Invalid("public_url is required"))?;
        let data_dir = env("CROFT_COMPANION_DATA_DIR")
            .map(PathBuf::from)
            .or(file.data_dir)
            .unwrap_or_else(|| PathBuf::from("/data"));
        let master_key_file = env("CROFT_COMPANION_MASTER_KEY_FILE")
            .map(PathBuf::from)
            .or(file.master_key_file)
            .unwrap_or_else(|| data_dir.join("master.key"));
        if listen.parse::<std::net::SocketAddr>().is_err() {
            return Err(ConfigError::Invalid(
                "listen must be an address and port, such as 0.0.0.0:8480",
            ));
        }
        if !public_url_is_ok(&public_url) {
            return Err(ConfigError::Invalid(
                "public_url must start with https:// (plain http only for localhost)",
            ));
        }
        Ok(Self {
            listen,
            public_url: public_url.trim_end_matches('/').to_owned(),
            data_dir,
            master_key_file,
        })
    }

    /// Reads the settings file at `path` (a missing file is an empty one) and the environment.
    pub fn load(path: Option<&Path>) -> Result<Self, ConfigError> {
        let text = match path {
            Some(p) => std::fs::read_to_string(p)
                .map_err(|_| ConfigError::Unreadable(format!("cannot read {}", p.display())))?,
            None => String::new(),
        };
        Self::parse(&text, &|name| std::env::var(name).ok())
    }
}

fn public_url_is_ok(url: &str) -> bool {
    if let Some(rest) = url.strip_prefix("https://") {
        return !rest.is_empty();
    }
    let Some(rest) = url.strip_prefix("http://") else {
        return false;
    };
    let host = rest.split(['/', ':']).next().unwrap_or("");
    host == "localhost" || host == "127.0.0.1"
}

/// Reads the master key, refusing a file other users can read.
pub fn load_master_key(path: &Path) -> Result<Key, ConfigError> {
    let meta = std::fs::metadata(path).map_err(|_| ConfigError::NoMasterKey(path.to_owned()))?;
    if exposed(&meta) {
        return Err(ConfigError::MasterKeyExposed(path.to_owned()));
    }
    let text = zeroize::Zeroizing::new(
        std::fs::read_to_string(path).map_err(|_| ConfigError::NoMasterKey(path.to_owned()))?,
    );
    Key::from_hex(&text).map_err(|_| ConfigError::MasterKeyDamaged(path.to_owned()))
}

#[cfg(unix)]
fn exposed(meta: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;
    meta.permissions().mode() & 0o077 != 0
}

#[cfg(not(unix))]
fn exposed(_: &std::fs::Metadata) -> bool {
    false
}

/// Writes `key` to a new file at `path`, readable by its owner only. Never overwrites a file.
pub fn write_master_key(path: &Path, key: &Key) -> Result<(), ConfigError> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut f = options
        .open(path)
        .map_err(|_| ConfigError::Write(path.to_owned()))?;
    f.write_all(key.to_hex().as_bytes())
        .and_then(|_| f.write_all(b"\n"))
        .and_then(|_| f.sync_all())
        .map_err(|_| ConfigError::Write(path.to_owned()))
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn no_env(_: &str) -> Option<String> {
        None
    }

    #[test]
    fn the_file_and_the_environment_make_the_settings() {
        let c = Config::parse(
            "listen = \"0.0.0.0:9000\"\npublic_url = \"https://mail.example.org/\"\ndata_dir = \"/srv/c\"\n",
            &no_env,
        )
        .unwrap();
        assert_eq!(c.listen, "0.0.0.0:9000");
        assert_eq!(c.public_url, "https://mail.example.org");
        assert_eq!(c.master_key_file, PathBuf::from("/srv/c/master.key"));
        let env = |n: &str| (n == "CROFT_COMPANION_LISTEN").then(|| "127.0.0.1:1234".to_owned());
        let c = Config::parse("public_url = \"https://x.example\"", &env).unwrap();
        assert_eq!(c.listen, "127.0.0.1:1234");
    }

    #[test]
    fn unknown_keys_plain_http_and_a_missing_url_are_refused() {
        assert!(matches!(
            Config::parse("public_url = \"https://x\"\nlisten_on = \"x\"", &no_env),
            Err(ConfigError::Unreadable(_))
        ));
        assert!(Config::parse("public_url = \"http://mail.example.org\"", &no_env).is_err());
        assert!(Config::parse("public_url = \"http://localhost:8480\"", &no_env).is_ok());
        assert!(Config::parse("", &no_env).is_err());
        assert!(Config::parse("public_url = \"https://x\"\nlisten = \"nope\"", &no_env).is_err());
    }

    #[test]
    fn the_master_key_file_is_private_and_never_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("master.key");
        assert!(matches!(
            load_master_key(&p),
            Err(ConfigError::NoMasterKey(_))
        ));
        let k = Key::generate().unwrap();
        write_master_key(&p, &k).unwrap();
        assert_eq!(load_master_key(&p).unwrap().to_hex(), k.to_hex());
        assert!(
            write_master_key(&p, &Key::generate().unwrap()).is_err(),
            "never overwritten"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o644)).unwrap();
            assert!(matches!(
                load_master_key(&p),
                Err(ConfigError::MasterKeyExposed(_))
            ));
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o600)).unwrap();
        }
        std::fs::write(&p, "not a key").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o600)).unwrap();
        }
        assert!(matches!(
            load_master_key(&p),
            Err(ConfigError::MasterKeyDamaged(_))
        ));
    }
}
