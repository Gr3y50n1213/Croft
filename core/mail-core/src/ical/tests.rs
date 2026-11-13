//! Tests of the iCalendar reader, its zones, repeats and occurrences, and the writer (T39a).
//! Events, names and addresses are made up.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use super::expand::{occurrences, span};
use super::rrule;
use super::write::{self, EventFields, Repeat};
use super::zone::{iana, Zones};
use super::*;
use jiff::tz::TimeZone;

fn ts(s: &str) -> Timestamp {
    s.parse().expect("timestamp")
}

fn wall(s: &str) -> DateTime {
    s.parse().expect("datetime")
}

fn object(event_lines: &str) -> String {
    format!("BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//Example//Test//EN\r\nBEGIN:VEVENT\r\nUID:test-1@example.org\r\n{}\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n", event_lines.trim().replace('\n', "\r\n"))
}

/// The starts in `[from, to)` as wall times in `zone`.
fn starts_in(ics: &str, zone: &str, from: &str, to: &str) -> Vec<String> {
    let cal = parse(ics).expect("parses");
    let tz = TimeZone::get(zone).expect("zone");
    occurrences(&cal, &tz, ts(from), ts(to))
        .occurrences
        .iter()
        .map(|o| {
            o.start
                .to_zoned(tz.clone())
                .strftime("%Y-%m-%d %H:%M")
                .to_string()
        })
        .collect()
}

fn days(ics: &str, from: &str, to: &str) -> Vec<String> {
    starts_in(ics, "America/New_York", from, to)
        .into_iter()
        .map(|s| s[..10].to_owned())
        .collect()
}

const NY: &str = "DTSTART;TZID=America/New_York:";

// ---- Reading ----

#[test]
fn an_object_is_written_back_as_it_was_read() {
    let text = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//Example//Test//EN\r\nX-WR-CALNAME:Team\r\nBEGIN:VEVENT\r\nUID:a@example.org\r\nDTSTART:20261113T090000Z\r\nSUMMARY:Server migration\\, part 2\r\nX-EXAMPLE-COLOUR;X-PARAM=\"a:b\":keep me\r\nBEGIN:X-SOMETHING\r\nX-FIELD:kept\r\nEND:X-SOMETHING\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";
    let cal = parse(text).expect("parses");
    assert_eq!(cal.to_text(), text);
    let e = &cal.events()[0];
    assert_eq!(e.summary.as_deref(), Some("Server migration, part 2"));
    assert_eq!(e.start, Some(When::Utc(ts("2026-11-13T09:00:00Z"))));
}

#[test]
fn folded_lines_are_joined_and_long_lines_folded_at_75_octets() {
    let text = "BEGIN:VCALENDAR\nBEGIN:VEVENT\nUID:b@example.org\nDESCRIPTION:Runs the \n DNS side\nEND:VEVENT\nEND:VCALENDAR\n";
    let cal = parse(text).expect("parses");
    assert_eq!(
        cal.events()[0].description.as_deref(),
        Some("Runs the DNS side")
    );
    let mut long = cal.clone();
    let master = long.master_mut().expect("master");
    master.set_lines(
        "DESCRIPTION",
        vec![make("DESCRIPTION", &[], &"é".repeat(100))],
    );
    for line in long.to_text().split("\r\n") {
        assert!(line.len() <= 75, "{} octets", line.len());
    }
    assert_eq!(
        parse(&long.to_text()).expect("again").events()[0].description,
        Some("é".repeat(100))
    );
}

#[test]
fn every_form_of_start_end_and_duration_is_read() {
    let cal = parse(&object(
        "DTSTART;VALUE=DATE:20261113\nDTEND;VALUE=DATE:20261115\nRECURRENCE-ID;TZID=Europe/London:20261113T090000\nRDATE:20261120T100000Z,20261121T100000Z\nEXDATE;TZID=Europe/London:20261127T090000\nDURATION:P1DT2H30M",
    ))
    .expect("parses");
    let e = &cal.events()[0];
    assert_eq!(e.start, Some(When::Date(jiff::civil::date(2026, 11, 13))));
    assert_eq!(e.end, Some(When::Date(jiff::civil::date(2026, 11, 15))));
    assert_eq!(
        e.recurrence_id,
        Some(When::Zoned(
            wall("2026-11-13T09:00"),
            "Europe/London".into()
        ))
    );
    assert_eq!(e.rdates.len(), 2);
    assert_eq!(e.exdates.len(), 1);
    assert_eq!(e.duration, Some(86_400 + 2 * 3600 + 30 * 60));
    assert_eq!(parse_duration("-PT15M"), Some(-900));
    assert_eq!(parse_duration("P2W"), Some(14 * 86_400));
    assert_eq!(parse_duration("P"), None);
    assert_eq!(parse_duration("PT1D"), None);
    let floating = parse(&object("DTSTART:20261113T090000")).expect("parses");
    assert_eq!(
        floating.events()[0].start,
        Some(When::Floating(wall("2026-11-13T09:00")))
    );
}

