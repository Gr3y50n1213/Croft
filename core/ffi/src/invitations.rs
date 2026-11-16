// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Invitations in mail (T40a): S22's card and S37's answer buttons. An invitation is whatever anyone mails, so:
//! nothing is sent or stored when a message is opened; an answer goes to the organiser only and carries only what
//! iTIP needs; the copy kept in the calendar has no alarms of the sender's choosing; a cancellation removes an event
//! only when it comes from the stored event's own organiser. Nothing of an invitation or an address is logged.

use std::sync::Arc;

use croft_engine::{NewMessage, SendError};
use croft_mail_core::compose::{self as mail, Address, CalendarPart, Outgoing};
use croft_mail_core::ical::expand::occurrence_at;
use croft_mail_core::ical::imip::{self, Answer, Cancelled, Method};
use croft_mail_core::ical::{self, write, Calendar, Event};
use croft_storage::calendar::{self as store, StoredEvent};
use croft_storage::messages::Message;
use croft_storage::reading;
use croft_storage::{accounts, identities};
use jiff::Timestamp;
use rusqlite::Connection;

use crate::calendar::{first_occurrence, person, zone_or_utc, EventPerson};
use crate::core::{now_ms, Core};
use crate::error::CoreError;
use crate::reading::ReadError;

/// Most attendees an [`InvitationInfo`] lists.
const MAX_LISTED: usize = 50;
/// Most clashes an [`InvitationInfo`] lists.
const MAX_CLASHES: usize = 3;
/// Longest text of a title or a place on the card.
const CARD_TEXT_CHARS: usize = 300;

/// What an invitation message is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum InvitationKind {
    /// An invitation to answer (new or changed).
    Request,
    /// An event to add, with nothing to answer.
    Publish,
    /// The organiser called it off.
    Cancel,
    /// Someone answered an invitation.
    Reply,
}

/// An answer to an invitation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum RsvpAnswer {
    /// Yes (`ACCEPTED`).
    Yes,
    /// Maybe (`TENTATIVE`).
    Maybe,
    /// No (`DECLINED`).
    No,
}

impl From<RsvpAnswer> for Answer {
    fn from(a: RsvpAnswer) -> Self {
        match a {
            RsvpAnswer::Yes => Answer::Accepted,
            RsvpAnswer::Maybe => Answer::Tentative,
            RsvpAnswer::No => Answer::Declined,
        }
    }
}

impl From<Answer> for RsvpAnswer {
    fn from(a: Answer) -> Self {
        match a {
            Answer::Accepted => RsvpAnswer::Yes,
            Answer::Tentative => RsvpAnswer::Maybe,
            Answer::Declined => RsvpAnswer::No,
        }
    }
}

/// One of the person's own events at the same time as an invitation.
#[derive(Clone, PartialEq, Eq, uniffi::Record)]
pub struct InvitationClash {
    /// The event.
    pub event_id: i64,
    /// Its title.
    pub title: String,
    /// When the occurrence starts, Unix ms.
    pub start_ms: i64,
    /// When it ends.
    pub end_ms: i64,
}

