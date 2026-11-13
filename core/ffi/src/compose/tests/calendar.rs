//! T39a: calendars end to end for a real account against the in-process CalDAV fake: the first
//! sync, the window across zones and repeats, a new event going out, a change to one occurrence
//! and to all, deletes, hidden calendars, reminders, and accounts kept apart.

use croft_test_support::carddav::{DavServer, CALENDAR};

use super::*;
use crate::calendar::{CalendarSyncResult, EventInput, EventScope, RepeatPreset};

const LONDON: &str = "Europe/London";
// 2026-11-16 (a Monday) 00:00Z, and a week after.
const MON: i64 = 1_794_787_200_000;
const WEEK: i64 = 7 * 86_400_000;
const HOUR: i64 = 3_600_000;

fn standup() -> String {
    "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//Example//Test//EN\r\nBEGIN:VEVENT\r\nUID:standup\r\nDTSTART;TZID=Europe/London:20261102T093000\r\nDTEND;TZID=Europe/London:20261102T094500\r\nRRULE:FREQ=WEEKLY;BYDAY=MO,WE\r\nSUMMARY:Stand-up\r\nX-KEEP-ME:yes\r\nBEGIN:VALARM\r\nACTION:DISPLAY\r\nTRIGGER:-PT10M\r\nEND:VALARM\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n".into()
}

fn once(uid: &str, start: &str, summary: &str) -> String {
    format!("BEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VEVENT\r\nUID:{uid}\r\nDTSTART:{start}\r\nDURATION:PT1H\r\nSUMMARY:{summary}\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n")
}

fn sync(m: &Mail) -> CalendarSyncResult {
    block(m.core().clone().sync_calendars(m.account))
        .unwrap()
        .result
}

fn titles(m: &Mail, from: i64, to: i64) -> Vec<(String, i64)> {
    m.core()
        .events_between(from, to, LONDON.into())
        .unwrap()
        .occurrences
        .into_iter()
        .map(|o| (o.title, o.start_ms))
        .collect()
}

fn input(title: &str, start: i64, repeat: RepeatPreset) -> EventInput {
    EventInput {
        title: title.into(),
        all_day: false,
        start_ms: start,
        end_ms: start + HOUR,
        zone: Some(LONDON.into()),
        location: "Room 2".into(),
        notes: String::new(),
        repeat,
        reminders: vec![15],
    }
}

