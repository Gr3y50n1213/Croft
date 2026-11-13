// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Calendars (T39a): each account's CalDAV calendars, kept on the device and synced both ways
//! with the same rules as contacts (the mail host, the account's own credential and pin, the
//! credential only after the server asks, changes sent conditional on the etag, a server change
//! never overwritten). Events are read and written by mail-core's iCalendar code; the screens ask
//! for the occurrences in a window of time in the device's zone. Nothing of an event is logged.

use std::collections::{HashMap, HashSet};

use croft_mail_core::ical::expand::{occurrences, Occurrence};
use croft_mail_core::ical::write::{self, EventFields, Repeat};
use croft_mail_core::ical::zone::iana;
use croft_mail_core::ical::{self, Calendar, Event, When};
use croft_protocols::caldav::{CalDav, WriteOutcome};
use croft_protocols::BackendError;
use croft_storage::{accounts, calendar as store};
use jiff::tz::TimeZone;
use jiff::Timestamp;

use crate::contacts::new_uid;
use crate::core::{now_ms, Core};
use crate::error::CoreError;

/// The longest window [`Core::events_between`] answers (a little over a year).
const MAX_WINDOW_MS: i64 = 400 * 86_400_000;
/// The most occurrences one window returns across all calendars.
const MAX_OCCURRENCES: usize = 10_000;

/// A calendar of an account.
#[derive(Clone, PartialEq, Eq, uniffi::Record)]
pub struct CalendarInfo {
    /// Its id.
    pub id: i64,
    /// Its account.
    pub account: i64,
    /// Its name.
    pub name: String,
    /// Its colour (`#RRGGBB`), when the server gives one.
    pub color: Option<String>,
    /// Whether its events are shown.
    pub shown: bool,
}

/// How a calendar sync went.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum CalendarSyncResult {
    /// Done.
    Done,
    /// The server offers no calendars (or the account has no calendar sync: the sample mailbox,
    /// an OAuth account).
    NotOffered,
    /// The server could not be reached; try later.
    Unreachable,
    /// The server refused the sign-in.
    SignInNeeded,
}

/// A calendar sync's result and how many of this device's changes lost to a server change.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Record)]
pub struct CalendarSyncOutcome {
    /// How it went.
    pub result: CalendarSyncResult,
    /// Changes made here that the server had changed too (the server's version was kept).
    pub conflicts: u32,
}

/// How an event repeats, as S38 offers it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum RepeatPreset {
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
    /// A rule Croft did not write: kept as it is when saved (shown read-only).
    Custom,
}

impl From<Repeat> for RepeatPreset {
    fn from(r: Repeat) -> Self {
        match r {
            Repeat::None => RepeatPreset::None,
            Repeat::Daily => RepeatPreset::Daily,
            Repeat::Weekly => RepeatPreset::Weekly,
            Repeat::Monthly => RepeatPreset::Monthly,
            Repeat::Yearly => RepeatPreset::Yearly,
            Repeat::Keep => RepeatPreset::Custom,
        }
    }
}

impl From<RepeatPreset> for Repeat {
    fn from(r: RepeatPreset) -> Self {
        match r {
            RepeatPreset::None => Repeat::None,
            RepeatPreset::Daily => Repeat::Daily,
            RepeatPreset::Weekly => Repeat::Weekly,
            RepeatPreset::Monthly => Repeat::Monthly,
            RepeatPreset::Yearly => Repeat::Yearly,
            RepeatPreset::Custom => Repeat::Keep,
        }
    }
}

/// One occurrence in a window.
#[derive(Clone, PartialEq, Eq, uniffi::Record)]
pub struct OccurrenceInfo {
    /// The event's id.
    pub event_id: i64,
    /// Its account.
    pub account: i64,
    /// Its calendar.
    pub calendar: i64,
    /// Its calendar's colour.
    pub color: Option<String>,
    /// When it starts (Unix ms).
    pub start_ms: i64,
    /// When it ends (Unix ms).
    pub end_ms: i64,
    /// A whole-day event: start and end are midnights in the zone asked for.
    pub all_day: bool,
    /// The occurrence's original start: what a change to this one occurrence names.
    pub key_ms: i64,
    /// Its title (may be empty).
    pub title: String,
    /// Its place.
    pub location: Option<String>,
    /// The event repeats.
    pub repeats: bool,
    /// It repeats in a way Croft does not expand: only its first occurrence is shown.
    pub repeats_unsupported: bool,
    /// Its time zone was not found: the time was read as UTC.
    pub zone_unknown: bool,
    /// The event has a change waiting to be sent.
    pub pending: bool,
}

