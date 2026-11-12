// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The protocol side of the test server: connection loop and command handlers.

use crate::{Caps, Folder, Message, Misbehavior, Shared, State, TlsMode};
use std::collections::BTreeSet;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tokio::sync::watch;
use tokio_rustls::TlsAcceptor;

/// How a connection loop ended.
enum End<S> {
    Closed,
    Upgrade(S),
}

#[derive(Default)]
struct Sess {
    authed: bool,
    selected: Option<(String, bool)>,
    condstore: bool,
    qresync: bool,
}

pub(crate) async fn serve_connection(
    tcp: TcpStream,
    state: Shared,
    acceptor: Option<TlsAcceptor>,
    mut kill: watch::Receiver<u64>,
) {
    // Only a drop_connections() call made after this connection starts should close it.
    kill.mark_unchanged();
    let mode = {
        let mut st = state.lock().expect("lock");
        st.connections += 1;
        st.opts.tls
    };
    match (mode, acceptor) {
        (TlsMode::Implicit, Some(a)) => {
            if let Ok(tls) = a.accept(tcp).await {
                let _ = run(tls, &state, true, true, &mut kill).await;
            }
        }
        (TlsMode::StartTls, Some(a)) => {
            if let End::Upgrade(tcp) = run(tcp, &state, false, true, &mut kill).await {
                if let Ok(tls) = a.accept(tcp).await {
                    let _ = run(tls, &state, true, false, &mut kill).await;
                }
            }
        }
        _ => {
            let _ = run(tcp, &state, true, true, &mut kill).await;
        }
    }
}

fn capability_text(caps: &Caps, tls: TlsMode, secure: bool) -> String {
    let mut c = String::from("IMAP4rev1");
    if tls == TlsMode::StartTls && !secure {
        c.push_str(" STARTTLS LOGINDISABLED");
    } else {
        c.push_str(" AUTH=XOAUTH2");
    }
    for (on, name) in [
        (caps.move_cmd, "MOVE"),
        (caps.uidplus, "UIDPLUS"),
        (caps.special_use, "SPECIAL-USE"),
        (caps.list_extended, "LIST-EXTENDED"),
        (caps.idle, "IDLE"),
        (caps.condstore, "CONDSTORE"),
        (caps.qresync, "QRESYNC"),
        (caps.enable, "ENABLE"),
    ] {
        if on {
            c.push(' ');
            c.push_str(name);
        }
    }
    c
}