#[test]
fn calendars_sync_both_ways_and_answer_windows_with_repeats_and_overrides() {
    let m = Mail::new();
    let dav = block(DavServer::start_with("alice", crate::tests::PASSWORD));
    dav.put_event("standup.ics", &standup());
    dav.put_event("review.ics", &once("review", "20261117T140000Z", "Review"));
    m.core().set_carddav_url_for_tests(m.account, &dav.origin());

    assert_eq!(sync(&m), CalendarSyncResult::Done);
    let cals = m.core().calendars(m.account).unwrap();
    assert_eq!(cals.len(), 1, "the tasks-only calendar is left out");
    assert_eq!(cals[0].color.as_deref(), Some("#3A7D44"));

    // The week of 16 November: Monday and Wednesday stand-ups at 09:30 London (GMT), and the review.
    let week = titles(&m, MON, MON + WEEK);
    assert_eq!(
        week,
        [
            ("Stand-up".to_owned(), MON + 9 * HOUR + 30 * 60_000),
            ("Review".to_owned(), MON + 86_400_000 + 14 * HOUR),
            (
                "Stand-up".to_owned(),
                MON + 2 * 86_400_000 + 9 * HOUR + 30 * 60_000
            ),
        ]
    );
    let first = m
        .core()
        .events_between(MON, MON + WEEK, LONDON.into())
        .unwrap()
        .occurrences[0]
        .clone();
    assert!(first.repeats);

    // Move Wednesday's stand-up to 11:00, for that day only.
    let wed = m
        .core()
        .events_between(MON, MON + WEEK, LONDON.into())
        .unwrap()
        .occurrences[2]
        .clone();
    let mut moved = input(
        "Stand-up (late)",
        MON + 2 * 86_400_000 + 11 * HOUR,
        RepeatPreset::None,
    );
    moved.end_ms = moved.start_ms + 15 * 60_000;
    m.core()
        .save_event(
            m.account,
            Some(wed.event_id),
            wed.calendar,
            moved,
            EventScope::One { key_ms: wed.key_ms },
            LONDON.into(),
        )
        .unwrap();
    let detail = m
        .core()
        .get_event(m.account, wed.event_id, Some(wed.key_ms), LONDON.into())
        .unwrap()
        .unwrap();
    assert_eq!(detail.title, "Stand-up (late)");
    assert_eq!(detail.start_ms, MON + 2 * 86_400_000 + 11 * HOUR);
    assert!(detail.pending);
    // The other occurrences are untouched, and the next week too.
    assert_eq!(titles(&m, MON, MON + WEEK)[0].0, "Stand-up");
    assert_eq!(titles(&m, MON + WEEK, MON + 2 * WEEK)[0].0, "Stand-up");

    // Delete next Monday's only.
    let next_mon = m
        .core()
        .events_between(MON + WEEK, MON + 2 * WEEK, LONDON.into())
        .unwrap()
        .occurrences[0]
        .clone();
    assert!(m
        .core()
        .delete_event(
            m.account,
            next_mon.event_id,
            EventScope::One {
                key_ms: next_mon.key_ms
            },
            LONDON.into()
        )
        .unwrap());
    assert_eq!(titles(&m, MON + WEEK, MON + 2 * WEEK).len(), 1);

    // Sent at the next sync, with the lines Croft does not edit kept.
    assert_eq!(sync(&m), CalendarSyncResult::Done);
    let on_server = dav
        .cards()
        .into_iter()
        .find(|(p, _)| p.ends_with("standup.ics"))
        .unwrap()
        .1;
    assert!(on_server.contains("X-KEEP-ME:yes"));
    assert!(on_server.contains("RECURRENCE-ID;TZID=Europe/London:20261118T093000"));
    assert!(on_server.contains("EXDATE;TZID=Europe/London:20261123T093000"));
    assert!(
        !m.core()
            .get_event(m.account, wed.event_id, None, LONDON.into())
            .unwrap()
            .unwrap()
            .pending
    );

    // A new weekly event, stored at once and created on the server at the next sync.
    let new = m
        .core()
        .save_event(
            m.account,
            None,
            cals[0].id,
            input("Planning", MON + 15 * HOUR, RepeatPreset::Weekly),
            EventScope::All,
            LONDON.into(),
        )
        .unwrap();
    assert!(titles(&m, MON, MON + WEEK)
        .iter()
        .any(|(t, _)| t == "Planning"));
    let detail = m
        .core()
        .get_event(m.account, new, None, LONDON.into())
        .unwrap()
        .unwrap();
    assert_eq!(
        (
            detail.repeat,
            detail.reminders.clone(),
            detail.zone.as_deref()
        ),
        (RepeatPreset::Weekly, vec![15], Some(LONDON))
    );
    assert_eq!(sync(&m), CalendarSyncResult::Done);
    assert!(dav
        .cards()
        .iter()
        .any(|(p, t)| p.starts_with(CALENDAR) && t.contains("SUMMARY:Planning")));

    // Reminders: the stand-up's 10 minutes before, and Planning's 15.
    let due = m
        .core()
        .next_reminders(MON, MON + 86_400_000, LONDON.into())
        .unwrap();
    assert_eq!(
        due.iter()
            .map(|r| (r.title.as_str(), r.at_ms))
            .collect::<Vec<_>>(),
        [
            ("Stand-up", MON + 9 * HOUR + 20 * 60_000),
            ("Planning", MON + 15 * HOUR - 15 * 60_000)
        ]
    );

    // A change on the server comes in; one removed there goes.
    dav.put_event(
        "review.ics",
        &once("review", "20261117T150000Z", "Review (moved)"),
    );
    dav.remove_event("standup.ics");
    assert_eq!(sync(&m), CalendarSyncResult::Done);
    let week = titles(&m, MON, MON + WEEK);
    assert!(week
        .iter()
        .any(|(t, s)| t == "Review (moved)" && *s == MON + 86_400_000 + 15 * HOUR));
    assert!(!week.iter().any(|(t, _)| t.starts_with("Stand-up")));

    // A hidden calendar shows nothing; its events stay.
    assert!(m
        .core()
        .set_calendar_shown(m.account, cals[0].id, false)
        .unwrap());
    assert!(titles(&m, MON, MON + WEEK).is_empty());
    m.core()
        .set_calendar_shown(m.account, cals[0].id, true)
        .unwrap();

    // Delete the whole event: gone here at once, and on the server at the next sync.
    assert!(m
        .core()
        .delete_event(m.account, new, EventScope::All, LONDON.into())
        .unwrap());
    assert!(m
        .core()
        .get_event(m.account, new, None, LONDON.into())
        .unwrap()
        .is_none());
    assert_eq!(sync(&m), CalendarSyncResult::Done);
    assert!(!dav
        .cards()
        .iter()
        .any(|(_, t)| t.contains("SUMMARY:Planning")));
}