/// S22's card: what a message's invitation is. `Debug` prints no content.
#[derive(Clone, PartialEq, Eq, uniffi::Record)]
pub struct InvitationInfo {
    /// The message.
    pub message_id: i64,
    /// Its account.
    pub account: i64,
    /// What it is.
    pub kind: InvitationKind,
    /// The title (from the stored event when a cancellation names none).
    pub title: String,
    /// The place.
    pub location: String,
    /// The first occurrence's start, Unix ms (`None` when the message gives no time and nothing is stored).
    pub start_ms: Option<i64>,
    /// Its end.
    pub end_ms: Option<i64>,
    /// An all-day event.
    pub all_day: bool,
    /// It repeats.
    pub repeats: bool,
    /// Who organises it.
    pub organizer: Option<EventPerson>,
    /// The organiser is the message's sender. When not, the card says whose invitation it is: anyone can write any
    /// organiser.
    pub organizer_is_sender: bool,
    /// The person organises it (one of the account's addresses): nothing to answer.
    pub own_event: bool,
    /// Who is invited, at most 50.
    pub attendees: Vec<EventPerson>,
    /// How many are invited in all.
    pub attendee_count: u32,
    /// Which of the account's addresses is invited (`None`: the invitation came through a group address; an answer
    /// then adds the account's own address).
    pub invited_as: Option<String>,
    /// The person's answer so far (from the stored event when there is one).
    pub my_answer: Option<RsvpAnswer>,
    /// Up to three of the person's own events at the same time.
    pub clashes: Vec<InvitationClash>,
    /// The event as stored in one of the account's calendars.
    pub stored_event: Option<i64>,
    /// Its calendar.
    pub stored_calendar: Option<i64>,
    /// The stored event is newer than this message (a higher `SEQUENCE`): this message cannot be answered or applied.
    pub outdated: bool,
    /// The event in the person's calendar has another organiser (or is the person's own): this message cannot change
    /// it. Anyone can mail an update with any `UID` (T40a review).
    pub organizer_differs: bool,
    /// Yes, Maybe and No can be offered (for a `PUBLISH`: it can be added).
    pub can_respond: bool,
    /// A cancellation that can remove the stored event: from the stored event's own organiser, who is also the
    /// message's sender, and not the person.
    pub can_remove: bool,
    /// The cancellation calls off the whole event (else some occurrences).
    pub cancels_all: bool,
    /// For a reply: who answered, with their answer.
    pub replied_by: Option<EventPerson>,
}

impl std::fmt::Debug for InvitationInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InvitationInfo")
            .field("message_id", &self.message_id)
            .field("kind", &self.kind)
            .finish_non_exhaustive()
    }
}

impl std::fmt::Debug for InvitationClash {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InvitationClash")
            .field("event_id", &self.event_id)
            .finish_non_exhaustive()
    }
}

/// The words of a reply, in the person's language (the app writes them): its subject and a short text.
#[derive(Clone, PartialEq, Eq, uniffi::Record)]
pub struct ReplyWording {
    /// The subject (`Accepted: Lunch with Sam`). Empty: the core writes an English one.
    pub subject: String,
    /// The text part.
    pub body: String,
}

impl std::fmt::Debug for ReplyWording {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ReplyWording")
    }
}

/// Why an answer was not (wholly) carried out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum RsvpProblem {
    /// Not something to answer (not an invitation, or the person organises it).
    NotAnswerable,
    /// The stored event is newer than this message.
    Outdated,
    /// The stored event has another organiser (or is the person's own): this message cannot change it.
    OrganizerDiffers,
    /// The invitation names no usable organiser address.
    NoOrganizer,
    /// The account cannot send mail: the calendar was changed, no reply went.
    CannotSend,
    /// No calendar of the account can take the event (or the one chosen cannot): the reply went, nothing was added.
    NoCalendar,
}

/// What an answer did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Record)]
pub struct RsvpOutcome {
    /// The reply is in the Outbox.
    pub sent: bool,
    /// The event as stored now (added or updated), when it is.
    pub event_id: Option<i64>,
    /// What went wrong, if anything.
    pub problem: Option<RsvpProblem>,
}

/// A message's invitation, read and matched against the account.
struct Parsed {
    message: Message,
    invite: Calendar,
    method: Method,
    event: Event,
    own: Vec<String>,
    stored: Option<(StoredEvent, Calendar)>,
}

/// What may be done with a parsed invitation: one set of checks for the card, the answer and the cancellation, so they
/// never disagree (T40a review).
struct Assessment {
    organizer: String,
    own_event: bool,
    organizer_is_sender: bool,
    outdated: bool,
    organizer_differs: bool,
    can_respond: bool,
    can_remove: bool,
    cancelled: Option<Cancelled>,
}