async fn run<S: AsyncRead + AsyncWrite + Unpin>(
    stream: S,
    state: &Shared,
    secure: bool,
    greet: bool,
    kill: &mut watch::Receiver<u64>,
) -> End<S> {
    let mut rd = BufReader::new(stream);
    let mut sess = Sess::default();
    if greet {
        let text = {
            let st = state.lock().expect("lock");
            capability_text(&st.opts.caps, st.opts.tls, secure)
        };
        if rd
            .get_mut()
            .write_all(format!("* OK [CAPABILITY {text}] test server ready\r\n").as_bytes())
            .await
            .is_err()
        {
            return End::Closed;
        }
    }
    loop {
        let mut line = String::new();
        let n = tokio::select! {
            r = rd.read_line(&mut line) => r,
            _ = kill.changed() => return End::Closed,
        };
        if !matches!(n, Ok(k) if k > 0) {
            return End::Closed;
        }
        let line = line.trim_end_matches(['\r', '\n']).to_string();
        let (tag, rest) = match line.split_once(' ') {
            Some((t, r)) => (t.to_string(), r.to_string()),
            None => (line.clone(), String::new()),
        };
        let (cmd, args) = match rest.split_once(' ') {
            Some((c, a)) => (c.to_ascii_uppercase(), a.to_string()),
            None => (rest.to_ascii_uppercase(), String::new()),
        };
        let hang = {
            let mut st = state.lock().expect("lock");
            let logged = if cmd == "LOGIN" || cmd == "AUTHENTICATE" {
                format!("{cmd} ***")
            } else {
                rest.clone()
            };
            st.commands.push(logged);
            st.hang
        };
        if hang {
            tokio::select! {
                _ = std::future::pending::<()>() => {},
                _ = kill.changed() => return End::Closed,
            }
        }
        let toks = tokenize(&args);
        let out: String;
        let how = state.lock().expect("lock").misbehavior;
        if (cmd == "LOGIN" || cmd == "AUTHENTICATE") && how == Misbehavior::ByeOnLogin {
            if cmd == "AUTHENTICATE" {
                // Let the client send its credentials first, then hang up.
                let _ = rd.get_mut().write_all(b"+ \r\n").await;
                let mut resp = String::new();
                let _ = rd.read_line(&mut resp).await;
            }
            let _ = rd
                .get_mut()
                .write_all(b"* BYE Too many invalid login attempts\r\n")
                .await;
            return End::Closed;
        }
        if cmd == "LIST" && sess.authed {
            match how {
                Misbehavior::HugeLiteral(n) => {
                    let line = format!("* LIST () \"/\" {{{n}}}\r\nab");
                    let _ = rd.get_mut().write_all(line.as_bytes()).await;
                    kill.changed().await.ok();
                    return End::Closed;
                }
                Misbehavior::EndlessExists => {
                    let chunk = "* 1 EXISTS\r\n".repeat(512);
                    loop {
                        tokio::select! {
                            r = rd.get_mut().write_all(chunk.as_bytes()) => {
                                if r.is_err() {
                                    return End::Closed;
                                }
                            }
                            _ = kill.changed() => return End::Closed,
                        }
                        tokio::task::yield_now().await;
                    }
                }
                _ => {}
            }
        }
        match cmd.as_str() {
            "STARTTLS" => {
                let ok = {
                    let st = state.lock().expect("lock");
                    st.opts.tls == TlsMode::StartTls && !secure
                };
                if ok {
                    let _ = rd
                        .get_mut()
                        .write_all(format!("{tag} OK Begin TLS negotiation now\r\n").as_bytes())
                        .await;
                    return End::Upgrade(rd.into_inner());
                }
                out = format!("{tag} BAD STARTTLS not available\r\n");
            }
            "AUTHENTICATE" => {
                let _ = rd.get_mut().write_all(b"+ \r\n").await;
                let mut resp = String::new();
                if !matches!(rd.read_line(&mut resp).await, Ok(k) if k > 0) {
                    return End::Closed;
                }
                let (good, unavailable) = {
                    let st = state.lock().expect("lock");
                    (
                        check_xoauth(resp.trim(), &st.opts.username, &st.opts.password),
                        st.login_unavailable,
                    )
                };
                if unavailable {
                    out = format!("{tag} NO [UNAVAILABLE] Temporary authentication failure\r\n");
                } else if good && secure {
                    sess.authed = true;
                    out = format!("{tag} OK Authenticated\r\n");
                } else {
                    // RFC 7628: send the error challenge, wait for the empty line, then NO.
                    let _ = rd
                        .get_mut()
                        .write_all(b"+ eyJzdGF0dXMiOiI0MDEifQ==\r\n")
                        .await;
                    let mut empty = String::new();
                    if !matches!(rd.read_line(&mut empty).await, Ok(k) if k > 0) {
                        return End::Closed;
                    }
                    out = if how == Misbehavior::AuthFailedMentioningUnavailable {
                        format!(
                            "{tag} NO [AUTHENTICATIONFAILED] Authentication failed: account unavailable\r\n"
                        )
                    } else {
                        format!("{tag} NO [AUTHENTICATIONFAILED] Authentication failed.\r\n")
                    };
                }
            }
            "APPEND" => {
                out = append_command(&mut rd, state, &sess, &tag, &toks).await;
            }
            _ => {
                out = {
                    let mut st = state.lock().expect("lock");
                    handle(&mut st, &mut sess, secure, &tag, &cmd, &toks)
                };
                if cmd == "LOGOUT" {
                    let _ = rd.get_mut().write_all(out.as_bytes()).await;
                    return End::Closed;
                }
            }
        }
        if rd.get_mut().write_all(out.as_bytes()).await.is_err() {
            return End::Closed;
        }
    }
}

fn check_xoauth(b64: &str, user: &str, token: &str) -> bool {
    let Some(bytes) = decode_b64(b64) else {
        return false;
    };
    let text = String::from_utf8_lossy(&bytes).into_owned();
    text == format!("user={user}\u{1}auth=Bearer {token}\u{1}\u{1}")
}

fn decode_b64(s: &str) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    let mut buf = 0u32;
    let mut bits = 0;
    for c in s.trim_end_matches('=').bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return None,
        } as u32;
        buf = (buf << 6) | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push(((buf >> bits) & 0xff) as u8);
        }
    }
    Some(out)
}

// -------------------------------------------------------------------------------------------
// Tokens
// -------------------------------------------------------------------------------------------

#[derive(Debug, Clone)]
enum Tok {
    Atom(String),
    Str(String),
    List(Vec<Tok>),
}

impl Tok {
    fn text(&self) -> Option<&str> {
        match self {
            Tok::Atom(s) | Tok::Str(s) => Some(s),
            Tok::List(_) => None,
        }
    }
    fn list(&self) -> Option<&[Tok]> {
        match self {
            Tok::List(l) => Some(l),
            _ => None,
        }
    }
}

fn tokenize(s: &str) -> Vec<Tok> {
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;
    parse_toks(&chars, &mut i)
}

fn parse_toks(c: &[char], i: &mut usize) -> Vec<Tok> {
    let mut out = Vec::new();
    while *i < c.len() {
        match c[*i] {
            ' ' => *i += 1,
            ')' => {
                *i += 1;
                return out;
            }
            '(' => {
                *i += 1;
                out.push(Tok::List(parse_toks(c, i)));
            }
            '"' => {
                *i += 1;
                let mut s = String::new();
                while *i < c.len() && c[*i] != '"' {
                    if c[*i] == '\\' && *i + 1 < c.len() {
                        *i += 1;
                    }
                    s.push(c[*i]);
                    *i += 1;
                }
                *i += 1;
                out.push(Tok::Str(s));
            }
            _ => {
                let mut s = String::new();
                while *i < c.len() && c[*i] != ' ' && c[*i] != ')' {
                    if c[*i] == '[' {
                        while *i < c.len() && c[*i] != ']' {
                            s.push(c[*i]);
                            *i += 1;
                        }
                    }
                    if *i < c.len() {
                        s.push(c[*i]);
                        *i += 1;
                    }
                }
                out.push(Tok::Atom(s));
            }
        }
    }
    out
}

