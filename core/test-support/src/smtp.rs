// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! A small in-process SMTP submission server for fast tests. **Test helper, not product code**:
//! it listens on `127.0.0.1` only and keeps everything in memory.
//!
//! It understands EHLO/HELO, STARTTLS, AUTH PLAIN, LOGIN and XOAUTH2 (one fixed user name and
//! password, the password doubling as the accepted OAuth token), MAIL FROM, RCPT TO, DATA (with
//! dot-unstuffing), RSET, NOOP and QUIT. Tests can make any stage answer with a chosen reply
//! ([`SmtpServer::set_reply`], [`SmtpServer::set_rcpt_reply`]), make it misbehave the way a
//! crashing or hostile server would ([`SmtpServer::set_misbehavior`]), read the log of accepted
//! messages ([`SmtpServer::accepted`]) and the command log ([`SmtpServer::commands`], with sign-in
//! data left out).
//!
//! Security note for reviewers: a plaintext listener is allowed because it only ever binds to
//! loopback.

use crate::{tls_acceptor, TlsMode};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex, MutexGuard};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::watch;
use tokio_rustls::TlsAcceptor;

/// A point in the conversation where a test can force a reply.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Stage {
    /// The `AUTH` command (a forced reply replaces the whole exchange).
    Auth,
    /// `MAIL FROM`.
    MailFrom,
    /// Every `RCPT TO` that has no address-specific reply.
    RcptTo,
    /// The `DATA` command (a forced reply other than 354 ends the exchange for this message).
    Data,
    /// The reply after the final `.` line. A non-2xx reply means the message is not kept.
    EndOfData,
}

/// Ways the server can misbehave, for tests of how the sender copes. Set with
/// [`SmtpServer::set_misbehavior`]; the default is no misbehaviour.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SmtpMisbehavior {
    /// Behave.
    #[default]
    None,
    /// Read the whole message, then never answer (the connection stays open).
    HangAfterData,
    /// Read the whole message, **keep it** as accepted, then close the connection without a
    /// reply: the server took the message and the client cannot know.
    AcceptThenDrop,
    /// Read the whole message, **discard** it, then close the connection without a reply.
    DropWithoutAccepting,
    /// Answer the greeting with `554` instead of `220`.
    RefuseGreeting,
    /// After the `220` that answers `STARTTLS`, send extra plaintext bytes (a downgrade or
    /// injection attempt).
    InjectAfterStartTls,
    /// Close the connection as soon as the client sends an `AUTH` command (servers do this after
    /// too many failed attempts).
    HangUpOnAuth,
}

/// Settings for [`SmtpServer::start`].
#[derive(Debug, Clone)]
pub struct SmtpOptions {
    /// The one accepted login name.
    pub username: String,
    /// The one accepted password (also the accepted OAuth access token).
    pub password: String,
    /// Transport security (see [`TlsMode`]). With [`TlsMode::StartTls`], `AUTH` is refused and
    /// not advertised until the upgrade.
    pub tls: TlsMode,
    /// MAIL FROM is refused with `530` until the client has signed in.
    pub require_auth: bool,
    /// The `SIZE` limit advertised and enforced, in bytes.
    pub size_limit: Option<u64>,
    /// Advertise `8BITMIME`.
    pub eight_bit: bool,
    /// Advertise `SMTPUTF8`.
    pub smtputf8: bool,
    /// Mechanisms to advertise and accept, any of `PLAIN`, `LOGIN`, `XOAUTH2`.
    pub mechanisms: Vec<&'static str>,
}

impl Default for SmtpOptions {
    fn default() -> Self {
        Self {
            username: "alice".into(),
            password: "correct horse".into(),
            tls: TlsMode::None,
            require_auth: true,
            size_limit: Some(10 * 1024 * 1024),
            eight_bit: true,
            smtputf8: true,
            mechanisms: vec!["PLAIN", "LOGIN", "XOAUTH2"],
        }
    }
}

/// A message the server accepted (answered the final `.` with a 2xx).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcceptedMessage {
    /// The envelope sender.
    pub mail_from: String,
    /// The envelope recipients, in order.
    pub rcpt_to: Vec<String>,
    /// The message exactly as received, dot-unstuffed, CRLF line ends, without the final `.`.
    pub data: Vec<u8>,
    /// The user the client signed in as, if any.
    pub authenticated_as: Option<String>,
    /// Whether the connection was TLS.
    pub over_tls: bool,
}

