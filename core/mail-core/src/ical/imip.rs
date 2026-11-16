// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Invitations in mail (T40a, iTIP over iMIP: RFC 5546, RFC 6047). Anyone can mail an invitation, so nothing here
//! trusts it: the reply carries only what iTIP needs (no description, no other attendees), and the copy kept in the
//! person's calendar has no alarms of the sender's choosing. Pure: time is passed in.

use jiff::Timestamp;

use super::write::{utc_text, PRODID};
use super::{make, param_text, parse_line, Calendar, Component, Event, Line, When};

/// What an invitation message asks (its `METHOD`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    /// An invitation, new or changed.
    Request,
    /// An event to add, with nothing to answer.
    Publish,
    /// The organiser called it off (all of it, or some occurrences).
    Cancel,
    /// Someone answered an invitation.
    Reply,
}

/// The object's `METHOD`, when it is one Croft handles and the object has an event.
pub fn method(cal: &Calendar) -> Option<Method> {
    if !cal.has_events() {
        return None;
    }
    let m = cal.root.line("METHOD")?.value.trim().to_ascii_uppercase();
    match m.as_str() {
        "REQUEST" => Some(Method::Request),
        "PUBLISH" => Some(Method::Publish),
        "CANCEL" => Some(Method::Cancel),
        "REPLY" => Some(Method::Reply),
        _ => None,
    }
}

/// An attendee's answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Answer {
    /// Yes.
    Accepted,
    /// Maybe.
    Tentative,
    /// No.
    Declined,
}

impl Answer {
    /// The `PARTSTAT` value.
    pub fn partstat(self) -> &'static str {
        match self {
            Answer::Accepted => "ACCEPTED",
            Answer::Tentative => "TENTATIVE",
            Answer::Declined => "DECLINED",
        }
    }

    /// The answer a `PARTSTAT` gives (`None` for `NEEDS-ACTION` and anything else).
    pub fn of(partstat: Option<&str>) -> Option<Answer> {
        match partstat?.trim().to_ascii_uppercase().as_str() {
            "ACCEPTED" => Some(Answer::Accepted),
            "TENTATIVE" => Some(Answer::Tentative),
            "DECLINED" => Some(Answer::Declined),
            _ => None,
        }
    }
}

/// The bare address of a `mailto:` value (or of a bare address), trimmed.
pub fn bare_address(value: &str) -> &str {
    let v = value.trim();
    match v.get(..7) {
        Some(p) if p.eq_ignore_ascii_case("mailto:") => v.get(7..).unwrap_or("").trim(),
        _ => v,
    }
}

/// Whether two addresses are the same mailbox (compared without regard to case).
pub fn same_address(a: &str, b: &str) -> bool {
    let (a, b) = (bare_address(a), bare_address(b));
    !a.is_empty() && a.eq_ignore_ascii_case(b)
}

/// Which of `own` (the account's addresses) is invited to `event`: the first attendee whose address is one of them.
/// Returns the address as `own` spells it.
pub fn own_address(event: &Event, own: &[String]) -> Option<String> {
    event.attendees.iter().find_map(|p| {
        own.iter()
            .find(|o| same_address(&p.address, o))
            .map(|o| bare_address(o).to_owned())
    })
}

/// The event an invitation is about: its first `VEVENT` (the master when there is one).
fn first_event(cal: &Calendar) -> Option<&Component> {
    let events = || cal.root.children.iter().filter(|c| c.kind == "VEVENT");
    events()
        .find(|c| c.line("RECURRENCE-ID").is_none())
        .or_else(|| events().next())
}