fn assess(p: &Parsed) -> Assessment {
    let organizer = p
        .event
        .organizer
        .as_ref()
        .map(|o| o.address.clone())
        .or_else(|| p.stored.as_ref().and_then(|(_, c)| imip::organizer_of(c)))
        .unwrap_or_default();
    let is_own = |a: &str| p.own.iter().any(|o| imip::same_address(o, a));
    let own_event = is_own(&organizer);
    let organizer_is_sender = p
        .message
        .from_addr
        .as_deref()
        .is_some_and(|f| imip::same_address(f, &organizer));
    let outdated = p
        .stored
        .as_ref()
        .is_some_and(|(_, c)| imip::is_outdated(c, &p.invite));
    // A stored event can be changed only by its own organiser, and never when it is the person's own.
    let organizer_differs = p.stored.as_ref().is_some_and(|(_, c)| {
        let stored = imip::organizer_of(c).unwrap_or_default();
        !imip::same_address(&stored, &organizer) || is_own(&stored)
    });
    let usable = mail::is_valid_address(imip::bare_address(&organizer));
    let can_respond = match p.method {
        Method::Request => usable && !outdated && !own_event && !organizer_differs,
        Method::Publish => !own_event && p.stored.is_none(),
        Method::Cancel | Method::Reply => false,
    };
    let cancelled = (p.method == Method::Cancel)
        .then(|| imip::cancelled(&p.invite))
        .flatten();
    let can_remove = cancelled.is_some()
        && p.stored.is_some()
        && !outdated
        && !organizer_differs
        && !own_event
        && organizer_is_sender;
    Assessment {
        organizer,
        own_event,
        organizer_is_sender,
        outdated,
        organizer_differs,
        can_respond,
        can_remove,
        cancelled,
    }
}

/// The account's own addresses: its address and those of its identities.
pub(crate) fn own_addresses(conn: &Connection, account: i64) -> Result<Vec<String>, CoreError> {
    let acct = accounts::get(conn, account)?.ok_or(CoreError::NoSuchAccount)?;
    let mut out = vec![acct.email.clone()];
    // Read only: opening an invitation writes nothing (T40a review).
    for i in identities::list(conn, account)? {
        if !out.iter().any(|a| imip::same_address(a, &i.address)) {
            out.push(i.address);
        }
    }
    Ok(out)
}

fn clip(text: &str, max: usize) -> String {
    text.chars().filter(|c| !c.is_control()).take(max).collect()
}

/// The sender to answer as: the identity with the invited address, else the account's default one.
fn sender_for(
    conn: &Connection,
    account: i64,
    invited: Option<&str>,
) -> Result<(i64, Address), CoreError> {
    let acct = accounts::get(conn, account)?.ok_or(CoreError::NoSuchAccount)?;
    let all = identities::list_or_create(conn, account, &acct.label, &acct.email)?;
    let chosen = invited
        .and_then(|a| all.iter().find(|i| imip::same_address(&i.address, a)))
        .or_else(|| all.first())
        .ok_or(CoreError::NoSuchAccount)?;
    let name = chosen.display_name.trim();
    Ok((
        chosen.id,
        Address {
            name: (!name.is_empty()).then(|| name.to_owned()),
            address: chosen.address.clone(),
        },
    ))
}

impl Core {
    /// Reads message `message_id`'s invitation (fetching the message first when it is not stored).
    async fn read_invitation(&self, message_id: i64) -> Result<Option<Parsed>, ReadError> {
        let message = self
            .inner
            .resolve(&[message_id])?
            .into_iter()
            .next()
            .ok_or(ReadError::NoSuchMessage)?;
        let (account, id) = (message.account_id, message.id);
        self.inner.handle.fetch_body(account, id).await?;
        let handle = self.inner.handle.clone();
        let text = tokio::task::spawn_blocking(move || {
            let raw = handle
                .read(|db| reading::raw(db.connection(), account, id))?
                .ok_or(ReadError::Unreadable)?;
            croft_reader::calendar_part(&raw).map_err(|_| ReadError::Unreadable)
        })
        .await
        .map_err(|_| ReadError::Internal)??;
        let Some(text) = text else {
            return Ok(None);
        };
        let Ok(invite) = ical::parse(&text) else {
            return Ok(None);
        };
        let Some(method) = imip::method(&invite) else {
            return Ok(None);
        };
        let Some(event) = invite.events().into_iter().next() else {
            return Ok(None);
        };
        if event.uid.is_empty() {
            return Ok(None);
        }
        let (own, stored) = self.inner.handle.read(|db| {
            let c = db.connection();
            Ok::<_, CoreError>((
                own_addresses(c, account)?,
                store::find_by_uid(c, account, &event.uid)?,
            ))
        })?;
        let stored = stored.and_then(|s| ical::parse(&s.object).ok().map(|cal| (s, cal)));
        Ok(Some(Parsed {
            message,
            invite,
            method,
            event,
            own,
            stored,
        }))
    }