/// The occurrences in a window.
#[derive(Clone, PartialEq, Eq, uniffi::Record)]
pub struct EventsWindow {
    /// By start.
    pub occurrences: Vec<OccurrenceInfo>,
    /// Some were left out (too many in the window).
    pub truncated: bool,
}

/// Someone on an event.
#[derive(Clone, PartialEq, Eq, uniffi::Record)]
pub struct EventPerson {
    /// The address.
    pub address: String,
    /// The name, if given.
    pub name: Option<String>,
    /// The reply (`ACCEPTED`, `DECLINED`, `TENTATIVE`, `NEEDS-ACTION`), for attendees.
    pub status: Option<String>,
}

/// One event (or one occurrence of it) for S37 and S38.
#[derive(Clone, PartialEq, Eq, uniffi::Record)]
pub struct EventDetail {
    /// Its id.
    pub id: i64,
    /// Its account.
    pub account: i64,
    /// Its calendar.
    pub calendar: i64,
    /// The occurrence shown: its original start (for a change to this one only); `None` for an
    /// event that does not repeat.
    pub key_ms: Option<i64>,
    /// The title.
    pub title: String,
    /// The place.
    pub location: String,
    /// The notes.
    pub notes: String,
    /// Start (Unix ms).
    pub start_ms: i64,
    /// End (Unix ms).
    pub end_ms: i64,
    /// A whole-day event.
    pub all_day: bool,
    /// The IANA zone the event is written in, when it has one Croft knows.
    pub zone: Option<String>,
    /// Its time zone was not found: the time was read as UTC.
    pub zone_unknown: bool,
    /// How it repeats.
    pub repeat: RepeatPreset,
    /// Reminders, in minutes before the start.
    pub reminders: Vec<i64>,
    /// The organiser, for an invitation.
    pub organizer: Option<EventPerson>,
    /// The attendees.
    pub attendees: Vec<EventPerson>,
    /// `CONFIRMED`, `TENTATIVE` or `CANCELLED`, when given.
    pub status: Option<String>,
    /// The server changed it too: its version was kept.
    pub conflict: bool,
    /// A change waits to be sent.
    pub pending: bool,
}

/// What S38 saves.
#[derive(Clone, PartialEq, Eq, uniffi::Record)]
pub struct EventInput {
    /// The title.
    pub title: String,
    /// A whole-day event: `start_ms` is the first day's midnight and `end_ms` the midnight after
    /// the last day, both in the zone given to the call.
    pub all_day: bool,
    /// Start (Unix ms).
    pub start_ms: i64,
    /// End (Unix ms).
    pub end_ms: i64,
    /// The IANA zone to write a timed event in (else the zone given to the call).
    pub zone: Option<String>,
    /// The place.
    pub location: String,
    /// The notes.
    pub notes: String,
    /// How it repeats.
    pub repeat: RepeatPreset,
    /// Reminders, in minutes before the start (at most 10).
    pub reminders: Vec<i64>,
}

/// Which occurrences a change or a delete is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum EventScope {
    /// The whole event, every occurrence.
    All,
    /// Only the occurrence whose original start is `key_ms`.
    One {
        /// The occurrence's original start (Unix ms).
        key_ms: i64,
    },
}

/// A reminder to post.
#[derive(Clone, PartialEq, Eq, uniffi::Record)]
pub struct ReminderInfo {
    /// The event's id.
    pub event_id: i64,
    /// Its account.
    pub account: i64,
    /// The occurrence's original start.
    pub key_ms: i64,
    /// When to post it (Unix ms).
    pub at_ms: i64,
    /// When the occurrence starts (Unix ms).
    pub start_ms: i64,
    /// Its title.
    pub title: String,
}

fn zone_or_utc(zone: &str) -> TimeZone {
    iana(zone).unwrap_or(TimeZone::UTC)
}

fn ts(ms: i64) -> Timestamp {
    Timestamp::from_millisecond(ms).unwrap_or(Timestamp::UNIX_EPOCH)
}

fn person(p: &ical::Person) -> EventPerson {
    EventPerson {
        address: p.address.clone(),
        name: p.name.clone(),
        status: p.status.clone(),
    }
}