#[test]
fn people_alarms_and_status_are_read() {
    let cal = parse(&object(
        "DTSTART:20261113T090000Z\nSTATUS:tentative\nSEQUENCE:3\nORGANIZER;CN=Sam Rivera:mailto:sam@example.org\nATTENDEE;CN=\"Okafor, Mira\";PARTSTAT=ACCEPTED;ROLE=REQ-PARTICIPANT:mailto:mira@example.org\nATTENDEE;PARTSTAT=needs-action:MAILTO:nora@example.org\nBEGIN:VALARM\nACTION:DISPLAY\nTRIGGER:-PT15M\nEND:VALARM\nBEGIN:VALARM\nACTION:DISPLAY\nTRIGGER;RELATED=END:-PT5M\nEND:VALARM\nBEGIN:VALARM\nACTION:DISPLAY\nTRIGGER;VALUE=DATE-TIME:20261112T090000Z\nEND:VALARM\nBEGIN:VALARM\nACTION:DISPLAY\nTRIGGER:-P1D\nEND:VALARM",
    ))
    .expect("parses");
    let e = &cal.events()[0];
    assert_eq!(e.status.as_deref(), Some("TENTATIVE"));
    assert_eq!(e.sequence, 3);
    assert_eq!(
        e.organizer.as_ref().map(|p| p.address.as_str()),
        Some("sam@example.org")
    );
    assert_eq!(e.attendees.len(), 2);
    assert_eq!(e.attendees[0].name.as_deref(), Some("Okafor, Mira"));
    assert_eq!(e.attendees[0].status.as_deref(), Some("ACCEPTED"));
    assert_eq!(e.attendees[1].address, "nora@example.org");
    assert_eq!(e.attendees[1].status.as_deref(), Some("NEEDS-ACTION"));
    // Only the alarms relative to the start: 15 minutes and a day before.
    assert_eq!(e.reminders, vec![15, 1440]);
}

#[test]
fn broken_or_hostile_objects_are_refused_or_bounded() {
    assert_eq!(
        parse("BEGIN:VCARD\r\nEND:VCARD\r\n").err(),
        Some(IcalError::NotACalendar)
    );
    assert_eq!(
        parse("BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nEND:VCALENDAR\r\n").err(),
        Some(IcalError::NotACalendar)
    );
    assert_eq!(
        parse("BEGIN:VCALENDAR\r\nEND:VCALENDAR\r\nBEGIN:VCALENDAR\r\nEND:VCALENDAR\r\n").err(),
        Some(IcalError::NotACalendar)
    );
    assert_eq!(parse("SUMMARY:x\r\n").err(), Some(IcalError::NotACalendar));
    let big = format!(
        "BEGIN:VCALENDAR\r\nX-A:{}\r\nEND:VCALENDAR\r\n",
        "a".repeat(MAX_OBJECT)
    );
    assert_eq!(parse(&big).err(), Some(IcalError::TooLarge));
    let many = format!(
        "BEGIN:VCALENDAR\r\n{}END:VCALENDAR\r\n",
        "X-A:1\r\n".repeat(MAX_LINES)
    );
    assert_eq!(parse(&many).err(), Some(IcalError::TooLarge));
    let deep = format!(
        "BEGIN:VCALENDAR\r\n{}{}END:VCALENDAR\r\n",
        "BEGIN:X\r\n".repeat(MAX_DEPTH),
        "END:X\r\n".repeat(MAX_DEPTH)
    );
    assert_eq!(parse(&deep).err(), Some(IcalError::TooLarge));
    let components = format!(
        "BEGIN:VCALENDAR\r\n{}END:VCALENDAR\r\n",
        "BEGIN:X\r\nEND:X\r\n".repeat(MAX_COMPONENTS)
    );
    assert_eq!(parse(&components).err(), Some(IcalError::TooLarge));
    // Bad lines are skipped, control characters dropped, text capped.
    let cal = parse(&object(&format!(
        "DTSTART:2026-11-13\nnot a line\nSUMMARY:a\u{7}b{}",
        "c".repeat(MAX_TEXT_CHARS)
    )))
    .expect("parses");
    let e = &cal.events()[0];
    assert_eq!(e.start, None);
    let s = e.summary.as_deref().unwrap_or_default();
    assert!(s.starts_with("abc") && s.chars().count() == MAX_TEXT_CHARS);
    // Nothing of the event is printed.
    assert!(!format!("{cal:?}{e:?}").contains("abc"));
}

// ---- Time zones ----

