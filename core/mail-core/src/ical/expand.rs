// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The occurrences of a calendar object in a window of time: the master event's repeats (in its
//! own zone), plus `RDATE`s, minus `EXDATE`s, with `RECURRENCE-ID` overrides in place of the
//! occurrence they change. All-day events and floating times are in the viewer's zone.

use super::rrule::{self, Until};
use super::zone::{wall_to_instant, Zones};
use super::{Calendar, Event, When};
use jiff::civil::DateTime;
use jiff::tz::TimeZone;
use jiff::{SignedDuration, Timestamp};

/// The most occurrences of one object returned for one window.
pub const MAX_PER_WINDOW: usize = 5_000;
/// A margin around the window when it is turned into wall times (a zone is at most 26 hours from UTC).
const WALL_MARGIN_HOURS: i64 = 30;

/// One occurrence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Occurrence {
    /// When it starts.
    pub start: Timestamp,
    /// When it ends (equal to the start for an event with no length).
    pub end: Timestamp,
    /// A whole-day event.
    pub all_day: bool,
    /// The occurrence's original start (its `RECURRENCE-ID`): what a change to this one occurrence names.
    pub key: Timestamp,
    /// Which event of [`Calendar::events`] gives its title, place and the rest (0 is the master).
    pub source: usize,
    /// False when the event's zone was not found and its wall time was read as UTC.
    pub zone_known: bool,
}

/// The occurrences of one object in a window.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Expansion {
    /// In order of start.
    pub occurrences: Vec<Occurrence>,
    /// The event repeats in a way Croft does not expand: only its first occurrence is given.
    pub repeats_unsupported: bool,
    /// More than [`MAX_PER_WINDOW`] fell in the window; the rest were left out.
    pub truncated: bool,
}

/// What an object covers, for the query index: its first start, and the end of its last
/// occurrence (`None` when it repeats for ever).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    /// The first start.
    pub first: Timestamp,
    /// The last end, or `None` for an open repeat.
    pub last: Option<Timestamp>,
}

struct Master<'a> {
    event: &'a Event,
    start: &'a When,
    all_day: bool,
    /// The zone the repeats are made in.
    tz: TimeZone,
    /// The length: whole days for an all-day event, else seconds.
    length: Length,
    zone_known: bool,
}

#[derive(Clone, Copy)]
enum Length {
    Days(i64),
    Seconds(i64),
}

impl<'a> Master<'a> {
    fn of(event: &'a Event, zones: &Zones, local: &TimeZone) -> Option<Master<'a>> {
        let start = event.start.as_ref()?;
        let all_day = start.is_date();
        let tz = zones.zone_of(start, local);
        let resolved = zones.resolve(start, local);
        let length = if all_day {
            let days = match (&event.end, event.duration) {
                (Some(When::Date(end)), _) => {
                    let s = start.civil().date();
                    i64::from(s.until(*end).map(|sp| sp.get_days()).unwrap_or(1))
                }
                (_, Some(secs)) => secs / 86_400,
                _ => 1,
            };
            Length::Days(days.clamp(1, 3_660))
        } else {
            let secs = match (&event.end, event.duration) {
                (Some(end), _) => {
                    let e = zones.resolve(end, local).at;
                    e.as_second() - resolved.at.as_second()
                }
                (_, Some(secs)) => secs,
                _ => 0,
            };
            Length::Seconds(secs.clamp(0, 366 * 86_400))
        };
        Some(Master {
            event,
            start,
            all_day,
            tz,
            length,
            zone_known: resolved.zone_known,
        })
    }

    fn wall_start(&self) -> DateTime {
        match self.start {
            When::Utc(t) => self.tz.to_datetime(*t),
            other => other.civil(),
        }
    }

    fn end_of(&self, wall: DateTime, start: Timestamp) -> Timestamp {
        match self.length {
            Length::Days(d) => wall
                .date()
                .checked_add(jiff::Span::new().days(d))
                .map(|e| wall_to_instant(e.to_datetime(jiff::civil::Time::midnight()), &self.tz))
                .unwrap_or(start),
            Length::Seconds(s) => start
                .checked_add(SignedDuration::from_secs(s))
                .unwrap_or(start),
        }
    }

    fn length_seconds(&self) -> i64 {
        match self.length {
            Length::Days(d) => d * 86_400 + 3_600,
            Length::Seconds(s) => s,
        }
    }

    fn past_until(&self, until: Option<Until>) -> impl Fn(DateTime) -> bool + '_ {
        move |wall: DateTime| match until {
            None => false,
            Some(Until::Date(d)) => wall.date() > d,
            Some(Until::Wall(w)) => wall > w,
            Some(Until::Instant(t)) => wall_to_instant(wall, &self.tz) > t,
        }
    }
}