/// The iTIP reply to `invite` (RFC 5546 3.2.3): `METHOD:REPLY` and one `VEVENT` with the invitation's `UID`,
/// `SEQUENCE` and `RECURRENCE-ID` (when it has one), the `ORGANIZER` as an address only, the person as the one
/// `ATTENDEE` with `answer`, and `DTSTAMP`. Nothing else of the invitation is carried back. `None` when the invitation
/// has no event, no `UID` or no organiser address.
pub fn reply(
    invite: &Calendar,
    attendee: &str,
    answer: Answer,
    now: Timestamp,
) -> Option<Calendar> {
    let source = first_event(invite)?;
    let uid = source.line("UID")?.value.trim();
    if uid.is_empty() || uid.len() > 500 {
        return None;
    }
    let organizer = bare_address(&source.line("ORGANIZER")?.value).to_owned();
    if organizer.is_empty() || organizer.len() > 320 || attendee.trim().is_empty() {
        return None;
    }
    let sequence = source
        .line("SEQUENCE")
        .and_then(|l| l.value.trim().parse::<u32>().ok())
        .unwrap_or(0);
    let mut lines = vec![
        make("UID", &[], uid),
        make("DTSTAMP", &[], &utc_text(now)),
        make("SEQUENCE", &[], &sequence.to_string()),
    ];
    // RECURRENCE-ID again as the invitation wrote it (a date, a time in a zone or UTC): rebuilt from what was read.
    if let Some(rid) = source.line("RECURRENCE-ID").and_then(super::read_when) {
        lines.push(super::write::when_line("RECURRENCE-ID", &rid));
    }
    lines.push(make("ORGANIZER", &[], &format!("mailto:{organizer}")));
    lines.push(make(
        "ATTENDEE",
        &[("PARTSTAT", answer.partstat())],
        &format!("mailto:{}", bare_address(attendee)),
    ));
    Some(Calendar {
        root: Component {
            kind: "VCALENDAR".into(),
            lines: vec![
                make("VERSION", &[], "2.0"),
                make("PRODID", &[], PRODID),
                make("METHOD", &[], "REPLY"),
            ],
            children: vec![Component {
                kind: "VEVENT".into(),
                lines,
                children: vec![],
            }],
        },
    })
}

/// A copy of `line` with parameter `name` set to `value` (or removed with `None`), the rest kept.
fn with_param(line: &Line, name: &str, value: Option<&str>) -> Line {
    let mut raw = line.name.clone();
    let mut done = false;
    for (n, vs) in &line.params {
        if n == name {
            if let (Some(v), false) = (value, done) {
                raw.push_str(&format!(";{n}={}", param_text(v)));
                done = true;
            }
            continue;
        }
        let joined: Vec<String> = vs.iter().map(|v| param_text(v)).collect();
        raw.push_str(&format!(";{n}={}", joined.join(",")));
    }
    if let (Some(v), false) = (value, done) {
        raw.push_str(&format!(";{name}={}", param_text(v)));
    }
    raw.push(':');
    raw.push_str(&line.value);
    parse_line(&raw).unwrap_or_else(|| line.clone())
}

/// Sets the person's answer on every `VEVENT` of `cal` where `attendee` is an `ATTENDEE` (and drops `RSVP=TRUE`
/// there, as answered). Returns whether a line changed.
pub fn set_own_answer(cal: &mut Calendar, attendee: &str, answer: Answer) -> bool {
    let mut changed = false;
    for c in cal.root.children.iter_mut().filter(|c| c.kind == "VEVENT") {
        for l in c.lines.iter_mut() {
            if l.name == "ATTENDEE" && same_address(&l.value, attendee) {
                let next = with_param(
                    &with_param(l, "PARTSTAT", Some(answer.partstat())),
                    "RSVP",
                    None,
                );
                if next.raw != l.raw {
                    *l = next;
                    changed = true;
                }
            }
        }
    }
    changed
}

