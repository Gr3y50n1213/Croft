// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Sending mail: the SMTP submission client.
//!
//! [`SmtpSender::send`] opens one connection, secures it (implicit TLS, or `STARTTLS` before
//! anything secret is sent), signs in (`AUTH PLAIN`, `AUTH LOGIN` or `AUTH XOAUTH2`), submits one
//! message and closes. One connection per message keeps the "did the server get it?" question
//! simple: there is no pooled connection that might have died quietly.
//!
//! # Why this is not `lettre` or `mail-send`
//!
//! The task asked for `lettre`. Its rustls support (0.11.23) builds the `ClientConfig` itself and
//! offers no way to supply a certificate verifier, only "accept every certificate" or extra root
//! certificates, so Croft's rule "validation always on, a self-signed server only through an
//! explicit pinned fingerprint" cannot be expressed. `mail-send` 0.7 can take a custom TLS
//! connector, but it pulls in `rustls-platform-verifier` unconditionally, tries `CRAM-MD5` and
//! `DIGEST-MD5` with the password by default, reads replies without a size limit, and reports a
//! failure after the message body as one undifferentiated I/O error, which the duplicate-send rule
//! below needs to tell apart. SMTP submission is a small protocol, so this module speaks it
//! directly over `tokio` and `tokio-rustls`, which the IMAP adapter already uses: no new
//! dependency, the same TLS policy as IMAP, bounded replies, and every failure mapped by hand.
//!
//! # Errors
//!
//! - Cannot connect, a timeout, a dropped connection, or `4xx` at any step before the message is
//!   fully sent: [`BackendError::Transient`]. Nothing was delivered, so trying again is safe.
//! - `535`, `534`, `530` (and any other `5xx` to `AUTH`), no usable sign-in method, a certificate
//!   or pin that does not verify: [`BackendError::Auth`]. The account pauses, nothing is lost.
//! - Other `5xx`: [`BackendError::Permanent`] with a fixed reason that says what was refused (the
//!   sender, a recipient, the message) and never contains the server's text or an address.
//!
//! # The one case that is not retried
//!
//! After Croft has sent the final `.` line, the server may already have accepted the message. If
//! the connection then drops or times out before the reply, Croft cannot know. Calling that
//! `Transient` would resend and could deliver the message twice, so it is
//! [`BackendError::Permanent`] with [`MAYBE_SENT_REASON`]: the message goes to Failed, the person
//! checks Sent and decides. This trades a rare manual retry for never sending twice.
//!
//! # Secrets
//!
//! The password and token are only ever in `Zeroizing` buffers while the sign-in line is built and
//! sent. Nothing in this module logs. Error text is fixed wording.

mod config;
mod tls;
mod wire;

pub use config::{SmtpConfig, SmtpSecurity};
pub use wire::data_lines;

use crate::imap::{CertFingerprint, Credential};
use crate::types::BackendError;
use base64::Engine as _;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tls::Trust;
use tokio::net::{lookup_host, TcpStream};
use tokio::time::timeout;
use tokio_rustls::rustls::pki_types::ServerName;
use tokio_rustls::TlsConnector;
use wire::{Capabilities, Conn, Reply, Stream, WireError};
use zeroize::Zeroizing;

/// The reason shown when the server may have accepted a message but never said so. Also used by
/// the Outbox when the app was killed while a message was being sent.
pub const MAYBE_SENT_REASON: &str = "May not have been sent; check Sent before retrying";

/// Reason for a sender the server refused.
pub const REASON_SENDER: &str = "The server refused the sender address";
/// Reason for a recipient the server refused.
pub const REASON_RECIPIENT: &str = "The server refused a recipient address";
/// Reason for a message the server refused.
pub const REASON_MESSAGE: &str = "The server refused the message";
/// Reason for a message the server says is too big.
pub const REASON_TOO_LARGE: &str = "The message is larger than the server accepts";
/// 4xx.
const TRY_LATER: &str = "The server asked to try again later";