fn zone_name(when: Option<&When>) -> Option<String> {
    match when {
        Some(When::Zoned(_, tzid)) => iana(tzid).and_then(|tz| tz.iana_name().map(str::to_owned)),
        _ => None,
    }
}

impl Core {
    /// The account's CalDAV client, or `None` when it has none.
    async fn caldav(&self, account: i64) -> Result<Option<CalDav>, CoreError> {
        Ok(self
            .dav_access(account)
            .await?
            .and_then(|(config, credential)| CalDav::new(&config, &credential).ok()))
    }

    /// Sends the account's waiting changes, then brings in the server's.
    async fn sync_calendars_now(&self, account: i64) -> Result<CalendarSyncOutcome, CoreError> {
        let outcome = |result, conflicts| CalendarSyncOutcome { result, conflicts };
        let dav = match self.caldav(account).await {
            Ok(Some(d)) => d,
            Ok(None) => return Ok(outcome(CalendarSyncResult::NotOffered, 0)),
            Err(CoreError::NoCredential) => {
                return Ok(outcome(CalendarSyncResult::SignInNeeded, 0))
            }
            Err(e) => return Err(e),
        };
        let net = |e: BackendError| match e {
            BackendError::Auth(_) => CalendarSyncResult::SignInNeeded,
            BackendError::Transient(_) => CalendarSyncResult::Unreachable,
            BackendError::Permanent(_) => CalendarSyncResult::NotOffered,
        };
        let found = match dav.discover().await {
            Ok(c) => c,
            Err(e) => return Ok(outcome(net(e), 0)),
        };
        if found.is_empty() {
            return Ok(outcome(CalendarSyncResult::NotOffered, 0));
        }
        let keep: Vec<String> = found.iter().map(|c| c.path.clone()).collect();
        self.inner.handle.read(|db| {
            let c = db.connection();
            for cal in &found {
                store::upsert_calendar(
                    c,
                    account,
                    &cal.path,
                    &cal.name,
                    cal.color.as_deref(),
                    cal.order,
                )?;
            }
            store::remove_calendars_not_in(c, account, &keep)
        })?;

        // Out: what was changed here, each confirmed only for the version sent.
        let mut conflicts = 0u32;
        let waiting = self
            .inner
            .handle
            .read(|db| store::pending(db.connection(), account))?;
        for (event, cal_path) in waiting {
            let rev = event.revision;
            let etag = match (&event.href, &event.etag) {
                (Some(_), Some(e)) => Some(e.clone()),
                (Some(href), None) => match dav.changes(&cal_path, None).await {
                    Ok(ch) => ch
                        .changed
                        .into_iter()
                        .find(|r| &r.path == href)
                        .map(|r| r.etag),
                    Err(e) => return Ok(outcome(net(e), conflicts)),
                },
                (None, _) => None,
            };
            let step = match (event.pending, &event.href) {
                (store::PENDING_CREATE, _) => {
                    let uid = ical::parse(&event.object)
                        .ok()
                        .and_then(|c| c.events().into_iter().next().map(|e| e.uid))
                        .filter(|u| {
                            !u.is_empty()
                                && u.len() <= 64
                                && u.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
                        })
                        .map_or_else(new_uid, Ok)?;
                    let href = format!("{}/{uid}.ics", cal_path.trim_end_matches('/'));
                    match dav.put(&href, &event.object, None).await {
                        // 412 on a create: most likely this one after a lost answer (the href comes from the UID).
                        Ok(WriteOutcome::Done | WriteOutcome::Conflict) => Ok(Step::Confirm(href)),
                        Ok(WriteOutcome::Gone) => Ok(Step::Keep),
                        Err(e) => Err(e),
                    }
                }
                (store::PENDING_UPDATE, Some(href)) => match &etag {
                    Some(e) => match dav.put(href, &event.object, Some(e)).await {
                        Ok(WriteOutcome::Done) => Ok(Step::Confirm(href.clone())),
                        Ok(WriteOutcome::Conflict) => Ok(Step::Conflict),
                        Ok(WriteOutcome::Gone) => Ok(Step::Gone),
                        Err(e) => Err(e),
                    },
                    None => Ok(Step::Gone),
                },
                (store::PENDING_DELETE, Some(href)) => match &etag {
                    Some(e) => match dav.delete(href, e).await {
                        Ok(WriteOutcome::Done | WriteOutcome::Gone) => {
                            Ok(Step::Confirm(href.clone()))
                        }
                        Ok(WriteOutcome::Conflict) => Ok(Step::Conflict),
                        Err(e) => Err(e),
                    },
                    None => Ok(Step::Confirm(href.clone())),
                },
                _ => Ok(Step::Keep),
            };
            match step {
                Ok(Step::Confirm(href)) => self
                    .inner
                    .handle
                    .read(|db| store::confirm(db.connection(), account, event.id, rev, &href))?,
                Ok(Step::Conflict) => {
                    conflicts += 1;
                    self.inner
                        .handle
                        .read(|db| store::mark_conflict(db.connection(), account, event.id, rev))?;
                }
                Ok(Step::Gone) => self
                    .inner
                    .handle
                    .read(|db| store::remove_gone(db.connection(), account, event.id, rev))?,
                Ok(Step::Keep) => {}
                Err(e) => return Ok(outcome(net(e), conflicts)),
            }
        }

        // In: what changed on the server, stored one batch at a time.
        let stored = self
            .inner
            .handle
            .read(|db| store::calendars(db.connection(), account))?;
        for cal in stored {
            let changes = match dav.changes(&cal.path, cal.sync_token.as_deref()).await {
                Ok(c) => c,
                Err(e) => return Ok(outcome(net(e), conflicts)),
            };
            let known: HashMap<String, String> = self
                .inner
                .handle
                .read(|db| store::server_events(db.connection(), account, cal.id))?
                .into_iter()
                .collect();
            let wanted: Vec<String> = changes
                .changed
                .iter()
                .filter(|r| known.get(&r.path) != Some(&r.etag))
                .map(|r| r.path.clone())
                .collect();
            for batch in wanted.chunks(croft_protocols::caldav::MULTIGET_BATCH) {
                let fetched = match dav.fetch(&cal.path, batch).await {
                    Ok(f) => f,
                    Err(e) => return Ok(outcome(net(e), conflicts)),
                };
                let now = now_ms();
                self.inner.handle.read(|db| {
                    let c = db.connection();
                    for o in &fetched {
                        store::store_from_server(
                            c, account, cal.id, &o.path, &o.etag, &o.text, now,
                        )?;
                    }
                    Ok::<_, croft_storage::StorageError>(())
                })?;
            }
            let now = now_ms();
            self.inner.handle.read(|db| {
                let c = db.connection();
                for gone in &changes.removed {
                    store::remove_from_server(c, account, cal.id, gone)?;
                }
                if changes.full {
                    let listed: HashSet<&str> =
                        changes.changed.iter().map(|r| r.path.as_str()).collect();
                    for (href, _) in known.iter().filter(|(h, _)| !listed.contains(h.as_str())) {
                        store::remove_from_server(c, account, cal.id, href)?;
                    }
                }
                store::set_calendar_sync(
                    c,
                    account,
                    cal.id,
                    changes.sync_token.as_deref(),
                    None,
                    now,
                )
            })?;
        }
        Ok(outcome(CalendarSyncResult::Done, conflicts))
    }

