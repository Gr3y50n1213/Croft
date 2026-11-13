// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! `RRULE` (RFC 5545, 3.3.10): read, and expanded into wall times in the event's own zone, so a
//! repeat at 09:00 stays at 09:00 across a change of daylight saving time.
//!
//! Croft expands `FREQ` `DAILY`, `WEEKLY`, `MONTHLY` and `YEARLY` with `INTERVAL`, `COUNT`, `UNTIL`,
//! `BYDAY` (with ordinals in monthly and yearly rules), `BYMONTHDAY`, `BYMONTH`, `BYSETPOS` and
//! `WKST`. A rule with any other part is "not supported": the event then shows only its first
//! occurrence, never a guess. `DTSTART` always counts as the first occurrence.

use super::{parse_date, parse_datetime};
use jiff::civil::{Date, DateTime, Weekday};
use jiff::Timestamp;

/// No occurrence after this year is made.
pub const LAST_YEAR: i16 = 2100;
/// The most periods (days, weeks, months or years) looked at for one rule, whatever the window.
pub const MAX_PERIODS: usize = 100_000;

/// How often.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Freq {
    /// Every day.
    Daily,
    /// Every week.
    Weekly,
    /// Every month.
    Monthly,
    /// Every year.
    Yearly,
}

/// Where the repeats end (`UNTIL`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Until {
    /// The last day (inclusive).
    Date(Date),
    /// The last wall time (inclusive), in the event's zone.
    Wall(DateTime),
    /// The last instant (inclusive).
    Instant(Timestamp),
}

/// A rule as read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rule {
    /// `FREQ`.
    pub freq: Freq,
    /// `INTERVAL` (1 when absent).
    pub interval: i32,
    /// `COUNT`.
    pub count: Option<u32>,
    /// `UNTIL`.
    pub until: Option<Until>,
    /// `BYDAY`: ordinal (0 for none) and day.
    pub by_day: Vec<(i32, Weekday)>,
    /// `BYMONTHDAY` (negative counts from the month's end).
    pub by_month_day: Vec<i32>,
    /// `BYMONTH`.
    pub by_month: Vec<i8>,
    /// `BYSETPOS`.
    pub by_set_pos: Vec<i32>,
    /// `WKST` (Monday when absent).
    pub week_start: Weekday,
    /// False when the rule has parts Croft does not expand.
    pub supported: bool,
}

/// Reads an `RRULE` value. `None` when it has no readable `FREQ` at all.
pub fn parse(text: &str) -> Option<Rule> {
    let mut rule = Rule {
        freq: Freq::Daily,
        interval: 1,
        count: None,
        until: None,
        by_day: Vec::new(),
        by_month_day: Vec::new(),
        by_month: Vec::new(),
        by_set_pos: Vec::new(),
        week_start: Weekday::Monday,
        supported: true,
    };
    let mut freq = None;
    for part in text.trim().split(';').filter(|p| !p.is_empty()).take(32) {
        let Some((k, v)) = part.split_once('=') else {
            rule.supported = false;
            continue;
        };
        let k = k.trim().to_ascii_uppercase();
        let v = v.trim().to_ascii_uppercase();
        match k.as_str() {
            "FREQ" => {
                freq = match v.as_str() {
                    "DAILY" => Some(Freq::Daily),
                    "WEEKLY" => Some(Freq::Weekly),
                    "MONTHLY" => Some(Freq::Monthly),
                    "YEARLY" => Some(Freq::Yearly),
                    "SECONDLY" | "MINUTELY" | "HOURLY" => {
                        rule.supported = false;
                        Some(Freq::Daily)
                    }
                    _ => None,
                }
            }
            "INTERVAL" => match v.parse::<i32>() {
                Ok(n) if (1..=10_000).contains(&n) => rule.interval = n,
                _ => rule.supported = false,
            },
            "COUNT" => match v.parse::<u32>() {
                Ok(n) if n >= 1 => rule.count = Some(n.min(1_000_000)),
                _ => rule.supported = false,
            },
            "UNTIL" => match read_until(&v) {
                Some(u) => rule.until = Some(u),
                None => rule.supported = false,
            },
            "BYDAY" => {
                for d in v.split(',').take(64) {
                    match read_byday(d.trim()) {
                        Some(x) => rule.by_day.push(x),
                        None => rule.supported = false,
                    }
                }
            }
            "BYMONTHDAY" => {
                for d in v.split(',').take(62) {
                    match d.trim().parse::<i32>() {
                        Ok(n) if n != 0 && (-31..=31).contains(&n) => rule.by_month_day.push(n),
                        _ => rule.supported = false,
                    }
                }
            }
            "BYMONTH" => {
                for m in v.split(',').take(12) {
                    match m.trim().parse::<i8>() {
                        Ok(n) if (1..=12).contains(&n) => rule.by_month.push(n),
                        _ => rule.supported = false,
                    }
                }
            }
            "BYSETPOS" => {
                for p in v.split(',').take(64) {
                    match p.trim().parse::<i32>() {
                        Ok(n) if n != 0 && (-366..=366).contains(&n) => rule.by_set_pos.push(n),
                        _ => rule.supported = false,
                    }
                }
            }
            "WKST" => match weekday(&v) {
                Some(w) => rule.week_start = w,
                None => rule.supported = false,
            },
            // Parts Croft does not expand (and anything unknown, such as RSCALE).
            _ => rule.supported = false,
        }
    }
    rule.freq = freq?;
    if rule.count.is_some() && rule.until.is_some() {
        // Both is not allowed; neither can be trusted.
        rule.supported = false;
    }
    if rule.freq != Freq::Monthly
        && rule.freq != Freq::Yearly
        && rule.by_day.iter().any(|(n, _)| *n != 0)
    {
        // Ordinals only make sense in monthly and yearly rules.
        rule.supported = false;
    }
    Some(rule)
}