#[test]
fn a_change_the_server_made_too_keeps_the_servers_version() {
    let m = Mail::new();
    let dav = block(DavServer::start_with("alice", crate::tests::PASSWORD));
    dav.put_event("review.ics", &once("review", "20261117T140000Z", "Review"));
    m.core().set_carddav_url_for_tests(m.account, &dav.origin());
    assert_eq!(sync(&m), CalendarSyncResult::Done);
    let o = m
        .core()
        .events_between(MON, MON + WEEK, LONDON.into())
        .unwrap()
        .occurrences[0]
        .clone();
    m.core()
        .save_event(
            m.account,
            Some(o.event_id),
            o.calendar,
            input("Review (mine)", o.start_ms, RepeatPreset::None),
            EventScope::All,
            LONDON.into(),
        )
        .unwrap();
    dav.put_event(
        "review.ics",
        &once("review", "20261117T160000Z", "Review (theirs)"),
    );
    let out = block(m.core().clone().sync_calendars(m.account)).unwrap();
    assert_eq!((out.result, out.conflicts), (CalendarSyncResult::Done, 1));
    // The next sync brings the server's version.
    assert_eq!(sync(&m), CalendarSyncResult::Done);
    let week = titles(&m, MON, MON + WEEK);
    assert_eq!(
        week,
        [("Review (theirs)".to_owned(), MON + 86_400_000 + 16 * HOUR)]
    );
    assert!(
        m.core()
            .get_event(m.account, o.event_id, None, LONDON.into())
            .unwrap()
            .unwrap()
            .conflict
    );
}

#[test]
fn a_wrong_password_asks_for_sign_in_and_another_accounts_events_stay_apart() {
    let m = Mail::new();
    let dav = block(DavServer::start_with(
        "alice",
        "a-different-made-up-password",
    ));
    m.core().set_carddav_url_for_tests(m.account, &dav.origin());
    assert_eq!(sync(&m), CalendarSyncResult::SignInNeeded);
    assert!(m.core().calendars(m.account).unwrap().is_empty());
    // Another account's calendar ids are not this one's.
    assert!(!m
        .core()
        .set_calendar_shown(m.account + 1, 1, false)
        .unwrap());
    assert!(m
        .core()
        .get_event(m.account + 1, 1, None, LONDON.into())
        .unwrap()
        .is_none());
}

fn series_with_exceptions() -> String {
    "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VEVENT\r\nUID:sync\r\nDTSTART:20261116T090000Z\r\nDTEND:20261116T093000Z\r\nRRULE:FREQ=WEEKLY;INTERVAL=1\r\nEXDATE:20261123T090000Z\r\nSUMMARY:Sync\r\nDESCRIPTION:Kept\\, as written\r\nBEGIN:VALARM\r\nACTION:AUDIO\r\nTRIGGER:-PT5M\r\nACKNOWLEDGED:20261101T000000Z\r\nEND:VALARM\r\nEND:VEVENT\r\nBEGIN:VEVENT\r\nUID:sync\r\nRECURRENCE-ID:20261130T090000Z\r\nDTSTART:20261130T110000Z\r\nDTEND:20261130T113000Z\r\nSUMMARY:Sync (moved)\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n".into()
}

