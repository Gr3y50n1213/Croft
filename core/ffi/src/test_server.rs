// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! `TestImapServer`: the in-process IMAP server of `croft-test-support` as a UniFFI object, so
//! that the Kotlin JVM tests can run the app's code against a real, local server (T16).
//!
//! **Test only.** This file is compiled only with the `test-server` feature. That feature is on
//! for the host library the JVM unit tests load and never for the Android library:
//! `android/app/build.gradle.kts` keeps it out of `rustFeatures` and
//! `android/tools/check-release-outputs.sh` fails when the release `.so` carries these symbols.
//! The server listens on `127.0.0.1` only, with TLS and a fresh self-signed certificate (the app
//! trusts it through the certificate fingerprint, as it does for any private server), and takes
//! one fixed made-up login. Nothing here is product code.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use croft_protocols::CertFingerprint;
use croft_test_support::{Misbehavior, ServerOptions, TestServer, TlsMode};
use tokio::runtime::{Builder, Runtime};

/// The one accepted login name.
const USERNAME: &str = "alice";

/// A local IMAP server for tests, with TLS, the folders INBOX, Sent, Drafts, Trash, Junk and
/// Archive, and one made-up login. Methods that name a folder use its IMAP name (`INBOX`,
/// `Archive`). Dropping the object (or [`TestImapServer::stop`]) stops the server.
#[derive(uniffi::Object)]
pub struct TestImapServer {
    // The server's tasks live on this runtime; it must outlive the server.
    rt: Runtime,
    server: Mutex<Option<TestServer>>,
    fingerprint: String,
    host: String,
    port: u16,
}

impl TestImapServer {
    fn server(&self) -> MutexGuard<'_, Option<TestServer>> {
        self.server.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Runs `f` on the server if it is still running.
    fn with<T>(&self, f: impl FnOnce(&TestServer) -> T) -> Option<T> {
        self.server().as_ref().map(f)
    }
}

/// A raw message with a unique id for `subject`, from a made-up sender.
fn raw(n: u64, subject: &str) -> String {
    croft_test_support::sample_message(
        &format!("test-{n}@test.invalid"),
        subject,
        "Ann Example <ann@example.org>",
    )
}

/// The subject line of a raw message, for tests that tell messages apart after a move.
fn subject_of(raw: &str) -> String {
    raw.lines()
        .find_map(|l| l.strip_prefix("Subject: "))
        .unwrap_or_default()
        .to_owned()
}

#[uniffi::export]
impl TestImapServer {
    /// Starts a server that accepts the login `alice` with `password`, with `message_count`
    /// messages ("Message 1" to "Message N", UIDs 1 to N) already in INBOX.
    #[uniffi::constructor]
    pub fn start(message_count: u32, password: String) -> Arc<Self> {
        // A runtime of its own: the server's connection tasks run here, apart from the core's.
        // Failing to build one is a broken test machine, and a test helper may panic.
        #[allow(clippy::expect_used)]
        let rt = Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
            .expect("a runtime for the test server");
        let server = rt.block_on(TestServer::start(ServerOptions {
            username: USERNAME.into(),
            password,
            tls: TlsMode::Implicit,
            ..ServerOptions::default()
        }));
        for n in 1..=u64::from(message_count) {
            server.add_message("INBOX", &raw(n, &format!("Message {n}")));
        }
        let fingerprint = server
            .certificate_der()
            .map(|der| CertFingerprint::of_der(&der).to_string())
            .unwrap_or_default();
        Arc::new(Self {
            host: server.host().to_owned(),
            port: server.port(),
            fingerprint,
            server: Mutex::new(Some(server)),
            rt,
        })
    }

    /// The address the server listens on (`127.0.0.1`).
    pub fn host(&self) -> String {
        self.host.clone()
    }

    /// The port the server listens on.
    pub fn port(&self) -> u16 {
        self.port
    }

    /// The accepted login name.
    pub fn username(&self) -> String {
        USERNAME.to_owned()
    }

    /// The SHA-256 fingerprint of the server's certificate, as `verify` and `add_account` take it
    /// as `pinned_fingerprint`.
    pub fn certificate_fingerprint(&self) -> String {
        self.fingerprint.clone()
    }

    /// Stops the server: its connections are cut and nothing listens any more. The port is not
    /// reused. Calling it again does nothing.
    pub fn stop(&self) {
        let server = self.server().take();
        if let Some(server) = server {
            // Dropping stops the accept loop and kills the open connections.
            self.rt.block_on(async move { drop(server) });
        }
    }