#[test]
fn zones_are_found_by_iana_name_with_a_prefix_or_from_the_vtimezone() {
    assert!(iana("Europe/London").is_some());
    assert!(iana("/mozilla.org/20050126_1/America/New_York").is_some());
    assert!(iana("/citadel.org/20190914_1/Europe/Berlin").is_some());
    assert!(iana("Not/A_Zone").is_none());
    // An Outlook-style name with its own VTIMEZONE (US Eastern rules).
    let text = "BEGIN:VCALENDAR\r\nBEGIN:VTIMEZONE\r\nTZID:Eastern Standard Time\r\nBEGIN:STANDARD\r\nDTSTART:16010101T020000\r\nTZOFFSETFROM:-0400\r\nTZOFFSETTO:-0500\r\nRRULE:FREQ=YEARLY;BYDAY=1SU;BYMONTH=11\r\nEND:STANDARD\r\nBEGIN:DAYLIGHT\r\nDTSTART:16010101T020000\r\nTZOFFSETFROM:-0500\r\nTZOFFSETTO:-0400\r\nRRULE:FREQ=YEARLY;BYDAY=2SU;BYMONTH=3\r\nEND:DAYLIGHT\r\nEND:VTIMEZONE\r\nBEGIN:VEVENT\r\nUID:c@example.org\r\nDTSTART;TZID=Eastern Standard Time:20260701T090000\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";
    let cal = parse(text).expect("parses");
    let zones = Zones::of(&cal);
    let summer = zones.wall_in(wall("2026-07-01T09:00"), "Eastern Standard Time");
    assert!(summer.zone_known);
    assert_eq!(summer.at, ts("2026-07-01T13:00:00Z"));
    let winter = zones.wall_in(wall("2026-12-01T09:00"), "Eastern Standard Time");
    assert_eq!(winter.at, ts("2026-12-01T14:00:00Z"));
    // A zone with only a standard part is a fixed offset.
    let fixed = text.replace("BEGIN:DAYLIGHT\r\nDTSTART:16010101T020000\r\nTZOFFSETFROM:-0500\r\nTZOFFSETTO:-0400\r\nRRULE:FREQ=YEARLY;BYDAY=2SU;BYMONTH=3\r\nEND:DAYLIGHT\r\n", "");
    let zones = Zones::of(&parse(&fixed).expect("parses"));
    assert_eq!(
        zones
            .wall_in(wall("2026-07-01T09:00"), "Eastern Standard Time")
            .at,
        ts("2026-07-01T14:00:00Z")
    );
    // An unknown zone reads the wall time as UTC and says so.
    let unknown = zones.wall_in(wall("2026-07-01T09:00"), "Nowhere Time");
    assert!(!unknown.zone_known);
    assert_eq!(unknown.at, ts("2026-07-01T09:00:00Z"));
}

#[test]
fn a_time_in_the_spring_gap_moves_forward_and_one_in_the_autumn_fold_takes_the_first() {
    let zones = Zones::default();
    // London, 29 March 2026: 01:00 to 02:00 does not exist.
    assert_eq!(
        zones.wall_in(wall("2026-03-29T01:30"), "Europe/London").at,
        ts("2026-03-29T01:30:00Z")
    );
    // 25 October 2026: 01:30 happens twice; the first is BST.
    assert_eq!(
        zones.wall_in(wall("2026-10-25T01:30"), "Europe/London").at,
        ts("2026-10-25T00:30:00Z")
    );
}

// ---- Repeats: RFC 5545's own examples (section 3.8.5.3) that Croft expands ----

#[test]
fn rfc_daily_for_10_occurrences() {
    let ics = object(&format!("{NY}19970902T090000\nRRULE:FREQ=DAILY;COUNT=10"));
    assert_eq!(
        starts_in(
            &ics,
            "America/New_York",
            "1997-01-01T00:00:00Z",
            "1998-01-01T00:00:00Z"
        ),
        (2..=11)
            .map(|d| format!("1997-09-{d:02} 09:00"))
            .collect::<Vec<_>>()
    );
}

#[test]
fn rfc_daily_until_and_every_10_days_5_times() {
    let ics = object(&format!(
        "{NY}19970902T090000\nRRULE:FREQ=DAILY;UNTIL=19971224T000000Z"
    ));
    let d = days(&ics, "1997-01-01T00:00:00Z", "1998-06-01T00:00:00Z");
    assert_eq!(d.first().map(String::as_str), Some("1997-09-02"));
    assert_eq!(d.last().map(String::as_str), Some("1997-12-23"));
    assert_eq!(d.len(), 113);
    let ics = object(&format!(
        "{NY}19970902T090000\nRRULE:FREQ=DAILY;INTERVAL=10;COUNT=5"
    ));
    assert_eq!(
        days(&ics, "1997-01-01T00:00:00Z", "1998-06-01T00:00:00Z"),
        [
            "1997-09-02",
            "1997-09-12",
            "1997-09-22",
            "1997-10-02",
            "1997-10-12"
        ]
    );
}