fn read_until(v: &str) -> Option<Until> {
    if let Some(utc) = v.strip_suffix('Z') {
        let dt = parse_datetime(utc)?;
        return Some(Until::Instant(
            dt.to_zoned(jiff::tz::TimeZone::UTC).ok()?.timestamp(),
        ));
    }
    if v.contains('T') {
        return parse_datetime(v).map(Until::Wall);
    }
    parse_date(v).map(Until::Date)
}

fn weekday(v: &str) -> Option<Weekday> {
    Some(match v {
        "MO" => Weekday::Monday,
        "TU" => Weekday::Tuesday,
        "WE" => Weekday::Wednesday,
        "TH" => Weekday::Thursday,
        "FR" => Weekday::Friday,
        "SA" => Weekday::Saturday,
        "SU" => Weekday::Sunday,
        _ => return None,
    })
}

fn read_byday(v: &str) -> Option<(i32, Weekday)> {
    let split = v.len().checked_sub(2)?;
    let day = weekday(v.get(split..)?)?;
    let n = v.get(..split)?;
    let n = if n.is_empty() {
        0
    } else {
        n.trim_start_matches('+').parse::<i32>().ok()?
    };
    if n != 0 && !(-53..=53).contains(&n) {
        return None;
    }
    Some((n, day))
}

/// The occurrences of `rule` from `start` (always the first), in order, as wall times, that fall
/// in `[from, to)`. `past_until` says whether an occurrence's wall time is after `UNTIL` (the
/// caller knows the zone an instant `UNTIL` is compared in). At most `max` are returned; the flag
/// says whether more were left out.
pub fn expand(
    rule: &Rule,
    start: DateTime,
    from: DateTime,
    to: DateTime,
    max: usize,
    past_until: &dyn Fn(DateTime) -> bool,
) -> (Vec<DateTime>, bool) {
    let mut out = Vec::new();
    let mut emitted: u64 = 0;
    let mut visit = |wall: DateTime, out: &mut Vec<DateTime>| -> Flow {
        if wall.year() > LAST_YEAR || past_until(wall) {
            return Flow::Stop;
        }
        if let Some(c) = rule.count {
            if emitted >= u64::from(c) {
                return Flow::Stop;
            }
        }
        emitted += 1;
        if wall >= to {
            return Flow::Stop;
        }
        if wall >= from {
            if out.len() >= max {
                return Flow::Full;
            }
            out.push(wall);
        }
        Flow::Go
    };
    match visit(start, &mut out) {
        Flow::Go => {}
        Flow::Stop => return (out, false),
        Flow::Full => return (out, true),
    }
    if !rule.supported {
        return (out, false);
    }
    let time = start.time();
    let first = start.date();
    for k in 0..MAX_PERIODS {
        let Some(step) = i64::try_from(k)
            .ok()
            .and_then(|k| k.checked_mul(i64::from(rule.interval)))
        else {
            break;
        };
        let Some(period) = period_start(rule, first, step) else {
            break;
        };
        if period.year() > LAST_YEAR {
            break;
        }
        let mut days = candidates(rule, first, period);
        if !rule.by_set_pos.is_empty() {
            days = set_positions(&days, &rule.by_set_pos);
        }
        for d in days {
            if d <= first {
                continue;
            }
            match visit(d.to_datetime(time), &mut out) {
                Flow::Go => {}
                Flow::Stop => return (out, false),
                Flow::Full => return (out, true),
            }
        }
    }
    (out, false)
}