/// What a successful send reports. It carries nothing the server said.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SendReceipt {
    /// How many recipients the server accepted (all of them: one refusal fails the send).
    pub recipients: usize,
    /// Size of the message as sent, in bytes.
    pub bytes: usize,
}

/// Sends mail for one account. Holds the settings and the credential; connects per message.
pub struct SmtpSender {
    config: SmtpConfig,
    credential: Credential,
}

impl std::fmt::Debug for SmtpSender {
    /// Prints the settings (host, port, security) and never the credential.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SmtpSender")
            .field("config", &self.config)
            .field("credential", &"***")
            .finish()
    }
}

/// A checked envelope.
struct Envelope<'a> {
    from: &'a str,
    to: Vec<&'a str>,
    needs_utf8: bool,
}

impl SmtpSender {
    /// Creates the sender. Nothing is sent yet. Fails with [`BackendError::Permanent`] when the
    /// settings are unusable, for example plaintext to a host that is not localhost.
    pub fn new(config: SmtpConfig, credential: Credential) -> Result<Self, BackendError> {
        config.validate()?;
        Ok(Self { config, credential })
    }

    /// The settings this sender was made with.
    pub fn config(&self) -> &SmtpConfig {
        &self.config
    }

    /// Submits one message. `envelope_from` and `recipients` are bare addresses (no display
    /// names, no angle brackets), exactly the ones the server should use: Bcc recipients belong
    /// in `recipients` and not in the headers. `mime` is the whole RFC 5322 message; line endings
    /// are normalised to CRLF and leading dots are stuffed on the way out.
    ///
    /// If any recipient is refused, nothing is sent (the `DATA` command is never issued), so a
    /// message is never delivered to only some of the people it was addressed to.
    ///
    /// See the module notes for the error classes and for the one case that is deliberately not
    /// retried.
    pub async fn send<R: AsRef<str>>(
        &self,
        envelope_from: &str,
        recipients: &[R],
        mime: &[u8],
    ) -> Result<SendReceipt, BackendError> {
        let envelope = check_envelope(envelope_from, recipients)?;
        let body = wire::encode_for_data(mime);
        let allowance = Duration::from_secs((body.len() / (16 * 1024)) as u64);
        let budget = self.config.deadline.saturating_add(allowance);
        // Set once the final `.` line has been written. After that a failure is ambiguous.
        let past_the_point_of_no_return = AtomicBool::new(false);
        let outcome = timeout(
            budget,
            self.run(&envelope, &body, &past_the_point_of_no_return),
        )
        .await;
        match outcome {
            Ok(Ok((receipt, mut conn))) => {
                // Outside the budget on purpose: the message is delivered, and nothing that goes
                // wrong while saying goodbye may turn that into an error.
                quit(&mut conn).await;
                Ok(receipt)
            }
            Ok(Err(e)) => Err(e),
            Err(_) if past_the_point_of_no_return.load(Ordering::SeqCst) => Err(maybe_sent()),
            Err(_) => Err(BackendError::Transient("sending took too long".to_owned())),
        }
    }

