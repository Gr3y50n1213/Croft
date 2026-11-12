// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! A small in-process IMAP server for fast tests (and, in [`SmtpServer`], an SMTP one, and in [`JmapServer`], a JMAP one). **Test helper, not product code**: it speaks
//! just enough IMAP4rev1 for the adapter in `croft-protocols`, listens on `127.0.0.1` only, and
//! keeps everything in memory.
//!
//! What it understands: CAPABILITY, STARTTLS, LOGIN, AUTHENTICATE XOAUTH2, ENABLE, NOOP, LOGOUT,
//! LIST (with `RETURN (SPECIAL-USE)`), STATUS, SELECT/EXAMINE (with CONDSTORE and QRESYNC),
//! UID FETCH (FLAGS, ENVELOPE, RFC822.SIZE, INTERNALDATE, header fields, `BODY[]` and
//! `BODY.PEEK[]` (only `BODY[]` sets `\Seen`), MODSEQ, CHANGEDSINCE),
//! UID STORE, UID COPY, UID MOVE, UID EXPUNGE, EXPUNGE, UID SEARCH (ALL, UID, DELETED, DRAFT, HEADER, TEXT, FROM, TO, CC, SUBJECT, OR, UNSEEN,
//! FLAGGED),
//! APPEND and CLOSE.
//!
//! What tests can do to it: switch capabilities off ([`TestServer::set_caps`]), change
//! UIDVALIDITY ([`TestServer::change_uid_validity`]), edit messages as "another client" would
//! ([`TestServer::modify_flags`], [`TestServer::remove_messages`]), cut every connection
//! ([`TestServer::drop_connections`]), make it stop answering ([`TestServer::set_hang`]), make it
//! misbehave the way a hostile or broken server would ([`TestServer::set_misbehavior`]), and read
//! the command log ([`TestServer::commands`], with passwords left out).
//!
//! Security note for reviewers: the server accepts one fixed user name and password given by the
//! test, and a plaintext listener is allowed because it only ever binds to loopback.

// This crate is test code: panics on a broken test setup are the right behaviour.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod jmap;
mod proto;
mod smtp;

pub use jmap::{Delivery, JmapOptions, JmapServer, RequestRecord, JMAP_PASSWORD, JMAP_USER};
pub use smtp::{AcceptedMessage, SmtpMisbehavior, SmtpOptions, SmtpServer, Stage};

use std::collections::BTreeSet;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex, MutexGuard};
use tokio::net::TcpListener;
use tokio::sync::watch;
use tokio_rustls::rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use tokio_rustls::rustls::{crypto::ring, version, ServerConfig};
use tokio_rustls::TlsAcceptor;

/// How the listener secures connections.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TlsMode {
    /// Plain TCP only.
    None,
    /// TLS from the first byte, with a fresh self-signed certificate for `localhost`/`127.0.0.1`.
    Implicit,
    /// Plain TCP with `STARTTLS` offered; `LOGIN` is refused until the upgrade.
    StartTls,
}

/// Which extensions the server advertises and honours. All on by default.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Caps {
    /// RFC 6851 `MOVE`.
    pub move_cmd: bool,
    /// RFC 4315 `UIDPLUS` (`UID EXPUNGE`, `COPYUID`, `APPENDUID`).
    pub uidplus: bool,
    /// RFC 6154 `SPECIAL-USE` attributes in `LIST`.
    pub special_use: bool,
    /// RFC 5258 `LIST-EXTENDED` (`RETURN (SPECIAL-USE)`).
    pub list_extended: bool,
    /// RFC 2177 `IDLE` (advertised only).
    pub idle: bool,
    /// RFC 7162 `CONDSTORE`.
    pub condstore: bool,
    /// RFC 7162 `QRESYNC` (needs `condstore` and `enable`).
    pub qresync: bool,
    /// RFC 5161 `ENABLE`.
    pub enable: bool,
}

impl Caps {
    /// Everything on.
    pub fn all() -> Self {
        Self {
            move_cmd: true,
            uidplus: true,
            special_use: true,
            list_extended: true,
            idle: true,
            condstore: true,
            qresync: true,
            enable: true,
        }
    }

    /// A server with none of the optional extensions: no MOVE, no UIDPLUS, no SPECIAL-USE.
    pub fn bare() -> Self {
        Self {
            move_cmd: false,
            uidplus: false,
            special_use: false,
            list_extended: false,
            idle: false,
            condstore: false,
            qresync: false,
            enable: false,
        }
    }
}