    /// Adds a new message to `folder` as mail arriving from another sender. Returns its UID
    /// (0 when the server is stopped or the folder does not exist).
    pub fn inject_message(&self, folder: String, subject: String) -> u32 {
        self.with(|s| {
            let n = u64::try_from(s.messages(&folder).len()).unwrap_or(0) + 1_000;
            // `add_message` panics on a missing folder: check first.
            if s.messages(&folder).is_empty() && !known_folder(&folder) {
                return 0;
            }
            s.add_message(&folder, &raw(n, &subject))
        })
        .unwrap_or(0)
    }

    /// Changes flags the way another client would (`\Seen`, `\Flagged`, ...); the server's
    /// mod-sequence moves, so the next sync picks it up.
    pub fn change_flags(&self, folder: String, uid: u32, add: Vec<String>, remove: Vec<String>) {
        let add: Vec<&str> = add.iter().map(String::as_str).collect();
        let remove: Vec<&str> = remove.iter().map(String::as_str).collect();
        self.with(|s| s.modify_flags(&folder, uid, &add, &remove));
    }

    /// The flags the server holds for a message, sorted. Empty when it does not exist.
    pub fn flags(&self, folder: String, uid: u32) -> Vec<String> {
        self.with(|s| {
            s.messages(&folder)
                .into_iter()
                .find(|m| m.uid == uid)
                .map(|m| m.flags.into_iter().collect())
                .unwrap_or_default()
        })
        .unwrap_or_default()
    }

    /// The UIDs in `folder`, ascending.
    pub fn uids(&self, folder: String) -> Vec<u32> {
        self.with(|s| s.messages(&folder).into_iter().map(|m| m.uid).collect())
            .unwrap_or_default()
    }

    /// The subject of each message in `folder`, in UID order (the way to tell which message
    /// landed where after a move).
    pub fn subjects(&self, folder: String) -> Vec<String> {
        self.with(|s| {
            s.messages(&folder)
                .into_iter()
                .map(|m| subject_of(&m.raw))
                .collect()
        })
        .unwrap_or_default()
    }

    /// The flags of the message with this subject in `folder` (first match), or empty.
    pub fn flags_of_subject(&self, folder: String, subject: String) -> Vec<String> {
        self.with(|s| {
            s.messages(&folder)
                .into_iter()
                .find(|m| subject_of(&m.raw) == subject)
                .map(|m| m.flags.into_iter().collect())
                .unwrap_or_default()
        })
        .unwrap_or_default()
    }

    /// Every command the server received since the start or the last `clear_commands`, without
    /// tags; the login is logged as `LOGIN ***`.
    pub fn commands(&self) -> Vec<String> {
        self.with(TestServer::commands).unwrap_or_default()
    }

    /// Forgets the command log.
    pub fn clear_commands(&self) {
        self.with(TestServer::clear_commands);
    }

    /// Makes a folder refuse changes (`NO [READ-ONLY]`): a permanent refusal for a move out of it.
    pub fn set_read_only(&self, folder: String, on: bool) {
        self.with(|s| s.set_read_only(&folder, on));
    }

    /// While on, every sign-in is answered `NO [UNAVAILABLE]`: a temporary failure, not a bad
    /// password.
    pub fn set_login_unavailable(&self, on: bool) {
        self.with(|s| s.set_login_unavailable(on));
    }

    /// While on, every sign-in is refused as a wrong password (`NO [AUTHENTICATIONFAILED]`),
    /// whatever the password: the server as it is after the person changed the password.
    pub fn set_reject_logins(&self, on: bool) {
        self.with(|s| {
            s.set_misbehavior(if on {
                Misbehavior::AuthFailedMentioningUnavailable
            } else {
                Misbehavior::None
            });
        });
    }

    /// Turns the `UIDPLUS` extension on or off (read at each new connection, so cut the connections after). Without it the
    /// server gives no `COPYUID`, and the new UID of a moved message is found later, by its Message-ID.
    pub fn set_uidplus(&self, on: bool) {
        self.with(|s| s.set_caps(|c| c.uidplus = on));
    }

    /// Turns the `MOVE` extension on or off (read at each new connection): without it a move is a copy, a delete flag and an
    /// expunge.
    pub fn set_move_extension(&self, on: bool) {
        self.with(|s| s.set_caps(|c| c.move_cmd = on));
    }

    /// Cuts every open connection at once (the server keeps accepting new ones).
    pub fn drop_connections(&self) {
        self.with(TestServer::drop_connections);
    }

    /// How many TCP connections the server has accepted so far.
    pub fn connection_count(&self) -> u32 {
        self.with(|s| u32::try_from(s.connection_count()).unwrap_or(u32::MAX))
            .unwrap_or(0)
    }
}