/// The occurrences of `cal` that overlap `[from, to)`, seen from `local`.
pub fn occurrences(cal: &Calendar, local: &TimeZone, from: Timestamp, to: Timestamp) -> Expansion {
    let events = cal.events();
    let zones = Zones::of(cal);
    let mut out = Expansion::default();
    let Some(master) = events
        .iter()
        .position(|e| e.recurrence_id.is_none())
        .and_then(|i| Master::of(&events[i], &zones, local).map(|m| (i, m)))
    else {
        // Only overrides (an invitation to one occurrence): each is shown on its own.
        for (i, e) in events.iter().enumerate() {
            if let Some(m) = Master::of(e, &zones, local) {
                let start = zones.resolve(m.start, local).at;
                push_if_overlaps(&mut out, single(&m, start, i), from, to);
            }
        }
        out.occurrences.sort_by_key(|o| o.start);
        return out;
    };
    let (mi, m) = master;
    let margin = SignedDuration::from_hours(WALL_MARGIN_HOURS);
    let length = SignedDuration::from_secs(m.length_seconds());
    let wall_from = m.tz.to_datetime(
        from.checked_sub(length)
            .and_then(|t| t.checked_sub(margin))
            .unwrap_or(from),
    );
    let wall_to = m.tz.to_datetime(to.checked_add(margin).unwrap_or(to));

    let rule = m.event.rrule.as_deref().map(rrule::parse);
    let mut walls = match &rule {
        Some(Some(r)) => {
            out.repeats_unsupported = !r.supported;
            let past = m.past_until(r.until);
            let (w, truncated) = rrule::expand(
                r,
                m.wall_start(),
                wall_from,
                wall_to,
                MAX_PER_WINDOW * 2,
                &past,
            );
            out.truncated |= truncated;
            w
        }
        Some(None) => {
            out.repeats_unsupported = true;
            in_window(m.wall_start(), wall_from, wall_to)
        }
        None => in_window(m.wall_start(), wall_from, wall_to),
    };
    if !m.event.rdates.is_empty() {
        for r in &m.event.rdates {
            let wall = match r {
                When::Utc(t) => m.tz.to_datetime(*t),
                When::Zoned(dt, tzid) => m.tz.to_datetime(zones.wall_in(*dt, tzid).at),
                other => other.civil(),
            };
            if wall >= wall_from && wall < wall_to && !walls.contains(&wall) {
                walls.push(wall);
            }
        }
    }
    let excluded: Vec<Timestamp> = m
        .event
        .exdates
        .iter()
        .map(|x| key_of(x, &m, &zones, local))
        .collect();
    let mut occ: Vec<Occurrence> = walls
        .into_iter()
        .map(|w| {
            let start = wall_to_instant(w, &m.tz);
            Occurrence {
                start,
                end: m.end_of(w, start),
                all_day: m.all_day,
                key: start,
                source: mi,
                zone_known: m.zone_known,
            }
        })
        .filter(|o| !excluded.contains(&o.key))
        .collect();
    // Overrides take the place of the occurrence they name; a cancelled one only removes it.
    for (i, e) in events.iter().enumerate() {
        let Some(rid) = &e.recurrence_id else {
            continue;
        };
        let key = key_of(rid, &m, &zones, local);
        occ.retain(|o| o.key != key);
        if e.status.as_deref() == Some("CANCELLED") || excluded.contains(&key) {
            continue;
        }
        if let Some(om) = Master::of(e, &zones, local) {
            let start = zones.resolve(om.start, local).at;
            let mut o = single(&om, start, i);
            o.key = key;
            occ.push(o);
        }
    }
    for o in occ {
        push_if_overlaps(&mut out, o, from, to);
    }
    out.occurrences.sort_by_key(|o| (o.start, o.key));
    if out.occurrences.len() > MAX_PER_WINDOW {
        out.occurrences.truncate(MAX_PER_WINDOW);
        out.truncated = true;
    }
    out
}