struct State {
    opts: SmtpOptions,
    replies: HashMap<Stage, (u16, String)>,
    rcpt_replies: HashMap<String, (u16, String)>,
    misbehavior: SmtpMisbehavior,
    accepted: Vec<AcceptedMessage>,
    commands: Vec<String>,
    connections: usize,
}

type Shared = Arc<Mutex<State>>;

/// A running server. Dropping it stops accepting connections.
pub struct SmtpServer {
    addr: SocketAddr,
    state: Shared,
    kill: watch::Sender<u64>,
    cert_der: Option<Vec<u8>>,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for SmtpServer {
    fn drop(&mut self) {
        self.task.abort();
        self.kill.send_modify(|g| *g += 1);
    }
}

impl SmtpServer {
    /// Starts a server on a free `127.0.0.1` port.
    pub async fn start(opts: SmtpOptions) -> SmtpServer {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("local addr");
        let (acceptor, cert_der) = if opts.tls == TlsMode::None {
            (None, None)
        } else {
            let (a, d) = tls_acceptor();
            (Some(a), Some(d))
        };
        let state: Shared = Arc::new(Mutex::new(State {
            opts,
            replies: HashMap::new(),
            rcpt_replies: HashMap::new(),
            misbehavior: SmtpMisbehavior::None,
            accepted: Vec::new(),
            commands: Vec::new(),
            connections: 0,
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
                        serve(tcp, state, acceptor, kill_rx).await;
                    });
                }
            })
        };
        SmtpServer {
            addr,
            state,
            kill,
            cert_der,
            task,
        }
    }

    /// Starts a plaintext server with default options.
    pub async fn start_default() -> SmtpServer {
        Self::start(SmtpOptions::default()).await
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

    /// Forces the reply at a stage, for example `(451, "try later")`. The reply text is sent as
    /// given. Applies to connections already open as well as new ones.
    pub fn set_reply(&self, stage: Stage, code: u16, text: &str) {
        self.st().replies.insert(stage, (code, text.to_string()));
    }

    /// Removes a forced reply.
    pub fn clear_reply(&self, stage: Stage) {
        self.st().replies.remove(&stage);
    }

    /// Forces the reply to `RCPT TO` for one address (case-insensitive).
    pub fn set_rcpt_reply(&self, address: &str, code: u16, text: &str) {
        self.st()
            .rcpt_replies
            .insert(address.to_ascii_lowercase(), (code, text.to_string()));
    }

    /// Makes the server misbehave (or stop).
    pub fn set_misbehavior(&self, how: SmtpMisbehavior) {
        self.st().misbehavior = how;
    }

    /// Changes the accepted size limit (also advertised in `EHLO`).
    pub fn set_size_limit(&self, limit: Option<u64>) {
        self.st().opts.size_limit = limit;
    }

    /// Every message the server accepted, oldest first.
    pub fn accepted(&self) -> Vec<AcceptedMessage> {
        self.st().accepted.clone()
    }

    /// The commands received, one per entry, with sign-in data replaced by `<redacted>`.
    pub fn commands(&self) -> Vec<String> {
        self.st().commands.clone()
    }

    /// Clears the command log.
    pub fn clear_commands(&self) {
        self.st().commands.clear();
    }

    /// How many connections have been accepted.
    pub fn connection_count(&self) -> usize {
        self.st().connections
    }
}

enum End<S> {
    Closed,
    Upgrade(S),
}

async fn serve(
    tcp: TcpStream,
    state: Shared,
    acceptor: Option<TlsAcceptor>,
    mut kill: watch::Receiver<u64>,
) {
    kill.mark_unchanged();
    let mode = {
        let mut st = state.lock().expect("lock");
        st.connections += 1;
        st.opts.tls
    };
    match (mode, acceptor) {
        (TlsMode::Implicit, Some(a)) => {
            if let Ok(tls) = a.accept(tcp).await {
                let _ = run(tls, &state, true, true).await;
            }
        }
        (TlsMode::StartTls, Some(a)) => {
            if let End::Upgrade(tcp) = run(tcp, &state, false, true).await {
                if let Ok(tls) = a.accept(tcp).await {
                    let _ = run(tls, &state, true, false).await;
                }
            }
        }
        _ => {
            let _ = run(tcp, &state, false, true).await;
        }
    }
}

#[derive(Default)]
struct Session {
    mail_from: Option<String>,
    rcpt_to: Vec<String>,
    user: Option<String>,
}

fn forced(state: &Shared, stage: Stage) -> Option<(u16, String)> {
    state.lock().expect("lock").replies.get(&stage).cloned()
}

fn line(code: u16, text: &str) -> String {
    format!("{code} {text}\r\n")
}

