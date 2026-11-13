// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! iCalendar (RFC 5545), read and written by Croft itself (T39a): the content lines of a calendar
//! object as a tree of components, the events in it, their time zones ([`zone`]) and their
//! repeats ([`rrule`]). Like the vCard reader, every line Croft does not change is written back as
//! it was read, so an edit never loses what another client stored.
//!
//! Pure: no clock and no I/O. Times are passed in; the device's own zone is a parameter.

pub mod expand;
pub mod rrule;
pub mod write;
pub mod zone;

use jiff::civil::{Date, DateTime};
use jiff::Timestamp;

/// The largest calendar object read, in bytes.
pub const MAX_OBJECT: usize = 1024 * 1024;
/// The most content lines in one object.
pub const MAX_LINES: usize = 20_000;
/// The most components (events, alarms, time zones and their parts) in one object.
pub const MAX_COMPONENTS: usize = 1_000;
/// The deepest nesting of components.
pub const MAX_DEPTH: usize = 8;
/// The longest text value kept, in characters.
pub const MAX_TEXT_CHARS: usize = 8_000;
/// The most attendees read from one event.
pub const MAX_ATTENDEES: usize = 500;
/// The most dates read from `RDATE` and `EXDATE` lines of one event.
pub const MAX_DATES: usize = 1_000;

/// Why a calendar object was not read. Fixed reasons: nothing of the object is in them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IcalError {
    /// Over one of the bounds above.
    TooLarge,
    /// Not a `VCALENDAR`, or its `BEGIN` and `END` lines do not match.
    NotACalendar,
}

impl std::fmt::Display for IcalError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            IcalError::TooLarge => "calendar object too large",
            IcalError::NotACalendar => "not a calendar object",
        })
    }
}

impl std::error::Error for IcalError {}

/// One content line, unfolded.
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct Line {
    /// The text as read (unfolded), written back as it was unless the property is rewritten.
    pub(crate) raw: String,
    /// Upper-case property name.
    pub(crate) name: String,
    /// Parameters: upper-case name, values (quotes removed).
    pub(crate) params: Vec<(String, Vec<String>)>,
    /// The value, still escaped.
    pub(crate) value: String,
}

impl Line {
    /// The first value of parameter `name`.
    pub(crate) fn param(&self, name: &str) -> Option<&str> {
        self.params
            .iter()
            .find(|(n, _)| n == name)
            .and_then(|(_, v)| v.first())
            .map(String::as_str)
    }
}

/// A component (`VCALENDAR`, `VEVENT`, `VALARM`, `VTIMEZONE`, `STANDARD`, ...) with its lines and
/// the components inside it, in the order read.
#[derive(Clone, PartialEq, Eq)]
pub struct Component {
    /// Upper-case name (`VEVENT`).
    pub kind: String,
    pub(crate) lines: Vec<Line>,
    /// The components inside, in order.
    pub children: Vec<Component>,
}

impl Component {
    fn new(kind: &str) -> Component {
        Component {
            kind: kind.to_owned(),
            lines: Vec::new(),
            children: Vec::new(),
        }
    }

    /// The first line called `name`.
    pub(crate) fn line(&self, name: &str) -> Option<&Line> {
        self.lines.iter().find(|l| l.name == name)
    }

    /// Every line called `name`.
    pub(crate) fn lines_named<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Line> {
        self.lines.iter().filter(move |l| l.name == name)
    }

    /// The text of the first line called `name`, unescaped.
    fn text(&self, name: &str) -> Option<String> {
        self.line(name)
            .map(|l| unescape(&l.value))
            .filter(|t| !t.is_empty())
    }

    /// Replaces every line called `name` with `new` (none to remove it), in the place of the first.
    pub(crate) fn set_lines(&mut self, name: &str, new: Vec<Line>) {
        let at = self.lines.iter().position(|l| l.name == name);
        self.lines.retain(|l| l.name != name);
        let at = at.unwrap_or(self.lines.len()).min(self.lines.len());
        for (i, l) in new.into_iter().enumerate() {
            self.lines.insert(at + i, l);
        }
    }

    /// Writes the component, folded at 75 octets, with CRLF.
    fn write(&self, out: &mut String) {
        fold_into(out, &format!("BEGIN:{}", self.kind));
        for l in &self.lines {
            fold_into(out, &l.raw);
        }
        for c in &self.children {
            c.write(out);
        }
        fold_into(out, &format!("END:{}", self.kind));
    }
}

