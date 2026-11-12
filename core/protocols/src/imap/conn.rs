// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! One authenticated IMAP connection and the commands run on it.
//!
//! `async-imap` supplies the framing, the response parser (via `imap-proto`) and the sign-in
//! handshakes. Almost every command goes through [`Conn::exec`], which sends a tagged command and
//! collects the untagged replies, because the typed helpers in `async-imap` (`uid_mv`, `uid_copy`)
//! throw away the `COPYUID` response code that Croft needs to re-point queued operations.

use super::config::{Credential, ImapConfig, Security};
use super::error::{self, OpError};
use super::stream::Stream;
use super::text::{
    bracketed_message_id, decode_encoded_words, folder_arg, quote, uid_set, uid_sets,
};
use super::text::{reference_ids, utf7_decode};
use super::tls::{client_config, Trust};
use crate::host::clean_host;
use crate::types::*;
use async_imap::imap_proto::rfc4315::UidSetMember;
use async_imap::imap_proto::{
    AttributeValue, MailboxDatum, MessageSection, NameAttribute, Response, ResponseCode,
    SectionPath, Status,
};
use async_imap::{Client, Session};
use base64::Engine as _;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt;
use std::ops::RangeInclusive;
use std::time::Duration;
use tokio::net::{lookup_host, TcpStream};
use tokio::time::timeout;
use tokio_rustls::rustls::pki_types::ServerName;
use tokio_rustls::TlsConnector;
use zeroize::Zeroizing;

/// UIDs per command line. Large selections are split so no command line gets huge.
const UIDS_PER_COMMAND: usize = 500;
/// Most UIDs accepted from a `COPYUID` set, so a hostile server cannot make us allocate wildly.
const MAX_COPYUID_UIDS: usize = 1_000_000;
/// Most replies read while waiting for the answer to the sign-in command.
const MAX_SIGN_IN_REPLIES: usize = 10_000;
/// How fast an `APPEND` literal is assumed to go out at least, so the deadline for a big message
/// grows with its size (one second per 16 KiB on top of the command deadline).
const APPEND_BYTES_PER_SECOND: usize = 16 * 1024;
/// How fast a fetched message is assumed to arrive at least, so the deadline for reading a big
/// message grows with the size the server announced (one second per 16 KiB on top of the command
/// deadline, the same allowance as for `APPEND`).
const FETCH_BYTES_PER_SECOND: u64 = 16 * 1024;

/// The flags of every message the last time a folder was read, with the mod-sequence it was read
/// at. Lets CONDSTORE/QRESYNC servers answer with deltas.
#[derive(Debug, Clone)]
pub(crate) struct FolderCache {
    uid_validity: u32,
    highest_modseq: u64,
    messages: BTreeMap<Uid, Flags>,
}

/// What `SELECT`/`EXAMINE` told us.
#[derive(Debug, Default, Clone, Copy)]
struct MailboxInfo {
    exists: u32,
    uid_validity: Option<u32>,
    uid_next: Option<u32>,
    highest_modseq: Option<u64>,
    read_only: bool,
}

/// `COPYUID` mapping, source UIDs and destination UIDs in matching order.
#[derive(Debug, Default)]
struct CopyUid {
    src: Vec<Uid>,
    dst: Vec<Uid>,
}

/// The parts of an untagged server reply Croft uses, copied out so nothing borrows the parser's
/// buffer.
enum Item {
    Exists(u32),
    UidValidity(u32),
    UidNext(u32),
    HighestModSeq(u64),
    Search(Vec<Uid>),
    Vanished(Vec<RangeInclusive<u32>>),
    List(ListItem),
    Status(FolderStatus, bool),
    /// Boxed: a fetched message body makes the item far larger than the other variants.
    Fetch(Box<FetchItem>),
}

struct ListItem {
    path: String,
    delimiter: Option<char>,
    selectable: bool,
    special: Option<SpecialUse>,
}

struct FetchItem {
    uid: Option<Uid>,
    flags: Option<Flags>,
    header: Option<RemoteHeader>,
    /// `RFC822.SIZE`.
    size: Option<u32>,
    /// The whole message, from `BODY[]` (the answer to `BODY.PEEK[]`).
    body: Option<Vec<u8>>,
}

/// Everything a finished command sent back.
struct Reply {
    items: Vec<Item>,
    copyuid: Option<CopyUid>,
    read_only: bool,
}

/// One authenticated session.
pub(crate) struct Conn {
    session: Session<Stream>,
    /// What the server can do (QRESYNC is true only once it has been enabled).
    pub caps: Capabilities,
    timeout: Duration,
    /// Overall limit for one command ([`ImapConfig::command_deadline`]).
    deadline: Duration,
    /// Cap on untagged replies per command ([`ImapConfig::max_untagged_replies`]).
    max_items: usize,
    /// Last-known flags per folder, kept across reconnects by the owner.
    pub cache: HashMap<String, FolderCache>,
}

impl fmt::Debug for Conn {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Conn").field("caps", &self.caps).finish()
    }
}

// ---------------------------------------------------------------------------------------------
// Connecting
// ---------------------------------------------------------------------------------------------

/// Opens the transport (TCP, then TLS or STARTTLS) and reads the greeting. Nothing secret has
/// been sent when this returns.
pub(crate) async fn connect_client(
    config: &ImapConfig,
    trust: &Trust,
) -> Result<Client<Stream>, OpError> {
    config.validate()?;
    let t = config.timeout;
    // The same cleaning `provider_for_host` applies, so both see one name (see `crate::host`).
    let host = clean_host(&config.host);
    let host = host.as_str();
    let addrs: Vec<_> = match timeout(t, lookup_host((host, config.port))).await {
        Err(_) => return Err(OpError::timed_out("looking up the server")),
        Ok(Err(_)) => return Err(OpError::dropped("could not look up the server")),
        Ok(Ok(it)) => it.collect(),
    };
    if addrs.is_empty() {
        return Err(OpError::dropped("could not look up the server"));
    }
    if config.security == Security::PlaintextForTestsOnly
        && !addrs.iter().all(|a| a.ip().is_loopback())
    {
        return Err(OpError::permanent(
            "unencrypted IMAP is only allowed for localhost",
        ));
    }
    let tcp = match timeout(t, TcpStream::connect(&addrs[..])).await {
        Err(_) => return Err(OpError::timed_out("connecting to the server")),
        Ok(Err(e)) => return Err(error::from_io(&e, "connecting")),
        Ok(Ok(tcp)) => tcp,
    };
    let _ = tcp.set_nodelay(true);

    match config.security {
        Security::Tls => {
            let stream = tls_upgrade(tcp, host, trust, t).await?;
            let mut client = Client::new(stream);
            read_greeting(&mut client, t).await?;
            Ok(client)
        }
        Security::PlaintextForTestsOnly => {
            let mut client = Client::new(Stream::Plain(tcp));
            read_greeting(&mut client, t).await?;
            Ok(client)
        }
        Security::StartTls => {
            let mut client = Client::new(Stream::Plain(tcp));
            read_greeting(&mut client, t).await?;
            match timeout(t, client.run_command_and_check_ok("STARTTLS", None)).await {
                Err(_) => return Err(OpError::timed_out("starting TLS")),
                Ok(Err(async_imap::error::Error::No(_) | async_imap::error::Error::Bad(_))) => {
                    return Err(OpError::permanent(
                        "the server does not support STARTTLS, so the connection was not made",
                    ));
                }
                Ok(Err(e)) => return Err(error::from_command(&e, "start TLS")),
                Ok(Ok(())) => {}
            }
            // Anything the server (or someone in the middle) sent after its OK is discarded with
            // the old client: the upgrade starts from a clean slate.
            let Stream::Plain(tcp) = client.into_inner() else {
                return Err(OpError::permanent("unexpected connection state"));
            };
            let stream = tls_upgrade(tcp, host, trust, t).await?;
            Ok(Client::new(stream))
        }
    }
}