#[test]
fn rfc_weekly_on_tuesday_and_thursday_until() {
    let ics = object(&format!(
        "{NY}19970902T090000\nRRULE:FREQ=WEEKLY;UNTIL=19971007T000000Z;WKST=SU;BYDAY=TU,TH"
    ));
    assert_eq!(
        days(&ics, "1997-01-01T00:00:00Z", "1998-01-01T00:00:00Z"),
        [
            "1997-09-02",
            "1997-09-04",
            "1997-09-09",
            "1997-09-11",
            "1997-09-16",
            "1997-09-18",
            "1997-09-23",
            "1997-09-25",
            "1997-09-30",
            "1997-10-02"
        ]
    );
}

#[test]
fn rfc_week_start_changes_which_days_an_every_other_week_rule_picks() {
    let mo = object(&format!(
        "{NY}19970805T090000\nRRULE:FREQ=WEEKLY;INTERVAL=2;COUNT=4;BYDAY=TU,SU;WKST=MO"
    ));
    assert_eq!(
        days(&mo, "1997-01-01T00:00:00Z", "1998-01-01T00:00:00Z"),
        ["1997-08-05", "1997-08-10", "1997-08-19", "1997-08-24"]
    );
    let su = object(&format!(
        "{NY}19970805T090000\nRRULE:FREQ=WEEKLY;INTERVAL=2;COUNT=4;BYDAY=TU,SU;WKST=SU"
    ));
    assert_eq!(
        days(&su, "1997-01-01T00:00:00Z", "1998-01-01T00:00:00Z"),
        ["1997-08-05", "1997-08-17", "1997-08-19", "1997-08-31"]
    );
}

#[test]
fn rfc_monthly_first_friday_and_second_to_last_monday() {
    let ics = object(&format!(
        "{NY}19970905T090000\nRRULE:FREQ=MONTHLY;COUNT=10;BYDAY=1FR"
    ));
    assert_eq!(
        days(&ics, "1997-01-01T00:00:00Z", "1999-01-01T00:00:00Z"),
        [
            "1997-09-05",
            "1997-10-03",
            "1997-11-07",
            "1997-12-05",
            "1998-01-02",
            "1998-02-06",
            "1998-03-06",
            "1998-04-03",
            "1998-05-01",
            "1998-06-05"
        ]
    );
    let ics = object(&format!(
        "{NY}19970922T090000\nRRULE:FREQ=MONTHLY;COUNT=6;BYDAY=-2MO"
    ));
    assert_eq!(
        days(&ics, "1997-01-01T00:00:00Z", "1999-01-01T00:00:00Z"),
        [
            "1997-09-22",
            "1997-10-20",
            "1997-11-17",
            "1997-12-22",
            "1998-01-19",
            "1998-02-16"
        ]
    );
}

#[test]
fn rfc_monthly_third_to_last_day_and_last_working_day() {
    let ics = object(&format!(
        "{NY}19970928T090000\nRRULE:FREQ=MONTHLY;BYMONTHDAY=-3"
    ));
    assert_eq!(
        days(&ics, "1997-01-01T00:00:00Z", "1998-03-01T00:00:00Z"),
        [
            "1997-09-28",
            "1997-10-29",
            "1997-11-28",
            "1997-12-29",
            "1998-01-29",
            "1998-02-26"
        ]
    );
    let ics = object(&format!(
        "{NY}19970930T090000\nRRULE:FREQ=MONTHLY;BYDAY=MO,TU,WE,TH,FR;BYSETPOS=-1"
    ));
    assert_eq!(
        days(&ics, "1997-01-01T00:00:00Z", "1998-04-01T00:00:00Z"),
        [
            "1997-09-30",
            "1997-10-31",
            "1997-11-28",
            "1997-12-31",
            "1998-01-30",
            "1998-02-27",
            "1998-03-31"
        ]
    );
}

#[test]
fn rfc_every_friday_the_13th_except_the_start() {
    let ics = object(&format!("{NY}19970902T090000\nEXDATE;TZID=America/New_York:19970902T090000\nRRULE:FREQ=MONTHLY;BYDAY=FR;BYMONTHDAY=13"));
    assert_eq!(
        days(&ics, "1997-01-01T00:00:00Z", "2000-12-31T00:00:00Z"),
        [
            "1998-02-13",
            "1998-03-13",
            "1998-11-13",
            "1999-08-13",
            "2000-10-13"
        ]
    );
}