/// Sets the person's answer, adding them as an `ATTENDEE` of every `VEVENT` when they are not on the list (an
/// invitation sent to a group address). Returns whether the object changed.
pub fn answer_as(cal: &mut Calendar, attendee: &str, answer: Answer) -> bool {
    if set_own_answer(cal, attendee, answer) {
        return true;
    }
    let listed = cal
        .root
        .children
        .iter()
        .filter(|c| c.kind == "VEVENT")
        .any(|c| {
            c.lines_named("ATTENDEE")
                .any(|l| same_address(&l.value, attendee))
        });
    if listed {
        return false;
    }
    let line = make(
        "ATTENDEE",
        &[("PARTSTAT", answer.partstat())],
        &format!("mailto:{}", bare_address(attendee)),
    );
    for c in cal.root.children.iter_mut().filter(|c| c.kind == "VEVENT") {
        c.lines.push(line.clone());
    }
    true
}

/// The object to keep in the person's calendar from an invitation: the invitation without its `METHOD` (a stored
/// object has none) and without its `VALARM`s (a stranger does not choose when the phone notifies), `ORGANIZER` marked
/// `SCHEDULE-AGENT=CLIENT` (Croft mails the replies, so a server that schedules does not send a second one), and the
/// person's answer set.
pub fn for_calendar(invite: &Calendar, attendee: &str, answer: Answer) -> Calendar {
    let mut cal = stored_copy(invite, true);
    answer_as(&mut cal, attendee, answer);
    cal
}

/// The object to keep from an invitation or a published event, before any answer (T40a review):
/// - only the `VTIMEZONE`s and the `VEVENT`s of the invitation's own `UID` (no `VTODO`, no other event);
/// - no `METHOD`, and no `VALARM` anywhere;
/// - `ORGANIZER` and every `ATTENDEE` marked `SCHEDULE-AGENT=CLIENT`, so a server that schedules never mails anyone on
///   the person's behalf;
/// - without `keep_attendees` (a `PUBLISH`, which has none by RFC 5546), no `ATTENDEE` at all.
pub fn stored_copy(invite: &Calendar, keep_attendees: bool) -> Calendar {
    let mut cal = invite.clone();
    cal.root.set_lines("METHOD", vec![]);
    let uid = first_event(invite)
        .and_then(|c| c.line("UID"))
        .map(|l| l.value.trim().to_owned())
        .unwrap_or_default();
    cal.root.children.retain(|c| {
        c.kind == "VTIMEZONE"
            || (c.kind == "VEVENT" && c.line("UID").is_some_and(|l| l.value.trim() == uid))
    });
    for c in cal.root.children.iter_mut().filter(|c| c.kind == "VEVENT") {
        c.children.clear();
        if !keep_attendees {
            c.set_lines("ATTENDEE", vec![]);
        }
        for l in c
            .lines
            .iter_mut()
            .filter(|l| l.name == "ORGANIZER" || l.name == "ATTENDEE")
        {
            *l = with_param(l, "SCHEDULE-AGENT", Some("CLIENT"));
        }
    }
    cal
}

/// Whether the object has a master event (a `VEVENT` without `RECURRENCE-ID`).
pub fn has_master(cal: &Calendar) -> bool {
    cal.root
        .children
        .iter()
        .any(|c| c.kind == "VEVENT" && c.line("RECURRENCE-ID").is_none())
}

/// A newer invitation applied to the stored object (T40a review): one with a master event replaces it (the caller keeps
/// the person's own reminders); one for some occurrences only (no master) replaces or adds just those overrides and
/// leaves the series and its other overrides as they are. The person's answer is set on what the invitation covers.
pub fn merge_update(
    stored: &Calendar,
    update: &Calendar,
    attendee: &str,
    answer: Answer,
) -> Calendar {
    if has_master(update) || !has_master(stored) {
        return for_calendar(update, attendee, answer);
    }
    let incoming = for_calendar(update, attendee, answer);
    let zones = super::zone::Zones::of(stored);
    let mut out = stored.clone();
    for c in incoming.root.children.iter() {
        match c.kind.as_str() {
            "VEVENT" => {
                let Some(rid) = c.line("RECURRENCE-ID").and_then(super::read_when) else {
                    continue;
                };
                out.root.children.retain(|o| {
                    !(o.kind == "VEVENT"
                        && o.line("RECURRENCE-ID")
                            .and_then(super::read_when)
                            .is_some_and(|w| super::write::same_moment(&zones, &w, &rid)))
                });
                out.root.children.push(c.clone());
            }
            "VTIMEZONE" => {
                let tzid = c.line("TZID").map(|l| l.value.clone());
                let known = out.root.children.iter().any(|o| {
                    o.kind == "VTIMEZONE" && o.line("TZID").map(|l| l.value.clone()) == tzid
                });
                if !known {
                    out.root.children.insert(0, c.clone());
                }
            }
            _ => {}
        }
    }
    out
}

