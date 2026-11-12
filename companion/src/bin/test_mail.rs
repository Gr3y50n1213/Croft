// This Source Code Form is subject to the terms of the GNU Affero General Public License,
// version 3 or later. See LICENSE-AGPL-3.0 at the repository root.

//! TEST ONLY (T31): the core's in-process IMAP test server, for the web app's end-to-end tests.
//! Starts it with `N` messages (`croft-test-mail [N]`, 30 by default) and a made-up password, prints
//! `{"host", "port", "username", "password", "fingerprint"}` as one JSON line, and serves until its
//! standard input closes (the test ends). Built only with `--features test-mail`.

use std::io::Read;

fn main() {
    let count = std::env::args()
        .nth(1)
        .and_then(|n| n.parse::<u32>().ok())
        .unwrap_or(30);
    let password = "made-up-test-mail-password";
    let server = croft_ffi::TestImapServer::start(count, password.into());
    // One HTML message, newest, with everything an email must not be able to do: a script, an
    // event handler, a remote image, a form and a link (the reading pane must still show it).
    server.inject_raw(
        "INBOX".into(),
        [
            "From: Hollis Printing <proofs@example.org>",
            "To: alice@example.test",
            "Subject: Your proof is ready",
            "Date: Tue, 03 Mar 2026 10:00:00 +0000",
            "Message-ID: <html-1@test.invalid>",
            "MIME-Version: 1.0",
            "Content-Type: text/html; charset=utf-8",
            "",
            "<p>Hello, <b>your proof</b> is ready.</p>",
            "<script>parent.document.body.textContent = 'escaped'</script>",
            "<img src=x onerror=\"parent.document.title='escaped'\">",
            "<img src=\"https://tracker.example/pixel.png\" alt=\"\">",
            "<form action=\"https://evil.example/\"><input name=q></form>",
            "<p><a href=\"https://example.org/proof\">Open the proof</a></p>",
        ]
        .join("\r\n"),
    );
    println!(
        "{}",
        serde_json::json!({
            "host": server.host(),
            "port": server.port(),
            "username": server.username(),
            "password": password,
            "fingerprint": server.certificate_fingerprint(),
        })
    );
    let mut sink = Vec::new();
    let _ = std::io::stdin().read_to_end(&mut sink);
    server.stop();
}