#[test]
fn rfc_yearly_in_june_and_july_the_20th_monday_and_election_day() {
    let ics = object(&format!(
        "{NY}19970610T090000\nRRULE:FREQ=YEARLY;COUNT=10;BYMONTH=6,7"
    ));
    let d = days(&ics, "1997-01-01T00:00:00Z", "2003-01-01T00:00:00Z");
    assert_eq!(d.len(), 10);
    assert_eq!(
        &d[..4],
        ["1997-06-10", "1997-07-10", "1998-06-10", "1998-07-10"]
    );
    assert_eq!(d.last().map(String::as_str), Some("2001-07-10"));
    let ics = object(&format!(
        "{NY}19970519T090000\nRRULE:FREQ=YEARLY;BYDAY=20MO"
    ));
    assert_eq!(
        days(&ics, "1997-01-01T00:00:00Z", "2000-01-01T00:00:00Z"),
        ["1997-05-19", "1998-05-18", "1999-05-17"]
    );
    let ics = object(&format!("{NY}19961105T090000\nRRULE:FREQ=YEARLY;INTERVAL=4;BYMONTH=11;BYDAY=TU;BYMONTHDAY=2,3,4,5,6,7,8"));
    assert_eq!(
        days(&ics, "1996-01-01T00:00:00Z", "2005-01-01T00:00:00Z"),
        ["1996-11-05", "2000-11-07", "2004-11-02"]
    );
}

#[test]
fn days_a_period_lacks_are_skipped_never_moved() {
    let leap = object("DTSTART;VALUE=DATE:20240229\nRRULE:FREQ=YEARLY;COUNT=3");
    assert_eq!(
        starts_in(&leap, "UTC", "2024-01-01T00:00:00Z", "2040-01-01T00:00:00Z"),
        ["2024-02-29 00:00", "2028-02-29 00:00", "2032-02-29 00:00"]
    );
    let monthly = object(&format!("{NY}20260131T090000\nRRULE:FREQ=MONTHLY;COUNT=4"));
    assert_eq!(
        days(&monthly, "2026-01-01T00:00:00Z", "2027-01-01T00:00:00Z"),
        ["2026-01-31", "2026-03-31", "2026-05-31", "2026-07-31"]
    );
}

#[test]
fn a_rule_croft_does_not_expand_shows_only_its_first_occurrence() {
    for rule in [
        "FREQ=DAILY;BYHOUR=9,17",
        "FREQ=HOURLY",
        "FREQ=YEARLY;BYWEEKNO=20",
        "FREQ=DAILY;COUNT=2;UNTIL=20270101T000000Z",
        "FREQ=WEEKLY;BYDAY=2MO",
        "FREQ=DAILY;X-FOO=1",
    ] {
        let ics = object(&format!("{NY}20261102T090000\nRRULE:{rule}"));
        let cal = parse(&ics).expect("parses");
        let e = occurrences(
            &cal,
            &TimeZone::UTC,
            ts("2026-01-01T00:00:00Z"),
            ts("2027-06-01T00:00:00Z"),
        );
        assert!(e.repeats_unsupported, "{rule}");
        assert_eq!(e.occurrences.len(), 1, "{rule}");
    }
    assert!(rrule::parse("INTERVAL=2").is_none());
}

#[test]
fn repeats_keep_their_wall_time_across_daylight_saving() {
    // New York, weekly at 09:00 across 1 November 2026 (EDT to EST).
    let ics = object(&format!("{NY}20261026T090000\nDTEND;TZID=America/New_York:20261026T100000\nRRULE:FREQ=WEEKLY;COUNT=2"));
    let cal = parse(&ics).expect("parses");
    let o = occurrences(
        &cal,
        &TimeZone::UTC,
        ts("2026-10-01T00:00:00Z"),
        ts("2026-12-01T00:00:00Z"),
    )
    .occurrences;
    assert_eq!(
        o.iter().map(|o| o.start).collect::<Vec<_>>(),
        [ts("2026-10-26T13:00:00Z"), ts("2026-11-02T14:00:00Z")]
    );
    assert_eq!(o[1].end, ts("2026-11-02T15:00:00Z"));
    // London, daily at 09:00 across 29 March 2026 (GMT to BST).
    let ics = object("DTSTART;TZID=Europe/London:20260328T090000\nRRULE:FREQ=DAILY;COUNT=2");
    let o = occurrences(
        &parse(&ics).expect("parses"),
        &TimeZone::UTC,
        ts("2026-03-01T00:00:00Z"),
        ts("2026-04-01T00:00:00Z"),
    )
    .occurrences;
    assert_eq!(
        o.iter().map(|o| o.start).collect::<Vec<_>>(),
        [ts("2026-03-28T09:00:00Z"), ts("2026-03-29T08:00:00Z")]
    );
}

#[test]
fn an_until_in_utc_is_compared_as_an_instant() {
    // The last day's 09:00 New York is 13:00Z: an UNTIL of 13:00Z includes it, 12:59Z does not.
    let incl = object(&format!(
        "{NY}20260901T090000\nRRULE:FREQ=DAILY;UNTIL=20260903T130000Z"
    ));
    assert_eq!(
        days(&incl, "2026-01-01T00:00:00Z", "2027-01-01T00:00:00Z").len(),
        3
    );
    let excl = object(&format!(
        "{NY}20260901T090000\nRRULE:FREQ=DAILY;UNTIL=20260903T125900Z"
    ));
    assert_eq!(
        days(&excl, "2026-01-01T00:00:00Z", "2027-01-01T00:00:00Z").len(),
        2
    );
}