    /// The stored object of event `id`, parsed.
    fn event_object(
        &self,
        account: i64,
        id: i64,
    ) -> Result<(store::StoredEvent, Calendar), CoreError> {
        let stored = self
            .inner
            .handle
            .read(|db| store::get(db.connection(), account, id))?
            .ok_or(CoreError::NoSuchMessage)?;
        let cal = ical::parse(&stored.object).map_err(|_| CoreError::Internal)?;
        Ok((stored, cal))
    }
}

/// What to do with a change made here, after the server answered.
enum Step {
    Confirm(String),
    Conflict,
    Gone,
    Keep,
}

fn fields_of(input: &EventInput, tz: &TimeZone) -> EventFields {
    let zone_name = if input.all_day {
        None
    } else {
        input
            .zone
            .as_deref()
            .and_then(iana)
            .or_else(|| Some(tz.clone()))
            .and_then(|z| z.iana_name().map(str::to_owned))
    };
    let write_tz = zone_name
        .as_deref()
        .and_then(iana)
        .unwrap_or_else(|| tz.clone());
    let start = write_tz.to_datetime(ts(input.start_ms));
    let end = write_tz.to_datetime(ts(input.end_ms.max(input.start_ms)));
    EventFields {
        title: input.title.chars().take(ical::MAX_TEXT_CHARS).collect(),
        all_day: input.all_day,
        start,
        end,
        zone: zone_name,
        location: input.location.chars().take(ical::MAX_TEXT_CHARS).collect(),
        notes: input.notes.chars().take(ical::MAX_TEXT_CHARS).collect(),
        repeat: input.repeat.into(),
        reminders: input.reminders.iter().copied().take(10).collect(),
    }
}