/// Ways the server can be made to misbehave, for tests of how the client copes with a hostile or
/// broken server. Set with [`TestServer::set_misbehavior`]; the default is no misbehaviour.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Misbehavior {
    /// Behave.
    #[default]
    None,
    /// Answer `LOGIN` and `AUTHENTICATE` with an untagged `* BYE` and close the connection, as
    /// servers do after too many failed sign-ins.
    ByeOnLogin,
    /// Refuse every sign-in with `NO [AUTHENTICATIONFAILED] Authentication failed: account
    /// unavailable`: an authentication failure whose free text contains the word "unavailable".
    AuthFailedMentioningUnavailable,
    /// Answer every `LIST` with a literal of this many bytes (`{500000000}`) and send two of them.
    HugeLiteral(u64),
    /// Answer every `LIST` with an endless stream of `* 1 EXISTS` lines and no tagged reply.
    EndlessExists,
    /// Answer every `UID SEARCH UID ...` with the UIDs asked for that exist, plus every other UID
    /// in the folder, plus one that does not exist (999999).
    SearchExtraUids,
    /// Refuse the text search keys (`TEXT`, `FROM`, `TO`, `CC`, `SUBJECT`, `OR`) with `BAD`, as a
    /// server without full-text search might. `HEADER`, `UID`, `ALL` and the flag keys still work.
    NoTextSearch,
}

/// Settings for [`TestServer::start`].
#[derive(Debug, Clone)]
pub struct ServerOptions {
    /// The one accepted login name.
    pub username: String,
    /// The one accepted password (also the accepted OAuth access token).
    pub password: String,
    /// Transport security.
    pub tls: TlsMode,
    /// Extensions on or off.
    pub caps: Caps,
}

impl Default for ServerOptions {
    fn default() -> Self {
        Self {
            username: "alice".into(),
            password: "correct horse".into(),
            tls: TlsMode::None,
            caps: Caps::all(),
        }
    }
}

/// A message as stored by the server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredMessage {
    /// The UID.
    pub uid: u32,
    /// Flags, for example `\Seen`.
    pub flags: BTreeSet<String>,
    /// The raw message text.
    pub raw: String,
}

pub(crate) struct Message {
    pub uid: u32,
    pub flags: BTreeSet<String>,
    pub raw: String,
    pub internal_date: String,
    pub modseq: u64,
}

pub(crate) struct Folder {
    pub name: String,
    pub special: Option<String>,
    pub uid_validity: u32,
    pub uid_next: u32,
    pub messages: Vec<Message>,
    pub highest_modseq: u64,
    /// `(uid, modseq at removal)` for QRESYNC.
    pub expunged: Vec<(u32, u64)>,
    pub read_only: bool,
}

impl Folder {
    fn new(name: &str, special: Option<&str>) -> Self {
        Self {
            name: name.to_string(),
            special: special.map(str::to_string),
            uid_validity: 1000,
            uid_next: 1,
            messages: Vec::new(),
            highest_modseq: 1,
            expunged: Vec::new(),
            read_only: false,
        }
    }
}

pub(crate) struct State {
    pub opts: ServerOptions,
    pub folders: Vec<Folder>,
    pub commands: Vec<String>,
    pub connections: usize,
    pub hang: bool,
    pub login_unavailable: bool,
    pub misbehavior: Misbehavior,
    /// Plain `LIST` also returns special-use attributes (as Dovecot does).
    pub plain_list_has_special_use: bool,
}

pub(crate) type Shared = Arc<Mutex<State>>;

/// A running server. Dropping it stops accepting connections.
pub struct TestServer {
    addr: SocketAddr,
    state: Shared,
    kill: watch::Sender<u64>,
    cert_der: Option<Vec<u8>>,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for TestServer {
    fn drop(&mut self) {
        self.task.abort();
        self.kill.send_modify(|g| *g += 1);
    }
}

fn tls_acceptor() -> (TlsAcceptor, Vec<u8>) {
    let ck =
        rcgen::generate_simple_self_signed(vec!["localhost".to_string(), "127.0.0.1".to_string()])
            .expect("certificate generation");
    let der = ck.cert.der().to_vec();
    let key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(ck.signing_key.serialize_der()));
    let cfg = ServerConfig::builder_with_provider(Arc::new(ring::default_provider()))
        .with_protocol_versions(&[&version::TLS13, &version::TLS12])
        .expect("protocol versions")
        .with_no_client_auth()
        .with_single_cert(vec![CertificateDer::from(der.clone())], key)
        .expect("server certificate");
    (TlsAcceptor::from(Arc::new(cfg)), der)
}

