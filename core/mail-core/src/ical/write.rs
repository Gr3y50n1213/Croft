// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Writing events: a new object, a change to the whole series, a change to one occurrence (a
//! `RECURRENCE-ID` override) and the deletion of one occurrence (an `EXDATE`). Only the properties
//! S38 edits are rewritten; every other line, alarm and component is kept as it was.

use super::escape;
use super::zone::{iana, wall_to_instant};
use super::{make, parse_duration, read_when, Calendar, Component, Line, When};
use jiff::civil::{Date, DateTime, Weekday};
use jiff::tz::TimeZone;
use jiff::Timestamp;

/// The product name written in new objects.
pub const PRODID: &str = "-//Croft//Croft//EN";

/// How an event repeats, as S38 offers it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Repeat {
    /// Once.
    None,
    /// Every day.
    Daily,
    /// Every week on the start's weekday.
    Weekly,
    /// Every month on the start's day.
    Monthly,
    /// Every year on the start's date.
    Yearly,
    /// A rule Croft did not write: kept as it is (shown read-only).
    Keep,
}

impl Repeat {
    /// The preset an existing rule is, or [`Repeat::Keep`] for any other rule.
    pub fn of(rule: Option<&str>, start: Date) -> Repeat {
        let Some(rule) = rule else {
            return Repeat::None;
        };
        let mut parts: Vec<String> = rule
            .split(';')
            .filter(|p| !p.trim().is_empty())
            .map(|p| p.trim().to_ascii_uppercase())
            .filter(|p| p != "INTERVAL=1" && p != "WKST=MO")
            .collect();
        parts.sort();
        let day = weekday_code(start.weekday());
        let is = |want: &[String]| {
            let mut w = want.to_vec();
            w.sort();
            w == parts
        };
        if is(&["FREQ=DAILY".into()]) {
            Repeat::Daily
        } else if is(&["FREQ=WEEKLY".into()]) || is(&["FREQ=WEEKLY".into(), format!("BYDAY={day}")])
        {
            Repeat::Weekly
        } else if is(&["FREQ=MONTHLY".into()])
            || is(&["FREQ=MONTHLY".into(), format!("BYMONTHDAY={}", start.day())])
        {
            Repeat::Monthly
        } else if is(&["FREQ=YEARLY".into()]) {
            Repeat::Yearly
        } else {
            Repeat::Keep
        }
    }

    fn rule(self, start: Date) -> Option<String> {
        match self {
            Repeat::None | Repeat::Keep => None,
            Repeat::Daily => Some("FREQ=DAILY".into()),
            Repeat::Weekly => Some(format!(
                "FREQ=WEEKLY;BYDAY={}",
                weekday_code(start.weekday())
            )),
            Repeat::Monthly => Some(format!("FREQ=MONTHLY;BYMONTHDAY={}", start.day())),
            Repeat::Yearly => Some("FREQ=YEARLY".into()),
        }
    }
}

/// What S38 edits.
#[derive(Clone, PartialEq, Eq)]
pub struct EventFields {
    /// The title.
    pub title: String,
    /// A whole-day event: [`EventFields::start`] and [`EventFields::end`] are dates at midnight,
    /// the end exclusive (a one-day event ends the next day).
    pub all_day: bool,
    /// The start, a wall time in [`EventFields::zone`].
    pub start: DateTime,
    /// The end, a wall time in [`EventFields::zone`].
    pub end: DateTime,
    /// The IANA zone of a timed event. `None` writes floating times (and is what all-day events use).
    pub zone: Option<String>,
    /// The place.
    pub location: String,
    /// The notes.
    pub notes: String,
    /// How it repeats.
    pub repeat: Repeat,
    /// Reminders, in minutes before the start.
    pub reminders: Vec<i64>,
}

impl std::fmt::Debug for EventFields {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EventFields")
            .field("all_day", &self.all_day)
            .field("repeat", &self.repeat)
            .field("reminders", &self.reminders.len())
            .finish_non_exhaustive()
    }
}

fn weekday_code(w: Weekday) -> &'static str {
    match w {
        Weekday::Monday => "MO",
        Weekday::Tuesday => "TU",
        Weekday::Wednesday => "WE",
        Weekday::Thursday => "TH",
        Weekday::Friday => "FR",
        Weekday::Saturday => "SA",
        Weekday::Sunday => "SU",
    }
}