/// The event of `events` that occurrence `o` shows.
fn source<'a>(events: &'a [Event], o: &Occurrence) -> Option<&'a Event> {
    events.get(o.source)
}

#[uniffi::export]
impl Core {
    /// Syncs the calendars of `account` now: sends the changes made here, then brings in the
    /// server's. One at a time (with contacts).
    pub async fn sync_calendars(
        self: std::sync::Arc<Self>,
        account: i64,
    ) -> Result<CalendarSyncOutcome, CoreError> {
        let core = self.clone();
        self.spawn(
            async move {
                let _one = core.inner.contact_syncs.lock().await;
                core.sync_calendars_now(account).await
            },
            false,
        )
        .await?
    }

    /// The calendars of `account`.
    pub fn calendars(&self, account: i64) -> Result<Vec<CalendarInfo>, CoreError> {
        Ok(self
            .inner
            .handle
            .read(|db| store::calendars(db.connection(), account))?
            .into_iter()
            .map(|c| CalendarInfo {
                id: c.id,
                account,
                name: c.name,
                color: c.color,
                shown: c.shown,
            })
            .collect())
    }

    /// Shows or hides a calendar's events. False when there is no such calendar.
    pub fn set_calendar_shown(
        &self,
        account: i64,
        id: i64,
        shown: bool,
    ) -> Result<bool, CoreError> {
        Ok(self
            .inner
            .handle
            .read(|db| store::set_shown(db.connection(), account, id, shown))?)
    }

    /// The occurrences of every account's shown calendars that overlap `[from_ms, to_ms)`, seen
    /// from `zone` (the device's IANA zone; UTC when unknown). At most a little over a year.
    pub fn events_between(
        &self,
        from_ms: i64,
        to_ms: i64,
        zone: String,
    ) -> Result<EventsWindow, CoreError> {
        let to_ms = to_ms.min(from_ms.saturating_add(MAX_WINDOW_MS));
        let tz = zone_or_utc(&zone);
        let (from, to) = (ts(from_ms), ts(to_ms));
        let rows = self.inner.handle.read(|db| {
            let c = db.connection();
            let mut out = Vec::new();
            for a in accounts::list(c)? {
                let colors: HashMap<i64, Option<String>> = store::calendars(c, a.id)?
                    .into_iter()
                    .map(|k| (k.id, k.color))
                    .collect();
                for e in store::in_window(c, a.id, from_ms, to_ms)? {
                    let color = colors.get(&e.calendar_id).cloned().flatten();
                    out.push((e, color));
                }
            }
            Ok::<_, CoreError>(out)
        })?;
        let mut out = EventsWindow {
            occurrences: Vec::new(),
            truncated: false,
        };
        for (e, color) in rows {
            let Ok(cal) = ical::parse(&e.object) else {
                continue;
            };
            let events = cal.events();
            let exp = occurrences(&cal, &tz, from, to);
            out.truncated |= exp.truncated;
            let repeats = events
                .first()
                .is_some_and(|m| m.rrule.is_some() || !m.rdates.is_empty());
            for o in &exp.occurrences {
                let Some(ev) = source(&events, o) else {
                    continue;
                };
                out.occurrences.push(OccurrenceInfo {
                    event_id: e.id,
                    account: e.account_id,
                    calendar: e.calendar_id,
                    color: color.clone(),
                    start_ms: o.start.as_millisecond(),
                    end_ms: o.end.as_millisecond(),
                    all_day: o.all_day,
                    key_ms: o.key.as_millisecond(),
                    title: ev.summary.clone().unwrap_or_default(),
                    location: ev.location.clone(),
                    repeats,
                    repeats_unsupported: exp.repeats_unsupported,
                    zone_unknown: !o.zone_known,
                    pending: e.pending != store::PENDING_NONE,
                });
            }
        }
        out.occurrences
            .sort_by_key(|o| (o.start_ms, o.end_ms, o.event_id));
        if out.occurrences.len() > MAX_OCCURRENCES {
            out.occurrences.truncate(MAX_OCCURRENCES);
            out.truncated = true;
        }
        Ok(out)
    }

