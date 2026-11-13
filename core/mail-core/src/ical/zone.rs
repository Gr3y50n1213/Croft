// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Time zones of a calendar object. A `TZID` is looked up as an IANA name in the time zone
//! database bundled with Croft (never the device's, so every device agrees), also with a common
//! prefix removed (`/mozilla.org/20050126_1/America/New_York`). Failing that, the object's own
//! `VTIMEZONE` is read: its latest standard and daylight parts and their yearly rules, turned into
//! a POSIX TZ rule. Failing that too, the time is read as UTC and marked "time zone unknown".

use super::{parse_datetime, Calendar, Component, When};
use jiff::civil::DateTime;
use jiff::tz::TimeZone;
use jiff::Timestamp;
use std::collections::HashMap;

/// The time zones a calendar object names, resolved once.
#[derive(Clone, Default)]
pub struct Zones {
    /// `TZID` to its zone, for the `VTIMEZONE`s that are not IANA names Croft knows.
    custom: HashMap<String, TimeZone>,
}

/// A time as an exact instant, and whether its zone was known.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Resolved {
    /// The instant.
    pub at: Timestamp,
    /// False when the zone was not found and the wall time was read as UTC.
    pub zone_known: bool,
}

impl Zones {
    /// The zones of `cal`'s `VTIMEZONE`s that are not IANA names.
    pub fn of(cal: &Calendar) -> Zones {
        let mut custom = HashMap::new();
        for (tzid, c) in cal.time_zones() {
            if iana(&tzid).is_some() {
                continue;
            }
            if let Some(tz) = from_vtimezone(c) {
                custom.insert(tzid, tz);
            }
        }
        Zones { custom }
    }

    /// The zone called `tzid`, if Croft can find it.
    pub fn zone(&self, tzid: &str) -> Option<TimeZone> {
        iana(tzid).or_else(|| self.custom.get(tzid).cloned())
    }

    /// `wall` in `tzid` as an instant (a time that does not exist moves forward; one that exists
    /// twice takes the first). An unknown zone reads the wall time as UTC.
    pub fn wall_in(&self, wall: DateTime, tzid: &str) -> Resolved {
        match self.zone(tzid) {
            Some(tz) => Resolved {
                at: wall_to_instant(wall, &tz),
                zone_known: true,
            },
            None => Resolved {
                at: wall_to_instant(wall, &TimeZone::UTC),
                zone_known: false,
            },
        }
    }

    /// `when` as an instant: a date is midnight and a floating time is the wall time, both in
    /// `local` (the viewer's zone).
    pub fn resolve(&self, when: &When, local: &TimeZone) -> Resolved {
        match when {
            When::Utc(t) => Resolved {
                at: *t,
                zone_known: true,
            },
            When::Zoned(dt, tzid) => self.wall_in(*dt, tzid),
            When::Date(_) | When::Floating(_) => Resolved {
                at: wall_to_instant(when.civil(), local),
                zone_known: true,
            },
        }
    }

    /// The zone `when` is written in (the viewer's for dates and floating times, UTC for exact
    /// times), for expanding repeats in it.
    pub fn zone_of(&self, when: &When, local: &TimeZone) -> TimeZone {
        match when {
            When::Utc(_) => TimeZone::UTC,
            When::Zoned(_, tzid) => self.zone(tzid).unwrap_or(TimeZone::UTC),
            When::Date(_) | When::Floating(_) => local.clone(),
        }
    }
}

/// A wall time in `tz` as an instant: forward through a gap, the earlier of a fold. Out of range
/// (which a parsed date cannot be) gives the Unix epoch.
pub fn wall_to_instant(wall: DateTime, tz: &TimeZone) -> Timestamp {
    tz.to_ambiguous_timestamp(wall)
        .compatible()
        .unwrap_or(Timestamp::UNIX_EPOCH)
}

/// `tzid` as an IANA name in the bundled database, whole or after a prefix
/// (`/mozilla.org/20050126_1/America/New_York` gives `America/New_York`).
pub fn iana(tzid: &str) -> Option<TimeZone> {
    let t = tzid.trim().trim_matches('"');
    if t.is_empty() || t.len() > 200 {
        return None;
    }
    if let Ok(tz) = TimeZone::get(t) {
        return Some(tz);
    }
    // Try every suffix that starts after a '/', longest first.
    let mut rest = t;
    while let Some((_, after)) = rest.split_once('/') {
        if !after.is_empty() {
            if let Ok(tz) = TimeZone::get(after) {
                return Some(tz);
            }
        }
        rest = after;
    }
    None
}

/// One part (`STANDARD` or `DAYLIGHT`) of a `VTIMEZONE`.
struct Part {
    daylight: bool,
    start: DateTime,
    offset_to: i32,
    /// The yearly rule as POSIX `Mm.w.d`, when it has one Croft can write.
    rule: Option<String>,
    /// The rule has an `UNTIL`: it ended.
    ended: bool,
}