    /// The person's own events overlapping `[start, end)`, other than `skip`, not declined.
    fn clashes(
        &self,
        start_ms: i64,
        end_ms: i64,
        all_day: bool,
        skip: Option<i64>,
        zone: &str,
    ) -> Result<Vec<InvitationClash>, CoreError> {
        let window = self.events_between(start_ms, end_ms.max(start_ms + 1), zone.to_owned())?;
        let mut out = Vec::new();
        let mut own_by_account: std::collections::HashMap<i64, Vec<String>> =
            std::collections::HashMap::new();
        for o in window.occurrences.into_iter().take(40) {
            if out.len() >= MAX_CLASHES {
                break;
            }
            if Some(o.event_id) == skip || (o.all_day && !all_day) || o.end_ms <= start_ms {
                continue;
            }
            // Not one the person declined (each account's addresses read once).
            if let std::collections::hash_map::Entry::Vacant(e) = own_by_account.entry(o.account) {
                let own = self
                    .inner
                    .handle
                    .read(|db| own_addresses(db.connection(), o.account))
                    .unwrap_or_default();
                e.insert(own);
            }
            let own = own_by_account.get(&o.account).cloned().unwrap_or_default();
            let declined = self
                .event_object(o.account, o.event_id)
                .ok()
                .is_some_and(|(_, cal)| answer_in(&cal, &own) == Some(RsvpAnswer::No));
            if !declined {
                out.push(InvitationClash {
                    event_id: o.event_id,
                    title: o.title,
                    start_ms: o.start_ms,
                    end_ms: o.end_ms,
                });
            }
        }
        Ok(out)
    }
}

/// The person's answer in `cal`, as attendee `own`.
pub(crate) fn answer_in(cal: &Calendar, own: &[String]) -> Option<RsvpAnswer> {
    let ev = cal.events().into_iter().next()?;
    let mine = ev
        .attendees
        .iter()
        .find(|p| own.iter().any(|a| imip::same_address(&p.address, a)))?;
    Answer::of(mine.status.as_deref()).map(RsvpAnswer::from)
}

#[uniffi::export]
impl Core {
    /// S22: the invitation message `message_id` carries, seen from `zone`, or `None` when it carries none. Reads only:
    /// nothing is sent or stored. Fetches the message when it is not stored yet.
    pub async fn invitation(
        self: Arc<Self>,
        message_id: i64,
        zone: String,
    ) -> Result<Option<InvitationInfo>, ReadError> {
        let core = self.clone();
        self.spawn(
            async move { core.invitation_now(message_id, zone).await },
            false,
        )
        .await
        .map_err(|_| ReadError::Internal)?
    }

    /// S22: answers the invitation in message `message_id` (only ever on the person's tap). Yes and Maybe add the event
    /// to `calendar` (a writable calendar of the message's account; `None`: its first shown one) or update it where it
    /// is stored; No updates a stored event and adds nothing. The reply goes to the organiser through the Outbox. A
    /// `PUBLISH` (nothing to answer) is only added, with Yes or Maybe.
    pub async fn respond_to_invitation(
        self: Arc<Self>,
        message_id: i64,
        answer: RsvpAnswer,
        calendar: Option<i64>,
        wording: ReplyWording,
    ) -> Result<RsvpOutcome, ReadError> {
        let core = self.clone();
        self.spawn(
            async move {
                core.respond_now(message_id, answer, calendar, wording)
                    .await
            },
            false,
        )
        .await
        .map_err(|_| ReadError::Internal)?
    }