/// A calendar object: one `VCALENDAR` (CalDAV keeps one event, with its overrides, per object).
#[derive(Clone, PartialEq, Eq)]
pub struct Calendar {
    /// The `VCALENDAR` component.
    pub root: Component,
}

/// Reads one calendar object.
pub fn parse(text: &str) -> Result<Calendar, IcalError> {
    parse_within(text, MAX_OBJECT, MAX_LINES, MAX_COMPONENTS)
}

/// The largest calendar file read from a link (T39c).
pub const MAX_FEED: usize = 5 * 1024 * 1024;
/// The most lines in a calendar file from a link.
pub const MAX_FEED_LINES: usize = 200_000;
/// The most components in a calendar file from a link.
pub const MAX_FEED_COMPONENTS: usize = 20_000;

/// Reads a whole calendar file from a link, with its own larger bounds; [`split_by_uid`] then makes
/// one object per event, each within the usual bounds.
pub fn parse_feed(text: &str) -> Result<Calendar, IcalError> {
    parse_within(text, MAX_FEED, MAX_FEED_LINES, MAX_FEED_COMPONENTS)
}

fn parse_within(
    text: &str,
    max_bytes: usize,
    max_lines: usize,
    max_components: usize,
) -> Result<Calendar, IcalError> {
    if text.len() > max_bytes {
        return Err(IcalError::TooLarge);
    }
    let lines = unfold(text, max_lines)?;
    let mut stack: Vec<Component> = Vec::new();
    let mut root: Option<Component> = None;
    let mut count = 0usize;
    for raw in lines {
        if raw.trim().is_empty() {
            continue;
        }
        let Some(line) = parse_line(&raw) else {
            continue;
        };
        if root.is_some() {
            // Anything after the end of the calendar.
            return Err(IcalError::NotACalendar);
        }
        match line.name.as_str() {
            "BEGIN" => {
                let kind = line.value.trim().to_ascii_uppercase();
                let named = !kind.is_empty()
                    && kind.len() <= 64
                    && kind.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-');
                if !named || (stack.is_empty() && kind != "VCALENDAR") {
                    return Err(IcalError::NotACalendar);
                }
                count += 1;
                if count > max_components || stack.len() >= MAX_DEPTH {
                    return Err(IcalError::TooLarge);
                }
                stack.push(Component::new(&kind));
            }
            "END" => {
                let kind = line.value.trim().to_ascii_uppercase();
                let done = stack.pop().ok_or(IcalError::NotACalendar)?;
                if done.kind != kind {
                    return Err(IcalError::NotACalendar);
                }
                match stack.last_mut() {
                    Some(parent) => parent.children.push(done),
                    None => root = Some(done),
                }
            }
            _ => match stack.last_mut() {
                Some(c) => c.lines.push(line),
                None => return Err(IcalError::NotACalendar),
            },
        }
    }
    root.map(|root| Calendar { root })
        .ok_or(IcalError::NotACalendar)
}

/// Unfolds lines: a line starting with a space or tab continues the one before. LF and CRLF both
/// end a line.
fn unfold(text: &str, max_lines: usize) -> Result<Vec<String>, IcalError> {
    let mut out: Vec<String> = Vec::new();
    for line in text.split('\n') {
        let line = line.strip_suffix('\r').unwrap_or(line);
        if let Some(rest) = line.strip_prefix([' ', '\t']).filter(|_| !out.is_empty()) {
            if let Some(last) = out.last_mut() {
                last.push_str(rest);
            }
        } else {
            if out.len() >= max_lines {
                return Err(IcalError::TooLarge);
            }
            out.push(line.to_owned());
        }
    }
    Ok(out)
}

/// Splits `NAME;PARAM=a,b;PARAM="c":value`. `None` for a line without a colon or a valid name.
pub(crate) fn parse_line(raw: &str) -> Option<Line> {
    let mut quoted = false;
    let mut colon = None;
    for (i, c) in raw.char_indices() {
        match c {
            '"' => quoted = !quoted,
            ':' if !quoted => {
                colon = Some(i);
                break;
            }
            _ => {}
        }
    }
    let colon = colon?;
    let (head, value) = (&raw[..colon], &raw[colon + 1..]);
    let mut parts = split_unquoted(head, ';').into_iter();
    let name = parts.next()?.trim().to_ascii_uppercase();
    if name.is_empty() || !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-') {
        return None;
    }
    let mut params = Vec::new();
    for p in parts {
        let Some((n, v)) = p.split_once('=') else {
            continue;
        };
        let values = split_unquoted(v, ',')
            .into_iter()
            .map(|v| v.trim().trim_matches('"').to_owned())
            .collect();
        params.push((n.trim().to_ascii_uppercase(), values));
    }
    Some(Line {
        raw: raw.to_owned(),
        name,
        params,
        value: value.to_owned(),
    })
}