/// The folders every test server starts with.
fn known_folder(name: &str) -> bool {
    ["INBOX", "Sent", "Drafts", "Trash", "Junk", "Archive"].contains(&name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_server_starts_with_messages_and_stops() {
        let server = TestImapServer::start(3, "made-up-password-0001".into());
        assert_eq!(server.host(), "127.0.0.1");
        assert!(server.port() > 0);
        assert_eq!(server.username(), "alice");
        // 32 bytes as colon-separated hex, the form `CertFingerprint` prints and parses.
        assert_eq!(server.certificate_fingerprint().len(), 95);
        assert_eq!(server.uids("INBOX".into()), vec![1, 2, 3]);
        assert_eq!(
            server.subjects("INBOX".into()),
            vec!["Message 1", "Message 2", "Message 3"]
        );
        assert_eq!(server.uids("Archive".into()), Vec::<u32>::new());

        // Inject mail and change a flag as another client would.
        let uid = server.inject_message("INBOX".into(), "Fresh".into());
        assert_eq!(uid, 4);
        assert_eq!(server.inject_message("NoSuchFolder".into(), "x".into()), 0);
        server.change_flags("INBOX".into(), 1, vec!["\\Seen".into()], vec![]);
        assert_eq!(server.flags("INBOX".into(), 1), vec!["\\Seen"]);
        assert_eq!(
            server.flags_of_subject("INBOX".into(), "Message 1".into()),
            vec!["\\Seen"]
        );
        server.change_flags("INBOX".into(), 1, vec![], vec!["\\Seen".into()]);
        assert!(server.flags("INBOX".into(), 1).is_empty());
        assert!(server.commands().is_empty());

        server.set_uidplus(false);
        server.set_move_extension(false);
        server.set_uidplus(true);
        server.set_move_extension(true);

        server.stop();
        assert!(server.uids("INBOX".into()).is_empty());
        assert_eq!(server.inject_message("INBOX".into(), "late".into()), 0);
        server.stop();
    }
}

#[uniffi::export]
impl TestImapServer {
    /// Changes the accepted password (and XOAUTH2 access token), as when a token expires (T21).
    pub fn set_password(&self, password: String) {
        self.with(|s| s.set_password(&password));
    }
}

/// A fake OAuth token endpoint on `127.0.0.1` for the JVM tests of the sign-in (T21). Starting
/// it makes the core send its token requests there instead of to Google or Microsoft (host builds
/// only: this feature never reaches Android). Every token is made up.
#[derive(uniffi::Object)]
pub struct TestTokenEndpoint {
    rt: Runtime,
    fake: croft_test_support::oauth::FakeTokenEndpoint,
}

fn query_param(url: &str, name: &str) -> Option<String> {
    let query = url.split_once('?')?.1;
    let raw = query
        .split('&')
        .find_map(|p| p.strip_prefix(&format!("{name}=")))?;
    Some(raw.replace("%3A", ":").replace("%2F", "/"))
}

#[uniffi::export]
impl TestTokenEndpoint {
    /// Starts it and points the core's token requests at it.
    #[uniffi::constructor]
    pub fn start() -> Arc<Self> {
        #[allow(clippy::expect_used)]
        let rt = Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
            .expect("a runtime for the token endpoint");
        let fake = rt.block_on(croft_test_support::oauth::FakeTokenEndpoint::start());
        crate::oauth::set_token_endpoint_for_tests(Some(fake.url()));
        Arc::new(Self { rt, fake })
    }

    /// Plays the browser after the person agreed on the provider's page: registers `code` for the
    /// PKCE challenge in `auth_url` (which gives `refresh_token`) and opens the redirect with the
    /// code and the `state`. Returns false when the URL has no loopback redirect.
    pub fn complete_sign_in(&self, auth_url: String, code: String, refresh_token: String) -> bool {
        let (Some(redirect), Some(state), Some(challenge)) = (
            query_param(&auth_url, "redirect_uri"),
            query_param(&auth_url, "state"),
            query_param(&auth_url, "code_challenge"),
        ) else {
            return false;
        };
        let Some(port) = redirect
            .rsplit(':')
            .next()
            .and_then(|p| p.parse::<u16>().ok())
        else {
            return false;
        };
        self.fake.add_code(&code, &challenge, &refresh_token);
        let target = format!("/?state={state}&code={code}");
        std::thread::spawn(move || {
            use std::io::{Read, Write};
            if let Ok(mut s) = std::net::TcpStream::connect(("127.0.0.1", port)) {
                let _ = write!(s, "GET {target} HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n");
                let mut page = String::new();
                let _ = s.read_to_string(&mut page);
            }
        });
        true
    }

    /// Each refresh gives a new refresh token.
    pub fn set_rotation(&self, on: bool) {
        self.fake.set_rotation(on);
    }

    /// The form bodies received.
    pub fn requests(&self) -> Vec<String> {
        self.fake.requests()
    }

    /// Stops sending the core's token requests here.
    pub fn stop(&self) {
        crate::oauth::set_token_endpoint_for_tests(None);
        let _ = &self.rt;
    }
}