impl TestServer {
    /// Starts a server on a free `127.0.0.1` port with the standard folders: INBOX, Sent, Drafts,
    /// Trash, Junk and Archive (the last five with SPECIAL-USE attributes), all empty.
    pub async fn start(opts: ServerOptions) -> TestServer {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("local addr");
        let (acceptor, cert_der) = if opts.tls == TlsMode::None {
            (None, None)
        } else {
            let (a, d) = tls_acceptor();
            (Some(a), Some(d))
        };
        let folders = vec![
            Folder::new("INBOX", None),
            Folder::new("Sent", Some("\\Sent")),
            Folder::new("Drafts", Some("\\Drafts")),
            Folder::new("Trash", Some("\\Trash")),
            Folder::new("Junk", Some("\\Junk")),
            Folder::new("Archive", Some("\\Archive")),
        ];
        let state: Shared = Arc::new(Mutex::new(State {
            opts,
            folders,
            commands: Vec::new(),
            connections: 0,
            hang: false,
            login_unavailable: false,
            misbehavior: Misbehavior::None,
            plain_list_has_special_use: true,
        }));
        let (kill, kill_rx) = watch::channel(0u64);
        let task = {
            let state = state.clone();
            tokio::spawn(async move {
                loop {
                    let Ok((tcp, _)) = listener.accept().await else {
                        break;
                    };
                    let state = state.clone();
                    let acceptor = acceptor.clone();
                    let kill_rx = kill_rx.clone();
                    tokio::spawn(async move {
                        proto::serve_connection(tcp, state, acceptor, kill_rx).await;
                    });
                }
            })
        };
        TestServer {
            addr,
            state,
            kill,
            cert_der,
            task,
        }
    }

    /// Starts a plaintext server with default options.
    pub async fn start_default() -> TestServer {
        Self::start(ServerOptions::default()).await
    }