enum Flow {
    Go,
    Stop,
    Full,
}

/// The first day of period `step` (in units of the frequency) counted from the one holding `first`.
fn period_start(rule: &Rule, first: Date, step: i64) -> Option<Date> {
    match rule.freq {
        Freq::Daily => add_days(first, step),
        Freq::Weekly => add_days(week_start(first, rule.week_start), step.checked_mul(7)?),
        Freq::Monthly => {
            let months = i64::from(first.year()) * 12 + i64::from(first.month()) - 1 + step;
            let y = i16::try_from(months.div_euclid(12)).ok()?;
            let m = i8::try_from(months.rem_euclid(12) + 1).ok()?;
            Date::new(y, m, 1).ok()
        }
        Freq::Yearly => {
            let y = i16::try_from(i64::from(first.year()) + step).ok()?;
            Date::new(y, 1, 1).ok()
        }
    }
}

fn add_days(d: Date, n: i64) -> Option<Date> {
    d.checked_add(jiff::Span::new().try_days(n).ok()?).ok()
}

fn week_start(d: Date, wkst: Weekday) -> Date {
    let back = (d.weekday().to_monday_zero_offset() - wkst.to_monday_zero_offset()).rem_euclid(7);
    add_days(d, -i64::from(back)).unwrap_or(d)
}

/// The days of one period that the rule picks, sorted, without duplicates.
fn candidates(rule: &Rule, first: Date, period: Date) -> Vec<Date> {
    let mut days = match rule.freq {
        Freq::Daily => {
            let d = period;
            let ok = (rule.by_month.is_empty() || rule.by_month.contains(&d.month()))
                && (rule.by_month_day.is_empty()
                    || rule
                        .by_month_day
                        .iter()
                        .any(|&n| month_day(d.year(), d.month(), n) == Some(d)))
                && (rule.by_day.is_empty() || rule.by_day.iter().any(|(_, w)| *w == d.weekday()));
            if ok {
                vec![d]
            } else {
                vec![]
            }
        }
        Freq::Weekly => {
            let wanted: Vec<Weekday> = if rule.by_day.is_empty() {
                vec![first.weekday()]
            } else {
                rule.by_day.iter().map(|(_, w)| *w).collect()
            };
            (0..7)
                .filter_map(|i| add_days(period, i))
                .filter(|d| wanted.contains(&d.weekday()))
                .filter(|d| rule.by_month.is_empty() || rule.by_month.contains(&d.month()))
                .collect()
        }
        Freq::Monthly => {
            if !rule.by_month.is_empty() && !rule.by_month.contains(&period.month()) {
                vec![]
            } else {
                month_days(rule, first, period.year(), period.month())
            }
        }
        Freq::Yearly => {
            let y = period.year();
            if !rule.by_month.is_empty() {
                rule.by_month
                    .iter()
                    .flat_map(|&m| month_days(rule, first, y, m))
                    .collect()
            } else if !rule.by_month_day.is_empty() {
                (1..=12)
                    .flat_map(|m| month_days(rule, first, y, m))
                    .collect()
            } else if !rule.by_day.is_empty() {
                year_weekdays(&rule.by_day, y)
            } else {
                // The start's month and day; a 29 February start skips the years without one.
                Date::new(y, first.month(), first.day())
                    .ok()
                    .into_iter()
                    .collect()
            }
        }
    };
    days.sort_unstable();
    days.dedup();
    days
}