// ---- Occurrences in a window ----

#[test]
fn overrides_replace_their_occurrence_and_cancelled_ones_remove_it() {
    let text = format!(
        "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nUID:d@example.org\r\n{NY}20261102T090000\r\nDTEND;TZID=America/New_York:20261102T093000\r\nSUMMARY:Stand-up\r\nRRULE:FREQ=DAILY;COUNT=5\r\nEND:VEVENT\r\nBEGIN:VEVENT\r\nUID:d@example.org\r\nRECURRENCE-ID;TZID=America/New_York:20261103T090000\r\n{NY}20261103T110000\r\nDTEND;TZID=America/New_York:20261103T120000\r\nSUMMARY:Stand-up (moved)\r\nEND:VEVENT\r\nBEGIN:VEVENT\r\nUID:d@example.org\r\nRECURRENCE-ID;TZID=America/New_York:20261105T090000\r\n{NY}20261105T090000\r\nSTATUS:CANCELLED\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n"
    );
    let cal = parse(&text).expect("parses");
    let events = cal.events();
    assert!(events[0].recurrence_id.is_none());
    let e = occurrences(
        &cal,
        &TimeZone::UTC,
        ts("2026-11-01T00:00:00Z"),
        ts("2026-12-01T00:00:00Z"),
    );
    let starts: Vec<_> = e.occurrences.iter().map(|o| o.start).collect();
    assert_eq!(
        starts,
        [
            ts("2026-11-02T14:00:00Z"),
            ts("2026-11-03T16:00:00Z"),
            ts("2026-11-04T14:00:00Z"),
            ts("2026-11-06T14:00:00Z")
        ]
    );
    let moved = &e.occurrences[1];
    assert_eq!(
        events[moved.source].summary.as_deref(),
        Some("Stand-up (moved)")
    );
    assert_eq!(moved.key, ts("2026-11-03T14:00:00Z"));
    assert_eq!(moved.end, ts("2026-11-03T17:00:00Z"));
}

#[test]
fn all_day_events_are_whole_days_where_the_viewer_is() {
    let ics = object("DTSTART;VALUE=DATE:20261113\nDTEND;VALUE=DATE:20261115\nSUMMARY:Conference");
    let cal = parse(&ics).expect("parses");
    let tokyo = TimeZone::get("Asia/Tokyo").expect("zone");
    let o = &occurrences(
        &cal,
        &tokyo,
        ts("2026-11-01T00:00:00Z"),
        ts("2026-12-01T00:00:00Z"),
    )
    .occurrences[0];
    assert!(o.all_day);
    assert_eq!(o.start, ts("2026-11-12T15:00:00Z"));
    assert_eq!(o.end, ts("2026-11-14T15:00:00Z"));
    // Touching the window's edge is not overlapping it.
    assert!(occurrences(
        &cal,
        &tokyo,
        ts("2026-11-14T15:00:00Z"),
        ts("2026-11-20T00:00:00Z")
    )
    .occurrences
    .is_empty());
    // A date with no end lasts one day.
    let one = parse(&object("DTSTART;VALUE=DATE:20261113")).expect("parses");
    let o = &occurrences(
        &one,
        &TimeZone::UTC,
        ts("2026-11-01T00:00:00Z"),
        ts("2026-12-01T00:00:00Z"),
    )
    .occurrences[0];
    assert_eq!(o.end, ts("2026-11-14T00:00:00Z"));
}

#[test]
fn rdates_add_occurrences_and_a_window_far_ahead_of_the_start_is_reached() {
    let ics = object("DTSTART:20261102T090000Z\nRDATE:20261110T150000Z\nRRULE:FREQ=WEEKLY;COUNT=2");
    let cal = parse(&ics).expect("parses");
    let o = occurrences(
        &cal,
        &TimeZone::UTC,
        ts("2026-11-01T00:00:00Z"),
        ts("2026-12-01T00:00:00Z"),
    )
    .occurrences;
    assert_eq!(
        o.iter().map(|o| o.start).collect::<Vec<_>>(),
        [
            ts("2026-11-02T09:00:00Z"),
            ts("2026-11-09T09:00:00Z"),
            ts("2026-11-10T15:00:00Z")
        ]
    );
    let daily = parse(&object("DTSTART:19900101T090000Z\nRRULE:FREQ=DAILY")).expect("parses");
    let o = occurrences(
        &daily,
        &TimeZone::UTC,
        ts("2026-11-13T00:00:00Z"),
        ts("2026-11-15T00:00:00Z"),
    )
    .occurrences;
    assert_eq!(o.len(), 2);
}