#[test]
fn a_title_only_edit_writes_only_the_title_and_keeps_every_exception() {
    let m = Mail::new();
    let dav = block(DavServer::start_with("alice", crate::tests::PASSWORD));
    dav.put_event("sync.ics", &series_with_exceptions());
    m.core().set_carddav_url_for_tests(m.account, &dav.origin());
    assert_eq!(sync(&m), CalendarSyncResult::Done);
    let o = m
        .core()
        .events_between(MON, MON + WEEK, LONDON.into())
        .unwrap()
        .occurrences[0]
        .clone();
    // What S38 would send back: the event as get_event gives it, with a new title only.
    let d = m
        .core()
        .get_event(m.account, o.event_id, None, LONDON.into())
        .unwrap()
        .unwrap();
    assert_eq!(d.repeat, RepeatPreset::Weekly);
    let input = EventInput {
        title: "Weekly sync".into(),
        all_day: d.all_day,
        start_ms: d.start_ms,
        end_ms: d.end_ms,
        zone: d.zone.clone(),
        location: d.location.clone(),
        notes: d.notes.clone(),
        repeat: d.repeat,
        reminders: d.reminders.clone(),
    };
    m.core()
        .save_event(
            m.account,
            Some(o.event_id),
            o.calendar,
            input,
            EventScope::All,
            LONDON.into(),
        )
        .unwrap();
    assert_eq!(sync(&m), CalendarSyncResult::Done);
    let text = dav
        .cards()
        .into_iter()
        .find(|(p, _)| p.ends_with("sync.ics"))
        .unwrap()
        .1;
    for kept in [
        "DTSTART:20261116T090000Z",
        "RRULE:FREQ=WEEKLY;INTERVAL=1",
        "EXDATE:20261123T090000Z",
        "RECURRENCE-ID:20261130T090000Z",
        "DESCRIPTION:Kept\\, as written",
        "ACTION:AUDIO",
        "ACKNOWLEDGED:20261101T000000Z",
    ] {
        assert!(text.contains(kept), "{kept} lost");
    }
    assert!(text.contains("SUMMARY:Weekly sync"));
}

#[test]
fn the_version_a_write_made_is_kept_and_a_refused_change_never_blocks_the_rest() {
    let m = Mail::new();
    let dav = block(DavServer::start_with("alice", crate::tests::PASSWORD));
    dav.put_event("review.ics", &once("review", "20261117T140000Z", "Review"));
    m.core().set_carddav_url_for_tests(m.account, &dav.origin());
    assert_eq!(sync(&m), CalendarSyncResult::Done);
    let cal = m.core().calendars(m.account).unwrap()[0].id;
    let o = m
        .core()
        .events_between(MON, MON + WEEK, LONDON.into())
        .unwrap()
        .occurrences[0]
        .clone();
    let edit = |title: &str| {
        let mut i = input(title, o.start_ms, RepeatPreset::None);
        i.end_ms = o.end_ms;
        m.core()
            .save_event(
                m.account,
                Some(o.event_id),
                o.calendar,
                i,
                EventScope::All,
                LONDON.into(),
            )
            .unwrap();
    };
    // Two edits in a row, each sent: the second is conditional on the version the first made.
    edit("Review 1");
    assert_eq!(sync(&m), CalendarSyncResult::Done);
    edit("Review 2");
    let out = block(m.core().clone().sync_calendars(m.account)).unwrap();
    assert_eq!((out.result, out.conflicts), (CalendarSyncResult::Done, 0));
    assert!(dav
        .cards()
        .iter()
        .any(|(_, t)| t.contains("SUMMARY:Review 2")));
    // A server that refuses every write: the change is marked and the rest of the sync still runs.
    dav.set_refuse_puts(true);
    edit("Review 3");
    m.core()
        .save_event(
            m.account,
            None,
            cal,
            input("New one", MON + 10 * HOUR, RepeatPreset::None),
            EventScope::All,
            LONDON.into(),
        )
        .unwrap();
    dav.put_event(
        "other.ics",
        &once("other", "20261118T140000Z", "From elsewhere"),
    );
    let out = block(m.core().clone().sync_calendars(m.account)).unwrap();
    assert_eq!((out.result, out.conflicts), (CalendarSyncResult::Done, 2));
    assert!(titles(&m, MON, MON + WEEK)
        .iter()
        .any(|(t, _)| t == "From elsewhere"));
    assert!(
        m.core()
            .get_event(m.account, o.event_id, None, LONDON.into())
            .unwrap()
            .unwrap()
            .conflict
    );
    // Nothing waits any more: the next sync sends nothing.
    let before = dav.requests().iter().filter(|r| r.method == "PUT").count();
    assert_eq!(sync(&m), CalendarSyncResult::Done);
    assert_eq!(
        dav.requests().iter().filter(|r| r.method == "PUT").count(),
        before
    );
}