    fn st(&self) -> MutexGuard<'_, State> {
        self.state.lock().expect("server state lock")
    }

    /// `127.0.0.1`.
    pub fn host(&self) -> &'static str {
        "127.0.0.1"
    }

    /// The port the server listens on.
    pub fn port(&self) -> u16 {
        self.addr.port()
    }

    /// The accepted login name.
    pub fn username(&self) -> String {
        self.st().opts.username.clone()
    }

    /// The accepted password.
    pub fn password(&self) -> String {
        self.st().opts.password.clone()
    }

    /// The DER bytes of the server's certificate, when TLS is on. Tests hash these to pin it.
    pub fn certificate_der(&self) -> Option<Vec<u8>> {
        self.cert_der.clone()
    }

    /// Changes which extensions are advertised. Takes effect for the next `CAPABILITY` and so for
    /// new connections.
    pub fn set_caps(&self, change: impl FnOnce(&mut Caps)) {
        change(&mut self.st().opts.caps);
    }

    /// Whether plain `LIST` (without `RETURN`) includes special-use attributes. On by default.
    pub fn set_plain_list_special_use(&self, on: bool) {
        self.st().plain_list_has_special_use = on;
    }

    /// Adds a folder (`special` is an attribute such as `\Archive`).
    pub fn add_folder(&self, name: &str, special: Option<&str>) {
        let mut st = self.st();
        if !st.folders.iter().any(|f| f.name == name) {
            st.folders.push(Folder::new(name, special));
        }
    }

    /// Adds a message with no flags and returns its UID.
    pub fn add_message(&self, folder: &str, raw: &str) -> u32 {
        self.add_message_with_flags(folder, raw, &[])
    }

    /// Adds a message with flags and returns its UID.
    pub fn add_message_with_flags(&self, folder: &str, raw: &str, flags: &[&str]) -> u32 {
        let mut st = self.st();
        proto::append(
            &mut st,
            folder,
            flags.iter().map(|f| f.to_string()).collect(),
            raw,
        )
        .expect("folder exists")
    }

    /// Everything stored in a folder, in UID order.
    pub fn messages(&self, folder: &str) -> Vec<StoredMessage> {
        let st = self.st();
        st.folders
            .iter()
            .find(|f| f.name == folder)
            .map(|f| {
                f.messages
                    .iter()
                    .map(|m| StoredMessage {
                        uid: m.uid,
                        flags: m.flags.clone(),
                        raw: m.raw.clone(),
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// The folder's current UIDVALIDITY.
    pub fn uid_validity(&self, folder: &str) -> u32 {
        let st = self.st();
        st.folders
            .iter()
            .find(|f| f.name == folder)
            .map(|f| f.uid_validity)
            .unwrap_or(0)
    }

    /// Pretends the folder was deleted and recreated: a new UIDVALIDITY, and the messages
    /// renumbered from UID 1.
    pub fn change_uid_validity(&self, folder: &str, new_validity: u32) {
        let mut st = self.st();
        if let Some(f) = st.folders.iter_mut().find(|f| f.name == folder) {
            f.uid_validity = new_validity;
            f.highest_modseq += 1;
            let modseq = f.highest_modseq;
            for (i, m) in f.messages.iter_mut().enumerate() {
                m.uid = u32::try_from(i + 1).unwrap_or(u32::MAX);
                m.modseq = modseq;
            }
            f.uid_next = u32::try_from(f.messages.len() + 1).unwrap_or(u32::MAX);
            f.expunged.clear();
        }
    }

    /// Changes flags the way another client would (bumps the mod-sequence).
    pub fn modify_flags(&self, folder: &str, uid: u32, add: &[&str], remove: &[&str]) {
        let mut st = self.st();
        if let Some(f) = st.folders.iter_mut().find(|f| f.name == folder) {
            f.highest_modseq += 1;
            let modseq = f.highest_modseq;
            if let Some(m) = f.messages.iter_mut().find(|m| m.uid == uid) {
                for a in add {
                    m.flags.insert((*a).to_string());
                }
                for r in remove {
                    m.flags.remove(*r);
                }
                m.modseq = modseq;
            }
        }
    }

    /// Removes messages the way another client's expunge would.
    pub fn remove_messages(&self, folder: &str, uids: &[u32]) {
        let mut st = self.st();
        if let Some(f) = st.folders.iter_mut().find(|f| f.name == folder) {
            f.highest_modseq += 1;
            let modseq = f.highest_modseq;
            f.messages.retain(|m| {
                let gone = uids.contains(&m.uid);
                if gone {
                    f.expunged.push((m.uid, modseq));
                }
                !gone
            });
        }
    }

    /// Makes a folder read-only (`EXAMINE`-like even for `SELECT`).
    pub fn set_read_only(&self, folder: &str, on: bool) {
        let mut st = self.st();
        if let Some(f) = st.folders.iter_mut().find(|f| f.name == folder) {
            f.read_only = on;
        }
    }

    /// Every command line received since start (or the last [`clear_commands`](Self::clear_commands)),
    /// without the tag. Credentials never appear: `LOGIN` is logged as `LOGIN ***`.
    pub fn commands(&self) -> Vec<String> {
        self.st().commands.clone()
    }

    /// Forgets the command log.
    pub fn clear_commands(&self) {
        self.st().commands.clear();
    }

    /// How many TCP connections have been accepted.
    pub fn connection_count(&self) -> usize {
        self.st().connections
    }

    /// Closes every open client connection at once. New ones are still accepted.
    pub fn drop_connections(&self) {
        self.kill.send_modify(|g| *g += 1);
    }

    /// While on, the server reads commands but never answers them (to test timeouts).
    pub fn set_hang(&self, on: bool) {
        self.st().hang = on;
    }

    /// Makes the server misbehave in one of the ways listed in [`Misbehavior`], for every
    /// connection from now on.
    pub fn set_misbehavior(&self, how: Misbehavior) {
        self.st().misbehavior = how;
    }

    /// While on, `LOGIN` is answered `NO [UNAVAILABLE]` (a temporary failure, not a bad password).
    pub fn set_login_unavailable(&self, on: bool) {
        self.st().login_unavailable = on;
    }
}

/// A small RFC 5322 message for tests. `message_id` is given without angle brackets.
pub fn sample_message(message_id: &str, subject: &str, from: &str) -> String {
    format!(
        "From: {from}\r\nTo: bob@example.org\r\nSubject: {subject}\r\nDate: Mon, 02 Mar 2026 09:30:00 +0000\r\nMessage-ID: <{message_id}>\r\n\r\nHello, this is the body.\r\n"
    )
}

/// Like [`sample_message`] with `References` and `In-Reply-To` headers.
pub fn sample_reply(
    message_id: &str,
    subject: &str,
    from: &str,
    parent: &str,
    root: &str,
) -> String {
    format!(
        "From: {from}\r\nTo: bob@example.org\r\nSubject: {subject}\r\nDate: Tue, 03 Mar 2026 09:30:00 +0000\r\nMessage-ID: <{message_id}>\r\nReferences: <{root}> <{parent}>\r\nIn-Reply-To: <{parent}>\r\n\r\nReply body.\r\n"
    )
}