/// Whether `update` is older than what is stored (T40a review): its `SEQUENCE` against the stored master's, or, for an
/// update of one occurrence, against that stored occurrence's when there is one.
pub fn is_outdated(stored: &Calendar, update: &Calendar) -> bool {
    let seq = |c: &Component| {
        c.line("SEQUENCE")
            .and_then(|l| l.value.trim().parse::<u32>().ok())
            .unwrap_or(0)
    };
    let Some(incoming) = first_event(update) else {
        return false;
    };
    let rid = incoming.line("RECURRENCE-ID").and_then(super::read_when);
    let zones = super::zone::Zones::of(stored);
    let same = rid.as_ref().and_then(|r| {
        stored.root.children.iter().find(|o| {
            o.kind == "VEVENT"
                && o.line("RECURRENCE-ID")
                    .and_then(super::read_when)
                    .is_some_and(|w| super::write::same_moment(&zones, &w, r))
        })
    });
    match same.or_else(|| first_event(stored)) {
        Some(existing) => seq(incoming) < seq(existing),
        None => false,
    }
}

/// Most occurrences one cancellation removes.
pub const MAX_CANCELLED: usize = 50;

/// What a cancellation calls off.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Cancelled {
    /// The whole event (a `VEVENT` without `RECURRENCE-ID`).
    All,
    /// These occurrences only.
    Occurrences(Vec<When>),
}

/// What `cancel` (a `METHOD:CANCEL` object) calls off.
pub fn cancelled(cancel: &Calendar) -> Option<Cancelled> {
    let events = cancel.events();
    if events.is_empty() {
        return None;
    }
    if events.iter().any(|e| !e.is_override) {
        return Some(Cancelled::All);
    }
    // At most 50 occurrences from one message (T40a review).
    let rids: Vec<When> = events
        .into_iter()
        .filter_map(|e| e.recurrence_id)
        .take(MAX_CANCELLED)
        .collect();
    (!rids.is_empty()).then_some(Cancelled::Occurrences(rids))
}

/// The organiser's address of the object's first event.
pub fn organizer_of(cal: &Calendar) -> Option<String> {
    first_event(cal)?
        .line("ORGANIZER")
        .map(|l| bare_address(&l.value).to_owned())
        .filter(|a| !a.is_empty())
}