    async fn run(
        &self,
        env: &Envelope<'_>,
        body: &[u8],
        past_the_point_of_no_return: &AtomicBool,
    ) -> Result<(SendReceipt, Conn), BackendError> {
        let (mut conn, caps) = self.open(&self.trust()).await?;
        self.sign_in(&mut conn, &caps).await?;

        // MAIL FROM, with the extensions the server offered.
        let mut cmd = format!("MAIL FROM:<{}>", env.from);
        if let Some(limit) = caps.size.filter(|l| *l > 0) {
            if body.len() as u64 > limit {
                return Err(BackendError::Permanent(REASON_TOO_LARGE.into()));
            }
        }
        if caps.size.is_some() {
            cmd.push_str(&format!(" SIZE={}", body.len()));
        }
        if wire::is_8bit(body) {
            if !caps.eight_bit {
                return Err(BackendError::Permanent(
                    "The server does not accept 8-bit messages".into(),
                ));
            }
            cmd.push_str(" BODY=8BITMIME");
        }
        if env.needs_utf8 {
            if !caps.smtputf8 {
                return Err(BackendError::Permanent(
                    "The server does not accept addresses with accents or non-Latin letters".into(),
                ));
            }
            cmd.push_str(" SMTPUTF8");
        }
        let reply = command(&mut conn, &cmd).await?;
        if !reply.is_ok() {
            return Err(refused(&reply, REASON_SENDER));
        }
        for rcpt in &env.to {
            let reply = command(&mut conn, &format!("RCPT TO:<{rcpt}>")).await?;
            if !reply.is_ok() {
                quit(&mut conn).await;
                return Err(refused(&reply, REASON_RECIPIENT));
            }
        }

        let reply = command(&mut conn, "DATA").await?;
        if reply.code != 354 {
            quit(&mut conn).await;
            return Err(refused(&reply, REASON_MESSAGE));
        }
        // Until the terminator is out the server cannot have accepted the message, so a failure
        // here is an ordinary retryable one.
        conn.write_chunked(body).await.map_err(|e| net_error(&e))?;
        past_the_point_of_no_return.store(true, Ordering::SeqCst);
        if conn.write(b".\r\n").await.is_err() {
            return Err(maybe_sent());
        }
        let reply = match conn.read_reply_within(None).await {
            Ok(r) => r,
            Err(_) => return Err(maybe_sent()),
        };
        if reply.is_ok() {
            let receipt = SendReceipt {
                recipients: env.to.len(),
                bytes: body.len(),
            };
            return Ok((receipt, conn));
        }
        // A definite answer that is not success: the server did not take the message (or, for an
        // answer that is neither success nor refusal, below, we cannot tell).
        past_the_point_of_no_return.store(false, Ordering::SeqCst);
        if reply.is_temporary() {
            // The server said, in so many words, that it did not take the message.
            return Err(BackendError::Transient(TRY_LATER.into()));
        }
        if reply.code == 552 {
            return Err(BackendError::Permanent(REASON_TOO_LARGE.into()));
        }
        if (500..600).contains(&reply.code) {
            return Err(refused(&reply, REASON_MESSAGE));
        }
        // A reply that is neither success nor refusal: do not guess.
        Err(maybe_sent())
    }

    /// Connects, secures the connection, reads the greeting and returns what `EHLO` offered.
    /// Nothing secret has been sent when this returns.
    async fn open(&self, trust: &Trust) -> Result<(Conn, Capabilities), BackendError> {
        let cfg = &self.config;
        let t = cfg.timeout;
        let host = cfg
            .host
            .trim()
            .trim_start_matches('[')
            .trim_end_matches(']');
        let addrs: Vec<_> = match timeout(t, lookup_host((host, cfg.port))).await {
            Err(_) => {
                return Err(BackendError::Transient(
                    "looking up the server timed out".into(),
                ))
            }
            Ok(Err(_)) => {
                return Err(BackendError::Transient(
                    "could not look up the server".into(),
                ))
            }
            Ok(Ok(it)) => it.collect(),
        };
        if addrs.is_empty() {
            return Err(BackendError::Transient(
                "could not look up the server".into(),
            ));
        }
        // T26: the process's address policy (the Companion refuses its own network).
        let addrs = crate::netpolicy::filter(addrs);
        if addrs.is_empty() {
            return Err(BackendError::Permanent(crate::netpolicy::REFUSED.into()));
        }
        if cfg.security == SmtpSecurity::PlaintextForTestsOnly
            && !addrs.iter().all(|a| a.ip().is_loopback())
        {
            return Err(BackendError::Permanent(
                "unencrypted SMTP is only allowed for localhost".into(),
            ));
        }
        let tcp = match timeout(t, TcpStream::connect(&addrs[..])).await {
            Err(_) => {
                return Err(BackendError::Transient(
                    "connecting to the server timed out".into(),
                ))
            }
            Ok(Err(_)) => {
                return Err(BackendError::Transient(
                    "could not connect to the server".into(),
                ))
            }
            Ok(Ok(tcp)) => tcp,
        };
        let _ = tcp.set_nodelay(true);

        let mut conn = match cfg.security {
            SmtpSecurity::Tls => {
                let stream = self.tls_upgrade(tcp, host, trust).await?;
                Conn::new(stream, t)
            }
            SmtpSecurity::PlaintextForTestsOnly | SmtpSecurity::StartTls => {
                Conn::new(Stream::Plain(tcp), t)
            }
        };
        read_greeting(&mut conn).await?;
        let mut caps = ehlo(&mut conn, &cfg.ehlo_name).await?;

        if cfg.security == SmtpSecurity::StartTls {
            if !caps.starttls {
                // Same class as a certificate failure: the account pauses and the mail stays
                // queued, instead of the message failing for good.
                return Err(BackendError::Auth(
                    "the server did not offer a secure connection".into(),
                ));
            }
            let reply = command(&mut conn, "STARTTLS").await?;
            if reply.code != 220 {
                return Err(if reply.is_temporary() {
                    BackendError::Transient(TRY_LATER.into())
                } else {
                    BackendError::Permanent(
                        "the server refused STARTTLS, so the message was not sent".into(),
                    )
                });
            }
            // Bytes that arrived after the `220` were sent in plaintext by the server or by
            // someone in the middle; they must not be read as if they came over TLS.
            if conn.has_unread() {
                return Err(BackendError::Auth(
                    "the connection could not be secured safely".into(),
                ));
            }
            let Some(tcp) = conn.into_plain() else {
                return Err(BackendError::Permanent(
                    "unexpected connection state".into(),
                ));
            };
            let stream = self.tls_upgrade(tcp, host, trust).await?;
            conn = Conn::new(stream, t);
            // What the server offers can change once the channel is secure (AUTH often appears
            // only now), so the capabilities from before the upgrade are discarded.
            caps = ehlo(&mut conn, &cfg.ehlo_name).await?;
        }
        Ok((conn, caps))
    }