    /// S22: applies the cancellation in message `message_id`: removes the stored event (or the cancelled occurrences),
    /// only when the cancellation's organiser is the stored event's. Returns whether anything was removed.
    pub async fn apply_cancellation(self: Arc<Self>, message_id: i64) -> Result<bool, ReadError> {
        let core = self.clone();
        self.spawn(async move { core.cancel_now(message_id).await }, false)
            .await
            .map_err(|_| ReadError::Internal)?
    }
}

impl Core {
    async fn invitation_now(
        &self,
        message_id: i64,
        zone: String,
    ) -> Result<Option<InvitationInfo>, ReadError> {
        let Some(p) = self.read_invitation(message_id).await? else {
            return Ok(None);
        };
        let tz = zone_or_utc(&zone);
        let account = p.message.account_id;
        let kind = match p.method {
            Method::Request => InvitationKind::Request,
            Method::Publish => InvitationKind::Publish,
            Method::Cancel => InvitationKind::Cancel,
            Method::Reply => InvitationKind::Reply,
        };
        // The details: from the message, else (a bare cancellation) from the stored event.
        let detail_source = if p.event.start.is_none() {
            p.stored.as_ref().map_or(&p.invite, |(_, c)| c)
        } else {
            &p.invite
        };
        let detail = detail_source
            .events()
            .into_iter()
            .next()
            .unwrap_or_default();
        let occ = first_occurrence(detail_source, &tz);
        let organizer = p.event.organizer.as_ref().or(detail.organizer.as_ref());
        let a = assess(&p);
        let invited_as = imip::own_address(&p.event, &p.own);
        let my_answer = match &p.stored {
            Some((_, c)) => answer_in(c, &p.own),
            None => answer_in(&p.invite, &p.own),
        };
        let clashes = match (&occ, p.method) {
            (Some(o), Method::Request | Method::Publish) => self.clashes(
                o.start.as_millisecond(),
                o.end.as_millisecond(),
                o.all_day,
                p.stored.as_ref().map(|(s, _)| s.id),
                &zone,
            )?,
            _ => vec![],
        };
        let replied_by = (p.method == Method::Reply)
            .then(|| p.event.attendees.first().map(person))
            .flatten();
        Ok(Some(InvitationInfo {
            message_id: p.message.id,
            account,
            kind,
            title: clip(
                detail.summary.as_deref().unwrap_or_default(),
                CARD_TEXT_CHARS,
            ),
            location: clip(
                detail.location.as_deref().unwrap_or_default(),
                CARD_TEXT_CHARS,
            ),
            start_ms: occ.as_ref().map(|o| o.start.as_millisecond()),
            end_ms: occ.as_ref().map(|o| o.end.as_millisecond()),
            all_day: occ.as_ref().is_some_and(|o| o.all_day),
            repeats: detail.rrule.is_some() || !detail.rdates.is_empty(),
            organizer: organizer.map(person),
            organizer_is_sender: a.organizer_is_sender,
            own_event: a.own_event,
            attendee_count: u32::try_from(p.event.attendees.len()).unwrap_or(u32::MAX),
            attendees: p
                .event
                .attendees
                .iter()
                .take(MAX_LISTED)
                .map(person)
                .collect(),
            invited_as,
            my_answer,
            clashes,
            stored_event: p.stored.as_ref().map(|(s, _)| s.id),
            stored_calendar: p.stored.as_ref().map(|(s, _)| s.calendar_id),
            outdated: a.outdated,
            organizer_differs: a.organizer_differs,
            can_respond: a.can_respond,
            can_remove: a.can_remove,
            cancels_all: matches!(a.cancelled, Some(Cancelled::All)),
            replied_by,
        }))
    }