/// Splits at `sep` outside double quotes.
fn split_unquoted(text: &str, sep: char) -> Vec<String> {
    let mut out = vec![String::new()];
    let mut quoted = false;
    for c in text.chars() {
        if c == '"' {
            quoted = !quoted;
        }
        if c == sep && !quoted {
            out.push(String::new());
        } else if let Some(last) = out.last_mut() {
            last.push(c);
        }
    }
    out
}

/// Undoes text escaping (`\\`, `\,`, `\;`, `\n`), drops control characters other than line breaks,
/// and caps the text at [`MAX_TEXT_CHARS`].
pub(crate) fn unescape(text: &str) -> String {
    let mut out = String::with_capacity(text.len().min(MAX_TEXT_CHARS * 4));
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('n' | 'N') => out.push('\n'),
                Some(other) => out.push(other),
                None => {}
            }
        } else {
            out.push(c);
        }
    }
    out.chars()
        .filter(|c| !c.is_control() || *c == '\n')
        .take(MAX_TEXT_CHARS)
        .collect()
}

/// Escapes a text value for writing.
pub(crate) fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            ',' => out.push_str("\\,"),
            ';' => out.push_str("\\;"),
            '\n' => out.push_str("\\n"),
            c if c.is_control() => {}
            c => out.push(c),
        }
    }
    out
}

/// A parameter value: quoted when it holds a character that would end it.
fn param_text(value: &str) -> String {
    let clean: String = value
        .chars()
        .filter(|c| !c.is_control() && *c != '"')
        .collect();
    if clean.contains([';', ':', ',']) {
        format!("\"{clean}\"")
    } else {
        clean
    }
}

/// Appends `line` folded at 75 octets (never inside a character), with CRLF.
fn fold_into(out: &mut String, line: &str) {
    let mut width = 0;
    let mut first = true;
    for c in line.chars() {
        let len = c.len_utf8();
        let limit = if first { 75 } else { 74 };
        if width + len > limit {
            out.push_str("\r\n ");
            width = 0;
            first = false;
        }
        out.push(c);
        width += len;
    }
    out.push_str("\r\n");
}

/// Builds a line and parses it back, so it is exactly what a reader of the text sees.
pub(crate) fn make(name: &str, params: &[(&str, &str)], value: &str) -> Line {
    let mut raw = name.to_owned();
    for (n, v) in params {
        raw.push(';');
        raw.push_str(n);
        raw.push('=');
        raw.push_str(&param_text(v));
    }
    raw.push(':');
    raw.push_str(value);
    parse_line(&raw).unwrap_or(Line {
        raw,
        name: name.to_owned(),
        params: Vec::new(),
        value: value.to_owned(),
    })
}

impl Calendar {
    /// The object as text, CRLF, folded.
    pub fn to_text(&self) -> String {
        let mut out = String::new();
        self.root.write(&mut out);
        out
    }

    /// The `VEVENT`s, master first (the one without `RECURRENCE-ID`), then its overrides.
    pub fn events(&self) -> Vec<Event> {
        let mut out: Vec<Event> = self
            .root
            .children
            .iter()
            .filter(|c| c.kind == "VEVENT")
            .map(Event::read)
            .collect();
        out.sort_by_key(|e| e.is_override);
        out
    }

    /// The `VTIMEZONE` components, by `TZID`.
    pub fn time_zones(&self) -> Vec<(String, &Component)> {
        self.root
            .children
            .iter()
            .filter(|c| c.kind == "VTIMEZONE")
            .filter_map(|c| c.line("TZID").map(|l| (unescape(&l.value), c)))
            .collect()
    }

    /// The `VEVENT` component of the master event (the first without `RECURRENCE-ID`).
    pub fn master_mut(&mut self) -> Option<&mut Component> {
        self.root
            .children
            .iter_mut()
            .find(|c| c.kind == "VEVENT" && c.line("RECURRENCE-ID").is_none())
    }