async fn send<S: AsyncRead + AsyncWrite + Unpin>(io: &mut BufReader<S>, text: &str) -> bool {
    io.get_mut().write_all(text.as_bytes()).await.is_ok() && io.get_mut().flush().await.is_ok()
}

async fn read_line<S: AsyncRead + Unpin>(io: &mut BufReader<S>) -> Option<String> {
    let mut buf = String::new();
    match io.read_line(&mut buf).await {
        Ok(0) | Err(_) => None,
        Ok(_) => Some(buf.trim_end_matches(['\r', '\n']).to_string()),
    }
}

/// Runs one connection. `secure` is true on a TLS stream; `first` is true for the first stream of
/// the connection (the one that sends the greeting).
async fn run<S: AsyncRead + AsyncWrite + Unpin>(
    stream: S,
    state: &Shared,
    secure: bool,
    first: bool,
) -> End<S> {
    let mut io = BufReader::new(stream);
    let (opts, misbehavior) = {
        let st = state.lock().expect("lock");
        (st.opts.clone(), st.misbehavior)
    };
    if first {
        let greeting = if misbehavior == SmtpMisbehavior::RefuseGreeting {
            line(554, "no service")
        } else {
            line(220, "localhost test ESMTP")
        };
        if !send(&mut io, &greeting).await || misbehavior == SmtpMisbehavior::RefuseGreeting {
            return End::Closed;
        }
    }
    let mut sess = Session::default();
    loop {
        let Some(cmd) = read_line(&mut io).await else {
            return End::Closed;
        };
        let upper = cmd.to_ascii_uppercase();
        let verb = upper.split_whitespace().next().unwrap_or("").to_string();
        {
            let mut st = state.lock().expect("lock");
            if verb == "AUTH" {
                let mech = upper.split_whitespace().nth(1).unwrap_or("");
                st.commands.push(format!("AUTH {mech} <redacted>"));
            } else {
                st.commands.push(cmd.clone());
            }
        }
        let st_misbehavior = state.lock().expect("lock").misbehavior;
        match verb.as_str() {
            "EHLO" | "HELO" => {
                let mut out = String::from("250-localhost greets you\r\n");
                if let Some(limit) = opts.size_limit {
                    out.push_str(&format!("250-SIZE {limit}\r\n"));
                }
                if opts.eight_bit {
                    out.push_str("250-8BITMIME\r\n");
                }
                if opts.smtputf8 {
                    out.push_str("250-SMTPUTF8\r\n");
                }
                let must_upgrade = opts.tls == TlsMode::StartTls && !secure;
                if must_upgrade {
                    out.push_str("250-STARTTLS\r\n");
                } else if !opts.mechanisms.is_empty() {
                    out.push_str(&format!("250-AUTH {}\r\n", opts.mechanisms.join(" ")));
                }
                out.push_str("250 PIPELINING\r\n");
                if !send(&mut io, &out).await {
                    return End::Closed;
                }
            }
            "STARTTLS" => {
                if opts.tls == TlsMode::StartTls && !secure {
                    // With the injection misbehaviour the extra bytes go out in the same write
                    // as the 220, so they arrive together (as a real attacker would send them).
                    let mut reply = line(220, "ready to start TLS");
                    if st_misbehavior == SmtpMisbehavior::InjectAfterStartTls {
                        reply.push_str("250 injected plaintext\r\n");
                    }
                    if !send(&mut io, &reply).await {
                        return End::Closed;
                    }
                    return End::Upgrade(io.into_inner());
                }
                if !send(&mut io, &line(502, "STARTTLS not available")).await {
                    return End::Closed;
                }
            }
            "AUTH" => {
                if st_misbehavior == SmtpMisbehavior::HangUpOnAuth {
                    return End::Closed;
                }
                let reply = auth(&mut io, &cmd, &opts, secure, state, &mut sess).await;
                match reply {
                    Some(text) => {
                        if !send(&mut io, &text).await {
                            return End::Closed;
                        }
                    }
                    None => return End::Closed,
                }
            }
            "MAIL" => {
                let reply = if let Some((c, t)) = forced(state, Stage::MailFrom) {
                    line(c, &t)
                } else if opts.require_auth && sess.user.is_none() {
                    line(530, "5.7.0 Authentication required")
                } else if let Some(addr) = path_of(&cmd) {
                    sess.mail_from = Some(addr);
                    sess.rcpt_to.clear();
                    line(250, "OK")
                } else {
                    line(501, "bad address")
                };
                if !send(&mut io, &reply).await {
                    return End::Closed;
                }
            }
            "RCPT" => {
                let addr = path_of(&cmd);
                let specific = addr.as_ref().and_then(|a| {
                    state
                        .lock()
                        .expect("lock")
                        .rcpt_replies
                        .get(&a.to_ascii_lowercase())
                        .cloned()
                });
                let reply = if let Some((c, t)) = specific {
                    line(c, &t)
                } else if let Some((c, t)) = forced(state, Stage::RcptTo) {
                    line(c, &t)
                } else if sess.mail_from.is_none() {
                    line(503, "need MAIL first")
                } else if let Some(a) = addr {
                    sess.rcpt_to.push(a);
                    line(250, "OK")
                } else {
                    line(501, "bad address")
                };
                if !send(&mut io, &reply).await {
                    return End::Closed;
                }
            }
            "DATA" => {
                if let Some((c, t)) = forced(state, Stage::Data) {
                    if !send(&mut io, &line(c, &t)).await {
                        return End::Closed;
                    }
                    continue;
                }
                if sess.mail_from.is_none() || sess.rcpt_to.is_empty() {
                    if !send(&mut io, &line(503, "need MAIL and RCPT first")).await {
                        return End::Closed;
                    }
                    continue;
                }
                if !send(&mut io, &line(354, "end data with <CR><LF>.<CR><LF>")).await {
                    return End::Closed;
                }
                let mut data = Vec::new();
                loop {
                    let Some(l) = read_line(&mut io).await else {
                        return End::Closed;
                    };
                    if l == "." {
                        break;
                    }
                    let l = l.strip_prefix('.').map_or(l.clone(), str::to_string);
                    data.extend_from_slice(l.as_bytes());
                    data.extend_from_slice(b"\r\n");
                }
                let record = AcceptedMessage {
                    mail_from: sess.mail_from.take().unwrap_or_default(),
                    rcpt_to: std::mem::take(&mut sess.rcpt_to),
                    data,
                    authenticated_as: sess.user.clone(),
                    over_tls: secure,
                };
                let now = state.lock().expect("lock").misbehavior;
                match now {
                    SmtpMisbehavior::HangAfterData => {
                        // Keep the connection open and say nothing.
                        std::future::pending::<()>().await;
                    }
                    SmtpMisbehavior::AcceptThenDrop => {
                        state.lock().expect("lock").accepted.push(record);
                        return End::Closed;
                    }
                    SmtpMisbehavior::DropWithoutAccepting => return End::Closed,
                    _ => {}
                }
                let too_big = opts
                    .size_limit
                    .is_some_and(|l| record.data.len() as u64 > l);
                let reply = if let Some((c, t)) = forced(state, Stage::EndOfData) {
                    if (200..300).contains(&c) {
                        state.lock().expect("lock").accepted.push(record);
                    }
                    line(c, &t)
                } else if too_big {
                    line(552, "5.3.4 message too big")
                } else {
                    let n = {
                        let mut st = state.lock().expect("lock");
                        st.accepted.push(record);
                        st.accepted.len()
                    };
                    line(250, &format!("OK queued as {n}"))
                };
                if !send(&mut io, &reply).await {
                    return End::Closed;
                }
            }
            "RSET" => {
                sess.mail_from = None;
                sess.rcpt_to.clear();
                if !send(&mut io, &line(250, "OK")).await {
                    return End::Closed;
                }
            }
            "NOOP" => {
                if !send(&mut io, &line(250, "OK")).await {
                    return End::Closed;
                }
            }
            "QUIT" => {
                let _ = send(&mut io, &line(221, "bye")).await;
                return End::Closed;
            }
            _ => {
                if !send(&mut io, &line(500, "command not recognized")).await {
                    return End::Closed;
                }
            }
        }
    }
}