    async fn respond_now(
        &self,
        message_id: i64,
        answer: RsvpAnswer,
        calendar: Option<i64>,
        wording: ReplyWording,
    ) -> Result<RsvpOutcome, ReadError> {
        let Some(p) = self.read_invitation(message_id).await? else {
            return Ok(outcome(false, None, Some(RsvpProblem::NotAnswerable)));
        };
        let account = p.message.account_id;
        let a = assess(&p);
        if !a.can_respond {
            let why = if a.outdated {
                RsvpProblem::Outdated
            } else if a.organizer_differs {
                RsvpProblem::OrganizerDiffers
            } else if p.method == Method::Request
                && !a.own_event
                && !mail::is_valid_address(imip::bare_address(&a.organizer))
            {
                RsvpProblem::NoOrganizer
            } else {
                RsvpProblem::NotAnswerable
            };
            let stored = p.stored.as_ref().map(|(s, _)| s.id);
            return Ok(outcome(false, stored, Some(why)));
        }
        if p.method == Method::Publish {
            if answer == RsvpAnswer::No {
                return Ok(outcome(false, None, None));
            }
            // No attendees kept: a server that schedules has no one to mail (T40a review).
            let object = imip::stored_copy(&p.invite, false).to_text();
            let added = self.add_to_calendar(account, calendar, &object)?;
            return Ok(outcome(
                false,
                added,
                added.is_none().then_some(RsvpProblem::NoCalendar),
            ));
        }
        let organizer = imip::bare_address(&a.organizer).to_owned();
        let invited = imip::own_address(&p.event, &p.own);
        let (identity, from) = self
            .inner
            .handle
            .read(|db| sender_for(db.connection(), account, invited.as_deref()))?;
        let me = invited.unwrap_or_else(|| from.address.clone());
        let ans: Answer = answer.into();
        // The reply first: when none can be written, nothing is changed (T40a review).
        let Some(reply) = imip::reply(&p.invite, &me, ans, Timestamp::now()) else {
            return Ok(outcome(false, None, Some(RsvpProblem::NoOrganizer)));
        };

        // Then the calendar: the person's answer is theirs whatever happens to the mail.
        let mut problem = None;
        let event_id = match &p.stored {
            Some((s, stored_cal)) => {
                let newer = !imip::has_master(&p.invite)
                    || imip::sequence_of(&p.invite) > imip::sequence_of(stored_cal);
                let next = if newer {
                    // A newer version (or an update of some occurrences): merged, the person's own reminders kept.
                    let mut next = imip::merge_update(stored_cal, &p.invite, &me, ans);
                    if imip::has_master(&p.invite) {
                        keep_reminders(stored_cal, &mut next);
                    }
                    next
                } else {
                    let mut next = stored_cal.clone();
                    imip::answer_as(&mut next, &me, ans);
                    next
                };
                let text = next.to_text();
                let updated = self.inner.handle.read(|db| {
                    store::update_local(db.connection(), account, s.id, &text, now_ms())
                })?;
                updated.then_some(s.id)
            }
            None if answer == RsvpAnswer::No => None,
            None => {
                let object = imip::for_calendar(&p.invite, &me, ans).to_text();
                let added = self.add_to_calendar(account, calendar, &object)?;
                if added.is_none() {
                    problem = Some(RsvpProblem::NoCalendar);
                }
                added
            }
        };
        let sent = self.send_reply(
            account,
            identity,
            &from,
            &organizer,
            &reply,
            ans,
            &wording,
            p.message.message_id_hdr.as_deref(),
            p.event.summary.as_deref().unwrap_or_default(),
        )?;
        if !sent {
            problem = Some(RsvpProblem::CannotSend);
        }
        Ok(outcome(sent, event_id, problem))
    }