// -------------------------------------------------------------------------------------------
// UID sets
// -------------------------------------------------------------------------------------------

fn parse_set(s: &str, max: u32) -> Vec<(u32, u32)> {
    let num = |t: &str| {
        if t == "*" {
            max
        } else {
            t.parse().unwrap_or(0)
        }
    };
    s.split(',')
        .map(|part| match part.split_once(':') {
            Some((a, b)) => {
                let (a, b) = (num(a), num(b));
                (a.min(b), a.max(b))
            }
            None => (num(part), num(part)),
        })
        .collect()
}

fn in_set(set: &[(u32, u32)], uid: u32) -> bool {
    set.iter().any(|(a, b)| uid >= *a && uid <= *b)
}

fn uid_list(uids: &[u32]) -> String {
    uids.iter()
        .map(u32::to_string)
        .collect::<Vec<_>>()
        .join(",")
}

// -------------------------------------------------------------------------------------------
// State changes shared with the public API
// -------------------------------------------------------------------------------------------

pub(crate) fn append(
    st: &mut State,
    folder: &str,
    flags: BTreeSet<String>,
    raw: &str,
) -> Option<u32> {
    let f = st.folders.iter_mut().find(|f| f.name == folder)?;
    let uid = f.uid_next;
    f.uid_next += 1;
    f.highest_modseq += 1;
    f.messages.push(Message {
        uid,
        flags,
        raw: raw.to_string(),
        internal_date: format!(
            "01-Jan-2026 10:{:02}:{:02} +0000",
            (uid / 60) % 60,
            uid % 60
        ),
        modseq: f.highest_modseq,
    });
    Some(uid)
}