/// The address inside `<...>` of a `MAIL FROM:<a@b>` or `RCPT TO:<a@b>` line.
fn path_of(cmd: &str) -> Option<String> {
    let start = cmd.find('<')? + 1;
    let end = cmd[start..].find('>')? + start;
    let addr = &cmd[start..end];
    (!addr.is_empty()).then(|| addr.to_string())
}

/// Handles `AUTH ...`. Returns the final reply, or `None` when the client went away.
async fn auth<S: AsyncRead + AsyncWrite + Unpin>(
    io: &mut BufReader<S>,
    cmd: &str,
    opts: &SmtpOptions,
    secure: bool,
    state: &Shared,
    sess: &mut Session,
) -> Option<String> {
    use base64_lite::{decode, encode};
    if let Some((c, t)) = forced(state, Stage::Auth) {
        return Some(line(c, &t));
    }
    if opts.tls == TlsMode::StartTls && !secure {
        return Some(line(530, "5.7.0 Must issue a STARTTLS command first"));
    }
    let mut parts = cmd.split_whitespace();
    parts.next();
    let mech = parts.next().unwrap_or("").to_ascii_uppercase();
    let initial = parts.next().map(str::to_string);
    if !opts.mechanisms.iter().any(|m| *m == mech) {
        return Some(line(504, "5.5.4 mechanism not supported"));
    }
    let ok = |sess: &mut Session, user: &str| {
        sess.user = Some(user.to_string());
        line(235, "2.7.0 Authentication successful")
    };
    let fail = || line(535, "5.7.8 Authentication credentials invalid");
    match mech.as_str() {
        "PLAIN" => {
            let encoded = match initial {
                Some(i) => i,
                None => {
                    send(io, "334 \r\n").await.then_some(())?;
                    read_line(io).await?
                }
            };
            let raw = decode(&encoded).unwrap_or_default();
            let mut fields = raw.split(|b| *b == 0);
            let _authzid = fields.next();
            let user = String::from_utf8_lossy(fields.next().unwrap_or_default()).into_owned();
            let pass = String::from_utf8_lossy(fields.next().unwrap_or_default()).into_owned();
            Some(if user == opts.username && pass == opts.password {
                ok(sess, &user)
            } else {
                fail()
            })
        }
        "LOGIN" => {
            send(io, &format!("334 {}\r\n", encode(b"Username:")))
                .await
                .then_some(())?;
            let user = String::from_utf8_lossy(&decode(&read_line(io).await?).unwrap_or_default())
                .into_owned();
            send(io, &format!("334 {}\r\n", encode(b"Password:")))
                .await
                .then_some(())?;
            let pass = String::from_utf8_lossy(&decode(&read_line(io).await?).unwrap_or_default())
                .into_owned();
            Some(if user == opts.username && pass == opts.password {
                ok(sess, &user)
            } else {
                fail()
            })
        }
        "XOAUTH2" => {
            let encoded = match initial {
                Some(i) => i,
                None => {
                    send(io, "334 \r\n").await.then_some(())?;
                    read_line(io).await?
                }
            };
            let raw = String::from_utf8_lossy(&decode(&encoded).unwrap_or_default()).into_owned();
            let user = raw
                .strip_prefix("user=")
                .and_then(|r| r.split('\u{1}').next())
                .unwrap_or("")
                .to_string();
            let token = raw
                .split('\u{1}')
                .find_map(|f| f.strip_prefix("auth=Bearer "))
                .unwrap_or("")
                .to_string();
            if user == opts.username && token == opts.password {
                Some(ok(sess, &user))
            } else {
                // The protocol: a 334 with a JSON error, the client answers with an empty line,
                // and only then comes the failure.
                let json = encode(br#"{"status":"401","schemes":"Bearer","scope":"mail"}"#);
                send(io, &format!("334 {json}\r\n")).await.then_some(())?;
                let _ = read_line(io).await?;
                Some(fail())
            }
        }
        _ => Some(line(504, "mechanism not supported")),
    }
}

/// A tiny base64 so this crate needs no extra dependency.
mod base64_lite {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

    pub(super) fn encode(data: &[u8]) -> String {
        let mut out = String::new();
        for chunk in data.chunks(3) {
            let b = [
                chunk[0],
                chunk.get(1).copied().unwrap_or(0),
                chunk.get(2).copied().unwrap_or(0),
            ];
            let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
            out.push(ALPHABET[(n >> 18) as usize & 63] as char);
            out.push(ALPHABET[(n >> 12) as usize & 63] as char);
            out.push(if chunk.len() > 1 {
                ALPHABET[(n >> 6) as usize & 63] as char
            } else {
                '='
            });
            out.push(if chunk.len() > 2 {
                ALPHABET[n as usize & 63] as char
            } else {
                '='
            });
        }
        out
    }

    pub(super) fn decode(text: &str) -> Option<Vec<u8>> {
        let mut out = Vec::new();
        let mut acc = 0u32;
        let mut bits = 0;
        for c in text.trim().bytes() {
            if c == b'=' {
                break;
            }
            let v = ALPHABET.iter().position(|a| *a == c)? as u32;
            acc = (acc << 6) | v;
            bits += 6;
            if bits >= 8 {
                bits -= 8;
                out.push((acc >> bits) as u8);
                acc &= (1 << bits) - 1;
            }
        }
        Some(out)
    }
}