fn date_text(d: Date) -> String {
    format!("{:04}{:02}{:02}", d.year(), d.month(), d.day())
}

fn datetime_text(dt: DateTime) -> String {
    format!(
        "{}T{:02}{:02}{:02}",
        date_text(dt.date()),
        dt.hour(),
        dt.minute(),
        dt.second()
    )
}

fn utc_text(t: Timestamp) -> String {
    format!("{}Z", datetime_text(t.to_zoned(TimeZone::UTC).datetime()))
}

/// A date or time property line.
pub(crate) fn when_line(name: &str, w: &When) -> Line {
    match w {
        When::Date(d) => make(name, &[("VALUE", "DATE")], &date_text(*d)),
        When::Utc(t) => make(name, &[], &utc_text(*t)),
        When::Zoned(dt, tz) => make(name, &[("TZID", tz)], &datetime_text(*dt)),
        When::Floating(dt) => make(name, &[], &datetime_text(*dt)),
    }
}

fn start_end(fields: &EventFields) -> (When, When) {
    if fields.all_day {
        let s = fields.start.date();
        let e = fields.end.date().max(s.tomorrow().unwrap_or(s));
        (When::Date(s), When::Date(e))
    } else {
        let end = fields.end.max(fields.start);
        match fields.zone.as_deref().filter(|z| iana(z).is_some()) {
            Some(z) => (
                When::Zoned(fields.start, z.to_owned()),
                When::Zoned(end, z.to_owned()),
            ),
            None => (When::Floating(fields.start), When::Floating(end)),
        }
    }
}

fn text_line(c: &mut Component, name: &str, text: &str) {
    let t = text.trim();
    let new = if t.is_empty() {
        vec![]
    } else {
        vec![make(name, &[], &escape(t))]
    };
    c.set_lines(name, new);
}

fn alarm(minutes_before: i64) -> Component {
    let m = minutes_before.clamp(-40_320, 40_320);
    let trigger = if m >= 0 {
        format!("-PT{m}M")
    } else {
        format!("PT{}M", -m)
    };
    Component {
        kind: "VALARM".into(),
        lines: vec![
            make("ACTION", &[], "DISPLAY"),
            make("DESCRIPTION", &[], "Reminder"),
            make("TRIGGER", &[], &trigger),
        ],
        children: vec![],
    }
}

/// Replaces the alarms Croft reads (relative to the start) with `reminders`; others stay.
fn set_reminders(c: &mut Component, reminders: &[i64]) {
    c.children.retain(|a| {
        if a.kind != "VALARM" {
            return true;
        }
        let Some(t) = a.line("TRIGGER") else {
            return true;
        };
        let absolute = t
            .param("VALUE")
            .is_some_and(|v| v.eq_ignore_ascii_case("DATE-TIME"));
        let from_end = t
            .param("RELATED")
            .is_some_and(|r| r.eq_ignore_ascii_case("END"));
        absolute || from_end || parse_duration(&t.value).is_none()
    });
    let mut r = reminders.to_vec();
    r.sort_unstable();
    r.dedup();
    for m in r.into_iter().take(10) {
        c.children.push(alarm(m));
    }
}

/// Writes the edited properties on `c`.
fn write_fields(c: &mut Component, fields: &EventFields, now: Timestamp, with_rule: bool) {
    let (start, end) = start_end(fields);
    text_line(c, "SUMMARY", &fields.title);
    c.set_lines("DTSTART", vec![when_line("DTSTART", &start)]);
    c.set_lines("DURATION", vec![]);
    c.set_lines("DTEND", vec![when_line("DTEND", &end)]);
    text_line(c, "LOCATION", &fields.location);
    text_line(c, "DESCRIPTION", &fields.notes);
    if with_rule && fields.repeat != Repeat::Keep {
        let rule = fields.repeat.rule(fields.start.date());
        c.set_lines(
            "RRULE",
            rule.map(|r| vec![make("RRULE", &[], &r)])
                .unwrap_or_default(),
        );
    }
    set_reminders(c, &fields.reminders);
    let sequence = c
        .line("SEQUENCE")
        .and_then(|l| l.value.trim().parse::<u32>().ok())
        .map_or(0, |s| s.saturating_add(1));
    c.set_lines(
        "SEQUENCE",
        vec![make("SEQUENCE", &[], &sequence.to_string())],
    );
    c.set_lines("DTSTAMP", vec![make("DTSTAMP", &[], &utc_text(now))]);
    c.set_lines(
        "LAST-MODIFIED",
        vec![make("LAST-MODIFIED", &[], &utc_text(now))],
    );
}