    /// How this sender judges the server's certificate: the pin if there is one, else the roots.
    fn trust(&self) -> Trust {
        self.config.pinned_cert.map_or(Trust::Roots, Trust::Pinned)
    }

    async fn tls_upgrade(
        &self,
        tcp: TcpStream,
        host: &str,
        trust: &Trust,
    ) -> Result<Stream, BackendError> {
        let config = tls::client_config(trust).await?;
        let name = ServerName::try_from(host.to_string()).map_err(|_| {
            BackendError::Permanent("the server name is not a valid host name".into())
        })?;
        let connector = TlsConnector::from(config);
        match timeout(self.config.timeout, connector.connect(name, tcp)).await {
            Err(_) => Err(BackendError::Transient(
                "setting up the secure connection timed out".into(),
            )),
            Ok(Err(e)) => Err(tls_io_error(&e)),
            Ok(Ok(stream)) => Ok(Stream::Tls(Box::new(stream))),
        }
    }

    /// Signs in with the credential. Over plaintext (loopback tests only) the same lines are used.
    async fn sign_in(&self, conn: &mut Conn, caps: &Capabilities) -> Result<(), BackendError> {
        match &self.credential {
            Credential::Password(password) => {
                if caps.plain {
                    let mut raw = Zeroizing::new(Vec::new());
                    raw.push(0u8);
                    raw.extend_from_slice(self.config.username.as_bytes());
                    raw.push(0u8);
                    raw.extend_from_slice(password.as_bytes());
                    let line = auth_line("AUTH PLAIN ", &raw);
                    let reply = auth_exchange(conn, &line).await?;
                    finish_auth(conn, reply, None).await
                } else if caps.login {
                    let reply = auth_exchange(conn, b"AUTH LOGIN\r\n").await?;
                    if reply.code != 334 {
                        return Err(auth_failure(&reply));
                    }
                    let user = auth_line("", self.config.username.as_bytes());
                    let reply = auth_exchange(conn, &user).await?;
                    if reply.code != 334 {
                        return Err(auth_failure(&reply));
                    }
                    let pass = auth_line("", password.as_bytes());
                    let reply = auth_exchange(conn, &pass).await?;
                    finish_auth(conn, reply, None).await
                } else {
                    Err(no_auth_method())
                }
            }
            Credential::OAuth2 { user, access_token } => {
                if !caps.xoauth2 {
                    return Err(no_auth_method());
                }
                let mut raw = Zeroizing::new(Vec::new());
                raw.extend_from_slice(b"user=");
                raw.extend_from_slice(user.as_bytes());
                raw.extend_from_slice(b"\x01auth=Bearer ");
                raw.extend_from_slice(access_token.as_bytes());
                raw.extend_from_slice(b"\x01\x01");
                let line = auth_line("AUTH XOAUTH2 ", &raw);
                let reply = auth_exchange(conn, &line).await?;
                // A refused token is answered with `334` and a JSON error; the protocol wants an
                // empty line next, and then the real failure code.
                finish_auth(conn, reply, Some(b"\r\n")).await
            }
        }
    }
}