async fn tls_upgrade(
    tcp: TcpStream,
    host: &str,
    trust: &Trust,
    t: Duration,
) -> Result<Stream, OpError> {
    let cfg = client_config(trust).await?;
    let name = ServerName::try_from(host.to_string())
        .map_err(|_| OpError::permanent("the server name is not a valid host name"))?;
    let connector = TlsConnector::from(cfg);
    match timeout(t, connector.connect(name, tcp)).await {
        Err(_) => Err(OpError::timed_out("setting up the secure connection")),
        Ok(Err(e)) => Err(error::from_io(&e, "setting up the secure connection")),
        Ok(Ok(tls)) => Ok(Stream::Tls(Box::new(tls))),
    }
}

async fn read_greeting(client: &mut Client<Stream>, t: Duration) -> Result<(), OpError> {
    match timeout(t, client.read_response()).await {
        Err(_) => Err(OpError::timed_out("waiting for the server greeting")),
        Ok(Err(e)) => Err(error::from_io(&e, "reading the server greeting")),
        Ok(Ok(None)) => Err(OpError::dropped("the server closed the connection")),
        Ok(Ok(Some(resp))) => match resp.parsed() {
            Response::Data {
                status: Status::Ok, ..
            } => Ok(()),
            Response::Data {
                status: Status::Bye,
                ..
            } => Err(OpError::dropped("the server is not accepting connections")),
            _ => Err(OpError::dropped("unexpected server greeting")),
        },
    }
}

fn caps_from_names(names: &[String]) -> Capabilities {
    let has = |wanted: &str| names.iter().any(|n| n.eq_ignore_ascii_case(wanted));
    let condstore = has("CONDSTORE");
    Capabilities {
        move_cmd: has("MOVE"),
        uidplus: has("UIDPLUS"),
        special_use: has("SPECIAL-USE"),
        idle: has("IDLE"),
        condstore: condstore || has("QRESYNC"),
        qresync: has("QRESYNC") && has("ENABLE"),
    }
}

fn names_of_library_caps(caps: &async_imap::types::Capabilities) -> Vec<String> {
    use async_imap::types::Capability as C;
    caps.iter()
        .map(|c| match c {
            C::Imap4rev1 => "IMAP4rev1".to_string(),
            C::Auth(a) => format!("AUTH={a}"),
            C::Atom(a) => a.clone(),
        })
        .collect()
}

