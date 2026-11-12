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
    /// A secret file (the OIDC client secret) cannot be used: the path and why.
    SecretFile(PathBuf, &'static str),
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
            Self::SecretFile(p, why) => write!(f, "the secret file {} {why}", p.display()),
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
    trusted_proxies: Option<Vec<String>>,
    oidc: Option<FileOidc>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct FileOidc {
    issuer: Option<String>,
    client_id: Option<String>,
    client_secret_file: Option<PathBuf>,
    name: Option<String>,
    create_users: Option<bool>,
    required_group: Option<String>,
    groups_claim: Option<String>,
}

/// Sign-in through an OpenID Connect provider (T25).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OidcSettings {
    /// The provider's issuer URL, exactly as its discovery document gives it (`https://`).
    pub issuer: String,
    /// The Companion's client id at the provider.
    pub client_id: String,
    /// The file holding the client secret (outside the data folder, mode 0600).
    pub client_secret_file: PathBuf,
    /// The provider's name on the sign-in button.
    pub name: String,
    /// Whether a first sign-in by an unknown person creates a user (only with `required_group`).
    pub create_users: bool,
    /// The group a new person must be in (read from `groups_claim`).
    pub required_group: Option<String>,
    /// The ID token claim that lists the person's groups.
    pub groups_claim: String,
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
    /// The reverse proxies whose `X-Forwarded-For` is believed (for the client address that rate
    /// limits count by). Empty: the address of the connection itself.
    pub trusted_proxies: Vec<std::net::IpAddr>,
    /// Sign-in through an OpenID Connect provider, if set up.
    pub oidc: Option<OidcSettings>,
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
            .unwrap_or_else(|| PathBuf::from("/keys/master.key"));
        // The key next to the database would put both in one backup, and anyone who can write the
        // data folder could replace it.
        if master_key_file.starts_with(&data_dir) {
            return Err(ConfigError::Invalid(
                "master_key_file must be outside data_dir (keep the key on another volume)",
            ));
        }
        if listen.parse::<std::net::SocketAddr>().is_err() {
            return Err(ConfigError::Invalid(
                "listen must be an address and port, such as 0.0.0.0:8480",
            ));
        }
        let proxies: Vec<String> = match env("CROFT_COMPANION_TRUSTED_PROXIES") {
            Some(list) => list
                .split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
                .collect(),
            None => file.trusted_proxies.unwrap_or_default(),
        };
        let trusted_proxies = proxies
            .iter()
            .map(|p| p.parse::<std::net::IpAddr>())
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| {
                ConfigError::Invalid("trusted_proxies must be IP addresses, such as 172.18.0.2")
            })?;
        let oidc = oidc_settings(file.oidc, env, &data_dir)?;
        if !public_url_is_ok(&public_url) {
            return Err(ConfigError::Invalid(
                "public_url must be https:// and a domain name (plain http only for localhost; no IP address)",
            ));
        }
        Ok(Self {
            listen,
            public_url: public_url.trim_end_matches('/').to_owned(),
            data_dir,
            master_key_file,
            trusted_proxies,
            oidc,
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

fn oidc_settings(
    file: Option<FileOidc>,
    env: &dyn Fn(&str) -> Option<String>,
    data_dir: &Path,
) -> Result<Option<OidcSettings>, ConfigError> {
    let file = file.unwrap_or_default();
    let issuer = env("CROFT_COMPANION_OIDC_ISSUER").or(file.issuer);
    let client_id = env("CROFT_COMPANION_OIDC_CLIENT_ID").or(file.client_id);
    let secret = env("CROFT_COMPANION_OIDC_CLIENT_SECRET_FILE")
        .map(PathBuf::from)
        .or(file.client_secret_file);
    let (issuer, client_id, client_secret_file) = match (issuer, client_id, secret) {
        (None, None, None) => return Ok(None),
        (Some(i), Some(c), Some(s)) => (i, c, s),
        _ => {
            return Err(ConfigError::Invalid(
                "oidc needs issuer, client_id and client_secret_file together",
            ))
        }
    };
    // The issuer is compared character for character with the provider's own: no trimming.
    if !issuer.starts_with("https://") || issuer.contains(['?', '#', '@']) || issuer.len() > 512 {
        return Err(ConfigError::Invalid(
            "oidc.issuer must be the provider's https:// issuer URL",
        ));
    }
    if client_id.trim().is_empty() || client_id.len() > 256 {
        return Err(ConfigError::Invalid("oidc.client_id is empty or too long"));
    }
    if client_secret_file.starts_with(data_dir) {
        return Err(ConfigError::Invalid(
            "oidc.client_secret_file must be outside data_dir",
        ));
    }
    let create_users = file.create_users.unwrap_or(false);
    if create_users && file.required_group.is_none() {
        return Err(ConfigError::Invalid(
            "oidc.create_users needs oidc.required_group (only members of that group are created)",
        ));
    }
    Ok(Some(OidcSettings {
        issuer,
        client_id,
        client_secret_file,
        name: file
            .name
            .filter(|n| !n.trim().is_empty())
            .unwrap_or_else(|| "single sign-on".to_owned()),
        create_users,
        required_group: file.required_group,
        groups_claim: file.groups_claim.unwrap_or_else(|| "groups".to_owned()),
    }))
}

fn public_url_is_ok(url: &str) -> bool {
    let (secure, rest) = if let Some(r) = url.strip_prefix("https://") {
        (true, r)
    } else if let Some(r) = url.strip_prefix("http://") {
        (false, r)
    } else {
        return false;
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    // No user name or password in the address (`http://localhost:x@evil.example`).
    if authority.is_empty() || authority.contains('@') {
        return false;
    }
    let host = authority.rsplit_once(':').map_or(authority, |(h, port)| {
        if port.bytes().all(|b| b.is_ascii_digit()) {
            h
        } else {
            authority
        }
    });
    // Passkeys are bound to a domain name: an IP address (v4, or v6 in brackets) cannot be used.
    if host.starts_with('[') || host.parse::<std::net::IpAddr>().is_ok() {
        return false;
    }
    secure || host == "localhost"
}

/// Reads the master key. Refused: a symbolic link, a file other users can read, and a file that
/// changed between the check and the read (the check is made on the opened file itself).
pub fn load_master_key(path: &Path) -> Result<Key, ConfigError> {
    let text = read_private_file(path, 256).map_err(|e| match e {
        PrivateFile::Missing => ConfigError::NoMasterKey(path.to_owned()),
        PrivateFile::Exposed => ConfigError::MasterKeyExposed(path.to_owned()),
        PrivateFile::Unreadable => ConfigError::MasterKeyDamaged(path.to_owned()),
    })?;
    Key::from_hex(&text).map_err(|_| ConfigError::MasterKeyDamaged(path.to_owned()))
}

/// Reads a secret kept in a file (the OIDC client secret, T25) with the master key's rules: no
/// symbolic link, nobody else may read it, checked on the opened file. Surrounding white space is
/// dropped; an empty file is refused.
pub fn load_secret_file(path: &Path) -> Result<zeroize::Zeroizing<String>, ConfigError> {
    let text = read_private_file(path, 4096).map_err(|e| match e {
        PrivateFile::Missing => ConfigError::SecretFile(path.to_owned(), "is missing"),
        PrivateFile::Exposed => ConfigError::SecretFile(
            path.to_owned(),
            "can be read by other users or is a link; run `chmod 600` on the file itself",
        ),
        PrivateFile::Unreadable => ConfigError::SecretFile(path.to_owned(), "cannot be read"),
    })?;
    let trimmed = zeroize::Zeroizing::new(text.trim().to_owned());
    if trimmed.is_empty() {
        return Err(ConfigError::SecretFile(path.to_owned(), "is empty"));
    }
    Ok(trimmed)
}

enum PrivateFile {
    Missing,
    Exposed,
    Unreadable,
}

fn read_private_file(path: &Path, max: u64) -> Result<zeroize::Zeroizing<String>, PrivateFile> {
    use std::io::Read;
    let link = std::fs::symlink_metadata(path).map_err(|_| PrivateFile::Missing)?;
    if link.file_type().is_symlink() || !link.is_file() {
        return Err(PrivateFile::Exposed);
    }
    let file = std::fs::File::open(path).map_err(|_| PrivateFile::Missing)?;
    let opened = file.metadata().map_err(|_| PrivateFile::Missing)?;
    if exposed(&opened) || !same_file(&link, &opened) {
        return Err(PrivateFile::Exposed);
    }
    let mut text = zeroize::Zeroizing::new(String::new());
    file.take(max)
        .read_to_string(&mut text)
        .map_err(|_| PrivateFile::Unreadable)?;
    Ok(text)
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

#[cfg(unix)]
fn same_file(a: &std::fs::Metadata, b: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;
    a.dev() == b.dev() && a.ino() == b.ino()
}

#[cfg(not(unix))]
fn same_file(_: &std::fs::Metadata, _: &std::fs::Metadata) -> bool {
    true
}

/// Makes the file's folder entry durable (a key file that vanished after a power cut would lose
/// every user's data).
pub fn sync_parent(path: &Path) {
    if let Some(dir) = path.parent() {
        if let Ok(d) = std::fs::File::open(dir) {
            let _ = d.sync_all();
        }
    }
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
        .map_err(|_| ConfigError::Write(path.to_owned()))?;
    sync_parent(path);
    Ok(())
}

/// Creates a folder (and its parents) readable by the owner only.
pub fn create_private_dir(path: &Path) -> Result<(), ConfigError> {
    let mut b = std::fs::DirBuilder::new();
    b.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        b.mode(0o700);
    }
    b.create(path)
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
        assert_eq!(c.master_key_file, PathBuf::from("/keys/master.key"));
        let env = |n: &str| (n == "CROFT_COMPANION_LISTEN").then(|| "127.0.0.1:1234".to_owned());
        let c = Config::parse("public_url = \"https://x.example\"", &env).unwrap();
        assert_eq!(c.listen, "127.0.0.1:1234");
        assert!(c.trusted_proxies.is_empty());
    }

    #[test]
    fn oidc_settings_come_together_or_not_at_all() {
        let c = Config::parse("public_url = \"https://x.example\"", &no_env).unwrap();
        assert!(c.oidc.is_none());
        let full = "public_url = \"https://x.example\"\n[oidc]\nissuer = \"https://id.example/realms/a\"\nclient_id = \"croft\"\nclient_secret_file = \"/keys/oidc\"\n";
        let o = Config::parse(full, &no_env).unwrap().oidc.unwrap();
        assert_eq!(o.issuer, "https://id.example/realms/a");
        assert!(!o.create_users);
        assert_eq!(o.groups_claim, "groups");
        for bad in [
            "public_url = \"https://x.example\"\n[oidc]\nissuer = \"https://id.example\"\n",
            &full.replace("https://id.example/realms/a", "http://id.example"),
            &full.replace("/keys/oidc", "/data/oidc"),
            &format!("{full}create_users = true\n"),
            &format!("{full}unknown = 1\n"),
        ] {
            assert!(Config::parse(bad, &no_env).is_err(), "{bad}");
        }
        assert!(Config::parse(
            &format!("{full}create_users = true\nrequired_group = \"mail\"\n"),
            &no_env
        )
        .is_ok());
    }

    #[test]
    fn a_secret_file_follows_the_master_keys_rules() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("secret");
        assert!(load_secret_file(&p).is_err());
        std::fs::write(&p, "  s3cret-made-up\n").unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(load_secret_file(&p).is_err(), "readable by others");
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert_eq!(load_secret_file(&p).unwrap().as_str(), "s3cret-made-up");
        let link = dir.path().join("link");
        std::os::unix::fs::symlink(&p, &link).unwrap();
        assert!(load_secret_file(&link).is_err(), "a link");
        let msg = load_secret_file(&link).unwrap_err().to_string();
        assert!(!msg.contains("s3cret"));
    }

    #[test]
    fn trusted_proxies_are_addresses_from_the_file_or_the_environment() {
        let c = Config::parse(
            "public_url = \"https://x.example\"\ntrusted_proxies = [\"127.0.0.1\", \"::1\"]",
            &no_env,
        )
        .unwrap();
        assert_eq!(c.trusted_proxies.len(), 2);
        let env = |n: &str| {
            (n == "CROFT_COMPANION_TRUSTED_PROXIES").then(|| "172.18.0.2, 10.0.0.1".to_owned())
        };
        let c = Config::parse("public_url = \"https://x.example\"", &env).unwrap();
        assert_eq!(
            c.trusted_proxies,
            vec![
                "172.18.0.2".parse::<std::net::IpAddr>().unwrap(),
                "10.0.0.1".parse().unwrap()
            ]
        );
        assert!(Config::parse(
            "public_url = \"https://x.example\"\ntrusted_proxies = [\"proxy.local\"]",
            &no_env
        )
        .is_err());
    }

    #[test]
    fn unknown_keys_plain_http_and_a_missing_url_are_refused() {
        assert!(matches!(
            Config::parse("public_url = \"https://x\"\nlisten_on = \"x\"", &no_env),
            Err(ConfigError::Unreadable(_))
        ));
        assert!(Config::parse("public_url = \"http://mail.example.org\"", &no_env).is_err());
        assert!(Config::parse("public_url = \"http://localhost:8480\"", &no_env).is_ok());
        // Passkeys need a domain name (review finding: the settings took an address `serve` refused).
        for ip in [
            "http://127.0.0.1:8480",
            "https://192.0.2.1",
            "https://[2001:db8::1]:443",
        ] {
            assert!(
                Config::parse(&format!("public_url = \"{ip}\""), &no_env).is_err(),
                "{ip}"
            );
        }
        for bad in [
            "http://localhost:x@evil.example/",
            "https://user@mail.example.org",
            "http://localhost.evil.example",
            "http://127.0.0.1.evil",
        ] {
            assert!(
                Config::parse(&format!("public_url = \"{bad}\""), &no_env).is_err(),
                "{bad}"
            );
        }
        assert!(
            Config::parse(
                "public_url = \"https://x\"\ndata_dir = \"/srv\"\nmaster_key_file = \"/srv/k\"",
                &no_env
            )
            .is_err(),
            "key inside data_dir"
        );
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
        // A symbolic link to a key is refused.
        #[cfg(unix)]
        {
            let link = dir.path().join("link.key");
            std::os::unix::fs::symlink(&p, &link).unwrap();
            assert!(matches!(
                load_master_key(&link),
                Err(ConfigError::MasterKeyExposed(_))
            ));
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
