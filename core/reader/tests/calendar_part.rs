// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! T40a: `calendar_part` finds a message's invitation, and nothing else.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use base64::Engine;
use croft_reader::{calendar_part, MAX_CALENDAR_PART};

const ICS: &str = "BEGIN:VCALENDAR\r\nMETHOD:REQUEST\r\nBEGIN:VEVENT\r\nUID:lunch-1\r\nSUMMARY:Lunch \u{e0} deux\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";

fn head(ct: &str) -> String {
    format!("From: Sam <sam@example.org>\r\nTo: me@example.com\r\nSubject: Invitation\r\nMessage-ID: <i@example.org>\r\nMIME-Version: 1.0\r\nContent-Type: {ct}\r\n\r\n")
}

#[test]
fn the_calendar_part_of_an_alternative_is_found_in_any_encoding() {
    let b64 = base64::engine::general_purpose::STANDARD.encode(ICS);
    let raw = format!(
        "{}--b\r\nContent-Type: text/plain\r\n\r\nYou're invited.\r\n--b\r\nContent-Type: text/calendar; charset=utf-8; method=REQUEST\r\nContent-Transfer-Encoding: base64\r\n\r\n{b64}\r\n--b--\r\n",
        head("multipart/alternative; boundary=\"b\"")
    );
    assert_eq!(calendar_part(raw.as_bytes()).unwrap().as_deref(), Some(ICS));
    let qp = ICS.replace('\u{e0}', "=C3=A0");
    let raw = format!(
        "{}--b\r\nContent-Type: text/plain\r\n\r\nHi\r\n--b\r\nContent-Type: text/calendar; charset=utf-8\r\nContent-Transfer-Encoding: quoted-printable\r\n\r\n{qp}--b--\r\n",
        head("multipart/mixed; boundary=\"b\"")
    );
    assert!(calendar_part(raw.as_bytes())
        .unwrap()
        .unwrap()
        .contains("SUMMARY:Lunch \u{e0} deux"));
}

#[test]
fn an_attached_ics_file_counts_but_a_text_calendar_part_comes_first() {
    let b64 = base64::engine::general_purpose::STANDARD.encode(ICS);
    let raw = format!(
        "{}--b\r\nContent-Type: text/plain\r\n\r\nSee attached.\r\n--b\r\nContent-Type: application/octet-stream; name=\"invite.ics\"\r\nContent-Disposition: attachment; filename=\"invite.ics\"\r\nContent-Transfer-Encoding: base64\r\n\r\n{b64}\r\n--b--\r\n",
        head("multipart/mixed; boundary=\"b\"")
    );
    assert_eq!(calendar_part(raw.as_bytes()).unwrap().as_deref(), Some(ICS));
    let other = ICS.replace("lunch-1", "other");
    let raw = format!(
        "{}--b\r\nContent-Type: application/ics; name=\"a.ics\"\r\n\r\n{other}--b\r\nContent-Type: text/calendar\r\n\r\n{ICS}--b--\r\n",
        head("multipart/mixed; boundary=\"b\"")
    );
    assert!(calendar_part(raw.as_bytes())
        .unwrap()
        .unwrap()
        .contains("UID:lunch-1"));
}

#[test]
fn no_part_an_oversized_part_and_a_forwarded_invitation_give_none() {
    let raw = format!("{}Just text.\r\n", head("text/plain"));
    assert_eq!(calendar_part(raw.as_bytes()).unwrap(), None);
    let big = format!(
        "{}{}",
        head("text/calendar"),
        "X".repeat(MAX_CALENDAR_PART + 10)
    );
    assert_eq!(calendar_part(big.as_bytes()).unwrap(), None);
    let inner = format!("{}{ICS}", head("text/calendar"));
    let raw = format!(
        "{}--b\r\nContent-Type: text/plain\r\n\r\nFYI\r\n--b\r\nContent-Type: message/rfc822\r\n\r\n{inner}\r\n--b--\r\n",
        head("multipart/mixed; boundary=\"b\"")
    );
    assert_eq!(calendar_part(raw.as_bytes()).unwrap(), None);
    // A whole message that is the calendar.
    let whole = format!("{}{ICS}", head("text/calendar; method=REQUEST"));
    assert_eq!(
        calendar_part(whole.as_bytes()).unwrap().as_deref(),
        Some(ICS)
    );
}
