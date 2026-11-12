// This Source Code Form is subject to the terms of the GNU Affero General Public License,
// version 3 or later. See LICENSE-AGPL-3.0 at the repository root.

//! The command line (T23): `init`, `user add`, `user list`, `rotate-master-key`, `serve`.
//! Output never contains a key.

use std::path::{Path, PathBuf};

use crate::config::{self, Config, ConfigError};
use crate::crypto::Key;
use crate::store::{Role, Store, StoreError};

/// Why a command failed (shown to the operator; never a key).
#[derive(Debug)]
pub enum CliError {
    /// Bad arguments; the usage text.
    Usage(String),
    /// Settings or the master key.
    Config(ConfigError),
    /// The database.
    Store(StoreError),
    /// The server stopped with an error.
    Serve(String),
}

impl std::fmt::Display for CliError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Usage(u) => f.write_str(u),
            Self::Config(e) => write!(f, "{e}"),
            Self::Store(e) => write!(f, "{e}"),
            Self::Serve(e) => write!(f, "the server stopped: {e}"),
        }
    }
}

impl std::error::Error for CliError {}

impl From<ConfigError> for CliError {
    fn from(e: ConfigError) -> Self {
        Self::Config(e)
    }
}

impl From<StoreError> for CliError {
    fn from(e: StoreError) -> Self {
        Self::Store(e)
    }
}

/// The usage text.
pub const USAGE: &str = "usage: croft-companion [--config FILE] <command>
commands:
  init                         create the data folder, the master key and the database (never overwrites a key)
  user add --name NAME [--admin]
  user list
  rotate-master-key            re-wrap every user's key under a new master key
  serve                        run the server
  healthcheck                  ask the running server's /healthz (for the container's health check)";

/// The parsed command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    /// `init`
    Init,
    /// `user add`
    UserAdd {
        /// Login name.
        name: String,
        /// Admin role.
        admin: bool,
    },
    /// `user list`
    UserList,
    /// `rotate-master-key`
    RotateMasterKey,
    /// `serve`
    Serve,
    /// `healthcheck`
    Healthcheck,
}

/// Parses the arguments (without the program name): the settings file and the command.
pub fn parse(args: &[String]) -> Result<(Option<PathBuf>, Command), CliError> {
    let usage = || CliError::Usage(USAGE.to_owned());
    let mut config = None;
    let mut rest: Vec<&str> = Vec::new();
    let mut i = 0;
    while i < args.len() {
        if args[i] == "--config" {
            config = Some(PathBuf::from(args.get(i + 1).ok_or_else(usage)?));
            i += 2;
        } else {
            rest.push(&args[i]);
            i += 1;
        }
    }
    let command = match rest.as_slice() {
        ["init"] => Command::Init,
        ["serve"] => Command::Serve,
        ["healthcheck"] => Command::Healthcheck,
        ["rotate-master-key"] => Command::RotateMasterKey,
        ["user", "list"] => Command::UserList,
        ["user", "add", tail @ ..] => {
            let mut name = None;
            let mut admin = false;
            let mut j = 0;
            while j < tail.len() {
                match tail[j] {
                    "--name" => {
                        name = tail.get(j + 1).map(|s| (*s).to_owned());
                        j += 2;
                    }
                    "--admin" => {
                        admin = true;
                        j += 1;
                    }
                    _ => return Err(usage()),
                }
            }
            let name = name.filter(|n| !n.trim().is_empty()).ok_or_else(usage)?;
            Command::UserAdd { name, admin }
        }
        _ => return Err(usage()),
    };
    Ok((config, command))
}

fn pending_key_path(key: &Path) -> PathBuf {
    let mut p = key.as_os_str().to_owned();
    p.push(".new");
    PathBuf::from(p)
}