    /// Whether the object has any `VEVENT`.
    pub fn has_events(&self) -> bool {
        self.root.children.iter().any(|c| c.kind == "VEVENT")
    }
}

/// Splits a calendar file (from a link) into one object per event `UID`, each with the file's
/// own `VCALENDAR` lines and the `VTIMEZONE`s its events name, in the order of first appearance.
/// Events without a `UID` are left out. At most [`MAX_COMPONENTS`] objects.
pub fn split_by_uid(cal: &Calendar) -> Vec<(String, Calendar)> {
    let mut order: Vec<String> = Vec::new();
    let mut groups: std::collections::HashMap<String, Vec<Component>> =
        std::collections::HashMap::new();
    for c in cal.root.children.iter().filter(|c| c.kind == "VEVENT") {
        let Some(uid) = c
            .line("UID")
            .map(|l| l.value.trim().to_owned())
            .filter(|u| !u.is_empty() && u.len() <= 500)
        else {
            continue;
        };
        if !groups.contains_key(&uid) {
            if order.len() >= MAX_COMPONENTS {
                break;
            }
            order.push(uid.clone());
        }
        groups.entry(uid).or_default().push(c.clone());
    }
    let zones: Vec<(String, &Component)> = cal.time_zones();
    order
        .into_iter()
        .filter_map(|uid| {
            let events = groups.remove(&uid)?;
            let used: std::collections::HashSet<String> = events
                .iter()
                .flat_map(|e| e.lines.iter())
                .filter_map(|l| l.param("TZID").map(str::to_owned))
                .collect();
            let mut children: Vec<Component> = zones
                .iter()
                .filter(|(tzid, _)| used.contains(tzid))
                .map(|(_, c)| (*c).clone())
                .collect();
            children.extend(events);
            let root = Component {
                kind: "VCALENDAR".into(),
                lines: cal.root.lines.clone(),
                children,
            };
            Some((uid, Calendar { root }))
        })
        .collect()
}

/// A date or time as an iCalendar property gives it.
#[derive(Clone, PartialEq, Eq)]
pub enum When {
    /// A whole day (`VALUE=DATE`).
    Date(Date),
    /// An exact time (`...Z`).
    Utc(Timestamp),
    /// A wall time in the named zone (`TZID=...`).
    Zoned(DateTime, String),
    /// A wall time with no zone: the viewer's.
    Floating(DateTime),
}

impl When {
    /// The wall date and time (midnight for a date; the UTC wall time for an exact time).
    pub fn civil(&self) -> DateTime {
        match self {
            When::Date(d) => d.to_datetime(jiff::civil::Time::midnight()),
            When::Utc(t) => t.to_zoned(jiff::tz::TimeZone::UTC).datetime(),
            When::Zoned(dt, _) | When::Floating(dt) => *dt,
        }
    }

    /// Whether it is a whole day.
    pub fn is_date(&self) -> bool {
        matches!(self, When::Date(_))
    }
}

impl std::fmt::Debug for When {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Times are not personal on their own, but the zone name is kept out with everything else.
        match self {
            When::Date(_) => f.write_str("When::Date"),
            When::Utc(_) => f.write_str("When::Utc"),
            When::Zoned(..) => f.write_str("When::Zoned"),
            When::Floating(_) => f.write_str("When::Floating"),
        }
    }
}

/// Reads a `DTSTART`-like line. `None` when the value cannot be read.
pub(crate) fn read_when(line: &Line) -> Option<When> {
    read_when_value(line.value.trim(), line)
}

fn read_when_value(v: &str, line: &Line) -> Option<When> {
    let date_only = line
        .param("VALUE")
        .is_some_and(|x| x.eq_ignore_ascii_case("DATE"))
        || (v.len() == 8 && !v.contains('T'));
    if date_only {
        return parse_date(v).map(When::Date);
    }
    if let Some(utc) = v.strip_suffix('Z').or_else(|| v.strip_suffix('z')) {
        let dt = parse_datetime(utc)?;
        let ts = dt.to_zoned(jiff::tz::TimeZone::UTC).ok()?.timestamp();
        return Some(When::Utc(ts));
    }
    let dt = parse_datetime(v)?;
    match line.param("TZID").map(str::trim).filter(|t| !t.is_empty()) {
        Some(tz) => Some(When::Zoned(dt, tz.chars().take(200).collect())),
        None => Some(When::Floating(dt)),
    }
}

