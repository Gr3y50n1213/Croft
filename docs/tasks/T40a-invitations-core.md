# T40a: Invitations in the core

Folders: `core/` (reader, mail-core, storage, ffi). The screens (S22's card, S37's buttons) are T40b.

Depends on: T39a (the calendar in the core), T18c (the Outbox). Read first: `docs/ux/screens.md` (S22, S37), the reading
wireframe (`design/wireframes/11_C1_reading.png`, S22), RFC 5546 (iTIP) and RFC 6047 (iMIP). An invitation is content
anyone can mail, and answering it sends mail and writes to the calendar: an independent review before merging.

## Goal
A message that carries a calendar invitation shows what it is (title, time, place, who organises it, who is invited, a
clash with the person's own events), and the person can answer Yes, Maybe or No: Croft mails the answer to the organiser
and adds the event to the calendar they choose (or updates it there). A cancellation can take the event out again. An
event already in the calendar that invites the person can be answered from S37 the same way.

## Spec
- **Reading an invitation** (`croft-reader`): the first `text/calendar` part of a message (or an attached `.ics` file /
  `application/ics`), at most 1 MiB, decoded as text. Nothing else in the message changes.
- **What it is** (`mail-core`, pure): the object's `METHOD`:
  - `REQUEST`: an invitation (new or changed; `SEQUENCE` says which is newer).
  - `PUBLISH`: an event to add, with nothing to answer.
  - `CANCEL`: the organiser called it off.
  - `REPLY`: someone answered the person's own invitation: shown as "Sam accepted", nothing changes.
  - Anything else (or no `METHOD`): not an invitation; nothing is shown.
- **`invitation(message_id, zone)`** (FFI) gives `InvitationInfo`:
  - the kind, `UID`, title, start and end (with all-day), whether it repeats, place;
  - the organiser, and whether the organiser's address is the message's sender (when not, the card says whose
    invitation it is: anyone can write any organiser);
  - the attendees (at most 50, with their answers, and how many in all);
  - the person's own place on it: which of the account's addresses is invited, and its current answer;
  - up to three clashes: the person's own events (shown calendars, not cancelled, not declined) that overlap the first
    occurrence, by title and time;
  - whether it is already in a calendar (and which event), and whether this message is older than what is stored
    (a lower `SEQUENCE`: then it cannot be answered from this message);
  - for a `REPLY`: who answered and how.
- **`respond_to_invitation(message_id, answer, calendar_id)`** (Yes, Maybe, No):
  - Only on the person's tap: nothing is ever sent or stored when a message is opened.
  - The reply (RFC 6047): a message from the invited address to the organiser only, `Subject: Accepted: <title>` (or
    Tentative, Declined), a short text part and a `text/calendar; method=REPLY` part with `METHOD:REPLY` and one
    `VEVENT`: `UID`, `SEQUENCE`, `DTSTAMP`, `RECURRENCE-ID` when the invitation has one, `ORGANIZER`, and one `ATTENDEE`
    (the person's address, `PARTSTAT`). Nothing else from the invitation (no description, no other attendees) goes back.
    It goes through the Outbox like any message (the Undo window, the Sent copy).
  - The calendar:
    - Yes or Maybe with the event not yet stored: it is added to `calendar_id` (a writable CalDAV calendar of the same
      account), with the person's `PARTSTAT`, the invitation's `VALARM`s removed (a stranger does not choose when the
      phone notifies), and `SCHEDULE-AGENT=CLIENT` on the `ORGANIZER` so a server that schedules does not send a second
      reply.
    - Already stored: the person's `PARTSTAT` is changed there (and a newer `SEQUENCE` replaces the stored event's
      details, keeping the person's own reminders).
    - No with the event not stored: nothing is added.
  - No mail is sent when the account has no submitter, no organiser address is valid, or the invitation is older than
    what is stored; the outcome says why.
- **`apply_cancellation(message_id)`**: for a `CANCEL` whose organiser matches the stored event's: the stored event is
  removed (a cancelled single occurrence is removed from the series). Never for an event whose organiser differs.
- **S37 (`respond_to_event(account, event_id, answer)`)**: an event where the person is an attendee and not the
  organiser: the `PARTSTAT` changes in the calendar and the reply is mailed as above. `EventDetail` gains `my_answer` and
  `can_respond`.
- **Bounds**: the calendar part is read under the existing object bounds; titles, places and names are clipped as in
  T39a; at most 500 attendees are read and 50 returned. Nothing of the invitation or the addresses is logged.
- **The core (FFI v22).**

## Done when
- reader tests: the calendar part found in `multipart/alternative`, as an attachment, base64 and quoted-printable; none in
  a message without one; a part over the cap left out.
- mail-core tests: the kinds, the person's own attendee line found (case-insensitive, any of the account's addresses),
  the reply's exact lines (and nothing else carried over), a cancelled occurrence, `SEQUENCE` order.
- FFI tests (in-process SMTP and CalDAV fakes): Yes adds the event and queues the reply to the organiser only; Maybe then
  No changes the stored answer; an older invitation cannot be answered; a cancellation removes the event and a forged
  one (another organiser) does not; S37's answer; nothing happens on open.
- An independent review, with its findings fixed.
- CI green.