/// Adds a `VTIMEZONE` for `zone` when the object has none for it (RFC 5545 asks for one per `TZID`).
fn ensure_vtimezone(cal: &mut Calendar, zone: &str, around: DateTime) {
    let present =
        cal.root.children.iter().any(|c| {
            c.kind == "VTIMEZONE" && c.line("TZID").is_some_and(|l| l.value.trim() == zone)
        });
    if present {
        return;
    }
    let Some(tz) = iana(zone) else {
        return;
    };
    let at = wall_to_instant(around, &tz);
    let at_index = cal
        .root
        .children
        .iter()
        .position(|c| c.kind == "VEVENT")
        .unwrap_or(cal.root.children.len());
    cal.root.children.insert(at_index, vtimezone(zone, &tz, at));
}

/// A `VTIMEZONE` for `tz` near `at`: its standard and daylight parts with yearly rules, from the
/// transitions of the year around it; one standard part for a zone without daylight saving.
pub fn vtimezone(name: &str, tz: &TimeZone, at: Timestamp) -> Component {
    let year_ago = at
        .checked_sub(jiff::SignedDuration::from_hours(24 * 366))
        .unwrap_or(at);
    let transitions: Vec<_> = tz.following(year_ago).take(4).collect();
    let mut parts: Vec<Component> = Vec::new();
    for want_dst in [true, false] {
        let Some(t) = transitions
            .iter()
            .rev()
            .find(|t| t.dst().is_dst() == want_dst)
        else {
            continue;
        };
        let before = tz.to_offset(
            t.timestamp()
                .checked_sub(jiff::SignedDuration::from_secs(1))
                .unwrap_or(t.timestamp()),
        );
        let after = t.offset();
        let wall = t.timestamp().to_zoned(TimeZone::fixed(before)).datetime();
        let d = wall.date();
        let week = if i32::from(d.day()) + 7 > i32::from(d.days_in_month()) {
            -1
        } else {
            (i32::from(d.day()) - 1) / 7 + 1
        };
        parts.push(Component {
            kind: if want_dst {
                "DAYLIGHT".into()
            } else {
                "STANDARD".into()
            },
            lines: vec![
                make("DTSTART", &[], &datetime_text(wall)),
                make("TZOFFSETFROM", &[], &offset_text(before.seconds())),
                make("TZOFFSETTO", &[], &offset_text(after.seconds())),
                make("TZNAME", &[], &escape(t.abbreviation())),
                make(
                    "RRULE",
                    &[],
                    &format!(
                        "FREQ=YEARLY;BYMONTH={};BYDAY={week}{}",
                        d.month(),
                        weekday_code(d.weekday())
                    ),
                ),
            ],
            children: vec![],
        });
    }
    if parts.is_empty() {
        let off = tz.to_offset(at).seconds();
        parts.push(Component {
            kind: "STANDARD".into(),
            lines: vec![
                make("DTSTART", &[], "19700101T000000"),
                make("TZOFFSETFROM", &[], &offset_text(off)),
                make("TZOFFSETTO", &[], &offset_text(off)),
            ],
            children: vec![],
        });
    }
    Component {
        kind: "VTIMEZONE".into(),
        lines: vec![make("TZID", &[], name)],
        children: parts,
    }
}

fn offset_text(secs: i32) -> String {
    let sign = if secs < 0 { '-' } else { '+' };
    let a = secs.abs();
    if a % 60 == 0 {
        format!("{sign}{:02}{:02}", a / 3600, (a / 60) % 60)
    } else {
        format!("{sign}{:02}{:02}{:02}", a / 3600, (a / 60) % 60, a % 60)
    }
}

/// A new calendar object with one event.
pub fn new_object(fields: &EventFields, uid: &str, now: Timestamp) -> Calendar {
    let mut event = Component {
        kind: "VEVENT".into(),
        lines: vec![make("UID", &[], uid), make("CREATED", &[], &utc_text(now))],
        children: vec![],
    };
    write_fields(&mut event, fields, now, true);
    let mut cal = Calendar {
        root: Component {
            kind: "VCALENDAR".into(),
            lines: vec![make("VERSION", &[], "2.0"), make("PRODID", &[], PRODID)],
            children: vec![event],
        },
    };
    if let (false, Some(z)) = (fields.all_day, fields.zone.as_deref()) {
        ensure_vtimezone(&mut cal, z, fields.start);
    }
    cal
}