/// The `SEQUENCE` of the object's first event (0 when none).
pub fn sequence_of(cal: &Calendar) -> u32 {
    first_event(cal)
        .and_then(|c| c.line("SEQUENCE"))
        .and_then(|l| l.value.trim().parse().ok())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

    use super::*;
    use crate::ical::parse;

    const INVITE: &str = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//Example//EN\r\nMETHOD:REQUEST\r\nBEGIN:VEVENT\r\nUID:lunch-1@example.org\r\nSEQUENCE:2\r\nDTSTAMP:20261101T090000Z\r\nDTSTART:20261106T120000Z\r\nDTEND:20261106T130000Z\r\nSUMMARY:Lunch with Sam\r\nDESCRIPTION:Secret agenda\r\nLOCATION:The Mill\r\nORGANIZER;CN=Sam Rivera:mailto:sam@example.org\r\nATTENDEE;CN=Sam Rivera;PARTSTAT=ACCEPTED:mailto:sam@example.org\r\nATTENDEE;CN=Mira;PARTSTAT=NEEDS-ACTION;RSVP=TRUE:mailto:mira@example.net\r\nATTENDEE;PARTSTAT=NEEDS-ACTION;RSVP=TRUE;ROLE=REQ-PARTICIPANT:MAILTO:Me@Example.COM\r\nBEGIN:VALARM\r\nACTION:DISPLAY\r\nTRIGGER:-PT1M\r\nDESCRIPTION:Click this\r\nEND:VALARM\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";

    fn now() -> Timestamp {
        "2026-11-16T10:00:00Z".parse().unwrap()
    }

    #[test]
    fn the_kind_comes_from_method_and_only_known_ones_count() {
        let cal = parse(INVITE).unwrap();
        assert_eq!(method(&cal), Some(Method::Request));
        for (m, want) in [
            ("PUBLISH", Some(Method::Publish)),
            ("cancel", Some(Method::Cancel)),
            ("REPLY", Some(Method::Reply)),
            ("COUNTER", None),
        ] {
            let c = parse(&INVITE.replace("METHOD:REQUEST", &format!("METHOD:{m}"))).unwrap();
            assert_eq!(method(&c), want, "{m}");
        }
        assert_eq!(
            method(&parse(&INVITE.replace("METHOD:REQUEST\r\n", "")).unwrap()),
            None
        );
        let no_event = "BEGIN:VCALENDAR\r\nMETHOD:REQUEST\r\nEND:VCALENDAR\r\n";
        assert_eq!(method(&parse(no_event).unwrap()), None);
    }

    #[test]
    fn the_persons_own_line_is_found_by_any_of_their_addresses_without_regard_to_case() {
        let cal = parse(INVITE).unwrap();
        let ev = &cal.events()[0];
        let own = vec!["other@example.com".to_owned(), "me@example.com".to_owned()];
        assert_eq!(own_address(ev, &own).as_deref(), Some("me@example.com"));
        assert_eq!(own_address(ev, &["nobody@example.com".to_owned()]), None);
        assert!(same_address("MAILTO:a@B.org", "a@b.org"));
        assert!(!same_address("", ""));
    }

    #[test]
    fn the_reply_carries_only_what_itip_needs() {
        let cal = parse(INVITE).unwrap();
        let r = reply(&cal, "me@example.com", Answer::Tentative, now()).unwrap();
        let text = r.to_text();
        assert_eq!(
            text,
            "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//Croft//Croft//EN\r\nMETHOD:REPLY\r\nBEGIN:VEVENT\r\nUID:lunch-1@example.org\r\nDTSTAMP:20261116T100000Z\r\nSEQUENCE:2\r\nORGANIZER:mailto:sam@example.org\r\nATTENDEE;PARTSTAT=TENTATIVE:mailto:me@example.com\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n"
        );
        for never in ["Secret", "mira", "The Mill", "VALARM", "Lunch"] {
            assert!(!text.contains(never), "{never}");
        }
        // One occurrence: its RECURRENCE-ID goes back.
        let one = INVITE.replace(
            "SEQUENCE:2\r\n",
            "SEQUENCE:2\r\nRECURRENCE-ID;TZID=Europe/London:20261113T120000\r\n",
        );
        let r = reply(
            &parse(&one).unwrap(),
            "me@example.com",
            Answer::Declined,
            now(),
        )
        .unwrap();
        assert!(r
            .to_text()
            .contains("RECURRENCE-ID;TZID=Europe/London:20261113T120000\r\n"));
        // No organiser, no reply.
        let orphan = INVITE.replace("ORGANIZER;CN=Sam Rivera:mailto:sam@example.org\r\n", "");
        assert!(reply(
            &parse(&orphan).unwrap(),
            "me@example.com",
            Answer::Accepted,
            now()
        )
        .is_none());
    }

    #[test]
    fn the_calendar_copy_has_no_method_no_alarm_client_scheduling_and_the_answer() {
        let cal = parse(INVITE).unwrap();
        let kept = for_calendar(&cal, "me@example.com", Answer::Accepted);
        // Unfolded: the lines are longer than 75 octets now.
        let text = kept.to_text().replace("\r\n ", "");
        assert!(!text.contains("METHOD"));
        assert!(!text.contains("VALARM"));
        assert!(
            text.contains("ORGANIZER;CN=Sam Rivera;SCHEDULE-AGENT=CLIENT:mailto:sam@example.org")
        );
        assert!(text.contains(
            "ATTENDEE;PARTSTAT=ACCEPTED;ROLE=REQ-PARTICIPANT;SCHEDULE-AGENT=CLIENT:MAILTO:Me@Example.COM"
        ));
        // Mira's answer is untouched; every attendee is marked for client scheduling (T40a review: a server that
        // schedules never mails anyone on the person's behalf).
        assert!(text.contains(
            "ATTENDEE;CN=Mira;PARTSTAT=NEEDS-ACTION;RSVP=TRUE;SCHEDULE-AGENT=CLIENT:mailto:mira@example.net"
        ));
        // Changing the answer again changes only that line.
        let mut again = kept.clone();
        assert!(set_own_answer(
            &mut again,
            "me@example.com",
            Answer::Declined
        ));
        assert!(!set_own_answer(
            &mut again,
            "me@example.com",
            Answer::Declined
        ));
        assert_eq!(
            Answer::of(again.events()[0].attendees[2].status.as_deref()),
            Some(Answer::Declined)
        );
        assert_eq!(sequence_of(&kept), 2);
        assert_eq!(organizer_of(&kept).as_deref(), Some("sam@example.org"));
    }

    #[test]
    fn answering_from_a_group_address_adds_the_person_once() {
        let mut cal = parse(INVITE).unwrap();
        assert!(answer_as(&mut cal, "team@example.com", Answer::Accepted));
        assert!(!answer_as(&mut cal, "team@example.com", Answer::Accepted));
        let ev = &cal.events()[0];
        assert_eq!(ev.attendees.len(), 4);
        assert_eq!(ev.attendees[3].address, "team@example.com");
        assert!(answer_as(&mut cal, "team@example.com", Answer::Declined));
        assert_eq!(cal.events()[0].attendees.len(), 4);
    }

    #[test]
    fn a_bare_cr_or_nul_never_survives_into_a_reply_or_the_stored_copy() {
        let hostile = INVITE
            .replace("UID:lunch-1@example.org", "UID:abc\rX-INJ:1")
            .replace(
                "SUMMARY:Lunch with Sam",
                "SUMMARY:x\rBEGIN:VALARM\rTRIGGER:-PT1M\rEND:VALARM\0",
            );
        let cal = parse(&hostile).unwrap();
        let r = reply(&cal, "me@example.com", Answer::Accepted, now())
            .unwrap()
            .to_text();
        assert!(r.contains("UID:abcX-INJ:1\r\n"), "{r}");
        let kept = for_calendar(&cal, "me@example.com", Answer::Accepted).to_text();
        for t in [&r, &kept] {
            assert!(!t.replace("\r\n", "").contains(['\r', '\0']));
        }
        // The would-be alarm is text inside the title now, never a component of its own.
        assert!(!kept.contains("\r\nBEGIN:VALARM"));
    }

    #[test]
    fn the_stored_copy_keeps_only_the_invitations_own_event_and_zones() {
        let extra = INVITE.replace(
            "END:VCALENDAR\r\n",
            "BEGIN:VTODO\r\nUID:todo\r\nBEGIN:VALARM\r\nTRIGGER:-PT1M\r\nEND:VALARM\r\nEND:VTODO\r\nBEGIN:VEVENT\r\nUID:other\r\nDTSTART:20261106T120000Z\r\nEND:VEVENT\r\nBEGIN:VTIMEZONE\r\nTZID:Own\r\nEND:VTIMEZONE\r\nEND:VCALENDAR\r\n",
        );
        let kept = stored_copy(&parse(&extra).unwrap(), true).to_text();
        assert!(!kept.contains("VTODO") && !kept.contains("UID:other") && !kept.contains("VALARM"));
        assert!(kept.contains("TZID:Own"));
        // A published event keeps no attendee at all.
        let published = stored_copy(&parse(&extra).unwrap(), false).to_text();
        assert!(!published.contains("ATTENDEE"));
        assert!(published.contains("ORGANIZER;CN=Sam Rivera;SCHEDULE-AGENT=CLIENT"));
    }

    #[test]
    fn an_update_of_one_occurrence_changes_only_that_occurrence() {
        let series = INVITE
            .replace(
                "DTEND:20261106T130000Z\r\n",
                "DTEND:20261106T130000Z\r\nRRULE:FREQ=WEEKLY\r\n",
            )
            .replace("METHOD:REQUEST\r\n", "");
        let stored = for_calendar(&parse(&series).unwrap(), "me@example.com", Answer::Accepted);
        let one = "BEGIN:VCALENDAR\r\nMETHOD:REQUEST\r\nBEGIN:VEVENT\r\nUID:lunch-1@example.org\r\nSEQUENCE:3\r\nRECURRENCE-ID:20261113T120000Z\r\nDTSTART:20261113T140000Z\r\nDTEND:20261113T150000Z\r\nSUMMARY:Lunch later\r\nORGANIZER:mailto:sam@example.org\r\nATTENDEE;PARTSTAT=NEEDS-ACTION:mailto:me@example.com\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";
        let update = parse(one).unwrap();
        assert!(!is_outdated(&stored, &update));
        let merged = merge_update(&stored, &update, "me@example.com", Answer::Declined);
        let events = merged.events();
        assert_eq!(events.len(), 2, "the series and the one changed occurrence");
        assert!(
            events[0].rrule.is_some() && events[0].summary.as_deref() == Some("Lunch with Sam")
        );
        assert_eq!(
            Answer::of(events[0].attendees[2].status.as_deref()),
            Some(Answer::Accepted)
        );
        assert_eq!(events[1].summary.as_deref(), Some("Lunch later"));
        assert_eq!(
            Answer::of(events[1].attendees[0].status.as_deref()),
            Some(Answer::Declined)
        );
        // The same occurrence again replaces it, never adds a second; an older one is outdated.
        let again = merge_update(&merged, &update, "me@example.com", Answer::Accepted);
        assert_eq!(again.events().len(), 2);
        let older = parse(&one.replace("SEQUENCE:3", "SEQUENCE:1")).unwrap();
        assert!(is_outdated(&again, &older));
    }

    #[test]
    fn a_cancellation_names_the_whole_event_or_its_occurrences() {
        let all = parse(&INVITE.replace("METHOD:REQUEST", "METHOD:CANCEL")).unwrap();
        assert_eq!(cancelled(&all), Some(Cancelled::All));
        let one = "BEGIN:VCALENDAR\r\nMETHOD:CANCEL\r\nBEGIN:VEVENT\r\nUID:s\r\nRECURRENCE-ID:20261113T120000Z\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";
        let Some(Cancelled::Occurrences(rids)) = cancelled(&parse(one).unwrap()) else {
            panic!("occurrences expected")
        };
        assert_eq!(rids.len(), 1);
        // At most 50 from one message.
        let mut many = String::from("BEGIN:VCALENDAR\r\nMETHOD:CANCEL\r\n");
        for d in 0..200 {
            many.push_str(&format!(
                "BEGIN:VEVENT\r\nUID:s\r\nRECURRENCE-ID:2026{:02}{:02}T120000Z\r\nEND:VEVENT\r\n",
                1 + d / 28,
                1 + d % 28
            ));
        }
        many.push_str("END:VCALENDAR\r\n");
        let Some(Cancelled::Occurrences(rids)) = cancelled(&parse(&many).unwrap()) else {
            panic!("occurrences expected")
        };
        assert_eq!(rids.len(), MAX_CANCELLED);
    }
}