/// Opens the database and checks the master key. Finishes a rotation that was cut short after
/// the database had moved to the new key (`<key file>.new` is then the key in force).
pub fn open(config: &Config) -> Result<(Store, Key), CliError> {
    let store = Store::open(&config.database())?;
    let key = config::load_master_key(&config.master_key_file)?;
    match store.check_master_key(&key) {
        Ok(()) => Ok((store, key)),
        Err(e) => {
            let pending = pending_key_path(&config.master_key_file);
            if let Ok(new) = config::load_master_key(&pending) {
                if store.check_master_key(&new).is_ok() {
                    std::fs::rename(&pending, &config.master_key_file)
                        .map_err(|_| ConfigError::Write(config.master_key_file.clone()))?;
                    return Ok((store, new));
                }
            }
            Err(e.into())
        }
    }
}

/// Runs a command other than `serve`; returns what to print.
pub fn run(config: &Config, command: &Command) -> Result<String, CliError> {
    match command {
        Command::Init => {
            std::fs::create_dir_all(&config.data_dir)
                .map_err(|_| ConfigError::Write(config.data_dir.clone()))?;
            let created = if config.master_key_file.exists() {
                false
            } else {
                config::write_master_key(
                    &config.master_key_file,
                    &Key::generate().map_err(StoreError::from)?,
                )?;
                true
            };
            let (store, _) = open(config)?;
            drop(store);
            Ok(if created {
                format!(
                    "Created the master key at {}. Back it up somewhere safe: without it no user's data can be read.",
                    config.master_key_file.display()
                )
            } else {
                "Already set up: the master key was kept.".to_owned()
            })
        }
        Command::UserAdd { name, admin } => {
            let (mut store, key) = open(config)?;
            let role = if *admin { Role::Admin } else { Role::User };
            let user = store.add_user(&key, name, role)?;
            Ok(format!("Added user {} (id {}).", user.name, user.id))
        }
        Command::UserList => {
            let (store, _) = open(config)?;
            let mut out = String::new();
            for u in store.users()? {
                out.push_str(&format!(
                    "{}\t{}\t{}{}\n",
                    u.id,
                    u.name,
                    if u.role == Role::Admin {
                        "admin"
                    } else {
                        "user"
                    },
                    if u.disabled { "\tdisabled" } else { "" }
                ));
            }
            Ok(out)
        }
        Command::RotateMasterKey => {
            let (mut store, old) = open(config)?;
            let new = Key::generate().map_err(StoreError::from)?;
            let pending = pending_key_path(&config.master_key_file);
            // A leftover from a rotation that never reached the database: not the key in force.
            let _ = std::fs::remove_file(&pending);
            // The new key is on disk before the database moves to it, so a crash in between
            // leaves a key for every state (see `open`).
            config::write_master_key(&pending, &new)?;
            let n = store.rotate_master_key(&old, &new)?;
            std::fs::rename(&pending, &config.master_key_file)
                .map_err(|_| ConfigError::Write(config.master_key_file.clone()))?;
            Ok(format!(
                "Rotated the master key ({n} user keys re-wrapped). Back up the new key file; the old one opens nothing now."
            ))
        }
        Command::Serve => Err(CliError::Usage(USAGE.to_owned())),
        Command::Healthcheck => healthcheck(&config.listen),
    }
}

