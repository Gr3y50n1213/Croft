// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The SMTP wire: a byte stream (plain or TLS), reply parsing with hard limits, the `EHLO`
//! answer, and the encoding of a message for `DATA`.
//!
//! Nothing here keeps or prints what the server says. A reply is reduced to its three-digit code
//! and, for `EHLO` only, the capabilities Croft looks at.

use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, ReadBuf};
use tokio::net::TcpStream;
use tokio::time::timeout;
use tokio_rustls::client::TlsStream;

/// Longest reply line accepted (RFC 5321 says 512; some servers send more in `EHLO`).
const MAX_LINE: usize = 4096;
/// Most lines in one reply.
const MAX_LINES: usize = 128;

/// A connection that is plaintext TCP (before `STARTTLS`, or a loopback test server) or TLS.
#[derive(Debug)]
pub(crate) enum Stream {
    /// Unencrypted.
    Plain(TcpStream),
    /// TLS 1.2 or newer.
    Tls(Box<TlsStream<TcpStream>>),
}

impl AsyncRead for Stream {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        match self.get_mut() {
            Stream::Plain(s) => Pin::new(s).poll_read(cx, buf),
            Stream::Tls(s) => Pin::new(s.as_mut()).poll_read(cx, buf),
        }
    }
}

impl AsyncWrite for Stream {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        match self.get_mut() {
            Stream::Plain(s) => Pin::new(s).poll_write(cx, buf),
            Stream::Tls(s) => Pin::new(s.as_mut()).poll_write(cx, buf),
        }
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.get_mut() {
            Stream::Plain(s) => Pin::new(s).poll_flush(cx),
            Stream::Tls(s) => Pin::new(s.as_mut()).poll_flush(cx),
        }
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.get_mut() {
            Stream::Plain(s) => Pin::new(s).poll_shutdown(cx),
            Stream::Tls(s) => Pin::new(s.as_mut()).poll_shutdown(cx),
        }
    }
}

/// Why a read or write on the wire did not complete.
#[derive(Debug)]
pub(crate) enum WireError {
    /// An I/O error (reset, broken pipe, a TLS alert). May wrap a rustls error.
    Io(io::Error),
    /// Nothing arrived (or could be written) within the step timeout.
    Timeout,
    /// The server closed the connection.
    Closed,
    /// The server sent something that is not SMTP, or too much of it.
    Protocol,
}

/// One complete reply: its code and its text lines (without the code).
#[derive(Debug)]
pub(crate) struct Reply {
    /// The three-digit code.
    pub code: u16,
    /// The text of each line. Only `EHLO` looks at this.
    pub lines: Vec<String>,
}

impl Reply {
    /// First digit 2.
    pub fn is_ok(&self) -> bool {
        (200..300).contains(&self.code)
    }

    /// First digit 4.
    pub fn is_temporary(&self) -> bool {
        (400..500).contains(&self.code)
    }
}

/// What `EHLO` told us, reduced to what Croft uses.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct Capabilities {
    /// `STARTTLS`.
    pub starttls: bool,
    /// `AUTH PLAIN`.
    pub plain: bool,
    /// `AUTH LOGIN`.
    pub login: bool,
    /// `AUTH XOAUTH2`.
    pub xoauth2: bool,
    /// `8BITMIME`.
    pub eight_bit: bool,
    /// `SMTPUTF8`.
    pub smtputf8: bool,
    /// `SIZE n`: the largest message in bytes (`Some(0)` or `None` means no stated limit).
    pub size: Option<u64>,
}