    async fn cancel_now(&self, message_id: i64) -> Result<bool, ReadError> {
        let Some(p) = self.read_invitation(message_id).await? else {
            return Ok(false);
        };
        let a = assess(&p);
        let (true, Some((stored, cal))) = (a.can_remove, &p.stored) else {
            return Ok(false);
        };
        let account = p.message.account_id;
        match a.cancelled {
            Some(Cancelled::All) => Ok(self
                .inner
                .handle
                .read(|db| store::delete_local(db.connection(), account, stored.id))?),
            Some(Cancelled::Occurrences(rids)) => {
                let mut next = cal.clone();
                let mut changed = false;
                let zones = ical::zone::Zones::of(cal);
                for rid in &rids {
                    // Only a real occurrence of the stored series (T40a review: made-up ones would only add EXDATEs).
                    let at = zones.resolve(rid, &jiff::tz::TimeZone::UTC).at;
                    if occurrence_at(cal, &jiff::tz::TimeZone::UTC, at).is_none() {
                        continue;
                    }
                    changed |= write::delete_one(&mut next, rid, Timestamp::now());
                }
                if !changed {
                    return Ok(false);
                }
                let text = next.to_text();
                Ok(self.inner.handle.read(|db| {
                    store::update_local(db.connection(), account, stored.id, &text, now_ms())
                })?)
            }
            None => Ok(false),
        }
    }
}

#[uniffi::export]
impl Core {
    /// S37: answers event `event_id` of `account`, where the person is invited (not the organiser): the answer is
    /// changed in the calendar and mailed to the organiser.
    pub fn respond_to_event(
        &self,
        account: i64,
        event_id: i64,
        answer: RsvpAnswer,
        wording: ReplyWording,
    ) -> Result<RsvpOutcome, CoreError> {
        let (stored, cal) = self.event_object(account, event_id)?;
        let (own, read_only) = self.inner.handle.read(|db| {
            let c = db.connection();
            Ok::<_, CoreError>((
                own_addresses(c, account)?,
                store::is_read_only_event(c, account, event_id)?,
            ))
        })?;
        let Some(ev) = cal.events().into_iter().next() else {
            return Ok(outcome(false, None, Some(RsvpProblem::NotAnswerable)));
        };
        let organizer = ev
            .organizer
            .as_ref()
            .map(|o| o.address.clone())
            .unwrap_or_default();
        let Some(me) = imip::own_address(&ev, &own) else {
            return Ok(outcome(false, None, Some(RsvpProblem::NotAnswerable)));
        };
        if read_only || own.iter().any(|a| imip::same_address(a, &organizer)) {
            return Ok(outcome(false, None, Some(RsvpProblem::NotAnswerable)));
        }
        if !mail::is_valid_address(imip::bare_address(&organizer)) {
            return Ok(outcome(false, None, Some(RsvpProblem::NoOrganizer)));
        }
        let ans: Answer = answer.into();
        let Some(reply) = imip::reply(&cal, &me, ans, Timestamp::now()) else {
            return Ok(outcome(false, None, Some(RsvpProblem::NoOrganizer)));
        };
        let mut next = cal.clone();
        imip::answer_as(&mut next, &me, ans);
        let text = next.to_text();
        if !self
            .inner
            .handle
            .read(|db| store::update_local(db.connection(), account, stored.id, &text, now_ms()))?
        {
            return Ok(outcome(false, None, Some(RsvpProblem::NotAnswerable)));
        }
        let (identity, from) = self
            .inner
            .handle
            .read(|db| sender_for(db.connection(), account, Some(&me)))?;
        let sent = self.send_reply(
            account,
            identity,
            &from,
            imip::bare_address(&organizer),
            &reply,
            ans,
            &wording,
            None,
            ev.summary.as_deref().unwrap_or_default(),
        )?;
        Ok(outcome(
            sent,
            Some(stored.id),
            (!sent).then_some(RsvpProblem::CannotSend),
        ))
    }
}

fn outcome(sent: bool, event_id: Option<i64>, problem: Option<RsvpProblem>) -> RsvpOutcome {
    RsvpOutcome {
        sent,
        event_id,
        problem,
    }
}