/// Connects to a submission server, completes the TLS handshake (for `STARTTLS`, after the
/// `STARTTLS` command) **without trusting the certificate**, reads the leaf certificate's SHA-256
/// fingerprint, and disconnects. No credential and no message is ever sent.
///
/// This is for the account-setup screen: when a server's certificate does not validate, the person
/// is shown this fingerprint to compare with the one the server's owner published, and may then
/// pin it with [`SmtpConfig::with_pinned_cert`]. Calling it trusts nothing by itself. Any
/// `pinned_cert` already in `config` is ignored. Returns `Permanent` for
/// [`SmtpSecurity::PlaintextForTestsOnly`], which has no certificate.
pub async fn probe_certificate(config: &SmtpConfig) -> Result<CertFingerprint, BackendError> {
    config.validate()?;
    if config.security == SmtpSecurity::PlaintextForTestsOnly {
        return Err(BackendError::Permanent(
            "an unencrypted connection has no certificate".into(),
        ));
    }
    let slot = Arc::new(Mutex::new(None));
    // The sender is only a carrier for the settings: the placeholder credential is never sent,
    // because `open` stops before signing in.
    let sender = SmtpSender {
        config: config.clone(),
        credential: Credential::password(String::new()),
    };
    let (mut conn, _caps) = sender.open(&Trust::Capture(slot.clone())).await?;
    quit(&mut conn).await;
    let seen = slot.lock().ok().and_then(|g| *g);
    seen.ok_or_else(|| BackendError::Transient("the server sent no certificate".into()))
}

/// `prefix` + base64(`raw`) + CRLF, in a buffer that is wiped when dropped.
fn auth_line(prefix: &str, raw: &[u8]) -> Zeroizing<Vec<u8>> {
    let encoded = Zeroizing::new(base64::engine::general_purpose::STANDARD.encode(raw));
    let mut line = Zeroizing::new(Vec::with_capacity(prefix.len() + encoded.len() + 2));
    line.extend_from_slice(prefix.as_bytes());
    line.extend_from_slice(encoded.as_bytes());
    line.extend_from_slice(b"\r\n");
    line
}

fn no_auth_method() -> BackendError {
    BackendError::Auth("the server offers no sign-in method Croft can use".into())
}

/// Sends a sign-in line and reads the answer. A server that hangs up here is, as with IMAP,
/// treated as a refusal of the sign-in: servers drop the connection on a failed attempt, and
/// repeating a wrong password locks accounts.
async fn auth_exchange(conn: &mut Conn, line: &[u8]) -> Result<Reply, BackendError> {
    if let Err(e) = conn.write(line).await {
        return Err(sign_in_error(&e));
    }
    conn.read_reply().await.map_err(|e| sign_in_error(&e))
}

fn sign_in_error(e: &WireError) -> BackendError {
    match e {
        WireError::Closed => BackendError::Auth(
            "the server ended the connection while signing in, which usually means the sign-in \
             details are wrong"
                .into(),
        ),
        WireError::Io(io)
            if matches!(
                io.kind(),
                std::io::ErrorKind::ConnectionReset
                    | std::io::ErrorKind::ConnectionAborted
                    | std::io::ErrorKind::UnexpectedEof
                    | std::io::ErrorKind::BrokenPipe
            ) && tls_error_in(io).is_none() =>
        {
            BackendError::Auth(
                "the server ended the connection while signing in, which usually means the \
                 sign-in details are wrong"
                    .into(),
            )
        }
        other => net_error(other),
    }
}