async fn append_command<S: AsyncRead + AsyncWrite + Unpin>(
    rd: &mut BufReader<S>,
    state: &Shared,
    sess: &Sess,
    tag: &str,
    toks: &[Tok],
) -> String {
    if !sess.authed {
        return format!("{tag} NO Not authenticated\r\n");
    }
    let Some(folder) = toks.first().and_then(Tok::text) else {
        return format!("{tag} BAD Missing folder\r\n");
    };
    let flags: BTreeSet<String> = toks
        .iter()
        .find_map(Tok::list)
        .map(|l| {
            l.iter()
                .filter_map(|t| t.text().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    let size = toks.last().and_then(Tok::text).and_then(|t| {
        t.trim_matches(|c| c == '{' || c == '}' || c == '+')
            .parse::<usize>()
            .ok()
    });
    let Some(size) = size else {
        return format!("{tag} BAD Missing literal\r\n");
    };
    let exists = state
        .lock()
        .expect("lock")
        .folders
        .iter()
        .any(|f| f.name == folder);
    if !exists {
        return format!("{tag} NO [TRYCREATE] No such folder\r\n");
    }
    let _ = rd
        .get_mut()
        .write_all(b"+ Ready for literal data\r\n")
        .await;
    let mut buf = vec![0u8; size];
    if rd.read_exact(&mut buf).await.is_err() {
        return String::new();
    }
    let mut rest = String::new();
    let _ = rd.read_line(&mut rest).await;
    let raw = String::from_utf8_lossy(&buf).into_owned();
    let mut st = state.lock().expect("lock");
    let uses_uidplus = st.opts.caps.uidplus;
    match append(&mut st, folder, flags, &raw) {
        Some(uid) => {
            let validity = st
                .folders
                .iter()
                .find(|f| f.name == folder)
                .map(|f| f.uid_validity)
                .unwrap_or(0);
            if uses_uidplus {
                format!("{tag} OK [APPENDUID {validity} {uid}] Append completed\r\n")
            } else {
                format!("{tag} OK Append completed\r\n")
            }
        }
        None => format!("{tag} NO [TRYCREATE] No such folder\r\n"),
    }
}

// -------------------------------------------------------------------------------------------
// Commands
// -------------------------------------------------------------------------------------------

fn flags_text(flags: &BTreeSet<String>) -> String {
    format!("({})", flags.iter().cloned().collect::<Vec<_>>().join(" "))
}

fn quote(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

fn handle(
    st: &mut State,
    sess: &mut Sess,
    secure: bool,
    tag: &str,
    cmd: &str,
    toks: &[Tok],
) -> String {
    let caps = st.opts.caps;
    match cmd {
        "CAPABILITY" => format!(
            "* CAPABILITY {}\r\n{tag} OK Capability completed\r\n",
            capability_text(&caps, st.opts.tls, secure)
        ),
        "NOOP" | "CHECK" => format!("{tag} OK Done\r\n"),
        "LOGOUT" => format!("* BYE Logging out\r\n{tag} OK Logout completed\r\n"),
        "LOGIN" => {
            if st.opts.tls == TlsMode::StartTls && !secure {
                return format!("{tag} NO [PRIVACYREQUIRED] Plaintext login is disabled\r\n");
            }
            if st.login_unavailable {
                return format!("{tag} NO [UNAVAILABLE] Temporary authentication failure\r\n");
            }
            if st.misbehavior == Misbehavior::AuthFailedMentioningUnavailable {
                return format!(
                    "{tag} NO [AUTHENTICATIONFAILED] Authentication failed: account unavailable\r\n"
                );
            }
            let user = toks.first().and_then(Tok::text).unwrap_or("");
            let pass = toks.get(1).and_then(Tok::text).unwrap_or("");
            if user == st.opts.username && pass == st.opts.password {
                sess.authed = true;
                format!(
                    "{tag} OK [CAPABILITY {}] Logged in\r\n",
                    capability_text(&caps, st.opts.tls, secure)
                )
            } else {
                format!("{tag} NO [AUTHENTICATIONFAILED] Authentication failed.\r\n")
            }
        }
        _ if !sess.authed => format!("{tag} NO Not authenticated\r\n"),
        "ENABLE" => {
            if !caps.enable {
                return format!("{tag} BAD Unknown command\r\n");
            }
            let wanted: Vec<String> = toks
                .iter()
                .filter_map(Tok::text)
                .map(str::to_ascii_uppercase)
                .collect();
            let mut enabled = Vec::new();
            if wanted.iter().any(|w| w == "QRESYNC") && caps.qresync {
                sess.qresync = true;
                sess.condstore = true;
                enabled.push("QRESYNC");
            }
            if wanted.iter().any(|w| w == "CONDSTORE") && caps.condstore {
                sess.condstore = true;
                enabled.push("CONDSTORE");
            }
            format!("* ENABLED {}\r\n{tag} OK Enabled\r\n", enabled.join(" "))
        }
        "CREATE" => {
            let Some(name) = toks.first().and_then(Tok::text) else {
                return format!("{tag} BAD Missing folder\r\n");
            };
            if st.folders.iter().any(|f| f.name == name) {
                return format!("{tag} NO [ALREADYEXISTS] Mailbox already exists\r\n");
            }
            st.folders.push(Folder::new(name, None));
            format!("{tag} OK Create completed\r\n")
        }
        "LIST" => list(st, tag, toks),
        "STATUS" => status(st, sess, tag, toks),
        "SELECT" | "EXAMINE" => select(st, sess, tag, cmd == "SELECT", toks),
        "CLOSE" => {
            sess.selected = None;
            format!("{tag} OK Closed\r\n")
        }
        "EXPUNGE" => expunge(st, sess, tag, None),
        "UID" => {
            let Some(sub) = toks
                .first()
                .and_then(Tok::text)
                .map(str::to_ascii_uppercase)
            else {
                return format!("{tag} BAD Missing UID command\r\n");
            };
            let rest = &toks[1..];
            match sub.as_str() {
                "FETCH" => uid_fetch(st, sess, tag, rest),
                "STORE" => uid_store(st, sess, tag, rest),
                "COPY" => copy_or_move(st, sess, tag, rest, false),
                "MOVE" => {
                    if !caps.move_cmd {
                        return format!("{tag} BAD Unknown command MOVE\r\n");
                    }
                    copy_or_move(st, sess, tag, rest, true)
                }
                "EXPUNGE" => {
                    if !caps.uidplus {
                        return format!("{tag} BAD Unknown command UID EXPUNGE\r\n");
                    }
                    expunge(st, sess, tag, rest.first().and_then(Tok::text))
                }
                "SEARCH" => uid_search(st, sess, tag, rest),
                _ => format!("{tag} BAD Unknown UID command\r\n"),
            }
        }
        _ => format!("{tag} BAD Unknown command\r\n"),
    }
}

fn list(st: &State, tag: &str, toks: &[Tok]) -> String {
    let caps = st.opts.caps;
    let returns_special = toks
        .iter()
        .any(|t| matches!(t, Tok::Atom(a) if a.eq_ignore_ascii_case("RETURN")));
    if returns_special && !caps.list_extended {
        return format!("{tag} BAD LIST-EXTENDED is not supported\r\n");
    }
    let show_special = caps.special_use && (returns_special || st.plain_list_has_special_use);
    let mut out = String::new();
    for f in &st.folders {
        let mut attrs = vec!["\\HasNoChildren".to_string()];
        if show_special {
            if let Some(s) = &f.special {
                attrs.push(s.clone());
            }
        }
        out.push_str(&format!(
            "* LIST ({}) \"/\" {}\r\n",
            attrs.join(" "),
            quote(&f.name)
        ));
    }
    out.push_str(&format!("{tag} OK List completed\r\n"));
    out
}

fn find<'a>(st: &'a State, name: &str) -> Option<&'a Folder> {
    st.folders.iter().find(|f| f.name == name)
}

fn status(st: &State, sess: &mut Sess, tag: &str, toks: &[Tok]) -> String {
    let Some(name) = toks.first().and_then(Tok::text) else {
        return format!("{tag} BAD Missing folder\r\n");
    };
    let Some(f) = find(st, name) else {
        return format!("{tag} NO [NONEXISTENT] Mailbox doesn't exist\r\n");
    };
    let mut parts = Vec::new();
    for item in toks.get(1).and_then(Tok::list).unwrap_or(&[]) {
        match item.text().map(str::to_ascii_uppercase).as_deref() {
            Some("UIDVALIDITY") => parts.push(format!("UIDVALIDITY {}", f.uid_validity)),
            Some("UIDNEXT") => parts.push(format!("UIDNEXT {}", f.uid_next)),
            Some("MESSAGES") => parts.push(format!("MESSAGES {}", f.messages.len())),
            Some("HIGHESTMODSEQ") => {
                if !st.opts.caps.condstore {
                    return format!("{tag} BAD HIGHESTMODSEQ needs CONDSTORE\r\n");
                }
                sess.condstore = true;
                parts.push(format!("HIGHESTMODSEQ {}", f.highest_modseq));
            }
            _ => {}
        }
    }
    format!(
        "* STATUS {} ({})\r\n{tag} OK Status completed\r\n",
        quote(name),
        parts.join(" ")
    )
}

fn select(st: &State, sess: &mut Sess, tag: &str, writable: bool, toks: &[Tok]) -> String {
    let Some(name) = toks.first().and_then(Tok::text) else {
        return format!("{tag} BAD Missing folder\r\n");
    };
    let Some(f) = find(st, name) else {
        sess.selected = None;
        return format!("{tag} NO [NONEXISTENT] Mailbox doesn't exist\r\n");
    };
    let caps = st.opts.caps;
    let mut qresync: Option<(u32, u64)> = None;
    for p in toks.get(1).and_then(Tok::list).unwrap_or(&[]) {
        match p {
            Tok::Atom(a) if a.eq_ignore_ascii_case("CONDSTORE") => {
                if !caps.condstore {
                    return format!("{tag} BAD CONDSTORE not supported\r\n");
                }
                sess.condstore = true;
            }
            Tok::Atom(a) if a.eq_ignore_ascii_case("QRESYNC") => {}
            Tok::List(l) => {
                if !sess.qresync {
                    return format!("{tag} BAD QRESYNC not enabled\r\n");
                }
                let uv = l.first().and_then(Tok::text).and_then(|t| t.parse().ok());
                let ms = l.get(1).and_then(Tok::text).and_then(|t| t.parse().ok());
                if let (Some(uv), Some(ms)) = (uv, ms) {
                    qresync = Some((uv, ms));
                }
            }
            _ => {}
        }
    }
    let read_write = writable && !f.read_only;
    sess.selected = Some((name.to_string(), read_write));
    let mut out = String::new();
    out.push_str("* FLAGS (\\Answered \\Flagged \\Deleted \\Seen \\Draft)\r\n");
    out.push_str(&format!("* {} EXISTS\r\n* 0 RECENT\r\n", f.messages.len()));
    out.push_str(&format!(
        "* OK [UIDVALIDITY {}] UIDs valid\r\n",
        f.uid_validity
    ));
    out.push_str(&format!(
        "* OK [UIDNEXT {}] Predicted next UID\r\n",
        f.uid_next
    ));
    out.push_str(
        "* OK [PERMANENTFLAGS (\\Answered \\Flagged \\Deleted \\Seen \\Draft \\*)] Limited\r\n",
    );
    if sess.condstore {
        out.push_str(&format!(
            "* OK [HIGHESTMODSEQ {}] Highest\r\n",
            f.highest_modseq
        ));
    }
    if let Some((uv, since)) = qresync {
        if uv == f.uid_validity {
            let gone: Vec<u32> = f
                .expunged
                .iter()
                .filter(|(_, m)| *m > since)
                .map(|(u, _)| *u)
                .collect();
            if !gone.is_empty() {
                out.push_str(&format!("* VANISHED (EARLIER) {}\r\n", uid_list(&gone)));
            }
            for (i, m) in f.messages.iter().enumerate() {
                if m.modseq > since {
                    out.push_str(&format!(
                        "* {} FETCH (UID {} MODSEQ ({}) FLAGS {})\r\n",
                        i + 1,
                        m.uid,
                        m.modseq,
                        flags_text(&m.flags)
                    ));
                }
            }
        }
    }
    let code = if read_write {
        "READ-WRITE"
    } else {
        "READ-ONLY"
    };
    out.push_str(&format!("{tag} OK [{code}] Select completed\r\n"));
    out
}

fn selected<'a>(st: &'a mut State, sess: &Sess) -> Option<(&'a mut Folder, bool)> {
    let (name, rw) = sess.selected.as_ref()?;
    let rw = *rw;
    st.folders
        .iter_mut()
        .find(|f| &f.name == name)
        .map(|f| (f, rw))
}

fn max_uid(f: &Folder) -> u32 {
    f.messages.iter().map(|m| m.uid).max().unwrap_or(0)
}

fn uid_fetch(st: &mut State, sess: &mut Sess, tag: &str, toks: &[Tok]) -> String {
    let caps = st.opts.caps;
    let Some((f, _)) = selected(st, sess) else {
        return format!("{tag} BAD No folder selected\r\n");
    };
    let set = parse_set(toks.first().and_then(Tok::text).unwrap_or(""), max_uid(f));
    let items: Vec<String> = match toks.get(1) {
        Some(Tok::List(l)) => l
            .iter()
            .filter_map(|t| t.text().map(str::to_string))
            .collect(),
        Some(t) => t
            .text()
            .map(|s| vec![s.to_string()])
            .into_iter()
            .flatten()
            .collect(),
        None => Vec::new(),
    };
    let mut changed_since = None;
    if let Some(Tok::List(m)) = toks.get(2) {
        if m.first()
            .and_then(Tok::text)
            .is_some_and(|t| t.eq_ignore_ascii_case("CHANGEDSINCE"))
        {
            if !caps.condstore {
                return format!("{tag} BAD CHANGEDSINCE needs CONDSTORE\r\n");
            }
            sess.condstore = true;
            changed_since = m
                .get(1)
                .and_then(Tok::text)
                .and_then(|t| t.parse::<u64>().ok());
        }
    }
    let Some((f, rw)) = selected(st, sess) else {
        return format!("{tag} BAD No folder selected\r\n");
    };
    // `BODY[]` (not `BODY.PEEK[]`) sets `\Seen` on a folder opened read-write, as RFC 3501 says,
    // so a test can tell whether a client peeked.
    let sets_seen = rw && items.iter().any(|i| i.eq_ignore_ascii_case("BODY[]"));
    if sets_seen {
        f.highest_modseq += 1;
        let modseq = f.highest_modseq;
        for m in f.messages.iter_mut() {
            if in_set(&set, m.uid) && m.flags.insert("\\Seen".to_string()) {
                m.modseq = modseq;
            }
        }
    }
    let mut out = String::new();
    for (i, m) in f.messages.iter().enumerate() {
        if !in_set(&set, m.uid) {
            continue;
        }
        if changed_since.is_some_and(|c| m.modseq <= c) {
            continue;
        }
        let mut parts = vec![format!("UID {}", m.uid)];
        if sess.condstore {
            parts.push(format!("MODSEQ ({})", m.modseq));
        }
        for item in &items {
            let up = item.to_ascii_uppercase();
            if up == "FLAGS" {
                parts.push(format!("FLAGS {}", flags_text(&m.flags)));
            } else if up == "RFC822.SIZE" {
                parts.push(format!("RFC822.SIZE {}", m.raw.len()));
            } else if up == "INTERNALDATE" {
                parts.push(format!("INTERNALDATE \"{}\"", m.internal_date));
            } else if up == "ENVELOPE" {
                parts.push(format!("ENVELOPE {}", envelope(&m.raw)));
            } else if up == "BODY.PEEK[]" || up == "BODY[]" {
                parts.push(format!("BODY[] {{{}}}\r\n{}", m.raw.len(), m.raw));
            } else if up.starts_with("BODY.PEEK[HEADER.FIELDS")
                || up.starts_with("BODY[HEADER.FIELDS")
            {
                let names: Vec<String> = item
                    .split_once('(')
                    .and_then(|(_, r)| r.split_once(')'))
                    .map(|(l, _)| l.split_whitespace().map(str::to_ascii_uppercase).collect())
                    .unwrap_or_default();
                let text = header_fields(&m.raw, &names);
                parts.push(format!(
                    "BODY[HEADER.FIELDS ({})] {{{}}}\r\n{}",
                    names.join(" "),
                    text.len(),
                    text
                ));
            }
        }
        out.push_str(&format!("* {} FETCH ({})\r\n", i + 1, parts.join(" ")));
    }
    out.push_str(&format!("{tag} OK Fetch completed\r\n"));
    out
}

fn uid_store(st: &mut State, sess: &mut Sess, tag: &str, toks: &[Tok]) -> String {
    let Some((f, rw)) = selected(st, sess) else {
        return format!("{tag} BAD No folder selected\r\n");
    };
    if !rw {
        return format!("{tag} NO [READ-ONLY] Folder is read-only\r\n");
    }
    let set = parse_set(toks.first().and_then(Tok::text).unwrap_or(""), max_uid(f));
    let mode = toks
        .get(1)
        .and_then(Tok::text)
        .unwrap_or("")
        .to_ascii_uppercase();
    let silent = mode.ends_with(".SILENT");
    let flags: Vec<String> = match toks.get(2) {
        Some(Tok::List(l)) => l
            .iter()
            .filter_map(|t| t.text().map(str::to_string))
            .collect(),
        Some(t) => t
            .text()
            .map(|s| vec![s.to_string()])
            .into_iter()
            .flatten()
            .collect(),
        None => Vec::new(),
    };
    f.highest_modseq += 1;
    let modseq = f.highest_modseq;
    let mut out = String::new();
    for (i, m) in f.messages.iter_mut().enumerate() {
        if !in_set(&set, m.uid) {
            continue;
        }
        if mode.starts_with('+') {
            m.flags.extend(flags.iter().cloned());
        } else if mode.starts_with('-') {
            for fl in &flags {
                m.flags.remove(fl);
            }
        } else {
            m.flags = flags.iter().cloned().collect();
        }
        m.modseq = modseq;
        if !silent {
            out.push_str(&format!(
                "* {} FETCH (UID {} FLAGS {})\r\n",
                i + 1,
                m.uid,
                flags_text(&m.flags)
            ));
        }
    }
    out.push_str(&format!("{tag} OK Store completed\r\n"));
    out
}

fn expunge(st: &mut State, sess: &mut Sess, tag: &str, only: Option<&str>) -> String {
    let qresync = sess.qresync;
    let Some((f, rw)) = selected(st, sess) else {
        return format!("{tag} BAD No folder selected\r\n");
    };
    if !rw {
        return format!("{tag} NO [READ-ONLY] Folder is read-only\r\n");
    }
    let set = only.map(|s| parse_set(s, max_uid(f)));
    let mut gone = Vec::new();
    for m in &f.messages {
        let wanted = set.as_ref().is_none_or(|s| in_set(s, m.uid));
        if wanted && m.flags.contains("\\Deleted") {
            gone.push(m.uid);
        }
    }
    let out = remove_uids(f, &gone, qresync);
    format!("{out}{tag} OK Expunge completed\r\n")
}

/// Removes messages and returns the untagged EXPUNGE or VANISHED lines.
fn remove_uids(f: &mut Folder, uids: &[u32], qresync: bool) -> String {
    if uids.is_empty() {
        return String::new();
    }
    f.highest_modseq += 1;
    let modseq = f.highest_modseq;
    let mut out = String::new();
    let mut removed = 0;
    let mut kept = Vec::new();
    for (i, m) in std::mem::take(&mut f.messages).into_iter().enumerate() {
        if uids.contains(&m.uid) {
            if !qresync {
                out.push_str(&format!("* {} EXPUNGE\r\n", i + 1 - removed));
            }
            removed += 1;
            f.expunged.push((m.uid, modseq));
        } else {
            kept.push(m);
        }
    }
    f.messages = kept;
    if qresync {
        out.push_str(&format!("* VANISHED {}\r\n", uid_list(uids)));
    }
    out
}

fn copy_or_move(st: &mut State, sess: &mut Sess, tag: &str, toks: &[Tok], is_move: bool) -> String {
    let caps = st.opts.caps;
    let qresync = sess.qresync;
    let Some(dest_name) = toks.get(1).and_then(Tok::text).map(str::to_string) else {
        return format!("{tag} BAD Missing destination\r\n");
    };
    let Some((src_name, rw)) = sess.selected.clone() else {
        return format!("{tag} BAD No folder selected\r\n");
    };
    if is_move && !rw {
        return format!("{tag} NO [READ-ONLY] Folder is read-only\r\n");
    }
    if !st.folders.iter().any(|f| f.name == dest_name) {
        return format!("{tag} NO [TRYCREATE] Mailbox doesn't exist\r\n");
    }
    if dest_name == src_name {
        return format!("{tag} NO Cannot copy to the same folder\r\n");
    }
    let src_idx = st.folders.iter().position(|f| f.name == src_name);
    let dst_idx = st.folders.iter().position(|f| f.name == dest_name);
    let (Some(si), Some(di)) = (src_idx, dst_idx) else {
        return format!("{tag} NO Folder missing\r\n");
    };
    let set = parse_set(
        toks.first().and_then(Tok::text).unwrap_or(""),
        max_uid(&st.folders[si]),
    );
    let picked: Vec<(u32, BTreeSet<String>, String, String)> = st.folders[si]
        .messages
        .iter()
        .filter(|m| in_set(&set, m.uid))
        .map(|m| {
            (
                m.uid,
                m.flags.clone(),
                m.raw.clone(),
                m.internal_date.clone(),
            )
        })
        .collect();
    if picked.is_empty() {
        return format!("{tag} OK Nothing to do\r\n");
    }
    let validity = st.folders[di].uid_validity;
    let mut src_uids = Vec::new();
    let mut dst_uids = Vec::new();
    for (uid, mut flags, raw, date) in picked {
        let d = &mut st.folders[di];
        let new_uid = d.uid_next;
        d.uid_next += 1;
        d.highest_modseq += 1;
        if is_move {
            flags.remove("\\Deleted");
        }
        d.messages.push(Message {
            uid: new_uid,
            flags,
            raw,
            internal_date: date,
            modseq: d.highest_modseq,
        });
        src_uids.push(uid);
        dst_uids.push(new_uid);
    }
    let code = format!(
        "COPYUID {validity} {} {}",
        uid_list(&src_uids),
        uid_list(&dst_uids)
    );
    if is_move {
        let mut out = String::new();
        if caps.uidplus {
            out.push_str(&format!("* OK [{code}] Moved\r\n"));
        }
        out.push_str(&remove_uids(&mut st.folders[si], &src_uids, qresync));
        out.push_str(&format!("{tag} OK Move completed\r\n"));
        out
    } else if caps.uidplus {
        format!("{tag} OK [{code}] Copy completed\r\n")
    } else {
        format!("{tag} OK Copy completed\r\n")
    }
}

fn uid_search(st: &mut State, sess: &mut Sess, tag: &str, toks: &[Tok]) -> String {
    let extra_uids = st.misbehavior == Misbehavior::SearchExtraUids
        && toks
            .first()
            .and_then(Tok::text)
            .is_some_and(|k| k.eq_ignore_ascii_case("UID"));
    let Some((f, _)) = selected(st, sess) else {
        return format!("{tag} BAD No folder selected\r\n");
    };
    let mut hits: Vec<&Message> = f.messages.iter().collect();
    let mut i = 0;
    while i < toks.len() {
        let key = toks[i].text().unwrap_or("").to_ascii_uppercase();
        match key.as_str() {
            "ALL" => i += 1,
            "DELETED" => {
                hits.retain(|m| m.flags.contains("\\Deleted"));
                i += 1;
            }
            "UNDELETED" => {
                hits.retain(|m| !m.flags.contains("\\Deleted"));
                i += 1;
            }
            "DRAFT" => {
                hits.retain(|m| m.flags.contains("\\Draft"));
                i += 1;
            }
            "UID" => {
                let set = parse_set(
                    toks.get(i + 1).and_then(Tok::text).unwrap_or(""),
                    f.messages.iter().map(|m| m.uid).max().unwrap_or(0),
                );
                hits.retain(|m| in_set(&set, m.uid));
                i += 2;
            }
            "HEADER" => {
                let name = toks
                    .get(i + 1)
                    .and_then(Tok::text)
                    .unwrap_or("")
                    .to_string();
                let needle = toks
                    .get(i + 2)
                    .and_then(Tok::text)
                    .unwrap_or("")
                    .to_ascii_lowercase();
                hits.retain(|m| {
                    header_value(&m.raw, &name)
                        .is_some_and(|v| v.to_ascii_lowercase().contains(&needle))
                });
                i += 3;
            }
            _ => return format!("{tag} BAD Unsupported search key\r\n"),
        }
    }
    let mut uids: Vec<String> = hits.iter().map(|m| m.uid.to_string()).collect();
    if extra_uids {
        // A server that answers with more than it was asked about.
        for m in &f.messages {
            if !uids.contains(&m.uid.to_string()) {
                uids.push(m.uid.to_string());
            }
        }
        uids.push("999999".to_string());
    }
    let line = if uids.is_empty() {
        "* SEARCH\r\n".to_string()
    } else {
        format!("* SEARCH {}\r\n", uids.join(" "))
    };
    format!("{line}{tag} OK Search completed\r\n")
}

// -------------------------------------------------------------------------------------------
// Message parsing for ENVELOPE and header fields
// -------------------------------------------------------------------------------------------

fn header_block(raw: &str) -> &str {
    raw.split("\r\n\r\n").next().unwrap_or(raw)
}

/// Header lines as (name, value) with continuation lines unfolded.
fn headers(raw: &str) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    for line in header_block(raw).split("\r\n") {
        if line.starts_with(' ') || line.starts_with('\t') {
            if let Some(last) = out.last_mut() {
                last.1.push(' ');
                last.1.push_str(line.trim());
            }
        } else if let Some((n, v)) = line.split_once(':') {
            out.push((n.trim().to_string(), v.trim().to_string()));
        }
    }
    out
}

