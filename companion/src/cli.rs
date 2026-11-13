// This Source Code Form is subject to the terms of the GNU Affero General Public License,
// version 3 or later. See LICENSE-AGPL-3.0 at the repository root.

//! The command line (T23, T24): `init`, `user add`, `user enrol`, `user list`,
//! `rotate-master-key`, `serve`. Output never contains a key; `user add` and `user enrol` print a
//! one-time enrolment link, which is meant for the person it names and nobody else.

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
  user add --name NAME [--admin]   add a user and print their one-time enrolment link (24 hours)
  user enrol --name NAME           print a new enrolment link: it replaces the user's passkeys,
                                   authenticator key and recovery codes, and signs out their devices
  user list
  user delete --name NAME          delete the user and everything theirs (stop the Companion first)
  rotate-master-key [--finish] re-wrap every user's key under a new master key (--finish completes one that was cut short)
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
    /// `user enrol`: a new enrolment link for an existing user.
    UserEnrol {
        /// Login name.
        name: String,
    },
    /// `user delete`: the user and everything theirs (T34e).
    UserDelete {
        /// Login name.
        name: String,
    },
    /// `user list`
    UserList,
    /// `rotate-master-key`; `finish`: complete one that was cut short.
    RotateMasterKey {
        /// `--finish`
        finish: bool,
    },
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
        ["rotate-master-key"] => Command::RotateMasterKey { finish: false },
        ["rotate-master-key", "--finish"] => Command::RotateMasterKey { finish: true },
        ["user", "list"] => Command::UserList,
        ["user", "delete", "--name", name] if !name.trim().is_empty() => Command::UserDelete {
            name: (*name).to_owned(),
        },
        ["user", "enrol", "--name", name] if !name.trim().is_empty() => Command::UserEnrol {
            name: (*name).to_owned(),
        },
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

/// Opens the database and checks the master key (it must open every user's key). A key file
/// left by a rotation that was cut short is **never adopted automatically** (anyone who can
/// write next to the key could otherwise plant one): the operator finishes it with
/// `rotate-master-key --finish`.
pub fn open(config: &Config) -> Result<(Store, Key), CliError> {
    let store = Store::open(&config.database())?;
    let key = config::load_master_key(&config.master_key_file)?;
    match store.check_master_key(&key) {
        Ok(()) => Ok((store, key)),
        Err(e) if pending_key_path(&config.master_key_file).exists() => Err(CliError::Usage(
            format!("{e}. A master key rotation did not finish: check the key file and run `croft-companion rotate-master-key --finish`."),
        )),
        Err(e) => Err(e.into()),
    }
}

/// The lock that keeps a rotation from running while the server or another command uses the
/// keys: shared for them, exclusive for a rotation.
fn lock(config: &Config, exclusive: bool) -> Result<std::fs::File, CliError> {
    let path = config.data_dir.join("companion.lock");
    let f = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&path)
        .map_err(|_| ConfigError::Write(path.clone()))?;
    let locked = if exclusive {
        f.try_lock().is_ok()
    } else {
        f.try_lock_shared().is_ok()
    };
    if !locked {
        return Err(CliError::Usage(if exclusive {
            "the Companion is running: stop the server (and any other command) first".into()
        } else {
            "a master key rotation is running: try again when it has finished".into()
        }));
    }
    Ok(f)
}

/// Takes the shared lock for the server's lifetime (see [`lock`]).
pub fn serve_lock(config: &Config) -> Result<std::fs::File, CliError> {
    lock(config, false)
}