fn names_of_code_caps(caps: &[async_imap::imap_proto::Capability<'_>]) -> Vec<String> {
    use async_imap::imap_proto::Capability as C;
    caps.iter()
        .map(|c| match c {
            C::Imap4rev1 => "IMAP4rev1".to_string(),
            C::Auth(a) => format!("AUTH={a}"),
            C::Atom(a) => a.to_string(),
        })
        .collect()
}

/// A string as a quoted IMAP string for `LOGIN`, kept in wiped memory because it holds a secret.
/// Quotes and backslashes are escaped; line breaks and NUL cannot be carried and refuse the
/// sign-in. Non-ASCII text is sent as UTF-8 the way `async-imap`'s own `login` did.
fn quote_secret(s: &str) -> Option<Zeroizing<String>> {
    let mut out = Zeroizing::new(String::with_capacity(s.len() + 2));
    out.push('"');
    for ch in s.chars() {
        match ch {
            '\r' | '\n' | '\0' => return None,
            '"' | '\\' => {
                out.push('\\');
                out.push(ch);
            }
            c => out.push(c),
        }
    }
    out.push('"');
    Some(out)
}

/// Runs `LOGIN` or `AUTHENTICATE XOAUTH2` and reads the answer itself instead of leaving it to
/// `async-imap`, which skips an untagged `BYE` and reports a closed connection as a generic loss.
/// Here a `BYE`, a closed connection or a reset while waiting for the answer is
/// [`error::sign_in_hangup`] (`Auth`): servers that hang up on a failed password are the common
/// case, and retrying would lock the account. Returns the session and the capabilities the
/// server put in its answer, if any.
async fn sign_in(
    mut client: Client<Stream>,
    config: &ImapConfig,
    credential: &Credential,
) -> Result<(Session<Stream>, Option<Vec<String>>), OpError> {
    let t = config.timeout;
    // The command line holds the secret. It lives in wiped memory and is gone after the send.
    let (command, oauth_payload): (Zeroizing<String>, Option<Zeroizing<String>>) = match credential
    {
        Credential::Password(password) => {
            let (Some(user), Some(pass)) = (quote_secret(&config.username), quote_secret(password))
            else {
                return Err(OpError::auth(
                    "the sign-in details contain characters IMAP cannot carry",
                ));
            };
            (Zeroizing::new(format!("LOGIN {} {}", *user, *pass)), None)
        }
        Credential::OAuth2 { user, access_token } => {
            let raw = Zeroizing::new(format!(
                "user={user}\u{1}auth=Bearer {}\u{1}\u{1}",
                **access_token
            ));
            let encoded =
                Zeroizing::new(base64::engine::general_purpose::STANDARD.encode(raw.as_bytes()));
            (
                Zeroizing::new("AUTHENTICATE XOAUTH2".to_string()),
                Some(encoded),
            )
        }
    };
    let wait = async {
        let id = match client.run_command(command.as_str()).await {
            Ok(id) => id,
            Err(e) => return Err(error::from_command(&e, "sign in")),
        };
        drop(command);
        let mut answered_challenge = false;
        let mut replies = 0usize;
        loop {
            let resp = match timeout(t, client.read_response()).await {
                Err(_) => return Err(OpError::timed_out("while signing in")),
                Ok(Err(e)) => return Err(error::from_io_during_sign_in(&e)),
                Ok(Ok(None)) => return Err(error::sign_in_hangup()),
                Ok(Ok(Some(r))) => r,
            };
            replies += 1;
            if replies > MAX_SIGN_IN_REPLIES {
                return Err(OpError::flooded("sign in"));
            }
            match resp.parsed() {
                Response::Done {
                    tag,
                    status,
                    outcome,
                } if *tag == id => {
                    return match status {
                        Status::Ok => {
                            let caps = match &outcome.code {
                                Some(ResponseCode::Capabilities(c)) => Some(names_of_code_caps(c)),
                                _ => None,
                            };
                            Ok(caps)
                        }
                        Status::No | Status::Bad => {
                            Err(error::sign_in_refusal(&error::detail_text(
                                outcome.code.as_ref(),
                                outcome.information.as_deref(),
                            )))
                        }
                        _ => Err(OpError::dropped("unexpected server reply while signing in")),
                    };
                }
                Response::Data {
                    status: Status::Bye,
                    ..
                } => return Err(error::sign_in_hangup()),
                Response::Continue(_) => {
                    if let Some(payload) = &oauth_payload {
                        // The XOAUTH2 string once; any later challenge is the server's error
                        // report, answered with an empty line (RFC 7628) so it then says `NO`.
                        let line: &str = if answered_challenge { "" } else { payload };
                        answered_challenge = true;
                        match client.run_command_untagged(line).await {
                            Ok(()) => {}
                            Err(e) => return Err(error::from_command(&e, "sign in")),
                        }
                    }
                }
                _ => {}
            }
        }
    };
    match timeout(config.command_deadline, wait).await {
        Err(_) => Err(OpError::too_slow("sign in")),
        Ok(Err(e)) => Err(e),
        Ok(Ok(caps)) => Ok((client.into_session(), caps)),
    }
}

impl Conn {
    /// Connects, signs in, reads the capabilities. The credential is used for the sign-in
    /// command only and is not kept by this value.
    pub(crate) async fn open(
        config: &ImapConfig,
        credential: &Credential,
    ) -> Result<Self, OpError> {
        let trust = match config.pinned_cert {
            Some(fp) => Trust::Pinned(fp),
            None => Trust::Roots,
        };
        let client = connect_client(config, &trust).await?;
        let t = config.timeout;
        let (mut session, login_caps) = sign_in(client, config, credential).await?;
        let names = match login_caps {
            Some(c) => c,
            None => match timeout(t, session.capabilities()).await {
                Err(_) => return Err(OpError::timed_out("reading server capabilities")),
                Ok(Err(e)) => return Err(error::from_command(&e, "read the server capabilities")),
                Ok(Ok(c)) => names_of_library_caps(&c),
            },
        };
        let mut conn = Conn {
            session,
            caps: caps_from_names(&names),
            timeout: t,
            deadline: config.command_deadline,
            max_items: config.max_untagged_replies,
            cache: HashMap::new(),
        };
        if conn.caps.qresync {
            match conn.exec("ENABLE QRESYNC", "enable QRESYNC").await {
                Ok(_) => {}
                Err(e) if e.broken => return Err(e),
                Err(_) => conn.caps.qresync = false,
            }
        }
        Ok(conn)
    }

    /// Politely ends the session. Errors are ignored: the connection is going away anyway.
    pub(crate) async fn logout(&mut self) {
        let _ = timeout(self.timeout, self.session.logout()).await;
    }

    // -----------------------------------------------------------------------------------------
    // Command plumbing
    // -----------------------------------------------------------------------------------------

    /// Sends one tagged command and reads until its tagged reply. `doing` completes the sentence
    /// "while trying to ..." in error messages.
    ///
    /// Two limits stop a server that never finishes: `timeout` is the longest silence between
    /// replies, `deadline` the longest the whole command may take however steadily replies come,
    /// and more than `max_items` untagged replies is refused. All three drop the connection and
    /// are `Transient`.
    async fn exec(&mut self, command: &str, doing: &str) -> Result<Reply, OpError> {
        self.exec_within(self.deadline, command, doing).await
    }

    /// [`Conn::exec`] with another overall deadline (a big message gets more time).
    async fn exec_within(
        &mut self,
        deadline: Duration,
        command: &str,
        doing: &str,
    ) -> Result<Reply, OpError> {
        match timeout(deadline, self.exec_inner(command, doing)).await {
            Ok(result) => result,
            Err(_) => Err(OpError::too_slow(doing)),
        }
    }

    async fn exec_inner(&mut self, command: &str, doing: &str) -> Result<Reply, OpError> {
        let t = self.timeout;
        let mut untagged = 0usize;
        let id = match timeout(t, self.session.run_command(command)).await {
            Err(_) => return Err(OpError::timed_out(&format!("while trying to {doing}"))),
            Ok(Err(e)) => return Err(error::from_command(&e, doing)),
            Ok(Ok(id)) => id,
        };
        let mut items = Vec::new();
        let mut copyuid = None;
        loop {
            let resp = match timeout(t, self.session.read_response()).await {
                Err(_) => return Err(OpError::timed_out(&format!("while trying to {doing}"))),
                Ok(Err(e)) => return Err(error::from_io(&e, doing)),
                Ok(Ok(None)) => {
                    return Err(OpError::dropped(format!(
                        "connection lost while trying to {doing}"
                    )))
                }
                Ok(Ok(Some(r))) => r,
            };
            #[derive(PartialEq)]
            enum Done {
                Ok,
                No,
                Other,
            }
            enum Step {
                Done(Done, String, Option<CopyUid>, bool),
                Bye,
                Copy(CopyUid),
                Keep(Option<Item>),
            }
            let step = match resp.parsed() {
                Response::Done {
                    tag,
                    status,
                    outcome,
                } if *tag == id => Step::Done(
                    match status {
                        Status::Ok => Done::Ok,
                        Status::No => Done::No,
                        _ => Done::Other,
                    },
                    // Inspected for response codes only; never copied into an error.
                    error::detail_text(outcome.code.as_ref(), outcome.information.as_deref()),
                    copyuid_of(outcome.code.as_ref()),
                    matches!(outcome.code, Some(ResponseCode::ReadOnly)),
                ),
                Response::Data {
                    status: Status::Bye,
                    ..
                } => Step::Bye,
                Response::Data { outcome, .. } => match copyuid_of(outcome.code.as_ref()) {
                    Some(c) => Step::Copy(c),
                    None => Step::Keep(item_from_code(outcome.code.as_ref())),
                },
                other => Step::Keep(item_of(other)),
            };
            if !matches!(step, Step::Done(..)) {
                untagged += 1;
                if untagged > self.max_items {
                    return Err(OpError::flooded(doing));
                }
            }
            match step {
                Step::Done(Done::Ok, _, done_copy, read_only) => {
                    return Ok(Reply {
                        items,
                        copyuid: done_copy.or(copyuid),
                        read_only,
                    });
                }
                Step::Done(Done::No, detail, _, _) => return Err(error::refusal(&detail, doing)),
                Step::Done(_, _, _, _) => {
                    return Err(OpError::permanent(format!(
                        "the server did not accept the request to {doing}"
                    )))
                }
                Step::Bye => {
                    return Err(OpError::dropped(format!(
                        "the server ended the session while trying to {doing}"
                    )))
                }
                Step::Copy(c) => copyuid = Some(c),
                Step::Keep(item) => items.extend(item),
            }
        }
    }

    /// `SELECT` (writable) or `EXAMINE` (read-only), with an optional parameter list such as
    /// `CONDSTORE`.
    async fn select(
        &mut self,
        folder: &str,
        writable: bool,
        params: Option<&str>,
    ) -> Result<MailboxInfo, OpError> {
        let q = folder_arg(folder).ok_or_else(bad_folder)?;
        let verb = if writable { "SELECT" } else { "EXAMINE" };
        let cmd = match params {
            Some(p) => format!("{verb} {q} ({p})"),
            None => format!("{verb} {q}"),
        };
        let reply = self.exec(&cmd, "open the folder").await?;
        let info = mailbox_info(&reply);
        if writable && info.read_only {
            return Err(OpError::permanent("the folder is read-only"));
        }
        Ok(info)
    }

    /// The UIDs in `wanted` (sorted, no duplicates) that still exist in the selected folder.
    ///
    /// The server's `SEARCH` answer is not trusted to stay inside the question: a buggy or hostile
    /// server could list other UIDs, and those would then be flagged, moved or expunged. The
    /// result is cut down to `wanted` and returned sorted without duplicates.
    async fn existing_uids(&mut self, wanted: &[Uid]) -> Result<Vec<Uid>, OpError> {
        let mut existing: Vec<Uid> = Vec::new();
        for set in uid_sets(wanted, UIDS_PER_COMMAND) {
            let reply = self
                .exec(&format!("UID SEARCH UID {set}"), "look up messages")
                .await?;
            existing.extend(search_results(&reply));
        }
        existing.retain(|u| wanted.binary_search(u).is_ok());
        existing.sort_unstable();
        existing.dedup();
        Ok(existing)
    }

    // -----------------------------------------------------------------------------------------
    // MailBackend operations
    // -----------------------------------------------------------------------------------------

    pub(crate) async fn list_folders(&mut self) -> Result<Vec<RemoteFolder>, OpError> {
        let reply = if self.caps.special_use {
            match self
                .exec("LIST \"\" \"*\" RETURN (SPECIAL-USE)", "list folders")
                .await
            {
                Ok(r) => r,
                Err(e) if e.broken => return Err(e),
                Err(_) => self.exec("LIST \"\" \"*\"", "list folders").await?,
            }
        } else {
            self.exec("LIST \"\" \"*\"", "list folders").await?
        };
        let mut out = Vec::new();
        for item in &reply.items {
            let Item::List(l) = item else {
                continue;
            };
            let special_use = if l.path.eq_ignore_ascii_case("INBOX") {
                SpecialUse::Inbox
            } else {
                // Attributes first, then the folder's name.
                l.special.unwrap_or_else(|| SpecialUse::from_name(&l.path))
            };
            out.push(RemoteFolder {
                path: l.path.clone(),
                delimiter: l.delimiter,
                special_use,
                selectable: l.selectable,
            });
        }
        Ok(out)
    }

    pub(crate) async fn folder_status(&mut self, folder: &str) -> Result<FolderStatus, OpError> {
        let q = folder_arg(folder).ok_or_else(bad_folder)?;
        let items = if self.caps.condstore {
            "UIDVALIDITY UIDNEXT MESSAGES HIGHESTMODSEQ"
        } else {
            "UIDVALIDITY UIDNEXT MESSAGES"
        };
        let reply = self
            .exec(&format!("STATUS {q} ({items})"), "read the folder's status")
            .await?;
        let mut status = FolderStatus::default();
        let mut have_validity = false;
        for item in &reply.items {
            if let Item::Status(st, has_validity) = item {
                status = *st;
                have_validity = *has_validity;
            }
        }
        if !have_validity {
            return Err(OpError::permanent("the server did not report UIDVALIDITY"));
        }
        Ok(status)
    }

    pub(crate) async fn snapshot(&mut self, folder: &str) -> Result<RemoteSnapshot, OpError> {
        let q = folder_arg(folder).ok_or_else(bad_folder)?;
        let prior = self.cache.remove(folder);

        // Cheapest path first: QRESYNC, then CONDSTORE deltas, then everything.
        let mut info = MailboxInfo::default();
        let mut messages: Option<BTreeMap<Uid, Flags>> = None;
        if self.caps.qresync {
            if let Some(p) = &prior {
                let cmd = format!(
                    "EXAMINE {q} (QRESYNC ({} {}))",
                    p.uid_validity, p.highest_modseq
                );
                let reply = self.exec(&cmd, "open the folder").await?;
                info = mailbox_info(&reply);
                if info.uid_validity == Some(p.uid_validity) {
                    let mut merged = p.messages.clone();
                    for item in &reply.items {
                        if let Item::Vanished(ranges) = item {
                            merged.retain(|u, _| !ranges.iter().any(|range| range.contains(u)));
                        }
                    }
                    apply_flag_fetches(&reply, &mut merged);
                    if merged.len() == info.exists as usize {
                        messages = Some(merged);
                    }
                }
            }
        }
        if messages.is_none() {
            let params = if self.caps.condstore {
                Some("CONDSTORE")
            } else {
                None
            };
            info = self.select(folder, false, params).await?;
            let mut base = None;
            if let (Some(p), Some(now)) = (&prior, info.highest_modseq) {
                if p.uid_validity == info.uid_validity.unwrap_or(0) && info.exists > 0 {
                    let mut merged = p.messages.clone();
                    if now != p.highest_modseq {
                        let reply = self
                            .exec(
                                &format!(
                                    "UID FETCH 1:* (FLAGS) (CHANGEDSINCE {})",
                                    p.highest_modseq
                                ),
                                "read changed flags",
                            )
                            .await?;
                        apply_flag_fetches(&reply, &mut merged);
                    }
                    if merged.len() != info.exists as usize {
                        // Something was expunged: keep only the UIDs the server still has.
                        let reply = self.exec("UID SEARCH ALL", "list messages").await?;
                        let present: HashSet<Uid> = search_results(&reply).into_iter().collect();
                        merged.retain(|u, _| present.contains(u));
                    }
                    if merged.len() == info.exists as usize {
                        base = Some(merged);
                    }
                }
            }
            messages = match base {
                Some(m) => Some(m),
                None => Some(self.full_flags(&info).await?),
            };
        }
        let messages = messages.unwrap_or_default();

        let uid_validity = info
            .uid_validity
            .ok_or_else(|| OpError::permanent("the server did not report UIDVALIDITY"))?;
        if self.caps.condstore {
            if let Some(modseq) = info.highest_modseq {
                self.cache.insert(
                    folder.to_string(),
                    FolderCache {
                        uid_validity,
                        highest_modseq: modseq,
                        messages: messages.clone(),
                    },
                );
            }
        }
        Ok(RemoteSnapshot {
            status: FolderStatus {
                uid_validity,
                uid_next: info.uid_next.unwrap_or(0),
                highest_modseq: info.highest_modseq,
                exists: u32::try_from(messages.len()).unwrap_or(u32::MAX),
            },
            messages,
        })
    }

    async fn full_flags(&mut self, info: &MailboxInfo) -> Result<BTreeMap<Uid, Flags>, OpError> {
        let mut messages = BTreeMap::new();
        if info.exists == 0 {
            return Ok(messages);
        }
        let reply = self
            .exec("UID FETCH 1:* (FLAGS)", "read the folder's flags")
            .await?;
        apply_flag_fetches(&reply, &mut messages);
        Ok(messages)
    }

    pub(crate) async fn fetch_headers(
        &mut self,
        folder: &str,
        uids: &[Uid],
    ) -> Result<Vec<RemoteHeader>, OpError> {
        let mut out = Vec::new();
        if uids.is_empty() {
            return Ok(out);
        }
        self.select(folder, false, None).await?;
        for set in uid_sets(uids, UIDS_PER_COMMAND) {
            let cmd = format!(
                "UID FETCH {set} (UID FLAGS RFC822.SIZE INTERNALDATE ENVELOPE \
                 BODY.PEEK[HEADER.FIELDS (REFERENCES IN-REPLY-TO)])"
            );
            let reply = self.exec(&cmd, "read message headers").await?;
            for item in reply.items {
                if let Item::Fetch(f) = item {
                    if let Some(h) = f.header {
                        out.push(h);
                    }
                }
            }
        }
        out.sort_by(|a, b| b.date_ms.cmp(&a.date_ms).then(b.uid.cmp(&a.uid)));
        Ok(out)
    }

    /// The whole message at `uid`, without marking it read: the folder is opened read-only
    /// (`EXAMINE`) and the content is fetched with `BODY.PEEK[]`. The size is read first
    /// (`RFC822.SIZE`), and a message over [`MAX_MESSAGE_BYTES`] is refused before any of it is
    /// downloaded. Answers from the server about other messages are ignored, and a body longer
    /// than the limit (a server that misreported the size) is refused too.
    pub(crate) async fn fetch_body(
        &mut self,
        folder: &str,
        uid: Uid,
    ) -> Result<RawMessage, OpError> {
        self.select(folder, false, None).await?;
        let reply = self
            .exec(
                &format!("UID FETCH {uid} (UID RFC822.SIZE)"),
                "read a message",
            )
            .await?;
        let size = reply.items.iter().find_map(|item| match item {
            Item::Fetch(f) if f.uid == Some(uid) => f.size,
            _ => None,
        });
        let Some(size) = size else {
            return Ok(RawMessage::Gone);
        };
        if u64::from(size) > MAX_MESSAGE_BYTES {
            return Ok(RawMessage::TooLarge {
                size: u64::from(size),
            });
        }
        let allowance = Duration::from_secs(u64::from(size) / FETCH_BYTES_PER_SECOND);
        let reply = self
            .exec_within(
                self.deadline.saturating_add(allowance),
                &format!("UID FETCH {uid} (UID BODY.PEEK[])"),
                "read a message",
            )
            .await?;
        let body = reply.items.into_iter().find_map(|item| match item {
            Item::Fetch(f) if f.uid == Some(uid) => f.body,
            _ => None,
        });
        Ok(match body {
            Some(b) if b.len() as u64 > MAX_MESSAGE_BYTES => RawMessage::TooLarge {
                size: b.len() as u64,
            },
            Some(b) => RawMessage::Found(b),
            None => RawMessage::Gone,
        })
    }

    pub(crate) async fn find_by_message_id(
        &mut self,
        folder: &str,
        message_id: &str,
    ) -> Result<Option<Uid>, OpError> {
        // Message-IDs are ASCII by RFC 5322. One that is not cannot be searched for.
        let Some(arg) = quote(&bracketed_message_id(message_id)) else {
            return Ok(None);
        };
        self.select(folder, false, None).await?;
        let reply = self
            .exec(
                &format!("UID SEARCH HEADER Message-ID {arg}"),
                "search for a message",
            )
            .await?;
        Ok(search_results(&reply).into_iter().max())
    }

    pub(crate) async fn set_flag(
        &mut self,
        folder: &str,
        uids: &[Uid],
        change: FlagChange,
    ) -> Result<(), OpError> {
        if uids.is_empty() {
            return Ok(());
        }
        let (flag, on) = match change {
            FlagChange::Seen(v) => ("\\Seen", v),
            FlagChange::Flagged(v) => ("\\Flagged", v),
        };
        let sign = if on { '+' } else { '-' };
        self.select(folder, true, None).await?;
        for set in uid_sets(uids, UIDS_PER_COMMAND) {
            self.exec(
                &format!("UID STORE {set} {sign}FLAGS.SILENT ({flag})"),
                "change message flags",
            )
            .await?;
        }
        Ok(())
    }

    /// Moves messages with `UID MOVE`, or, on a server without `MOVE`, with `UID COPY`, then
    /// `\\Deleted`, then expunge, in chunks of at most [`UIDS_PER_COMMAND`] UIDs.
    ///
    /// **A retry can leave duplicates, never lose mail.** The fallback is several commands. If the
    /// connection drops after a chunk was copied and before it was expunged, that chunk exists in
    /// both folders, and when the queue repeats the move the same messages are copied again, so
    /// the destination can hold duplicates. This is inherent in COPY-then-delete and is the
    /// price of never deleting a message that was not copied first. Servers with `MOVE` do it in
    /// one atomic command and do not have this problem.
    ///
    /// Without `UIDPLUS` the final expunge removes every `\\Deleted` message in the folder, so the
    /// check that no other message is marked deleted runs before the first chunk and again before
    /// each chunk's expunge (another client can mark one between chunks). If one has appeared the
    /// move stops with a `Permanent` refusal and nothing more is expunged.
    pub(crate) async fn move_to(
        &mut self,
        folder: &str,
        uids: &[Uid],
        dest: &str,
    ) -> Result<MoveOutcome, OpError> {
        let mut outcome = MoveOutcome::default();
        if uids.is_empty() {
            return Ok(outcome);
        }
        let dest_arg = folder_arg(dest).ok_or_else(bad_folder)?;
        self.select(folder, true, None).await?;
        // Only move what is still there, so repeating a move is harmless.
        let mut wanted: Vec<Uid> = uids.to_vec();
        wanted.sort_unstable();
        wanted.dedup();
        let existing = self.existing_uids(&wanted).await?;
        if existing.is_empty() {
            return Ok(outcome);
        }
        let fallback = !self.caps.move_cmd;
        if fallback && !self.caps.uidplus {
            // COPY + expunge without UID EXPUNGE would also remove other deleted messages.
            self.refuse_if_others_deleted(&existing).await?;
        }
        for chunk in existing.chunks(UIDS_PER_COMMAND) {
            let set = uid_set(chunk);
            if !fallback {
                let reply = self
                    .exec(&format!("UID MOVE {set} {dest_arg}"), "move messages")
                    .await?;
                record_copyuid(&mut outcome, reply.copyuid);
            } else {
                let reply = self
                    .exec(&format!("UID COPY {set} {dest_arg}"), "copy messages")
                    .await?;
                record_copyuid(&mut outcome, reply.copyuid);
                self.exec(
                    &format!("UID STORE {set} +FLAGS.SILENT (\\Deleted)"),
                    "mark messages deleted",
                )
                .await?;
                self.expunge_checked(&set, chunk).await?;
            }
        }
        Ok(outcome)
    }

    /// Marks messages `\\Deleted` and expunges them, in chunks of at most [`UIDS_PER_COMMAND`]
    /// UIDs. Repeating it is harmless: UIDs that are already gone are skipped.
    ///
    /// Without `UIDPLUS` the expunge removes every `\\Deleted` message in the folder, so the check
    /// that no other message is marked deleted runs before the first chunk and again before each
    /// chunk's expunge, because another client can mark one between chunks. If one has appeared
    /// the call stops with a `Permanent` refusal and nothing more is expunged (earlier chunks are
    /// already gone; a retry deletes the rest).
    pub(crate) async fn delete_forever(
        &mut self,
        folder: &str,
        uids: &[Uid],
    ) -> Result<(), OpError> {
        if uids.is_empty() {
            return Ok(());
        }
        self.select(folder, true, None).await?;
        let mut wanted: Vec<Uid> = uids.to_vec();
        wanted.sort_unstable();
        wanted.dedup();
        let existing = self.existing_uids(&wanted).await?;
        if existing.is_empty() {
            return Ok(());
        }
        if !self.caps.uidplus {
            self.refuse_if_others_deleted(&existing).await?;
        }
        for chunk in existing.chunks(UIDS_PER_COMMAND) {
            let set = uid_set(chunk);
            self.exec(
                &format!("UID STORE {set} +FLAGS.SILENT (\\Deleted)"),
                "mark messages deleted",
            )
            .await?;
            self.expunge_checked(&set, chunk).await?;
        }
        Ok(())
    }

    /// Expunges one chunk whose messages (`ours`, written as `set`) are already marked
    /// `\\Deleted`. Without UIDPLUS the plain `EXPUNGE` removes every `\\Deleted` message, so the
    /// check for other deleted messages is repeated here, right before each chunk's expunge: another
    /// client can mark a message deleted at any time between chunks, and removing it would lose it
    /// (in a move, before it was copied). If one has appeared, the chunk's own `\\Deleted` marks
    /// are taken off again (best effort) and the operation is refused with nothing expunged.
    async fn expunge_checked(&mut self, set: &str, ours: &[Uid]) -> Result<(), OpError> {
        if !self.caps.uidplus {
            if let Err(e) = self.refuse_if_others_deleted(ours).await {
                if !e.broken {
                    let _ = self
                        .exec(
                            &format!("UID STORE {set} -FLAGS.SILENT (\\Deleted)"),
                            "restore message flags",
                        )
                        .await;
                }
                return Err(e);
            }
        }
        self.expunge(set).await
    }

    /// Expunges `set`: `UID EXPUNGE` with UIDPLUS, otherwise a plain `EXPUNGE` (callers have
    /// checked that nothing else is marked deleted).
    async fn expunge(&mut self, set: &str) -> Result<(), OpError> {
        if self.caps.uidplus {
            self.exec(&format!("UID EXPUNGE {set}"), "remove messages")
                .await?;
        } else {
            self.exec("EXPUNGE", "remove messages").await?;
        }
        Ok(())
    }

    /// Without UIDPLUS, `EXPUNGE` removes every `\Deleted` message. Refuses (changing nothing) if
    /// the folder holds any that are not in `ours` (sorted).
    async fn refuse_if_others_deleted(&mut self, ours: &[Uid]) -> Result<(), OpError> {
        let reply = self
            .exec("UID SEARCH DELETED", "check for other deleted messages")
            .await?;
        if search_results(&reply)
            .iter()
            .any(|u| ours.binary_search(u).is_err())
        {
            return Err(OpError::permanent(
                "this folder has other messages marked deleted and the server cannot remove \
                 just these ones",
            ));
        }
        Ok(())
    }

    /// Creates a folder. Succeeds when it already exists.
    pub(crate) async fn create_folder(&mut self, folder: &str) -> Result<(), OpError> {
        let q = folder_arg(folder).ok_or_else(bad_folder)?;
        match self.exec(&format!("CREATE {q}"), "create the folder").await {
            Ok(_) => Ok(()),
            Err(OpError {
                error: BackendError::Permanent(m),
                ..
            }) if m.starts_with("that folder already exists") => Ok(()),
            Err(e) => Err(e),
        }
    }

    /// Adds a message to a folder and returns its new UID when the server says (`APPENDUID`).
    /// The overall deadline is the command deadline plus time to send the message at 16 KiB/s.
    pub(crate) async fn append(
        &mut self,
        folder: &str,
        flags: &[&str],
        message: &str,
    ) -> Result<Option<Uid>, OpError> {
        let allowance = Duration::from_secs((message.len() / APPEND_BYTES_PER_SECOND) as u64);
        match timeout(
            self.deadline.saturating_add(allowance),
            self.append_inner(folder, flags, message),
        )
        .await
        {
            Ok(result) => result,
            Err(_) => Err(OpError::too_slow("add a message")),
        }
    }

    async fn append_inner(
        &mut self,
        folder: &str,
        flags: &[&str],
        message: &str,
    ) -> Result<Option<Uid>, OpError> {
        let q = folder_arg(folder).ok_or_else(bad_folder)?;
        let flag_list = flags
            .iter()
            .filter(|f| f.starts_with('\\') || f.starts_with('$'))
            .filter(|f| {
                f.chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '\\' || c == '$')
            })
            .copied()
            .collect::<Vec<_>>()
            .join(" ");
        let cmd = format!("APPEND {q} ({flag_list}) {{{}}}", message.len());
        let t = self.timeout;
        let id = match timeout(t, self.session.run_command(&cmd)).await {
            Err(_) => return Err(OpError::timed_out("while trying to add a message")),
            Ok(Err(e)) => return Err(error::from_command(&e, "add a message")),
            Ok(Ok(id)) => id,
        };
        // Wait for the continuation request, send the literal, then read the tagged reply.
        let mut sent = false;
        let mut replies = 0usize;
        loop {
            let resp = match timeout(t, self.session.read_response()).await {
                Err(_) => return Err(OpError::timed_out("while trying to add a message")),
                Ok(Err(e)) => return Err(error::from_io(&e, "adding a message")),
                Ok(Ok(None)) => {
                    return Err(OpError::dropped("connection lost while adding a message"))
                }
                Ok(Ok(Some(r))) => r,
            };
            replies += 1;
            if replies > self.max_items {
                return Err(OpError::flooded("add a message"));
            }
            match resp.parsed() {
                Response::Continue(_) if !sent => {
                    sent = true;
                    match timeout(t, self.session.run_command_untagged(message)).await {
                        Err(_) => return Err(OpError::timed_out("while trying to add a message")),
                        Ok(Err(e)) => return Err(error::from_command(&e, "add a message")),
                        Ok(Ok(())) => {}
                    }
                }
                Response::Done {
                    tag,
                    status,
                    outcome,
                } if *tag == id => {
                    return match status {
                        Status::Ok => Ok(match &outcome.code {
                            Some(ResponseCode::AppendUid(_, set)) => {
                                expand_uids(set).into_iter().next()
                            }
                            _ => None,
                        }),
                        Status::No => Err(error::refusal(
                            &error::detail_text(
                                outcome.code.as_ref(),
                                outcome.information.as_deref(),
                            ),
                            "add a message",
                        )),
                        _ => Err(OpError::permanent("the server did not accept the message")),
                    };
                }
                _ => {}
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Parsing helpers
// ---------------------------------------------------------------------------------------------

fn bad_folder() -> OpError {
    OpError::permanent("the folder name contains characters IMAP cannot carry")
}

fn expand_uids(set: &[UidSetMember]) -> Vec<Uid> {
    let mut out = Vec::new();
    for m in set {
        match m {
            UidSetMember::Uid(u) => out.push(*u),
            UidSetMember::UidRange(r) => {
                for u in r.clone() {
                    if out.len() >= MAX_COPYUID_UIDS {
                        return out;
                    }
                    out.push(u);
                }
            }
        }
    }
    out
}

fn copyuid_of(code: Option<&ResponseCode<'_>>) -> Option<CopyUid> {
    match code {
        Some(ResponseCode::CopyUid(_validity, src, dst)) => Some(CopyUid {
            src: expand_uids(src),
            dst: expand_uids(dst),
        }),
        _ => None,
    }
}

fn record_copyuid(outcome: &mut MoveOutcome, copy: Option<CopyUid>) {
    if let Some(c) = copy {
        if c.src.len() == c.dst.len() {
            outcome.dest_uids.extend(c.src.into_iter().zip(c.dst));
        }
    }
}

fn mailbox_info(reply: &Reply) -> MailboxInfo {
    let mut info = MailboxInfo {
        read_only: reply.read_only,
        ..MailboxInfo::default()
    };
    for item in &reply.items {
        match item {
            Item::Exists(n) => info.exists = *n,
            Item::UidValidity(v) => info.uid_validity = Some(*v),
            Item::UidNext(v) => info.uid_next = Some(*v),
            Item::HighestModSeq(v) => info.highest_modseq = Some(*v),
            _ => {}
        }
    }
    info
}

fn search_results(reply: &Reply) -> Vec<Uid> {
    let mut out = Vec::new();
    for item in &reply.items {
        if let Item::Search(ids) = item {
            out.extend_from_slice(ids);
        }
    }
    out
}

/// Flags from a `FLAGS` fetch reply, merged into `into` by UID.
fn apply_flag_fetches(reply: &Reply, into: &mut BTreeMap<Uid, Flags>) {
    for item in &reply.items {
        if let Item::Fetch(fetch) = item {
            if let (Some(u), Some(f)) = (fetch.uid, &fetch.flags) {
                into.insert(u, f.clone());
            }
        }
    }
}

/// Copies the parts of an untagged response that Croft uses into an owned [`Item`].
fn item_of(resp: &Response<'_>) -> Option<Item> {
    match resp {
        Response::MailboxData(MailboxDatum::Exists(n)) => Some(Item::Exists(*n)),
        Response::MailboxData(MailboxDatum::Search(ids)) => Some(Item::Search(ids.clone())),
        Response::MailboxData(MailboxDatum::Status { status, .. }) => {
            use async_imap::types::StatusAttribute as S;
            let mut st = FolderStatus::default();
            let mut has_validity = false;
            for a in status {
                match a {
                    S::UidValidity(v) => {
                        st.uid_validity = *v;
                        has_validity = true;
                    }
                    S::UidNext(v) => st.uid_next = *v,
                    S::Messages(v) => st.exists = *v,
                    S::HighestModSeq(v) => st.highest_modseq = Some(*v),
                    _ => {}
                }
            }
            Some(Item::Status(st, has_validity))
        }
        Response::MailboxData(MailboxDatum::List(data)) => {
            let mut selectable = true;
            let mut special = None;
            for attr in &data.name_attributes {
                let found = match attr {
                    NameAttribute::NoSelect => {
                        selectable = false;
                        None
                    }
                    NameAttribute::All => Some(SpecialUse::All),
                    NameAttribute::Archive => Some(SpecialUse::Archive),
                    NameAttribute::Drafts => Some(SpecialUse::Drafts),
                    NameAttribute::Flagged => Some(SpecialUse::Flagged),
                    NameAttribute::Junk => Some(SpecialUse::Junk),
                    NameAttribute::Sent => Some(SpecialUse::Sent),
                    NameAttribute::Trash => Some(SpecialUse::Trash),
                    NameAttribute::Extension(text) => {
                        if text.eq_ignore_ascii_case("\\NonExistent") {
                            selectable = false;
                            None
                        } else {
                            SpecialUse::from_attribute(text)
                        }
                    }
                    _ => None,
                };
                special = special.or(found);
            }
            Some(Item::List(ListItem {
                path: utf7_decode(&data.name),
                delimiter: data.delimiter.as_deref().and_then(|d| d.chars().next()),
                selectable,
                special,
            }))
        }
        Response::Vanished { uids, .. } => Some(Item::Vanished(uids.clone())),
        Response::Fetch(_, attrs) => Some(Item::Fetch(Box::new(fetch_item(attrs)))),
        _ => None,
    }
}

fn item_from_code(code: Option<&ResponseCode<'_>>) -> Option<Item> {
    match code {
        Some(ResponseCode::UidValidity(v)) => Some(Item::UidValidity(*v)),
        Some(ResponseCode::UidNext(v)) => Some(Item::UidNext(*v)),
        Some(ResponseCode::HighestModSeq(v)) => Some(Item::HighestModSeq(*v)),
        _ => None,
    }
}

fn fetch_item(attrs: &[AttributeValue<'_>]) -> FetchItem {
    let mut uid = None;
    let mut flags = None;
    let mut has_envelope = false;
    let mut size = None;
    let mut body = None;
    for a in attrs {
        match a {
            AttributeValue::Uid(u) => uid = Some(*u),
            AttributeValue::Flags(f) => flags = Some(flags_from(f.iter().map(|s| s.as_ref()))),
            AttributeValue::Envelope(_) => has_envelope = true,
            AttributeValue::Rfc822Size(n) => size = Some(*n),
            // `BODY[]`: the whole message (no section, no partial range).
            AttributeValue::BodySection {
                section: None,
                index: None,
                data: Some(data),
            } => body = Some(data.to_vec()),
            AttributeValue::Rfc822(Some(data)) => body = Some(data.to_vec()),
            _ => {}
        }
    }
    FetchItem {
        uid,
        flags,
        size,
        body,
        header: if has_envelope {
            header_from(attrs)
        } else {
            None
        },
    }
}

/// System flags go to their fields, `\Recent` is ignored (the server sets it, not the person), and
/// everything not starting with a backslash is a keyword.
pub(crate) fn flags_from<'a>(raw: impl Iterator<Item = &'a str>) -> Flags {
    let mut flags = Flags::default();
    for f in raw {
        match f.to_ascii_lowercase().as_str() {
            "\\seen" => flags.seen = true,
            "\\flagged" => flags.flagged = true,
            "\\answered" => flags.answered = true,
            "\\draft" => flags.draft = true,
            "\\deleted" => flags.deleted = true,
            other if other.starts_with('\\') => {}
            _ => {
                flags.keywords.insert(f.to_string());
            }
        }
    }
    flags
}

fn text_of(bytes: &[u8]) -> String {
    decode_encoded_words(&String::from_utf8_lossy(bytes))
}

fn header_from(attrs: &[AttributeValue<'_>]) -> Option<RemoteHeader> {
    let mut uid = None;
    let mut size = 0;
    let mut date_ms = 0;
    let mut flags = Flags::default();
    let mut subject = String::new();
    let mut message_id = None;
    let mut from_name = None;
    let mut from_addr = None;
    let mut extra = Vec::new();
    let mut in_reply_to_env: Option<String> = None;
    for a in attrs {
        match a {
            AttributeValue::Uid(u) => uid = Some(*u),
            AttributeValue::Rfc822Size(s) => size = *s,
            AttributeValue::Flags(f) => flags = flags_from(f.iter().map(|s| s.as_ref())),
            AttributeValue::InternalDate(d) => {
                date_ms = chrono::DateTime::parse_from_str(d.trim(), "%d-%b-%Y %H:%M:%S %z")
                    .map(|dt| dt.timestamp_millis())
                    .unwrap_or(0);
            }
            AttributeValue::Envelope(env) => {
                subject = env.subject.as_deref().map(text_of).unwrap_or_default();
                message_id = env
                    .message_id
                    .as_deref()
                    .map(|b| String::from_utf8_lossy(b).trim().to_string())
                    .filter(|s| !s.is_empty());
                in_reply_to_env = env
                    .in_reply_to
                    .as_deref()
                    .map(|b| String::from_utf8_lossy(b).into_owned());
                if let Some(first) = env.from.as_ref().and_then(|v| v.first()) {
                    from_name = first
                        .name
                        .as_deref()
                        .map(text_of)
                        .filter(|s| !s.trim().is_empty());
                    if let Some(mailbox) = first.mailbox.as_deref() {
                        let mailbox = String::from_utf8_lossy(mailbox);
                        from_addr = Some(match first.host.as_deref() {
                            Some(host) => format!("{mailbox}@{}", String::from_utf8_lossy(host)),
                            None => mailbox.into_owned(),
                        });
                    }
                }
            }
            AttributeValue::BodySection {
                section: Some(SectionPath::Full(MessageSection::Header)),
                data: Some(data),
                ..
            } => extra = reference_ids(data),
            _ => {}
        }
    }
    let in_reply_to = in_reply_to_env
        .map(|irt| reference_ids(format!("In-Reply-To: {irt}\r\n").as_bytes()))
        .unwrap_or_default();
    let references = super::text::unique_ids(extra.into_iter().chain(in_reply_to));
    Some(RemoteHeader {
        uid: uid?,
        message_id,
        references,
        subject,
        from_name,
        from_addr,
        date_ms,
        size,
        flags,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flags_are_split_into_fields_and_keywords() {
        let f = flags_from(["\\Seen", "\\FLAGGED", "\\Recent", "$Junk", "Work"].into_iter());
        assert!(f.seen && f.flagged && !f.answered && !f.deleted);
        assert_eq!(
            f.keywords.iter().map(String::as_str).collect::<Vec<_>>(),
            vec!["$Junk", "Work"]
        );
    }

    #[test]
    fn copyuid_pairs_follow_the_server_order() {
        let mut out = MoveOutcome::default();
        record_copyuid(
            &mut out,
            Some(CopyUid {
                src: vec![4, 5, 6],
                dst: vec![40, 41, 42],
            }),
        );
        assert_eq!(out.dest_uids.get(&5), Some(&41));
        // A malformed pair of sets (different lengths) is ignored rather than guessed at.
        let mut bad = MoveOutcome::default();
        record_copyuid(
            &mut bad,
            Some(CopyUid {
                src: vec![1, 2],
                dst: vec![9],
            }),
        );
        assert!(bad.dest_uids.is_empty());
    }

    #[test]
    fn uid_set_members_expand() {
        let set = vec![UidSetMember::Uid(3), UidSetMember::UidRange(10..=12)];
        assert_eq!(expand_uids(&set), vec![3, 10, 11, 12]);
    }
}