/// Asks the server on `listen` (on this machine) for `/healthz`.
fn healthcheck(listen: &str) -> Result<String, CliError> {
    use std::io::{Read, Write};
    let down = || CliError::Serve("the server is not answering".into());
    let mut addr: std::net::SocketAddr = listen.parse().map_err(|_| down())?;
    if addr.ip().is_unspecified() {
        addr.set_ip(std::net::Ipv4Addr::LOCALHOST.into());
    }
    let timeout = std::time::Duration::from_secs(3);
    let mut s = std::net::TcpStream::connect_timeout(&addr, timeout).map_err(|_| down())?;
    s.set_read_timeout(Some(timeout)).map_err(|_| down())?;
    s.write_all(b"GET /healthz HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
        .map_err(|_| down())?;
    let mut answer = String::new();
    let _ = s.take(4096).read_to_string(&mut answer);
    if answer.starts_with("HTTP/1.1 200") {
        Ok("ok".into())
    } else {
        Err(down())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn config(dir: &Path) -> Config {
        Config {
            listen: "127.0.0.1:0".into(),
            public_url: "http://localhost".into(),
            data_dir: dir.join("data"),
            master_key_file: dir.join("data").join("master.key"),
        }
    }

    fn args(s: &str) -> Vec<String> {
        s.split_whitespace().map(str::to_owned).collect()
    }

    #[test]
    fn the_arguments_parse_and_bad_ones_print_the_usage() {
        assert_eq!(parse(&args("init")).unwrap(), (None, Command::Init));
        assert_eq!(
            parse(&args("--config /etc/c.toml user add --name ann --admin")).unwrap(),
            (
                Some(PathBuf::from("/etc/c.toml")),
                Command::UserAdd {
                    name: "ann".into(),
                    admin: true
                }
            )
        );
        for bad in [
            "",
            "user add",
            "user add --name",
            "frobnicate",
            "user add --name a --root",
        ] {
            assert!(
                matches!(parse(&args(bad)), Err(CliError::Usage(_))),
                "{bad}"
            );
        }
    }

    #[test]
    fn init_twice_keeps_the_key_and_users_are_listed_without_secrets() {
        let dir = tempfile::tempdir().unwrap();
        let c = config(dir.path());
        assert!(run(&c, &Command::Init)
            .unwrap()
            .contains("Created the master key"));
        let key = std::fs::read_to_string(&c.master_key_file).unwrap();
        assert!(run(&c, &Command::Init).unwrap().contains("kept"));
        assert_eq!(std::fs::read_to_string(&c.master_key_file).unwrap(), key);
        run(
            &c,
            &Command::UserAdd {
                name: "ann".into(),
                admin: true,
            },
        )
        .unwrap();
        run(
            &c,
            &Command::UserAdd {
                name: "bob".into(),
                admin: false,
            },
        )
        .unwrap();
        let list = run(&c, &Command::UserList).unwrap();
        assert_eq!(list, "1\tann\tadmin\n2\tbob\tuser\n");
        assert!(!list.contains(key.trim()));
    }

    #[test]
    fn rotation_replaces_the_key_file_and_a_cut_short_rotation_finishes_at_the_next_start() {
        let dir = tempfile::tempdir().unwrap();
        let c = config(dir.path());
        run(&c, &Command::Init).unwrap();
        run(
            &c,
            &Command::UserAdd {
                name: "ann".into(),
                admin: false,
            },
        )
        .unwrap();
        let before = std::fs::read_to_string(&c.master_key_file).unwrap();
        run(&c, &Command::RotateMasterKey).unwrap();
        let after = std::fs::read_to_string(&c.master_key_file).unwrap();
        assert_ne!(before, after);
        let (store, key) = open(&c).unwrap();
        store.data_key(&key, 1).unwrap();
        drop(store);

        // A crash after the database moved but before the rename: the old file is still in
        // place and `.new` holds the key in force.
        let pending = pending_key_path(&c.master_key_file);
        let (mut store, old) = open(&c).unwrap();
        let new = Key::generate().unwrap();
        config::write_master_key(&pending, &new).unwrap();
        store.rotate_master_key(&old, &new).unwrap();
        drop(store);
        let (store, key) = open(&c).unwrap();
        assert_eq!(key.to_hex(), new.to_hex());
        assert!(!pending.exists());
        store.data_key(&key, 1).unwrap();
    }

    #[test]
    fn a_wrong_master_key_opens_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let c = config(dir.path());
        run(&c, &Command::Init).unwrap();
        std::fs::remove_file(&c.master_key_file).unwrap();
        config::write_master_key(&c.master_key_file, &Key::generate().unwrap()).unwrap();
        assert!(matches!(
            open(&c),
            Err(CliError::Store(StoreError::Crypto(_)))
        ));
    }
}