    /// Event `id` of `account`, as the occurrence whose original start is `key_ms` (or its first
    /// occurrence), seen from `zone`. `None` when it is gone.
    pub fn get_event(
        &self,
        account: i64,
        id: i64,
        key_ms: Option<i64>,
        zone: String,
    ) -> Result<Option<EventDetail>, CoreError> {
        let (stored, cal) = match self.event_object(account, id) {
            Ok(x) => x,
            Err(CoreError::NoSuchMessage) => return Ok(None),
            Err(e) => return Err(e),
        };
        let tz = zone_or_utc(&zone);
        let events = cal.events();
        let Some(master) = events.first() else {
            return Ok(None);
        };
        let repeats = master.rrule.is_some() || !master.rdates.is_empty();
        // The occurrence: the one asked for, else the first.
        let (from, to) = match key_ms {
            Some(k) => (
                ts(k.saturating_sub(366 * 86_400_000)),
                ts(k.saturating_add(366 * 86_400_000)),
            ),
            None => (Timestamp::MIN, Timestamp::MAX),
        };
        let exp = occurrences(&cal, &tz, from, to);
        let occ = match key_ms {
            Some(k) => exp.occurrences.iter().find(|o| o.key.as_millisecond() == k),
            None => exp.occurrences.first(),
        };
        let Some(o) = occ else {
            return Ok(None);
        };
        let ev = source(&events, o).unwrap_or(master);
        let rule_start = master.start.as_ref().map(|s| s.civil().date());
        Ok(Some(EventDetail {
            id: stored.id,
            account: stored.account_id,
            calendar: stored.calendar_id,
            key_ms: repeats.then(|| o.key.as_millisecond()),
            title: ev.summary.clone().unwrap_or_default(),
            location: ev.location.clone().unwrap_or_default(),
            notes: ev.description.clone().unwrap_or_default(),
            start_ms: o.start.as_millisecond(),
            end_ms: o.end.as_millisecond(),
            all_day: o.all_day,
            zone: zone_name(ev.start.as_ref()),
            zone_unknown: !o.zone_known,
            repeat: rule_start
                .map(|d| Repeat::of(master.rrule.as_deref(), d))
                .unwrap_or(Repeat::None)
                .into(),
            reminders: ev.reminders.clone(),
            organizer: ev.organizer.as_ref().map(person),
            attendees: ev.attendees.iter().map(person).collect(),
            status: ev.status.clone(),
            conflict: stored.conflict,
            pending: stored.pending != store::PENDING_NONE,
        }))
    }

    /// Saves S38: a new event in `calendar` (when `id` is `None`), or a change to event `id`,
    /// to every occurrence or to one. Stored at once, sent at the next sync. Returns the event's id.
    pub fn save_event(
        &self,
        account: i64,
        id: Option<i64>,
        calendar: i64,
        input: EventInput,
        scope: EventScope,
        zone: String,
    ) -> Result<i64, CoreError> {
        let tz = zone_or_utc(&zone);
        let fields = fields_of(&input, &tz);
        let now = Timestamp::now();
        match id {
            None => {
                let uid = new_uid()?;
                let object = write::new_object(&fields, &uid, now).to_text();
                self.inner
                    .handle
                    .read(|db| {
                        store::create_local(db.connection(), account, calendar, &object, now_ms())
                    })
                    .map_err(|_| CoreError::NoSuchAccount)
            }
            Some(id) => {
                let (_, mut cal) = self.event_object(account, id)?;
                let done = match scope {
                    EventScope::All => write::edit_all(&mut cal, &fields, now),
                    EventScope::One { key_ms } => {
                        let rid = write::recurrence_id(&cal, ts(key_ms), &tz)
                            .ok_or(CoreError::Internal)?;
                        write::edit_one(&mut cal, &rid, &fields, now)
                    }
                };
                if !done {
                    return Err(CoreError::Internal);
                }
                let text = cal.to_text();
                if text.len() > ical::MAX_OBJECT {
                    return Err(CoreError::Internal);
                }
                let saved = self.inner.handle.read(|db| {
                    store::update_local(db.connection(), account, id, &text, now_ms())
                })?;
                if saved {
                    Ok(id)
                } else {
                    Err(CoreError::NoSuchMessage)
                }
            }
        }
    }

