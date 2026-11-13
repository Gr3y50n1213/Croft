// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Calendars (T39a): each account's CalDAV calendars, kept on the device and synced both ways
//! with the same rules as contacts (the mail host, the account's own credential and pin, the
//! credential only after the server asks, changes sent conditional on the etag, a server change
//! never overwritten). Events are read and written by mail-core's iCalendar code; the screens ask
//! for the occurrences in a window of time in the device's zone. Nothing of an event is logged.

use std::collections::{HashMap, HashSet};

use croft_mail_core::ical::expand::{occurrence_at, occurrences, Occurrence};
use croft_mail_core::ical::write::{self, Changed, EventFields, Repeat};
use croft_mail_core::ical::zone::{iana, Zones};
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
/// How long one window or reminders query may work before it says it was cut short.
const WINDOW_DEADLINE: std::time::Duration = std::time::Duration::from_secs(3);
/// The longest title or place in a window's list.
const LIST_TEXT_CHARS: usize = 200;
/// How far past the window [`Core::next_reminders`] looks for occurrences whose reminders fall in it.
const REMINDER_LEAD_MS: i64 = 35 * 86_400_000;

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
        // A listing per calendar, read at most once, for deletes whose etag is not known.
        let mut listings: HashMap<String, HashMap<String, String>> = HashMap::new();
        for (event, cal_path) in waiting {
            let rev = event.revision;
            let step = match (event.pending, &event.href, &event.etag) {
                (store::PENDING_CREATE, _, _) => {
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
                    match dav.put_tagged(&href, &event.object, None).await {
                        Ok(w) if w.outcome == WriteOutcome::Done => Ok(Step::Confirm(href, w.etag)),
                        // 412 on a create: most likely this one after a lost answer (the href comes from
                        // the UID); its etag comes with the next listing.
                        Ok(w) if w.outcome == WriteOutcome::Conflict => {
                            Ok(Step::Confirm(href, None))
                        }
                        Ok(_) => Ok(Step::Keep),
                        Err(e) => Err(e),
                    }
                }
                (store::PENDING_UPDATE, Some(href), Some(e)) => {
                    match dav.put_tagged(href, &event.object, Some(e)).await {
                        Ok(w) => Ok(match w.outcome {
                            WriteOutcome::Done => Step::Confirm(href.clone(), w.etag),
                            WriteOutcome::Conflict => Step::Conflict,
                            WriteOutcome::Gone => Step::Gone,
                        }),
                        Err(e) => Err(e),
                    }
                }
                // The version on the server is not known (the server gave no etag with the last write,
                // or the person changed it again before its listing): never sent blindly over a change
                // it has not seen. The server's version is kept and the event marked (T39a review).
                (store::PENDING_UPDATE, Some(_), None) => Ok(Step::Conflict),
                (store::PENDING_DELETE, Some(href), Some(e)) => match dav.delete(href, e).await {
                    Ok(WriteOutcome::Done | WriteOutcome::Gone) => {
                        Ok(Step::Confirm(href.clone(), None))
                    }
                    Ok(WriteOutcome::Conflict) => Ok(Step::Conflict),
                    Err(e) => Err(e),
                },
                (store::PENDING_DELETE, Some(href), None) => {
                    if !listings.contains_key(&cal_path) {
                        match dav.changes(&cal_path, None).await {
                            Ok(ch) => {
                                let map =
                                    ch.changed.into_iter().map(|r| (r.path, r.etag)).collect();
                                listings.insert(cal_path.clone(), map);
                            }
                            Err(e) => return Ok(outcome(net(e), conflicts)),
                        }
                    }
                    match listings.get(&cal_path).and_then(|l| l.get(href)) {
                        // Already gone on the server.
                        None => Ok(Step::Confirm(href.clone(), None)),
                        // There, in a version this device may not have seen: kept, and marked.
                        Some(_) => Ok(Step::Conflict),
                    }
                }
                _ => Ok(Step::Keep),
            };
            match step {
                Ok(Step::Confirm(href, etag)) => self.inner.handle.read(|db| {
                    store::confirm(
                        db.connection(),
                        account,
                        event.id,
                        rev,
                        &href,
                        etag.as_deref(),
                    )
                })?,
                // Refused for good (403, 415 and the like): marked and left, so it never blocks the rest.
                Ok(Step::Conflict) | Err(BackendError::Permanent(_)) => {
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
    /// Taken by the server at this address, in this version when it said.
    Confirm(String, Option<String>),
    Conflict,
    Gone,
    Keep,
}

fn fields_of(input: &EventInput, tz: &TimeZone, changed: Changed, repeat: Repeat) -> EventFields {
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
        repeat,
        reminders: input.reminders.iter().copied().take(10).collect(),
        changed,
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
    /// from `zone` (the device's IANA zone; UTC when unknown). At most a little over a year and
    /// [`MAX_OCCURRENCES`] occurrences; `truncated` says when some were left out. Titles and places
    /// are cut to 200 characters here ([`Core::get_event`] has them whole).
    pub fn events_between(
        &self,
        from_ms: i64,
        to_ms: i64,
        zone: String,
    ) -> Result<EventsWindow, CoreError> {
        let to_ms = to_ms.min(from_ms.saturating_add(MAX_WINDOW_MS));
        let tz = zone_or_utc(&zone);
        let (from, to) = (ts(from_ms), ts(to_ms));
        let mut kept: Vec<OccurrenceInfo> = Vec::new();
        let mut truncated = false;
        let clip = |t: &str| -> String { t.chars().take(LIST_TEXT_CHARS).collect() };
        let cut = self.each_in_window(from_ms, to_ms, |row, account, color, cal| {
            let events = cal.events();
            let exp = occurrences(cal, &tz, from, to);
            truncated |= exp.truncated;
            let repeats = events
                .iter()
                .find(|e| !e.is_override)
                .is_some_and(|m| m.rrule.is_some() || !m.rdates.is_empty());
            for o in &exp.occurrences {
                let Some(ev) = source(&events, o) else {
                    continue;
                };
                kept.push(OccurrenceInfo {
                    event_id: row.id,
                    account,
                    calendar: row.calendar_id,
                    color: color.map(str::to_owned),
                    start_ms: o.start.as_millisecond(),
                    end_ms: o.end.as_millisecond(),
                    all_day: o.all_day,
                    key_ms: o.key.as_millisecond(),
                    title: ev.summary.as_deref().map(clip).unwrap_or_default(),
                    location: ev.location.as_deref().map(clip),
                    repeats,
                    repeats_unsupported: exp.repeats_unsupported,
                    zone_unknown: !o.zone_known,
                    pending: row.pending,
                });
            }
            // Only the earliest are kept, so memory stays bounded whatever the calendars hold.
            if kept.len() > 2 * MAX_OCCURRENCES {
                sort_occurrences(&mut kept);
                kept.truncate(MAX_OCCURRENCES);
                truncated = true;
            }
        })?;
        sort_occurrences(&mut kept);
        if kept.len() > MAX_OCCURRENCES {
            kept.truncate(MAX_OCCURRENCES);
            truncated = true;
        }
        Ok(EventsWindow {
            occurrences: kept,
            truncated: truncated || cut,
        })
    }

    /// Event `id` of `account`, as the occurrence whose original start is `key_ms` (wherever it was
    /// moved), else as its first occurrence (what a change to every occurrence edits), seen from
    /// `zone`. `None` when it is gone.
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
        let Some(master) = events.iter().find(|e| !e.is_override) else {
            return Ok(None);
        };
        let repeats = master.rrule.is_some() || !master.rdates.is_empty();
        let occ = match key_ms {
            Some(k) => occurrence_at(&cal, &tz, ts(k)),
            None => first_occurrence(&cal, &tz),
        };
        let Some(o) = occ else {
            return Ok(None);
        };
        let ev = source(&events, &o).unwrap_or(master);
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

    /// Saves S38: a new event in `calendar` (when `id` is `None`), or a change to event `id`, to
    /// every occurrence (the times are then those of its first occurrence, as [`Core::get_event`]
    /// gives them with no key) or to one. Only what differs from the event as it is gets written.
    /// Stored at once, sent at the next sync. Returns the event's id.
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
        let now = Timestamp::now();
        let Some(id) = id else {
            let fields = fields_of(&input, &tz, Changed::ALL, input.repeat.into());
            let uid = new_uid()?;
            let object = write::new_object(&fields, &uid, now).to_text();
            return self
                .inner
                .handle
                .read(|db| {
                    store::create_local(db.connection(), account, calendar, &object, now_ms())
                })
                .map_err(|_| CoreError::NoSuchAccount);
        };
        let key = match scope {
            EventScope::All => None,
            EventScope::One { key_ms } => Some(key_ms),
        };
        let current = self
            .get_event(account, id, key, zone.clone())?
            .ok_or(CoreError::NoSuchMessage)?;
        let mut sorted_new = input.reminders.clone();
        sorted_new.sort_unstable();
        sorted_new.dedup();
        let times = input.start_ms != current.start_ms
            || input.end_ms != current.end_ms
            || input.all_day != current.all_day
            || (input.zone.is_some() && input.zone != current.zone);
        let changed = Changed {
            title: input.title != current.title,
            times,
            location: input.location != current.location,
            notes: input.notes != current.notes,
            reminders: sorted_new != current.reminders,
        };
        // The repeat is rewritten only when it changed, or when it follows the start (weekly on its
        // weekday, monthly on its day) and the start moved; a rule Croft did not write is kept.
        let repeat = match (input.repeat, current.repeat) {
            (RepeatPreset::Custom, _) => Repeat::Keep,
            (new, old) if new != old => new.into(),
            (new @ (RepeatPreset::Weekly | RepeatPreset::Monthly), _) if times => new.into(),
            _ => Repeat::Keep,
        };
        let fields = fields_of(&input, &tz, changed, repeat);
        let (_, mut cal) = self.event_object(account, id)?;
        let done = match key {
            None => write::edit_all(&mut cal, &fields, now),
            Some(key_ms) => {
                let rid = write::recurrence_id(&cal, ts(key_ms), &tz).ok_or(CoreError::Internal)?;
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
        if self
            .inner
            .handle
            .read(|db| store::update_local(db.connection(), account, id, &text, now_ms()))?
        {
            Ok(id)
        } else {
            Err(CoreError::NoSuchMessage)
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
    /// Each object is read and expanded once. Reminders more than five weeks before their
    /// occurrence are not found (OPEN-ITEMS). The app posts them.
    pub fn next_reminders(
        &self,
        from_ms: i64,
        to_ms: i64,
        zone: String,
    ) -> Result<Vec<ReminderInfo>, CoreError> {
        let to_ms = to_ms.min(from_ms.saturating_add(MAX_WINDOW_MS));
        let tz = zone_or_utc(&zone);
        let horizon = to_ms.saturating_add(REMINDER_LEAD_MS);
        let mut out = Vec::new();
        self.each_in_window(from_ms, horizon, |row, account, _, cal| {
            let events = cal.events();
            let exp = occurrences(cal, &tz, ts(from_ms), ts(horizon));
            for o in &exp.occurrences {
                let Some(ev) = source(&events, o) else {
                    continue;
                };
                if ev.status.as_deref() == Some("CANCELLED") {
                    continue;
                }
                let start = o.start.as_millisecond();
                for m in &ev.reminders {
                    let at = start.saturating_sub(m.saturating_mul(60_000));
                    if at >= from_ms && at < to_ms && out.len() < MAX_OCCURRENCES {
                        out.push(ReminderInfo {
                            event_id: row.id,
                            account,
                            key_ms: o.key.as_millisecond(),
                            at_ms: at,
                            start_ms: start,
                            title: ev
                                .summary
                                .as_deref()
                                .map(|t| t.chars().take(LIST_TEXT_CHARS).collect())
                                .unwrap_or_default(),
                        });
                    }
                }
            }
        })?;
        out.sort_by_key(|r| (r.at_ms, r.event_id, r.key_ms));
        Ok(out)
    }
}

impl Core {
    /// Calls `visit` with each event of every account's shown calendars that may touch
    /// `[from_ms, to_ms)`: its row, account, calendar colour and parsed object, one object at a time
    /// (T39a review: never every object in memory at once). Stops after [`WINDOW_DEADLINE`]. Returns
    /// whether some were left out (the row limit or the deadline).
    fn each_in_window(
        &self,
        from_ms: i64,
        to_ms: i64,
        mut visit: impl FnMut(&store::WindowRow, i64, Option<&str>, &Calendar),
    ) -> Result<bool, CoreError> {
        let deadline = std::time::Instant::now() + WINDOW_DEADLINE;
        let accounts = self
            .inner
            .handle
            .read(|db| accounts::list(db.connection()))?;
        let mut cut = false;
        for a in accounts {
            let (rows, colors) = self.inner.handle.read(|db| {
                let c = db.connection();
                let (rows, more) = store::in_window(c, a.id, from_ms, to_ms)?;
                cut |= more;
                let colors: HashMap<i64, Option<String>> = store::calendars(c, a.id)?
                    .into_iter()
                    .map(|k| (k.id, k.color))
                    .collect();
                Ok::<_, CoreError>((rows, colors))
            })?;
            for row in rows {
                if std::time::Instant::now() > deadline {
                    return Ok(true);
                }
                let Some(stored) = self
                    .inner
                    .handle
                    .read(|db| store::get(db.connection(), a.id, row.id))?
                else {
                    continue;
                };
                let Ok(cal) = ical::parse(&stored.object) else {
                    continue;
                };
                let color = colors.get(&row.calendar_id).cloned().flatten();
                visit(&row, a.id, color.as_deref(), &cal);
            }
        }
        Ok(cut)
    }
}

fn sort_occurrences(list: &mut [OccurrenceInfo]) {
    list.sort_by_key(|o| (o.start_ms, o.end_ms, o.event_id, o.key_ms));
}

/// The first occurrence of `cal` (the master's own start, else the first after it).
fn first_occurrence(cal: &Calendar, tz: &TimeZone) -> Option<Occurrence> {
    let events = cal.events();
    let master = events.iter().find(|e| !e.is_override)?;
    let start = Zones::of(cal).resolve(master.start.as_ref()?, tz).at;
    occurrence_at(cal, tz, start).or_else(|| {
        let to = start
            .checked_add(jiff::SignedDuration::from_millis(MAX_WINDOW_MS))
            .ok()?;
        occurrences(cal, tz, start, to)
            .occurrences
            .into_iter()
            .next()
    })
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