impl Capabilities {
    /// Reads the lines of a 250 reply to `EHLO` (the first line is the server's greeting).
    pub fn parse(lines: &[String]) -> Self {
        let mut caps = Self::default();
        for line in lines.iter().skip(1) {
            let mut words = line.split_ascii_whitespace();
            let Some(keyword) = words.next() else {
                continue;
            };
            let keyword = keyword.to_ascii_uppercase();
            match keyword.as_str() {
                "STARTTLS" => caps.starttls = true,
                "8BITMIME" => caps.eight_bit = true,
                "SMTPUTF8" => caps.smtputf8 = true,
                "SIZE" => caps.size = words.next().and_then(|w| w.parse().ok()),
                // `AUTH PLAIN LOGIN`, and the old `AUTH=PLAIN LOGIN` spelling some servers add.
                k if k == "AUTH" || k.starts_with("AUTH=") => {
                    let first = k.strip_prefix("AUTH=");
                    for mech in first.into_iter().chain(words.by_ref()) {
                        match mech.to_ascii_uppercase().as_str() {
                            "PLAIN" => caps.plain = true,
                            "LOGIN" => caps.login = true,
                            "XOAUTH2" => caps.xoauth2 = true,
                            _ => {}
                        }
                    }
                }
                _ => {}
            }
        }
        caps
    }
}

/// A connection with a read buffer and a step timeout.
pub(crate) struct Conn {
    io: Stream,
    buf: Vec<u8>,
    step: Duration,
}

impl Conn {
    /// Wraps a stream.
    pub fn new(io: Stream, step: Duration) -> Self {
        Self {
            io,
            buf: Vec::new(),
            step,
        }
    }

    /// True when bytes the server sent are waiting unread (after a `220` for `STARTTLS` this
    /// means someone injected plaintext that the TLS layer must not inherit).
    pub fn has_unread(&self) -> bool {
        !self.buf.is_empty()
    }

    /// Gives the plain TCP stream back for the TLS upgrade, or `None` if it was not plain.
    pub fn into_plain(self) -> Option<TcpStream> {
        match self.io {
            Stream::Plain(tcp) => Some(tcp),
            Stream::Tls(_) => None,
        }
    }

    /// Writes bytes, each write bounded by the step timeout, and flushes.
    pub async fn write(&mut self, bytes: &[u8]) -> Result<(), WireError> {
        let step = self.step;
        match timeout(step, async {
            self.io.write_all(bytes).await?;
            self.io.flush().await
        })
        .await
        {
            Err(_) => Err(WireError::Timeout),
            Ok(Err(e)) => Err(WireError::Io(e)),
            Ok(Ok(())) => Ok(()),
        }
    }

    /// Writes a body in chunks, each bounded by the step timeout, so a large message on a slow
    /// but moving link is not cut off by a timeout meant for one command.
    pub async fn write_chunked(&mut self, bytes: &[u8]) -> Result<(), WireError> {
        for chunk in bytes.chunks(16 * 1024) {
            self.write(chunk).await?;
        }
        Ok(())
    }

    /// Reads one reply (all its lines) within the step timeout.
    pub async fn read_reply(&mut self) -> Result<Reply, WireError> {
        let step = self.step;
        self.read_reply_within(Some(step)).await
    }

    /// Reads one reply. With `None` there is no per-step limit (the caller's overall deadline
    /// applies): used for the answer after the message body, which servers may delay.
    pub async fn read_reply_within(&mut self, step: Option<Duration>) -> Result<Reply, WireError> {
        match step {
            Some(t) => match timeout(t, self.read_reply_inner()).await {
                Err(_) => Err(WireError::Timeout),
                Ok(r) => r,
            },
            None => self.read_reply_inner().await,
        }
    }

    async fn read_reply_inner(&mut self) -> Result<Reply, WireError> {
        let mut code = 0u16;
        let mut lines = Vec::new();
        loop {
            let line = self.read_line().await?;
            let (this_code, more, text) = parse_line(&line).ok_or(WireError::Protocol)?;
            if lines.is_empty() {
                code = this_code;
            } else if this_code != code {
                return Err(WireError::Protocol);
            }
            if lines.len() >= MAX_LINES {
                return Err(WireError::Protocol);
            }
            lines.push(text);
            if !more {
                return Ok(Reply { code, lines });
            }
        }
    }