    /// Deletes event `id` (every occurrence), or one occurrence of it (written as an `EXDATE`).
    /// Sent at the next sync. False when there is no such event.
    pub fn delete_event(
        &self,
        account: i64,
        id: i64,
        scope: EventScope,
        zone: String,
    ) -> Result<bool, CoreError> {
        match scope {
            EventScope::All => Ok(self
                .inner
                .handle
                .read(|db| store::delete_local(db.connection(), account, id))?),
            EventScope::One { key_ms } => {
                let (_, mut cal) = match self.event_object(account, id) {
                    Ok(x) => x,
                    Err(CoreError::NoSuchMessage) => return Ok(false),
                    Err(e) => return Err(e),
                };
                let tz = zone_or_utc(&zone);
                let rid = write::recurrence_id(&cal, ts(key_ms), &tz).ok_or(CoreError::Internal)?;
                if !write::delete_one(&mut cal, &rid, Timestamp::now()) {
                    return Ok(false);
                }
                let text = cal.to_text();
                Ok(self.inner.handle.read(|db| {
                    store::update_local(db.connection(), account, id, &text, now_ms())
                })?)
            }
        }
    }

    /// The reminders due in `[from_ms, to_ms)` across shown calendars, by time, seen from `zone`.
    /// The app posts them; the window is at most a little over a year.
    pub fn next_reminders(
        &self,
        from_ms: i64,
        to_ms: i64,
        zone: String,
    ) -> Result<Vec<ReminderInfo>, CoreError> {
        // An occurrence's reminders come up to a few weeks before it: look that far past the window.
        const LOOKAHEAD_MS: i64 = 28 * 86_400_000;
        let window =
            self.events_between(from_ms, to_ms.saturating_add(LOOKAHEAD_MS), zone.clone())?;
        let tz = zone_or_utc(&zone);
        let mut out = Vec::new();
        let mut objects: HashMap<(i64, i64), Option<Calendar>> = HashMap::new();
        for o in window.occurrences {
            let cal = objects.entry((o.account, o.event_id)).or_insert_with(|| {
                self.event_object(o.account, o.event_id)
                    .ok()
                    .map(|(_, c)| c)
            });
            let Some(cal) = cal.as_ref() else { continue };
            let events = cal.events();
            let exp = occurrences(cal, &tz, ts(o.start_ms), ts(o.start_ms.saturating_add(1)));
            let Some(ev) = exp
                .occurrences
                .iter()
                .find(|x| x.key.as_millisecond() == o.key_ms)
                .and_then(|x| source(&events, x))
            else {
                continue;
            };
            if ev.status.as_deref() == Some("CANCELLED") {
                continue;
            }
            for m in &ev.reminders {
                let at = o.start_ms.saturating_sub(m.saturating_mul(60_000));
                if at >= from_ms && at < to_ms {
                    out.push(ReminderInfo {
                        event_id: o.event_id,
                        account: o.account,
                        key_ms: o.key_ms,
                        at_ms: at,
                        start_ms: o.start_ms,
                        title: o.title.clone(),
                    });
                }
            }
        }
        out.sort_by_key(|r| (r.at_ms, r.event_id, r.key_ms));
        Ok(out)
    }
}

impl std::fmt::Debug for CalendarInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CalendarInfo")
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}

impl std::fmt::Debug for OccurrenceInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Event text stays out of logs and error output.
        f.debug_struct("OccurrenceInfo")
            .field("event_id", &self.event_id)
            .finish_non_exhaustive()
    }
}

impl std::fmt::Debug for EventsWindow {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EventsWindow")
            .field("occurrences", &self.occurrences.len())
            .finish()
    }
}

impl std::fmt::Debug for EventPerson {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EventPerson").finish_non_exhaustive()
    }
}

impl std::fmt::Debug for EventDetail {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EventDetail")
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}

impl std::fmt::Debug for EventInput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EventInput")
            .field("all_day", &self.all_day)
            .finish_non_exhaustive()
    }
}

impl std::fmt::Debug for ReminderInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReminderInfo")
            .field("event_id", &self.event_id)
            .finish_non_exhaustive()
    }
}