/// Copies the `VALARM`s of `from`'s master event to `to`'s (the person's own reminders survive a newer version).
fn keep_reminders(from: &Calendar, to: &mut Calendar) {
    let alarms: Vec<ical::Component> = from
        .root
        .children
        .iter()
        .find(|c| c.kind == "VEVENT")
        .map(|c| {
            c.children
                .iter()
                .filter(|a| a.kind == "VALARM")
                .cloned()
                .collect()
        })
        .unwrap_or_default();
    if let Some(m) = to.master_mut() {
        m.children.retain(|a| a.kind != "VALARM");
        m.children.extend(alarms);
    }
}

impl Core {
    /// Adds `object` to `calendar` of `account` (it must be one of the account's own writable calendars), or to the
    /// first shown one. `None` when there is none.
    fn add_to_calendar(
        &self,
        account: i64,
        calendar: Option<i64>,
        object: &str,
    ) -> Result<Option<i64>, CoreError> {
        self.inner.handle.read(|db| {
            let c = db.connection();
            let writable: Vec<store::CalendarRow> = store::calendars(c, account)?
                .into_iter()
                .filter(|k| k.source_url.is_none() && !k.read_only)
                .collect();
            let target = match calendar {
                Some(id) => writable.iter().find(|k| k.id == id),
                None => writable.iter().find(|k| k.shown).or(writable.first()),
            };
            match target {
                Some(k) => Ok::<_, CoreError>(Some(store::create_local(
                    c,
                    account,
                    k.id,
                    object,
                    now_ms(),
                )?)),
                None => Ok(None),
            }
        })
    }

    /// Puts the iTIP reply in the Outbox: from `from` to `organizer` only. False when the account cannot send.
    #[allow(clippy::too_many_arguments)]
    fn send_reply(
        &self,
        account: i64,
        identity: i64,
        from: &Address,
        organizer: &str,
        reply: &Calendar,
        answer: Answer,
        wording: &ReplyWording,
        in_reply_to: Option<&str>,
        title: &str,
    ) -> Result<bool, CoreError> {
        let to = [Address::bare(organizer)];
        let mut subject = clip(wording.subject.trim(), CARD_TEXT_CHARS);
        if subject.is_empty() {
            let word = match answer {
                Answer::Accepted => "Accepted",
                Answer::Tentative => "Tentative",
                Answer::Declined => "Declined",
            };
            subject = format!("{word}: {}", clip(title, CARD_TEXT_CHARS));
        }
        let body: String = wording.body.chars().take(4000).collect();
        let mut seed = [0u8; 16];
        getrandom::fill(&mut seed).map_err(|_| CoreError::Internal)?;
        let mut boundary_seed = [0u8; 8];
        getrandom::fill(&mut boundary_seed).map_err(|_| CoreError::Internal)?;
        let message_id = mail::new_message_id(seed, &from.address);
        let references: Vec<String> = in_reply_to.map(str::to_owned).into_iter().collect();
        let reply_text = reply.to_text();
        let mime = mail::build_message(&Outgoing {
            from,
            to: &to,
            cc: &[],
            bcc: &[],
            subject: &subject,
            body: &body,
            in_reply_to,
            references: &references,
            message_id: &message_id,
            date_ms: now_ms(),
            as_draft: false,
            attachments: &[],
            calendar: Some(CalendarPart {
                method: "REPLY",
                text: &reply_text,
            }),
            boundary_seed,
        })
        .map_err(|_| CoreError::Internal)?;
        let message = NewMessage {
            account,
            identity: Some(identity),
            draft: None,
            envelope_from: from.address.clone(),
            recipients: vec![organizer.to_owned()],
        };
        match self.inner.handle.send(message, mime) {
            Ok(_) => Ok(true),
            Err(SendError::NoSubmitter(_) | SendError::NoOutbox | SendError::UnknownAccount(_)) => {
                Ok(false)
            }
            Err(_) => Err(CoreError::Internal),
        }
    }
}