/// The days of month `m` of year `y` the rule picks: `BYMONTHDAY` and `BYDAY` (both: the days in
/// both), else the start's day of the month.
fn month_days(rule: &Rule, first: Date, y: i16, m: i8) -> Vec<Date> {
    let by_md: Vec<Date> = rule
        .by_month_day
        .iter()
        .filter_map(|&n| month_day(y, m, n))
        .collect();
    let by_wd: Vec<Date> = rule
        .by_day
        .iter()
        .flat_map(|&(n, w)| month_weekdays(y, m, n, w))
        .collect();
    match (rule.by_month_day.is_empty(), rule.by_day.is_empty()) {
        (false, false) => by_md.into_iter().filter(|d| by_wd.contains(d)).collect(),
        (false, true) => by_md,
        (true, false) => by_wd,
        (true, true) => Date::new(y, m, first.day()).ok().into_iter().collect(),
    }
}

/// Day `n` of the month (negative from its end), if the month has it.
fn month_day(y: i16, m: i8, n: i32) -> Option<Date> {
    let first = Date::new(y, m, 1).ok()?;
    let len = i32::from(first.days_in_month());
    let day = if n > 0 { n } else { len + n + 1 };
    if day < 1 || day > len {
        return None;
    }
    Date::new(y, m, i8::try_from(day).ok()?).ok()
}

/// Weekday `w` in month `m`: the `n`th (negative from the end), or every one for 0.
fn month_weekdays(y: i16, m: i8, n: i32, w: Weekday) -> Vec<Date> {
    let Ok(first) = Date::new(y, m, 1) else {
        return vec![];
    };
    let len = i32::from(first.days_in_month());
    let offset = i32::from(
        (w.to_monday_zero_offset() - first.weekday().to_monday_zero_offset()).rem_euclid(7),
    );
    let all: Vec<i32> = (0..)
        .map(|k| 1 + offset + 7 * k)
        .take_while(|&d| d <= len)
        .collect();
    let picked: Vec<i32> = match n {
        0 => all,
        n if n > 0 => all
            .get(usize::try_from(n - 1).unwrap_or(usize::MAX))
            .copied()
            .into_iter()
            .collect(),
        n => {
            let i = i32::try_from(all.len()).unwrap_or(0) + n;
            usize::try_from(i)
                .ok()
                .and_then(|i| all.get(i))
                .copied()
                .into_iter()
                .collect()
        }
    };
    picked
        .into_iter()
        .filter_map(|d| Date::new(y, m, i8::try_from(d).ok()?).ok())
        .collect()
}

/// Weekdays across a whole year: the `n`th of the year (negative from its end), or every one for 0.
fn year_weekdays(by_day: &[(i32, Weekday)], y: i16) -> Vec<Date> {
    let Ok(jan1) = Date::new(y, 1, 1) else {
        return vec![];
    };
    let len = i32::from(jan1.days_in_year());
    let mut out = Vec::new();
    for &(n, w) in by_day {
        let offset = i32::from(
            (w.to_monday_zero_offset() - jan1.weekday().to_monday_zero_offset()).rem_euclid(7),
        );
        let all: Vec<i32> = (0..)
            .map(|k| offset + 7 * k)
            .take_while(|&d| d < len)
            .collect();
        let picked: Vec<i32> = match n {
            0 => all,
            n if n > 0 => all
                .get(usize::try_from(n - 1).unwrap_or(usize::MAX))
                .copied()
                .into_iter()
                .collect(),
            n => {
                let i = i32::try_from(all.len()).unwrap_or(0) + n;
                usize::try_from(i)
                    .ok()
                    .and_then(|i| all.get(i))
                    .copied()
                    .into_iter()
                    .collect()
            }
        };
        out.extend(
            picked
                .into_iter()
                .filter_map(|d| add_days(jan1, i64::from(d))),
        );
    }
    out
}

/// `BYSETPOS`: the picked positions (1-based, negative from the end) of a period's sorted days.
fn set_positions(days: &[Date], positions: &[i32]) -> Vec<Date> {
    let len = i32::try_from(days.len()).unwrap_or(i32::MAX);
    let mut out: Vec<Date> = positions
        .iter()
        .filter_map(|&p| {
            let i = if p > 0 { p - 1 } else { len + p };
            usize::try_from(i).ok().and_then(|i| days.get(i)).copied()
        })
        .collect();
    out.sort_unstable();
    out.dedup();
    out
}