/// Every date or time of an `RDATE` or `EXDATE` line (comma-separated). Periods give their start.
pub(crate) fn read_whens(line: &Line) -> Vec<When> {
    line.value
        .split(',')
        .take(MAX_DATES)
        .filter_map(|v| {
            let v = v.split('/').next().unwrap_or_default().trim();
            read_when_value(v, line)
        })
        .collect()
}

/// `YYYYMMDD`.
pub(crate) fn parse_date(v: &str) -> Option<Date> {
    if v.len() != 8 || !v.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let y: i16 = v[0..4].parse().ok()?;
    let m: i8 = v[4..6].parse().ok()?;
    let d: i8 = v[6..8].parse().ok()?;
    Date::new(y, m, d).ok()
}

/// `YYYYMMDDTHHMMSS` (a leap second 60 is read as 59).
pub(crate) fn parse_datetime(v: &str) -> Option<DateTime> {
    let (d, t) = v.split_once(['T', 't'])?;
    let date = parse_date(d)?;
    if t.len() != 6 || !t.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let h: i8 = t[0..2].parse().ok()?;
    let mi: i8 = t[2..4].parse().ok()?;
    let s: i8 = t[4..6].parse::<i8>().ok()?.min(59);
    let time = jiff::civil::Time::new(h, mi, s, 0).ok()?;
    Some(date.to_datetime(time))
}

/// `PnW`, `PnDTnHnMnS` (with an optional sign), in seconds. Weeks and days are taken as 24-hour
/// days here; all-day events use whole days.
pub(crate) fn parse_duration(v: &str) -> Option<i64> {
    let v = v.trim();
    let (sign, rest) = match v.as_bytes().first()? {
        b'-' => (-1, &v[1..]),
        b'+' => (1, &v[1..]),
        _ => (1, v),
    };
    let rest = rest.strip_prefix(['P', 'p'])?;
    let mut total: i64 = 0;
    let mut number = String::new();
    let mut in_time = false;
    let mut any = false;
    for c in rest.chars() {
        match c {
            '0'..='9' => {
                if number.len() >= 9 {
                    return None;
                }
                number.push(c);
            }
            'T' | 't' => in_time = true,
            _ => {
                let n: i64 = number.parse().ok()?;
                number.clear();
                let unit = match (c.to_ascii_uppercase(), in_time) {
                    ('W', false) => 7 * 86_400,
                    ('D', false) => 86_400,
                    ('H', true) => 3_600,
                    ('M', true) => 60,
                    ('S', true) => 1,
                    _ => return None,
                };
                total = total.checked_add(n.checked_mul(unit)?)?;
                any = true;
            }
        }
    }
    if !number.is_empty() || !any {
        return None;
    }
    Some(sign * total)
}

/// A person on an event: who organises it or who is invited.
#[derive(Clone, PartialEq, Eq, Default)]
pub struct Person {
    /// The address (`mailto:` removed).
    pub address: String,
    /// `CN`, if any.
    pub name: Option<String>,
    /// `PARTSTAT` in upper case (`ACCEPTED`, `DECLINED`, `TENTATIVE`, `NEEDS-ACTION`), for attendees.
    pub status: Option<String>,
    /// `ROLE` in upper case (`REQ-PARTICIPANT`, `OPT-PARTICIPANT`, `CHAIR`), for attendees.
    pub role: Option<String>,
}

fn read_person(line: &Line) -> Option<Person> {
    let v = line.value.trim();
    let address = v
        .strip_prefix("mailto:")
        .or_else(|| v.strip_prefix("MAILTO:"))
        .unwrap_or(v);
    if address.is_empty() || address.len() > 320 {
        return None;
    }
    let clip = |s: &str| -> String { s.chars().filter(|c| !c.is_control()).take(200).collect() };
    Some(Person {
        address: clip(address),
        name: line.param("CN").map(clip).filter(|n| !n.is_empty()),
        status: line.param("PARTSTAT").map(|s| clip(s).to_ascii_uppercase()),
        role: line.param("ROLE").map(|s| clip(s).to_ascii_uppercase()),
    })
}