#[test]
fn spans_close_for_finite_repeats_and_stay_open_otherwise() {
    let once = parse(&object("DTSTART:20261113T090000Z\nDTEND:20261113T100000Z")).expect("parses");
    assert_eq!(
        span(&once, &TimeZone::UTC).map(|s| (s.first, s.last)),
        Some((ts("2026-11-13T09:00:00Z"), Some(ts("2026-11-13T10:00:00Z"))))
    );
    let counted = parse(&object(
        "DTSTART:20261113T090000Z\nDURATION:PT1H\nRRULE:FREQ=WEEKLY;COUNT=3",
    ))
    .expect("parses");
    assert_eq!(
        span(&counted, &TimeZone::UTC).and_then(|s| s.last),
        Some(ts("2026-11-27T10:00:00Z"))
    );
    let open = parse(&object("DTSTART:20261113T090000Z\nRRULE:FREQ=WEEKLY")).expect("parses");
    assert_eq!(span(&open, &TimeZone::UTC).map(|s| s.last), Some(None));
}

// ---- Writing ----

fn fields(title: &str) -> EventFields {
    EventFields {
        title: title.into(),
        all_day: false,
        start: wall("2026-11-16T09:00"),
        end: wall("2026-11-16T09:30"),
        zone: Some("Europe/Berlin".into()),
        location: "Room 2".into(),
        notes: "Bring the plan; and the notes".into(),
        repeat: Repeat::Weekly,
        reminders: vec![10],
    }
}

#[test]
fn a_new_event_reads_back_with_its_zone_repeat_and_reminder() {
    let cal = write::new_object(
        &fields("Planning"),
        "new-1@example.org",
        ts("2026-11-13T12:00:00Z"),
    );
    let text = cal.to_text();
    let back = parse(&text).expect("parses");
    let e = &back.events()[0];
    assert_eq!(e.uid, "new-1@example.org");
    assert_eq!(e.summary.as_deref(), Some("Planning"));
    assert_eq!(e.notes_for_test(), Some("Bring the plan; and the notes"));
    assert_eq!(
        e.start,
        Some(When::Zoned(
            wall("2026-11-16T09:00"),
            "Europe/Berlin".into()
        ))
    );
    assert_eq!(e.rrule.as_deref(), Some("FREQ=WEEKLY;BYDAY=MO"));
    assert_eq!(e.reminders, vec![10]);
    // The VTIMEZONE written for Berlin gives the same times as the database.
    let vt = back.time_zones();
    assert_eq!(vt.len(), 1);
    let renamed = text.replace("Europe/Berlin", "Berlin Own");
    let zones = Zones::of(&parse(&renamed).expect("parses"));
    for w in [
        "2026-01-15T09:00",
        "2026-07-15T09:00",
        "2027-03-28T09:00",
        "2027-10-31T09:00",
    ] {
        assert_eq!(
            zones.wall_in(wall(w), "Berlin Own").at,
            Zones::default().wall_in(wall(w), "Europe/Berlin").at,
            "{w}"
        );
    }
    // A zone without daylight saving gets one standard part.
    let tokyo = write::vtimezone(
        "Asia/Tokyo",
        &TimeZone::get("Asia/Tokyo").expect("zone"),
        ts("2026-11-13T00:00:00Z"),
    );
    assert_eq!(tokyo.children.len(), 1);
}

impl Event {
    fn notes_for_test(&self) -> Option<&str> {
        self.description.as_deref()
    }
}

#[test]
fn editing_every_occurrence_keeps_what_croft_does_not_edit() {
    let text = format!("BEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VEVENT\r\nUID:e@example.org\r\nSEQUENCE:4\r\n{NY}20261102T090000\r\nRRULE:FREQ=DAILY;COUNT=5\r\nEXDATE;TZID=America/New_York:20261104T090000\r\nATTENDEE;CN=Mira:mailto:mira@example.org\r\nX-APPLE-TRAVEL-ADVISORY-BEHAVIOR:AUTOMATIC\r\nBEGIN:VALARM\r\nACTION:AUDIO\r\nTRIGGER;VALUE=DATE-TIME:20261101T090000Z\r\nEND:VALARM\r\nBEGIN:VALARM\r\nACTION:DISPLAY\r\nTRIGGER:-PT5M\r\nEND:VALARM\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n");
    let mut cal = parse(&text).expect("parses");
    let before = cal.events()[0].clone();
    let mut f = fields("Stand-up");
    f.zone = Some("America/New_York".into());
    f.start = wall("2026-11-02T09:00");
    f.end = wall("2026-11-02T09:15");
    f.repeat = Repeat::Keep;
    f.reminders = vec![30];
    assert!(write::edit_all(&mut cal, &f, ts("2026-11-01T00:00:00Z")));
    let out = cal.to_text();
    assert!(out.contains("X-APPLE-TRAVEL-ADVISORY-BEHAVIOR:AUTOMATIC"));
    assert!(out.contains("ATTENDEE;CN=Mira:mailto:mira@example.org"));
    assert!(out.contains("TRIGGER;VALUE=DATE-TIME:20261101T090000Z"));
    let e = &parse(&out).expect("parses").events()[0];
    assert_eq!(e.sequence, 5);
    assert_eq!(e.rrule, before.rrule);
    // Same start, same rule: the excluded date stays.
    assert_eq!(e.exdates.len(), 1);
    assert_eq!(e.reminders, vec![30]);
    // Moving the start drops the exclusions and overrides that named the old occurrences.
    f.start = wall("2026-11-02T10:00");
    f.end = wall("2026-11-02T10:15");
    write::edit_all(&mut cal, &f, ts("2026-11-01T00:00:00Z"));
    assert!(cal.events()[0].exdates.is_empty());
}