/// Completes a sign-in whose first answer is `reply`. `continuation` is what to send if the
/// server answers `334` (XOAUTH2's empty line); without it a `334` is a failure.
async fn finish_auth(
    conn: &mut Conn,
    mut reply: Reply,
    continuation: Option<&[u8]>,
) -> Result<(), BackendError> {
    if reply.code == 334 {
        if let Some(extra) = continuation {
            reply = auth_exchange(conn, extra).await?;
        }
    }
    if reply.code == 235 {
        Ok(())
    } else {
        Err(auth_failure(&reply))
    }
}

/// A refused or failed sign-in. `454` and other `4xx` answers are the server's own "try again
/// later" and stay retryable; every `5xx` (535, 534, 530, 538, ...) is a refusal of the sign-in.
fn auth_failure(reply: &Reply) -> BackendError {
    if reply.is_temporary() {
        BackendError::Transient(TRY_LATER.into())
    } else {
        BackendError::Auth("the server refused the sign-in details".into())
    }
}

/// `4xx` is retryable, `530`/`534`/`535` mean the sign-in is missing or no longer good, and any
/// other refusal is permanent with the given fixed reason.
fn refused(reply: &Reply, reason: &'static str) -> BackendError {
    if reply.is_temporary() {
        BackendError::Transient(TRY_LATER.into())
    } else if matches!(reply.code, 530 | 534 | 535) {
        BackendError::Auth("the server says the sign-in is not valid".into())
    } else {
        BackendError::Permanent(reason.into())
    }
}

fn maybe_sent() -> BackendError {
    BackendError::Permanent(MAYBE_SENT_REASON.into())
}

/// The rustls error inside an I/O error, if there is one.
fn tls_error_in(e: &std::io::Error) -> Option<&tokio_rustls::rustls::Error> {
    e.get_ref()
        .and_then(|inner| inner.downcast_ref::<tokio_rustls::rustls::Error>())
}

fn tls_io_error(e: &std::io::Error) -> BackendError {
    match tls_error_in(e) {
        Some(tls_error) => tls::from_tls(tls_error),
        None => BackendError::Transient("the secure connection could not be set up".into()),
    }
}

/// A wire failure before the message body is complete: nothing was delivered, so it is retryable,
/// except TLS errors, which keep their own classes.
fn net_error(e: &WireError) -> BackendError {
    match e {
        WireError::Io(io) => match tls_error_in(io) {
            Some(tls_error) => tls::from_tls(tls_error),
            None => BackendError::Transient("network error while sending".into()),
        },
        WireError::Timeout => BackendError::Transient("the server did not answer in time".into()),
        WireError::Closed => BackendError::Transient("the server closed the connection".into()),
        WireError::Protocol => {
            BackendError::Transient("the server sent an unexpected answer".into())
        }
    }
}

/// Sends one command line and reads its reply.
async fn command(conn: &mut Conn, line: &str) -> Result<Reply, BackendError> {
    let mut bytes = Vec::with_capacity(line.len() + 2);
    bytes.extend_from_slice(line.as_bytes());
    bytes.extend_from_slice(b"\r\n");
    conn.write(&bytes).await.map_err(|e| net_error(&e))?;
    conn.read_reply().await.map_err(|e| net_error(&e))
}

/// `QUIT`, best effort: the outcome of the send does not depend on it.
async fn quit(conn: &mut Conn) {
    let _ = timeout(Duration::from_secs(2), async {
        let _ = conn.write(b"QUIT\r\n").await;
        let _ = conn.read_reply().await;
    })
    .await;
}

