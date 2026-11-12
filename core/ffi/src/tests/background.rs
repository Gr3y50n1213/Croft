// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The background check through the surface, against the in-process IMAP server.

use super::*;

#[test]
fn the_first_check_notifies_nothing_then_new_mail_once_until_marked() {
    let rig = Rig::new(|s| {
        s.add_message("INBOX", &raw("a@example.org", "Already here"));
    });
    rig.wait_for_inbox(1);
    let first = block(rig.core.sync_once(rig.account, 8_000)).unwrap();
    assert!(first.new_mail.is_empty() && !first.timed_out && !first.transient_failure);

    let uid = rig
        .server
        .add_message("INBOX", &raw("b@example.org", "Lunch tomorrow"));
    let second = block(rig.core.sync_once(rig.account, 8_000)).unwrap();
    assert_eq!(second.new_mail.len(), 1);
    assert_eq!(second.new_mail[0].subject, "Lunch tomorrow");
    assert_eq!(second.new_mail[0].uid, uid);

    // The buttons and the tap check the message is still this one (a row id can be reused).
    let id = second.new_mail[0].message_id;
    assert!(rig.core.notified_message_is(rig.account, id, uid).unwrap());
    assert!(!rig
        .core
        .notified_message_is(rig.account, id, uid + 1)
        .unwrap());
    assert!(!rig
        .core
        .notified_message_is(rig.account + 99, id, uid)
        .unwrap());

    rig.core.mark_notified(rig.account, vec![uid]).unwrap();
    assert!(block(rig.core.sync_once(rig.account, 8_000))
        .unwrap()
        .new_mail
        .is_empty());
    assert_eq!(
        block(rig.core.sync_once(rig.account + 99, 8_000)).err(),
        Some(CoreError::NoSuchAccount)
    );
}

#[test]
fn the_sample_mailbox_is_never_checked() {
    let db = TempDb::new();
    let core = Core::open(db.path.display().to_string(), None, MemProvider::new()).unwrap();
    let demo = block(core.add_demo_account()).unwrap();
    let out = block(core.sync_once(demo.id, 8_000)).unwrap();
    assert!(out.new_mail.is_empty() && !out.needs_sign_in && !out.transient_failure);
}