    /// One line, without its line ending. Accepts a bare `LF` as well as `CRLF`.
    async fn read_line(&mut self) -> Result<Vec<u8>, WireError> {
        loop {
            if let Some(pos) = self.buf.iter().position(|&b| b == b'\n') {
                let mut line: Vec<u8> = self.buf.drain(..=pos).collect();
                line.pop();
                if line.last() == Some(&b'\r') {
                    line.pop();
                }
                return Ok(line);
            }
            if self.buf.len() > MAX_LINE {
                return Err(WireError::Protocol);
            }
            let mut chunk = [0u8; 2048];
            let n = self.io.read(&mut chunk).await.map_err(WireError::Io)?;
            if n == 0 {
                return Err(WireError::Closed);
            }
            self.buf.extend_from_slice(&chunk[..n]);
        }
    }
}

/// Splits `250-text` into (250, continues, "text"). `None` when the line is not a reply line.
fn parse_line(line: &[u8]) -> Option<(u16, bool, String)> {
    if line.len() < 3 || line.len() > MAX_LINE || !line[..3].iter().all(u8::is_ascii_digit) {
        return None;
    }
    let code = u16::from(line[0] - b'0') * 100
        + u16::from(line[1] - b'0') * 10
        + u16::from(line[2] - b'0');
    if !(200..600).contains(&code) {
        return None;
    }
    let (more, rest) = match line.get(3) {
        None => (false, &line[3..]),
        Some(b' ') => (false, &line[4..]),
        Some(b'-') => (true, &line[4..]),
        Some(_) => return None,
    };
    Some((code, more, String::from_utf8_lossy(rest).into_owned()))
}

/// True when the byte string has any byte of 0x80 or above.
pub(crate) fn is_8bit(bytes: &[u8]) -> bool {
    bytes.iter().any(|b| *b >= 0x80)
}

/// Where a line of a message ends: the one definition, kept in [`crate::lines`] (it has no I/O, so
/// code that only reads a message, such as the sample mailbox, can use it without this module).
pub use crate::lines::data_lines;

/// Prepares a message for `DATA`: every line ends in `CRLF` (a bare `LF` or `CR` is turned into
/// one, so a server cannot be confused about where the message ends; see [`data_lines`]), a line
/// that starts with a dot gets a second dot (RFC 5321 section 4.5.2), and the text ends with a
/// line end. The terminating `.` line is **not** included: the caller sends it separately so it
/// knows exactly when the point of no return was crossed.
pub(crate) fn encode_for_data(mime: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(mime.len() + mime.len() / 40 + 8);
    for line in data_lines(mime) {
        if line.first() == Some(&b'.') {
            out.push(b'.');
        }
        out.extend_from_slice(line);
        out.extend_from_slice(b"\r\n");
    }
    out
}