/// One `VEVENT` as Croft reads it.
#[derive(Clone, PartialEq, Eq, Default)]
pub struct Event {
    /// `UID`.
    pub uid: String,
    /// `SUMMARY`.
    pub summary: Option<String>,
    /// `LOCATION`.
    pub location: Option<String>,
    /// `DESCRIPTION`.
    pub description: Option<String>,
    /// `STATUS` in upper case (`CONFIRMED`, `TENTATIVE`, `CANCELLED`).
    pub status: Option<String>,
    /// `SEQUENCE`.
    pub sequence: u32,
    /// `DTSTART`.
    pub start: Option<When>,
    /// `DTEND`.
    pub end: Option<When>,
    /// `DURATION`, in seconds.
    pub duration: Option<i64>,
    /// `RRULE`, as written.
    pub rrule: Option<String>,
    /// `RDATE`s.
    pub rdates: Vec<When>,
    /// `EXDATE`s.
    pub exdates: Vec<When>,
    /// `RECURRENCE-ID`: this is an override of that occurrence.
    pub recurrence_id: Option<When>,
    /// It has a `RECURRENCE-ID` line (readable or not): an override, never the master.
    pub is_override: bool,
    /// `ORGANIZER`.
    pub organizer: Option<Person>,
    /// `ATTENDEE`s.
    pub attendees: Vec<Person>,
    /// The `VALARM`s with a trigger relative to the start, as minutes before it (negative: after).
    pub reminders: Vec<i64>,
}

impl Event {
    fn read(c: &Component) -> Event {
        let mut reminders: Vec<i64> = c
            .children
            .iter()
            .filter(|a| a.kind == "VALARM")
            .filter_map(|a| {
                let t = a.line("TRIGGER")?;
                // Only a trigger relative to the start (the default); one at a fixed time or relative to the end is kept
                // in the object but not shown.
                if t.param("VALUE")
                    .is_some_and(|v| v.eq_ignore_ascii_case("DATE-TIME"))
                    || t.param("RELATED")
                        .is_some_and(|r| r.eq_ignore_ascii_case("END"))
                {
                    return None;
                }
                parse_duration(&t.value).map(|s| -s / 60)
            })
            .collect();
        reminders.sort_unstable();
        reminders.dedup();
        Event {
            uid: c
                .line("UID")
                .map(|l| l.value.trim().chars().take(500).collect())
                .unwrap_or_default(),
            summary: c.text("SUMMARY"),
            location: c.text("LOCATION"),
            description: c.text("DESCRIPTION"),
            status: c
                .line("STATUS")
                .map(|l| l.value.trim().to_ascii_uppercase()),
            sequence: c
                .line("SEQUENCE")
                .and_then(|l| l.value.trim().parse().ok())
                .unwrap_or(0),
            start: c.line("DTSTART").and_then(read_when),
            end: c.line("DTEND").and_then(read_when),
            duration: c.line("DURATION").and_then(|l| parse_duration(&l.value)),
            rrule: c
                .line("RRULE")
                .map(|l| l.value.trim().chars().take(1000).collect()),
            rdates: c
                .lines_named("RDATE")
                .flat_map(read_whens)
                .take(MAX_DATES)
                .collect(),
            exdates: c
                .lines_named("EXDATE")
                .flat_map(read_whens)
                .take(MAX_DATES)
                .collect(),
            recurrence_id: c.line("RECURRENCE-ID").and_then(read_when),
            is_override: c.line("RECURRENCE-ID").is_some(),
            organizer: c.line("ORGANIZER").and_then(read_person),
            attendees: c
                .lines_named("ATTENDEE")
                .filter_map(read_person)
                .take(MAX_ATTENDEES)
                .collect(),
            reminders,
        }
    }
}

impl std::fmt::Debug for Component {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Event text stays out of logs and error output.
        f.debug_struct("Component")
            .field("kind", &self.kind)
            .field("lines", &self.lines.len())
            .field("children", &self.children)
            .finish()
    }
}

impl std::fmt::Debug for Calendar {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Calendar")
            .field("root", &self.root)
            .finish()
    }
}

impl std::fmt::Debug for Line {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Line").field("name", &self.name).finish()
    }
}

impl std::fmt::Debug for Person {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Person")
            .field("status", &self.status)
            .finish_non_exhaustive()
    }
}

impl std::fmt::Debug for Event {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Event")
            .field("start", &self.start)
            .field("repeats", &self.rrule.is_some())
            .field("override", &self.is_override)
            .field("attendees", &self.attendees.len())
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests;
