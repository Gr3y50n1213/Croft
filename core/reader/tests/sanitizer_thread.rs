// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The sanitiser's thread: one per process, cancelled at its deadline, never marking a message
//! for a reason that says nothing about it (review round 3, H6 and M1). Its own test binary, so
//! that no other test's sanitiser runs at the same time; the tests here take turns.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use croft_reader::render;

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(format!(
        "{}/tests/fixtures/{name}",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}

/// A sanitiser that only stops when it is cancelled (stands in for any slow input).
fn slow(
    _: &str,
    _: bool,
    _: &std::collections::HashMap<String, croft_reader::InlineImage>,
    cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
) -> croft_reader::Sanitized {
    let start = std::time::Instant::now();
    while !cancel.load(std::sync::atomic::Ordering::Relaxed)
        && start.elapsed() < std::time::Duration::from_secs(20)
    {
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    croft_reader::Sanitized::default()
}

/// The real sanitiser, cancellable (what `render` runs).
fn real(
    html: &str,
    remote: bool,
    images: &std::collections::HashMap<String, croft_reader::InlineImage>,
    cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
) -> croft_reader::Sanitized {
    croft_reader::sanitize_html_with(
        html,
        remote,
        images,
        &croft_reader::Options {
            cancel: Some(cancel),
            ..croft_reader::Options::default()
        },
    )
}

/// Waits (at most `limit`) until no sanitiser thread runs; returns how long that took.
fn wait_for_sanitiser_end(limit: std::time::Duration) -> std::time::Duration {
    let start = std::time::Instant::now();
    while croft_reader::sanitizer_running() && start.elapsed() < limit {
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    start.elapsed()
}

/// The tests that use the one sanitiser slot on purpose run one at a time (the others only wait
/// for it, within their deadline).
static SLOT_TESTS: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[test]
fn a_sanitiser_past_its_deadline_gives_the_text_now_and_no_mark() {
    // D, and round 3 M1: a deadline miss depends on the device's load, so it is never stored as
    // too complex (`html_pending`: the next open tries again). The text is shown at once.
    let _one = SLOT_TESTS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let raw = fixture("alternative.eml");
    let start = std::time::Instant::now();
    let r = croft_reader::render_with(&raw, false, std::time::Duration::from_millis(200), slow)
        .unwrap();
    assert!(start.elapsed() < std::time::Duration::from_secs(5));
    assert!(r.html_pending && !r.too_complex && r.truncated && r.html.is_none());
    assert!(r.text.unwrap().contains("Weekly news in plain text."));
    // H6: the sanitiser was cancelled at the deadline, and its thread has ended.
    assert!(
        wait_for_sanitiser_end(std::time::Duration::from_secs(5))
            < std::time::Duration::from_secs(5)
    );
    // An HTML-only message falls back to its HTML converted to text.
    let html_only = b"From: x@example.org\r\nSubject: H\r\nContent-Type: text/html\r\n\r\n<p>Only <b>html</b></p>\r\n";
    let r = croft_reader::render_with(
        html_only,
        false,
        std::time::Duration::from_millis(200),
        slow,
    )
    .unwrap();
    assert!(r.html_pending && !r.too_complex && r.text.unwrap().contains("Only html"));
    wait_for_sanitiser_end(std::time::Duration::from_secs(5));
    // The ordinary path renders.
    let r = render(&raw, false).unwrap();
    assert!(!r.too_complex && !r.html_pending && r.html.is_some());
}

#[test]
fn after_a_deadline_the_real_parser_stops_within_milliseconds() {
    // H6: the deadline sets the parser's cancel flag; it stops at its next charge, so the thread
    // ends at once instead of running on. An input well inside the budget that takes long in a
    // debug build: 14 000 spans with a 340-byte style each.
    let _one = SLOT_TESTS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let body = format!(
        "<span style=\"{}\">x</span>",
        "color:red;font-size:12px;margin:0;".repeat(10)
    )
    .repeat(14_000);
    let raw =
        format!("From: x@example.org\r\nSubject: Big\r\nContent-Type: text/html\r\n\r\n{body}\r\n");
    // Uncancelled it takes much longer than the deadline below (and than the wait after it).
    let start = std::time::Instant::now();
    let r = croft_reader::render_with(
        raw.as_bytes(),
        false,
        std::time::Duration::from_millis(30),
        real,
    )
    .unwrap();
    let returned = start.elapsed();
    assert!(r.html_pending && !r.too_complex && r.html.is_none());
    let ended = wait_for_sanitiser_end(std::time::Duration::from_secs(10));
    assert!(
        ended < std::time::Duration::from_millis(500),
        "the thread ran on for {ended:?} after the deadline (returned after {returned:?})"
    );
    // The same input, uncancelled, takes far longer than that wait.
    let start = std::time::Instant::now();
    let full = croft_reader::sanitize_html(&body, false, &Default::default());
    assert!(!full.too_complex);
    assert!(
        start.elapsed() > ended * 2,
        "{:?} vs {ended:?}",
        start.elapsed()
    );
}

#[test]
fn a_second_message_while_one_is_sanitised_gets_its_text_unmarked() {
    // H6: one sanitiser thread per process. A message that comes while the slot stays taken past
    // its own deadline is shown as text, not marked, and renders on a later open.
    let _one = SLOT_TESTS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let first = std::thread::spawn(|| {
        croft_reader::render_with(
            &fixture("alternative.eml"),
            false,
            std::time::Duration::from_millis(1500),
            slow,
        )
        .unwrap()
    });
    let start = std::time::Instant::now();
    while !croft_reader::sanitizer_running() && start.elapsed() < std::time::Duration::from_secs(5)
    {
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    assert!(croft_reader::sanitizer_running());
    let raw = fixture("alternative.eml");
    let second =
        croft_reader::render_with(&raw, false, std::time::Duration::from_millis(100), real)
            .unwrap();
    assert!(second.html_pending && !second.too_complex && second.html.is_none());
    assert!(second.text.unwrap().contains("Weekly news in plain text."));
    assert!(first.join().unwrap().html_pending);
    wait_for_sanitiser_end(std::time::Duration::from_secs(5));
    // The next open renders it.
    let again = render(&raw, false).unwrap();
    assert!(!again.html_pending && again.html.is_some());
}