/// Changes every occurrence: the master's properties. When its start or its repeat changes, the
/// overrides and excluded dates (which name the old occurrences) are dropped. False when the object
/// has no master event.
pub fn edit_all(cal: &mut Calendar, fields: &EventFields, now: Timestamp) -> bool {
    let Some(master) = cal.master_mut() else {
        return false;
    };
    let old_start = master.line("DTSTART").and_then(read_when);
    let old_rule = master.line("RRULE").map(|l| l.value.trim().to_owned());
    write_fields(master, fields, now, true);
    let new_start = master.line("DTSTART").and_then(read_when);
    let new_rule = master.line("RRULE").map(|l| l.value.trim().to_owned());
    if old_start != new_start || old_rule != new_rule {
        master.set_lines("EXDATE", vec![]);
        master.set_lines("RDATE", vec![]);
        cal.root
            .children
            .retain(|c| !(c.kind == "VEVENT" && c.line("RECURRENCE-ID").is_some()));
    }
    if let (false, Some(z)) = (fields.all_day, fields.zone.as_deref()) {
        ensure_vtimezone(cal, z, fields.start);
    }
    true
}

/// The `RECURRENCE-ID` for the occurrence that starts (originally) at `key`, in the form of the
/// master's `DTSTART` (a date, UTC, the same `TZID`, or floating in `local`).
pub fn recurrence_id(cal: &Calendar, key: Timestamp, local: &TimeZone) -> Option<When> {
    let master = cal
        .root
        .children
        .iter()
        .find(|c| c.kind == "VEVENT" && c.line("RECURRENCE-ID").is_none())?;
    let start = master.line("DTSTART").and_then(read_when)?;
    let zones = super::zone::Zones::of(cal);
    Some(match start {
        When::Date(_) => When::Date(local.to_datetime(key).date()),
        When::Utc(_) => When::Utc(key),
        When::Zoned(_, tzid) => {
            let tz = zones.zone(&tzid).unwrap_or(TimeZone::UTC);
            When::Zoned(tz.to_datetime(key), tzid)
        }
        When::Floating(_) => When::Floating(local.to_datetime(key)),
    })
}

/// Changes one occurrence (`rid`, from [`recurrence_id`]): an override with the same `UID`, made
/// from the master (its other properties and attendees kept) or the override already there.
pub fn edit_one(cal: &mut Calendar, rid: &When, fields: &EventFields, now: Timestamp) -> bool {
    let rid_line = when_line("RECURRENCE-ID", rid);
    let existing = cal.root.children.iter().position(|c| {
        c.kind == "VEVENT" && c.line("RECURRENCE-ID").and_then(read_when).as_ref() == Some(rid)
    });
    let mut over = match existing {
        Some(i) => cal.root.children.remove(i),
        None => {
            let Some(master) = cal.master_mut() else {
                return false;
            };
            let mut copy = master.clone();
            for name in ["RRULE", "RDATE", "EXDATE", "RECURRENCE-ID"] {
                copy.set_lines(name, vec![]);
            }
            copy
        }
    };
    over.set_lines("RECURRENCE-ID", vec![rid_line]);
    let mut f = fields.clone();
    f.repeat = Repeat::None;
    write_fields(&mut over, &f, now, false);
    over.set_lines("RRULE", vec![]);
    cal.root.children.push(over);
    if let (false, Some(z)) = (fields.all_day, fields.zone.as_deref()) {
        ensure_vtimezone(cal, z, fields.start);
    }
    true
}

/// Deletes one occurrence: an `EXDATE` on the master, and its override, if any, removed.
pub fn delete_one(cal: &mut Calendar, rid: &When, now: Timestamp) -> bool {
    cal.root.children.retain(|c| {
        !(c.kind == "VEVENT" && c.line("RECURRENCE-ID").and_then(read_when).as_ref() == Some(rid))
    });
    let Some(master) = cal.master_mut() else {
        return false;
    };
    let mut lines: Vec<Line> = master.lines_named("EXDATE").cloned().collect();
    lines.push(when_line("EXDATE", rid));
    master.set_lines("EXDATE", lines);
    master.set_lines("DTSTAMP", vec![make("DTSTAMP", &[], &utc_text(now))]);
    true
}