/// A `VTIMEZONE` as a zone: its latest standard and daylight parts that still run, as a POSIX TZ
/// rule; a standard part alone is a fixed offset.
fn from_vtimezone(c: &Component) -> Option<TimeZone> {
    let parts: Vec<Part> = c
        .children
        .iter()
        .filter(|p| p.kind == "STANDARD" || p.kind == "DAYLIGHT")
        .filter_map(read_part)
        .collect();
    let latest = |daylight: bool| {
        parts
            .iter()
            .filter(|p| p.daylight == daylight && !p.ended)
            .max_by_key(|p| p.start)
            .or_else(|| {
                parts
                    .iter()
                    .filter(|p| p.daylight == daylight)
                    .max_by_key(|p| p.start)
            })
    };
    let std = latest(false)?;
    let posix = match latest(true) {
        Some(dst) if !dst.ended && !std.ended && dst.rule.is_some() && std.rule.is_some() => {
            format!(
                "<STD>{}<DST>{},{}/{},{}/{}",
                posix_offset(std.offset_to),
                posix_offset(dst.offset_to),
                dst.rule.as_deref().unwrap_or_default(),
                posix_time(dst.start),
                std.rule.as_deref().unwrap_or_default(),
                posix_time(std.start),
            )
        }
        _ => format!("<STD>{}", posix_offset(std.offset_to)),
    };
    TimeZone::posix(&posix).ok()
}

fn read_part(p: &Component) -> Option<Part> {
    let start = parse_datetime(p.line("DTSTART")?.value.trim())?;
    let offset_to = parse_offset(&p.line("TZOFFSETTO")?.value)?;
    let (rule, ended) = match p.line("RRULE") {
        Some(l) => {
            let r = l.value.to_ascii_uppercase();
            (
                yearly_rule(&r, start),
                r.contains("UNTIL=") || r.contains("COUNT="),
            )
        }
        None => (None, false),
    };
    Some(Part {
        daylight: p.kind == "DAYLIGHT",
        start,
        offset_to,
        rule,
        ended,
    })
}

/// `+HHMM[SS]` or `-HHMM[SS]` in seconds east of UTC.
fn parse_offset(v: &str) -> Option<i32> {
    let v = v.trim();
    let (sign, digits) = match v.as_bytes().first()? {
        b'+' => (1, &v[1..]),
        b'-' => (-1, &v[1..]),
        _ => return None,
    };
    if !(digits.len() == 4 || digits.len() == 6) || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let h: i32 = digits[0..2].parse().ok()?;
    let m: i32 = digits[2..4].parse().ok()?;
    let s: i32 = if digits.len() == 6 {
        digits[4..6].parse().ok()?
    } else {
        0
    };
    if h > 25 || m > 59 || s > 59 {
        return None;
    }
    Some(sign * (h * 3600 + m * 60 + s))
}

/// A yearly `BYMONTH` and `BYDAY` rule (`BYDAY=2SU`, `-1SU`, or `SU` with `BYMONTHDAY=8,...,14`) as
/// POSIX `Mm.w.d`. `None` for anything else.
fn yearly_rule(rule: &str, start: DateTime) -> Option<String> {
    let mut freq = None;
    let mut month = None;
    let mut byday = None;
    let mut monthdays: Vec<i32> = Vec::new();
    for part in rule.split(';') {
        let (k, v) = part.split_once('=')?;
        match k {
            "FREQ" => freq = Some(v),
            "BYMONTH" => month = v.parse::<i32>().ok(),
            "BYDAY" => byday = Some(v),
            "BYMONTHDAY" => monthdays = v.split(',').filter_map(|d| d.parse().ok()).collect(),
            "UNTIL" | "COUNT" | "INTERVAL" | "WKST" => {}
            _ => return None,
        }
    }
    if freq != Some("YEARLY") {
        return None;
    }
    let month = month.unwrap_or(i32::from(start.month()));
    if !(1..=12).contains(&month) {
        return None;
    }
    let byday = byday?;
    let split = byday.len().checked_sub(2)?;
    let day_name = byday.get(split..)?;
    let ordinal = byday.get(..split)?;
    let d = ["SU", "MO", "TU", "WE", "TH", "FR", "SA"]
        .iter()
        .position(|x| *x == day_name)?;
    let week = if ordinal.is_empty() {
        // `BYDAY=SU;BYMONTHDAY=8,9,...,14`: the week the days fall in.
        let first = *monthdays.iter().min()?;
        if monthdays.len() != 7 {
            return None;
        }
        match first {
            1 | 8 | 15 | 22 => (first - 1) / 7 + 1,
            // The last seven days of a 31-, 30- or 28-day month.
            25 | 24 | 22.. if monthdays.iter().max() >= Some(&28) => 5,
            _ => return None,
        }
    } else {
        match ordinal.trim_start_matches('+').parse::<i32>().ok()? {
            n @ 1..=4 => n,
            -1 | 5 => 5,
            _ => return None,
        }
    };
    Some(format!("M{month}.{week}.{d}"))
}

/// POSIX offsets count west of UTC: UTC+01:00 is `-01:00:00`.
fn posix_offset(east: i32) -> String {
    let west = -east;
    let sign = if west < 0 { "-" } else { "+" };
    let a = west.abs();
    format!("{sign}{:02}:{:02}:{:02}", a / 3600, (a / 60) % 60, a % 60)
}

fn posix_time(t: DateTime) -> String {
    format!("{:02}:{:02}:{:02}", t.hour(), t.minute(), t.second())
}

impl std::fmt::Debug for Zones {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Zones")
            .field("custom", &self.custom.len())
            .finish()
    }
}