/// Checks an address for use inside `<...>` in `MAIL FROM` or `RCPT TO`. The check exists
/// because the address goes into a command line: a line break, a NUL, a space or an angle bracket
/// in it would let it add commands of its own. `None` means refused; `Some(true)` means the
/// address is valid but not ASCII (it needs the server's `SMTPUTF8`).
pub(crate) fn check_address(addr: &str) -> Option<bool> {
    if addr.is_empty() || addr.len() > 254 {
        return None;
    }
    if addr
        .chars()
        .any(|c| c.is_control() || c == '<' || c == '>' || c == ' ')
    {
        return None;
    }
    match addr.rsplit_once('@') {
        Some((local, domain)) if !local.is_empty() && !domain.is_empty() => Some(!addr.is_ascii()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn enc(s: &str) -> String {
        String::from_utf8(encode_for_data(s.as_bytes())).unwrap()
    }

    #[test]
    fn dot_stuffing_and_line_endings() {
        assert_eq!(enc("a\r\nb\r\n"), "a\r\nb\r\n");
        assert_eq!(enc("a\nb"), "a\r\nb\r\n");
        assert_eq!(enc("a\rb"), "a\r\nb\r\n");
        assert_eq!(enc(".hidden\r\n.\r\n..x"), "..hidden\r\n..\r\n...x\r\n");
        assert_eq!(enc("x.y\r\n"), "x.y\r\n");
        assert_eq!(enc(""), "");
        // A message that contains the end-of-data marker cannot end the DATA early.
        assert_eq!(
            enc("a\r\n.\r\nMAIL FROM:<x@y>\r\n"),
            "a\r\n..\r\nMAIL FROM:<x@y>\r\n"
        );
        // A lone LF followed by a dot is also a line start.
        assert_eq!(enc("a\n.\nb"), "a\r\n..\r\nb\r\n");
    }

    #[test]
    fn lines_end_at_crlf_lf_or_cr_and_nowhere_else() {
        let split = |s: &str| -> Vec<String> {
            data_lines(s.as_bytes())
                .map(|l| String::from_utf8_lossy(l).into_owned())
                .collect()
        };
        assert_eq!(split("a\r\nb\nc\rd"), ["a", "b", "c", "d"]);
        assert_eq!(split("a\r\n"), ["a"]);
        assert_eq!(split("a\r\n\r\nb"), ["a", "", "b"]);
        // A CR before a CRLF is a line end of its own.
        assert_eq!(split("a\r\r\nb"), ["a", "", "b"]);
        assert_eq!(split("a\n\rb"), ["a", "", "b"]);
        assert_eq!(split("\r"), [""]);
        assert!(split("").is_empty());
        // What the server receives is exactly these lines, each ended by CRLF.
        for text in ["a\rb\r\n\rc\n", "x\r\r\ny", ".a\n.b\r.c"] {
            let joined: String = data_lines(text.as_bytes())
                .map(|l| {
                    let dot = if l.first() == Some(&b'.') { "." } else { "" };
                    format!("{dot}{}\r\n", String::from_utf8_lossy(l))
                })
                .collect();
            assert_eq!(enc(text), joined);
        }
    }

    #[test]
    fn reply_lines_parse() {
        assert_eq!(parse_line(b"250 OK"), Some((250, false, "OK".into())));
        assert_eq!(
            parse_line(b"250-SIZE 100"),
            Some((250, true, "SIZE 100".into()))
        );
        assert_eq!(parse_line(b"220"), Some((220, false, String::new())));
        assert_eq!(parse_line(b"2500 x"), None);
        assert_eq!(parse_line(b"abc"), None);
        assert_eq!(parse_line(b"150 x"), None);
        assert_eq!(parse_line(b"250x"), None);
    }

    #[test]
    fn capabilities_are_read_from_ehlo() {
        let lines: Vec<String> = [
            "mail.example.org hello",
            "SIZE 1000",
            "8BITMIME",
            "AUTH PLAIN LOGIN xoauth2",
            "AUTH=PLAIN",
            "STARTTLS",
            "SMTPUTF8",
            "PIPELINING",
        ]
        .into_iter()
        .map(String::from)
        .collect();
        let c = Capabilities::parse(&lines);
        assert!(c.starttls && c.plain && c.login && c.xoauth2 && c.eight_bit && c.smtputf8);
        assert_eq!(c.size, Some(1000));
        // The first line is the greeting, never a capability.
        let c = Capabilities::parse(&["STARTTLS".to_string()]);
        assert!(!c.starttls);
    }

    #[test]
    fn addresses_cannot_carry_commands() {
        assert_eq!(check_address("a@example.org"), Some(false));
        assert_eq!(check_address("jörg@example.org"), Some(true));
        for bad in [
            "",
            "no-at",
            "@example.org",
            "a@",
            "a@b.c\r\nRCPT TO:<x@y>",
            "a@b.c\nDATA",
            "a@b.c>\r",
            "<a@b.c>",
            "a b@c.d",
            "a\0@b.c",
        ] {
            assert_eq!(check_address(bad), None, "{bad:?}");
        }
        assert_eq!(
            check_address(&format!("{}@example.org", "a".repeat(260))),
            None
        );
    }
}