#[test]
fn one_occurrence_is_changed_by_an_override_and_deleted_by_an_exdate() {
    let mut cal = write::new_object(
        &fields("Planning"),
        "f@example.org",
        ts("2026-11-13T12:00:00Z"),
    );
    let berlin = TimeZone::get("Europe/Berlin").expect("zone");
    let window = |cal: &Calendar| {
        occurrences(
            cal,
            &TimeZone::UTC,
            ts("2026-11-16T00:00:00Z"),
            ts("2026-12-08T00:00:00Z"),
        )
        .occurrences
    };
    let o = window(&cal);
    assert_eq!(o.len(), 4);
    // Move the second (23 November) to 14:00.
    let rid = write::recurrence_id(&cal, o[1].key, &berlin).expect("rid");
    assert_eq!(
        rid,
        When::Zoned(wall("2026-11-23T09:00"), "Europe/Berlin".into())
    );
    let mut f = fields("Planning (later)");
    f.start = wall("2026-11-23T14:00");
    f.end = wall("2026-11-23T15:00");
    assert!(write::edit_one(
        &mut cal,
        &rid,
        &f,
        ts("2026-11-14T00:00:00Z")
    ));
    let o = window(&cal);
    assert_eq!(o[1].start, ts("2026-11-23T13:00:00Z"));
    assert_eq!(
        cal.events()[o[1].source].summary.as_deref(),
        Some("Planning (later)")
    );
    // Changing it again replaces the same override.
    f.start = wall("2026-11-23T16:00");
    f.end = wall("2026-11-23T17:00");
    write::edit_one(&mut cal, &rid, &f, ts("2026-11-14T00:00:00Z"));
    assert_eq!(cal.events().len(), 2);
    // Delete the third, then the moved one.
    let third = write::recurrence_id(&cal, window(&cal)[2].key, &berlin).expect("rid");
    assert!(write::delete_one(
        &mut cal,
        &third,
        ts("2026-11-14T00:00:00Z")
    ));
    assert!(write::delete_one(
        &mut cal,
        &rid,
        ts("2026-11-14T00:00:00Z")
    ));
    let o = window(&cal);
    assert_eq!(
        o.iter().map(|o| o.start).collect::<Vec<_>>(),
        [ts("2026-11-16T08:00:00Z"), ts("2026-12-07T08:00:00Z")]
    );
    assert_eq!(cal.events().len(), 1);
}

#[test]
fn all_day_events_are_written_as_dates_with_an_exclusive_end() {
    let mut f = fields("Holiday");
    f.all_day = true;
    f.start = wall("2026-12-24T00:00");
    f.end = wall("2026-12-24T00:00");
    f.repeat = Repeat::Yearly;
    let cal = write::new_object(&f, "g@example.org", ts("2026-11-13T12:00:00Z"));
    let e = &cal.events()[0];
    assert_eq!(e.start, Some(When::Date(jiff::civil::date(2026, 12, 24))));
    assert_eq!(e.end, Some(When::Date(jiff::civil::date(2026, 12, 25))));
    assert!(cal.time_zones().is_empty());
}

#[test]
fn the_repeat_preset_of_a_rule_is_found_or_kept() {
    let monday = jiff::civil::date(2026, 11, 16);
    assert_eq!(Repeat::of(None, monday), Repeat::None);
    assert_eq!(Repeat::of(Some("FREQ=DAILY"), monday), Repeat::Daily);
    assert_eq!(
        Repeat::of(Some("FREQ=WEEKLY;BYDAY=MO;INTERVAL=1"), monday),
        Repeat::Weekly
    );
    assert_eq!(
        Repeat::of(Some("FREQ=WEEKLY;BYDAY=TU"), monday),
        Repeat::Keep
    );
    assert_eq!(
        Repeat::of(Some("FREQ=MONTHLY;BYMONTHDAY=16"), monday),
        Repeat::Monthly
    );
    assert_eq!(Repeat::of(Some("FREQ=YEARLY"), monday), Repeat::Yearly);
    assert_eq!(Repeat::of(Some("FREQ=DAILY;COUNT=5"), monday), Repeat::Keep);
}