async fn read_greeting(conn: &mut Conn) -> Result<(), BackendError> {
    let reply = conn.read_reply().await.map_err(|e| net_error(&e))?;
    if reply.code == 220 {
        Ok(())
    } else {
        // Refusing every connection (`554`) is as likely to be temporary as not; retry.
        Err(BackendError::Transient(
            "the server is not accepting connections".into(),
        ))
    }
}

async fn ehlo(conn: &mut Conn, name: &str) -> Result<Capabilities, BackendError> {
    let reply = command(conn, &format!("EHLO {name}")).await?;
    if reply.is_ok() {
        Ok(Capabilities::parse(&reply.lines))
    } else if reply.is_temporary() {
        Err(BackendError::Transient(TRY_LATER.into()))
    } else {
        Err(BackendError::Permanent(
            "the server did not accept the greeting (EHLO)".into(),
        ))
    }
}

/// Checks the sender and the recipients before any connection is made.
fn check_envelope<'a, R: AsRef<str>>(
    from: &'a str,
    recipients: &'a [R],
) -> Result<Envelope<'a>, BackendError> {
    let mut needs_utf8 = false;
    let bad = || BackendError::Permanent("an address is not valid".into());
    if recipients.is_empty() {
        return Err(BackendError::Permanent(
            "the message has no recipients".into(),
        ));
    }
    let mut check = |addr: &str| match wire::check_address(addr) {
        Some(utf8) => {
            needs_utf8 |= utf8;
            Ok(())
        }
        None => Err(bad()),
    };
    check(from)?;
    let mut to = Vec::with_capacity(recipients.len());
    for r in recipients {
        check(r.as_ref())?;
        to.push(r.as_ref());
    }
    Ok(Envelope {
        from,
        to,
        needs_utf8,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_of_the_sender_hides_the_credential() {
        let cfg = SmtpConfig::tls("smtp.example.org", "alice@example.org");
        let s = SmtpSender::new(cfg, Credential::password("hunter2")).expect("valid settings");
        let shown = format!("{s:?}");
        assert!(!shown.contains("hunter2") && !shown.contains("alice"));
    }

    #[test]
    fn plaintext_to_a_real_host_is_refused_at_construction() {
        let cfg = SmtpConfig::new(
            "mail.example.org",
            25,
            SmtpSecurity::PlaintextForTestsOnly,
            "u",
        );
        let err = SmtpSender::new(cfg, Credential::password("pw")).err();
        assert!(matches!(err, Some(BackendError::Permanent(_))));
    }

    #[test]
    fn bad_envelopes_are_refused_before_connecting() {
        let none: [&str; 0] = [];
        assert!(check_envelope("a@b.c", &none).is_err());
        assert!(check_envelope("a@b.c", &["x@y.z\r\nDATA"]).is_err());
        assert!(check_envelope("a@b.c\r\n", &["x@y.z"]).is_err());
        let ok = check_envelope("a@b.c", &["x@y.z", "q@r.s"]).expect("valid");
        assert_eq!(ok.to.len(), 2);
        assert!(!ok.needs_utf8);
        let utf = check_envelope("jörg@b.c", &["x@y.z"]).expect("valid");
        assert!(utf.needs_utf8);
    }

    #[test]
    fn classification_of_refusals() {
        let r = |code| Reply {
            code,
            lines: vec!["text with secret@example.org".into()],
        };
        assert!(matches!(refused(&r(450), "x"), BackendError::Transient(_)));
        assert!(matches!(refused(&r(530), "x"), BackendError::Auth(_)));
        assert!(matches!(refused(&r(535), "x"), BackendError::Auth(_)));
        assert!(matches!(refused(&r(550), "x"), BackendError::Permanent(m) if m == "x"));
        assert!(matches!(auth_failure(&r(454)), BackendError::Transient(_)));
        assert!(matches!(auth_failure(&r(535)), BackendError::Auth(_)));
        assert!(matches!(auth_failure(&r(504)), BackendError::Auth(_)));
        // None of them echoes the server's text.
        for code in [450, 530, 550] {
            let shown = format!("{:?}", refused(&r(code), "x"));
            assert!(!shown.contains("secret@"));
        }
    }
}