/// Runs a command other than `serve`; returns what to print.
pub fn run(config: &Config, command: &Command) -> Result<String, CliError> {
    match command {
        Command::Init => {
            config::create_private_dir(&config.data_dir)?;
            if let Some(dir) = config.master_key_file.parent() {
                config::create_private_dir(dir)?;
            }
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
            let _lock = lock(config, false)?;
            let (mut store, key) = open(config)?;
            let role = if *admin { Role::Admin } else { Role::User };
            let user = store.add_user(&key, name, role)?;
            let token = store.create_enrolment(user.id, None, crate::store::now_ms())?;
            Ok(format!(
                "Added user {} (id {}).\nGive them this one-time link (it works once, for 24 hours):\n{}",
                user.name,
                user.id,
                crate::api::enrolment_link(&config.public_url, &token).as_str()
            ))
        }
        Command::UserEnrol { name } => {
            let _lock = lock(config, false)?;
            let (mut store, _) = open(config)?;
            let user = store.user_by_name(name)?.ok_or(StoreError::NoSuchUser)?;
            let token = store.create_enrolment(user.id, None, crate::store::now_ms())?;
            Ok(format!(
                "Give {} this one-time link (it works once, for 24 hours). Using it replaces their \
                 passkeys, authenticator key and recovery codes and signs out their devices:\n{}",
                user.name,
                crate::api::enrolment_link(&config.public_url, &token).as_str()
            ))
        }
        Command::UserDelete { name } => {
            // Exclusive: the Companion must be stopped, so no running process keeps the user's
            // mail open (a running one would also find their sessions gone at once).
            let _lock = lock(config, true)?;
            let (mut store, _) = open(config)?;
            let user = store.user_by_name(name)?.ok_or(StoreError::NoSuchUser)?;
            match store.delete_user(user.id, crate::store::now_ms())? {
                crate::auth::DeleteSelf::Deleted => {
                    let cores = crate::mail::MailCores::new(config.data_dir.clone());
                    let _ = cores.sweep(&store.user_ids()?);
                    let left = cores.database(user.id).parent().is_some_and(|d| d.exists());
                    Ok(format!(
                        "Deleted user {} and everything theirs: sessions, passkeys, mail accounts{}.",
                        user.name,
                        if left {
                            " (their mail folder could not be removed yet: it goes at the next start)"
                        } else {
                            " and their mail on this Companion"
                        }
                    ))
                }
                _ => Err(CliError::Usage(
                    "That is the only admin: add another admin first (user add --name NAME --admin)."
                        .to_owned(),
                )),
            }
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
        Command::RotateMasterKey { finish: true } => {
            let _lock = lock(config, true)?;
            let pending = pending_key_path(&config.master_key_file);
            let store = Store::open(&config.database())?;
            let new = config::load_master_key(&pending)?;
            // Only a key that opens every user's key is adopted.
            store.check_master_key(&new)?;
            std::fs::rename(&pending, &config.master_key_file)
                .map_err(|_| ConfigError::Write(config.master_key_file.clone()))?;
            config::sync_parent(&config.master_key_file);
            Ok("Finished the master key rotation. Back up the key file.".into())
        }
        Command::RotateMasterKey { finish: false } => {
            let _lock = lock(config, true)?;
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
            config::sync_parent(&config.master_key_file);
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
            master_key_file: dir.join("keys").join("master.key"),
            trusted_proxies: Vec::new(),
            oidc: None,
            mail_allowed_networks: Vec::new(),
            web_dir: None,
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
    fn user_add_and_user_enrol_print_a_one_time_link_with_the_token_in_the_fragment() {
        let dir = tempfile::tempdir().unwrap();
        let c = config(dir.path());
        run(&c, &Command::Init).unwrap();
        let out = run(
            &c,
            &Command::UserAdd {
                name: "ann".into(),
                admin: false,
            },
        )
        .unwrap();
        let link = |out: &str| {
            out.lines()
                .find(|l| l.starts_with("http"))
                .unwrap()
                .to_owned()
        };
        let first = link(&out);
        assert!(first.starts_with("http://localhost/enrol#"), "{first}");
        let token = first.split('#').nth(1).unwrap();
        assert_eq!(token.len(), 43);
        let (mut store, _) = open(&c).unwrap();
        let now = crate::store::now_ms();
        assert!(store.enrolment_target(token, now).unwrap().is_some());
        drop(store);
        // A new link replaces the old one.
        let out = run(&c, &Command::UserEnrol { name: "ann".into() }).unwrap();
        let second = link(&out);
        assert_ne!(first, second);
        let (mut store, _) = open(&c).unwrap();
        assert!(store.enrolment_target(token, now).unwrap().is_none());
        drop(store);
        assert!(run(
            &c,
            &Command::UserEnrol {
                name: "nobody".into()
            }
        )
        .is_err());
        for bad in ["", "a b", "ann\u{0}", "émile", &"x".repeat(65)] {
            assert!(
                run(
                    &c,
                    &Command::UserAdd {
                        name: bad.to_owned(),
                        admin: false
                    }
                )
                .is_err(),
                "{bad:?}"
            );
        }
        assert_eq!(
            parse(&args("user enrol --name ann")).unwrap().1,
            Command::UserEnrol { name: "ann".into() }
        );
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
        run(&c, &Command::RotateMasterKey { finish: false }).unwrap();
        let after = std::fs::read_to_string(&c.master_key_file).unwrap();
        assert_ne!(before, after);
        let (store, key) = open(&c).unwrap();
        store.data_key(&key, 1).unwrap();
        drop(store);

        // A crash after the database moved but before the rename: the old file is still in
        // place and `.new` holds the key in force. Nothing is adopted until `--finish`.
        let pending = pending_key_path(&c.master_key_file);
        let (mut store, old) = open(&c).unwrap();
        let new = Key::generate().unwrap();
        config::write_master_key(&pending, &new).unwrap();
        store.rotate_master_key(&old, &new).unwrap();
        drop(store);
        assert!(matches!(open(&c), Err(CliError::Usage(m)) if m.contains("--finish")));
        run(&c, &Command::RotateMasterKey { finish: true }).unwrap();
        let (store, key) = open(&c).unwrap();
        assert_eq!(key.to_hex(), new.to_hex());
        assert!(!pending.exists());
        store.data_key(&key, 1).unwrap();
    }

    #[test]
    fn a_planted_key_file_is_never_adopted() {
        // Review finding: someone who can write next to the key plants `.new` and a matching
        // check value; the real key must stay, and `--finish` must refuse a key that does not
        // open the users' keys.
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
        let real = std::fs::read_to_string(&c.master_key_file).unwrap();
        let planted = Key::generate().unwrap();
        config::write_master_key(&pending_key_path(&c.master_key_file), &planted).unwrap();
        let sealed = planted
            .seal(
                crate::crypto::MASTER_CHECK_AAD,
                b"croft-companion master key",
            )
            .unwrap();
        let conn = rusqlite::Connection::open(c.database()).unwrap();
        conn.execute(
            "UPDATE meta SET value = ?1 WHERE key = 'master_check'",
            rusqlite::params![sealed],
        )
        .unwrap();
        drop(conn);
        assert!(open(&c).is_err());
        assert!(run(&c, &Command::RotateMasterKey { finish: true }).is_err());
        assert_eq!(
            std::fs::read_to_string(&c.master_key_file).unwrap(),
            real,
            "the real key is kept"
        );
    }

    #[test]
    fn a_rotation_waits_for_the_server_to_stop() {
        let dir = tempfile::tempdir().unwrap();
        let c = config(dir.path());
        run(&c, &Command::Init).unwrap();
        let server = serve_lock(&c).unwrap();
        assert!(
            matches!(run(&c, &Command::RotateMasterKey { finish: false }), Err(CliError::Usage(m)) if m.contains("stop the server"))
        );
        drop(server);
        run(&c, &Command::RotateMasterKey { finish: false }).unwrap();
    }

    #[test]
    fn user_delete_removes_the_user_and_their_mail_but_never_the_last_admin_or_while_serving() {
        let dir = tempfile::tempdir().unwrap();
        let c = config(dir.path());
        run(&c, &Command::Init).unwrap();
        for (name, admin) in [("ann", true), ("bob", false)] {
            run(
                &c,
                &Command::UserAdd {
                    name: name.into(),
                    admin,
                },
            )
            .unwrap();
        }
        let bob_mail = c.data_dir.join("users").join("2");
        std::fs::create_dir_all(&bob_mail).unwrap();
        std::fs::write(bob_mail.join("mail.db"), b"made-up").unwrap();
        let delete = |name: &str| Command::UserDelete { name: name.into() };
        // Not while the server runs.
        let server = serve_lock(&c).unwrap();
        assert!(
            matches!(run(&c, &delete("bob")), Err(CliError::Usage(m)) if m.contains("stop the server"))
        );
        drop(server);
        assert!(run(&c, &delete("bob"))
            .unwrap()
            .contains("Deleted user bob"));
        assert!(!bob_mail.exists());
        assert_eq!(run(&c, &Command::UserList).unwrap(), "1\tann\tadmin\n");
        // The only admin stays; an unknown name is an error.
        assert!(
            matches!(run(&c, &delete("ann")), Err(CliError::Usage(m)) if m.contains("only admin"))
        );
        assert!(run(&c, &delete("nobody")).is_err());
        assert_eq!(
            parse(&args("user delete --name bob")).unwrap().1,
            Command::UserDelete { name: "bob".into() }
        );
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
