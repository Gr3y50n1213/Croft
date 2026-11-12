// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! T22 against a real Stalwart in a container (`--features integration`; see
//! `test-servers/docker-compose.yml` and `test-servers/stalwart/setup.sh`): two JMAP accounts are
//! checked and added through the FFI, one sends to the other with EmailSubmission, and the
//! message shows in the other's Inbox. The passwords are the made-up ones of the test users.

use super::*;
use croft_protocols::MailSubmitter;

const ALICE: &str = "alice@example.org";
const ALICE_PW: &str = "croft-alice-purple-orbit-7421";
const BOB: &str = "bob@example.org";
const BOB_PW: &str = "croft-bob-yellow-lantern-5530";

fn session_url() -> String {
    let port = std::env::var("CROFT_TEST_STALWART_HTTP_PORT")
        .ok()
        .and_then(|v| v.parse::<u16>().ok())
        .unwrap_or(8080);
    format!("http://127.0.0.1:{port}/jmap/session")
}

fn discovered(email: &str) -> FfiDiscovered {
    FfiDiscovered {
        email: email.into(),
        imap: None,
        smtp: None,
        jmap: Some(session_url()),
        source: DiscoverySource::Autoconfig,
        provider: None,
        auth_hint: AuthKind::Password,
    }
}

fn add(core: &Core, provider: &MemProvider, email: &str, password: &str) -> i64 {
    let cred = || FfiCredential::password(password.as_bytes().to_vec());
    let pin = block(core.verify(discovered(email), cred(), None)).unwrap();
    let id = block(core.add_account(discovered(email), email.into(), pin, None, cred()))
        .unwrap()
        .id;
    provider.store(id, cred());
    id
}

#[test]
fn two_jmap_accounts_on_stalwart_send_and_receive_through_the_ffi() {
    let db = TempDb::new();
    let provider = MemProvider::new();
    let core = db.open(&provider);
    let alice = add(&core, &provider, ALICE, ALICE_PW);
    let bob = add(&core, &provider, BOB, BOB_PW);
    assert_eq!(
        block(core.verify(
            discovered(ALICE),
            FfiCredential::password(b"not-the-password".to_vec()),
            None
        ))
        .err(),
        Some(VerifyFailure::WrongPassword)
    );

    let subject = format!(
        "T22 over JMAP {}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    let mime = format!(
        "From: Alice <{ALICE}>\r\nTo: Bob <{BOB}>\r\nSubject: {subject}\r\n\
         Message-ID: <t22-{alice}@example.org>\r\nDate: Thu, 12 Nov 2026 10:00:00 +0000\r\n\r\nHello Bob.\r\n"
    );
    let config = croft_protocols::JmapConfig::new(session_url(), ALICE);
    let submitter = crate::jmap_account::JmapAccountSubmitter::new(alice, config, provider.clone());
    block(submitter.send(ALICE, &[BOB.to_string()], mime.as_bytes())).unwrap();

    let mut found = false;
    for _ in 0..120 {
        let _ = core.sync_now(bob);
        let inbox = core
            .list_folders(bob)
            .unwrap()
            .into_iter()
            .find(|f| f.special_use == "inbox");
        if let Some(inbox) = inbox {
            let rows = core
                .list_messages(
                    MessageScope::Folder {
                        account: bob,
                        folder_id: inbox.id,
                    },
                    None,
                    200,
                )
                .unwrap()
                .rows;
            if rows.iter().any(|r| r.subject == subject) {
                found = true;
                break;
            }
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    assert!(found, "the message reached Bob's Inbox");
}
