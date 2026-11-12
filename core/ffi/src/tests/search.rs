// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Search through the surface: the highlight ranges, local search on the device, and server
//! search against the in-process IMAP server (a hit the device did not have becomes a row).

use super::*;
use crate::search::{find_words, unmark, TextRange};

fn r(start: u32, end: u32) -> TextRange {
    TextRange { start, end }
}

#[test]
fn marks_become_utf16_ranges_and_stray_marks_are_dropped() {
    let (text, ranges) = unmark("Quarterly \u{2}budget\u{3} review");
    assert_eq!(text, "Quarterly budget review");
    assert_eq!(ranges, vec![r(10, 16)]);
    // An emoji is two UTF-16 units: the range after it counts them.
    let (text, ranges) = unmark("\u{1F600} \u{2}lunch\u{3}");
    assert_eq!(text, "\u{1F600} lunch");
    assert_eq!(ranges, vec![r(3, 8)]);
    // A lone end mark, and an unclosed start, mark nothing.
    assert_eq!(unmark("a\u{3}b\u{2}c"), ("abc".to_owned(), vec![]));
}

#[test]
fn server_hits_are_highlighted_by_word_ignoring_case() {
    let words = vec!["lunch".to_owned(), "plan".to_owned()];
    assert_eq!(
        find_words("Lunch plans for LUNCH", &words),
        vec![r(0, 5), r(6, 10), r(16, 21)]
    );
    assert_eq!(find_words("nothing here", &words), vec![]);
    assert_eq!(find_words("abc", &["".to_owned()]), vec![]);
}

#[test]
fn local_search_finds_and_highlights_and_says_bodies_were_not_all_searched() {
    let rig = Rig::new(|s| {
        s.add_message("INBOX", &raw("a@example.org", "Lunch on Friday"));
        s.add_message("INBOX", &raw("b@example.org", "Budget review"));
    });
    rig.wait_for_inbox(2);
    let found = rig
        .core
        .search_local(Some(rig.account), "lunch".into(), 50)
        .unwrap();
    assert_eq!(found.hits.len(), 1);
    let hit = &found.hits[0];
    assert_eq!(hit.row.subject, "Lunch on Friday");
    assert_eq!(hit.subject_ranges, vec![r(0, 5)]);
    assert!(found.bodies_partial, "no body is downloaded yet");
    // All accounts, an empty query, a bad date and an unknown account.
    assert_eq!(
        rig.core
            .search_local(None, "budget".into(), 50)
            .unwrap()
            .hits
            .len(),
        1
    );
    assert!(rig
        .core
        .search_local(None, "  ".into(), 50)
        .unwrap()
        .hits
        .is_empty());
    assert_eq!(
        rig.core
            .search_local(None, "after:someday".into(), 50)
            .err(),
        Some(SearchFailure::BadDate)
    );
    assert_eq!(
        rig.core
            .search_local(Some(rig.account + 99), "x".into(), 50)
            .err(),
        Some(SearchFailure::NoSuchAccount)
    );
}

#[test]
fn server_search_brings_in_a_message_the_device_did_not_have() {
    let rig = Rig::new(|s| {
        s.add_message("INBOX", &raw("a@example.org", "Lunch on Friday"));
    });
    rig.wait_for_inbox(1);
    // Arrives on the server after the sync: not on the device.
    rig.server
        .add_message("INBOX", &raw("late@example.org", "Lunch again"));
    assert_eq!(
        rig.core
            .search_local(Some(rig.account), "again".into(), 50)
            .unwrap()
            .hits
            .len(),
        0
    );
    let found = block(rig.core.search_server(Some(rig.account), "lunch".into())).unwrap();
    assert!(
        !found.unsupported && !found.offline && !found.failed && !found.sign_in_needed,
        "unsupported {} offline {} failed {} sign-in {}",
        found.unsupported,
        found.offline,
        found.failed,
        found.sign_in_needed
    );
    let subjects: Vec<&str> = found.hits.iter().map(|h| h.row.subject.as_str()).collect();
    assert_eq!(
        subjects,
        vec!["Lunch again", "Lunch on Friday"],
        "newest first"
    );
    assert_eq!(found.hits[0].subject_ranges, vec![r(0, 5)]);
    // Now a row like any other: local search finds it too.
    assert_eq!(
        rig.core
            .search_local(Some(rig.account), "again".into(), 50)
            .unwrap()
            .hits
            .len(),
        1
    );
    // Offline: the server is not asked, and the answer says so.
    rig.core.set_network_available(false).unwrap();
    let off = block(rig.core.search_server(Some(rig.account), "lunch".into())).unwrap();
    assert!(off.offline && off.hits.is_empty());
}

#[test]
fn the_sample_mailbox_is_never_searched_on_a_server() {
    let db = TempDb::new();
    let core = Core::open(db.path.display().to_string(), None, MemProvider::new()).unwrap();
    let demo = block(core.add_demo_account()).unwrap();
    let server = block(core.search_server(Some(demo.id), "the".into())).unwrap();
    assert!(server.hits.is_empty() && !server.failed && !server.unsupported);
    // Local search of the sample mailbox works as for any account.
    assert!(core.search_local(Some(demo.id), "the".into(), 50).is_ok());
}