fn header_value(raw: &str, name: &str) -> Option<String> {
    headers(raw)
        .into_iter()
        .find(|(n, _)| n.eq_ignore_ascii_case(name))
        .map(|(_, v)| v)
}

/// The raw header lines (folded as stored) for the named fields, ending with the blank line.
fn header_fields(raw: &str, names: &[String]) -> String {
    let mut out = String::new();
    let mut keep = false;
    for line in header_block(raw).split("\r\n") {
        if line.starts_with(' ') || line.starts_with('\t') {
            if keep {
                out.push_str(line);
                out.push_str("\r\n");
            }
        } else {
            let name = line
                .split(':')
                .next()
                .unwrap_or("")
                .trim()
                .to_ascii_uppercase();
            keep = names.contains(&name);
            if keep {
                out.push_str(line);
                out.push_str("\r\n");
            }
        }
    }
    out.push_str("\r\n");
    out
}

fn nstring(v: Option<String>) -> String {
    match v {
        Some(s) => quote(&s),
        None => "NIL".to_string(),
    }
}

fn address_list(v: Option<String>) -> String {
    let Some(v) = v else {
        return "NIL".to_string();
    };
    let items: Vec<String> = v
        .split(',')
        .map(|a| {
            let a = a.trim();
            let (name, addr) = match (a.rfind('<'), a.rfind('>')) {
                (Some(l), Some(r)) if l < r => (
                    Some(a[..l].trim().trim_matches('"').to_string()),
                    &a[l + 1..r],
                ),
                _ => (None, a),
            };
            let (mailbox, host) = addr.split_once('@').unwrap_or((addr, ""));
            format!(
                "({} NIL {} {})",
                nstring(name.filter(|n| !n.is_empty())),
                quote(mailbox),
                quote(host)
            )
        })
        .collect();
    format!("({})", items.join(""))
}

fn envelope(raw: &str) -> String {
    let h = |n: &str| header_value(raw, n);
    let from = address_list(h("From"));
    format!(
        "({} {} {} {} {} {} {} NIL {} {})",
        nstring(h("Date")),
        nstring(h("Subject")),
        from,
        from,
        from,
        address_list(h("To")),
        address_list(h("Cc")),
        nstring(h("In-Reply-To")),
        nstring(h("Message-ID")),
    )
}