/// The span of `cal` for the query index.
pub fn span(cal: &Calendar, local: &TimeZone) -> Option<Span> {
    let events = cal.events();
    let zones = Zones::of(cal);
    let mut first: Option<Timestamp> = None;
    let mut last: Option<Timestamp> = None;
    let mut open = false;
    for e in &events {
        let Some(m) = Master::of(e, &zones, local) else {
            continue;
        };
        let start = zones.resolve(m.start, local).at;
        first = Some(first.map_or(start, |f| f.min(start)));
        let mut end = m.end_of(m.wall_start(), start);
        if e.recurrence_id.is_none() {
            if let Some(r) = e
                .rrule
                .as_deref()
                .and_then(rrule::parse)
                .filter(|r| r.supported)
            {
                if r.count.is_none() && r.until.is_none() {
                    open = true;
                } else {
                    let until_wall = DateTime::constant(rrule::LAST_YEAR, 12, 31, 23, 59, 59, 0);
                    let past = m.past_until(r.until);
                    let (walls, _) = rrule::expand(
                        &r,
                        m.wall_start(),
                        m.wall_start(),
                        until_wall,
                        usize::MAX,
                        &past,
                    );
                    if let Some(w) = walls.last() {
                        end = m.end_of(*w, wall_to_instant(*w, &m.tz));
                    }
                }
            }
            for r in &e.rdates {
                let at = zones.resolve(r, local).at;
                let length = SignedDuration::from_secs(m.length_seconds());
                end = end.max(at.checked_add(length).unwrap_or(at));
            }
        }
        last = Some(last.map_or(end, |l| l.max(end)));
    }
    first.map(|first| Span {
        first,
        last: if open { None } else { last },
    })
}

fn in_window(wall: DateTime, from: DateTime, to: DateTime) -> Vec<DateTime> {
    if wall >= from && wall < to {
        vec![wall]
    } else {
        vec![]
    }
}

/// The instant an `EXDATE` or `RECURRENCE-ID` names, matched like the master's start: a date is
/// that day at the master's start time.
fn key_of(w: &When, m: &Master<'_>, zones: &Zones, local: &TimeZone) -> Timestamp {
    match w {
        When::Date(d) if !m.all_day => wall_to_instant(d.to_datetime(m.wall_start().time()), &m.tz),
        When::Floating(dt) => wall_to_instant(*dt, &m.tz),
        other => zones.resolve(other, local).at,
    }
}

fn single(m: &Master<'_>, start: Timestamp, source: usize) -> Occurrence {
    Occurrence {
        start,
        end: m.end_of(m.wall_start(), start),
        all_day: m.all_day,
        key: start,
        source,
        zone_known: m.zone_known,
    }
}

fn push_if_overlaps(out: &mut Expansion, o: Occurrence, from: Timestamp, to: Timestamp) {
    let overlaps = if o.end > o.start {
        o.start < to && o.end > from
    } else {
        o.start >= from && o.start < to
    };
    if overlaps {
        out.occurrences.push(o);
    }
}
