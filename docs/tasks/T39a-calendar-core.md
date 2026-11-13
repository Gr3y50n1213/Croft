# T39a: The calendar in the core

Folders: `core/` (protocols, mail-core, storage, ffi). The screens are T39b.

Depends on: T38a (the CardDAV client: discovery, the bounded XML reader, the challenge rule, the same-origin rule). Read first:
`docs/tasks/T38a-contacts-core.md`, `docs/ux/screens.md` (S34 to S39), RFC 4791 (CalDAV), RFC 6578 (sync-collection),
RFC 5545 (iCalendar), RFC 7986 (calendar colour). Security-sensitive (a new parser of server data, credentials sent to a DAV
service): an independent review before merging.

## Goal
Each account's calendars on its mail host are synced both ways. The core can answer "what happens between these two times" across
the calendars the person shows, with repeating events expanded in the right time zones, and can create, change and delete events.

## Spec
- **One DAV client (protocols).** The CardDAV code's shared parts move to `protocols/src/dav/`:
  - discovery;
  - the multistatus reader and its bounds;
  - the 401-challenge rule (the credential is sent only after a 401 that names its scheme);
  - the same-origin rule;
  - `sync-collection` with a full listing as fallback;
  - multiget in batches of 50;
  - conditional `PUT` and `DELETE`.

  CardDAV keeps its behaviour and its tests. CalDAV adds:
  - `/.well-known/caldav`;
  - `calendar-home-set`;
  - the calendar collections (`supported-calendar-component-set` with `VEVENT`), with display name, colour (Apple's
    `calendar-color`) and order;
  - `calendar-multiget`.

  Calendars that only hold tasks (`VTODO`) are skipped.
- **iCalendar (mail-core, pure).** A reader and writer of Croft's own, like the vCard one:
  - lines are unfolded and folded at 75 octets, and text is escaped;
  - bounds: object size 1 MiB, 20,000 lines, 1,000 components;
  - unknown properties and components are kept as written, and an edit rewrites only the properties it changed.
  - Read from `VEVENT`:
    - `UID`, `SUMMARY`, `LOCATION`, `DESCRIPTION`, `STATUS`, `SEQUENCE`;
    - `DTSTART`, `DTEND` or `DURATION`, as a date, a UTC time, a time with `TZID`, or a floating time;
    - `RRULE`, `RDATE`, `EXDATE`, and `RECURRENCE-ID` overrides in the same object;
    - `ORGANIZER`, and `ATTENDEE` with `PARTSTAT`, `ROLE` and `CN`;
    - `VALARM` with a relative `TRIGGER` (minutes before).
- **Time zones.** `jiff` with its bundled time zone database, so the result never depends on the phone's. A `TZID` is looked up
  as an IANA name, and also with a common prefix such as `/mozilla.org/.../` removed. Otherwise the object's own `VTIMEZONE`
  is read: its standard and daylight offsets, and its yearly `RRULE` of the `BYMONTH` and `BYDAY` form. Failing that, the time
  is treated as UTC and the event is marked "time zone unknown". Floating times are in the device's zone, which the app passes
  in.
- **Recurrence (mail-core, pure).**
  - `RRULE` with:
    - `FREQ` (`DAILY`, `WEEKLY`, `MONTHLY`, `YEARLY`), `INTERVAL`, `COUNT` and `UNTIL`;
    - `BYDAY`, with ordinals for monthly and yearly rules;
    - `BYMONTHDAY` (negative values too), `BYMONTH`, `BYSETPOS` and `WKST`.
  - Expansion is in the event's own zone, so 09:00 stays 09:00 across a DST change. A time that does not exist moves forward;
    an ambiguous one takes the first.
  - A rule with parts Croft does not expand (`BYHOUR`, `BYMINUTE`, `BYSECOND`, `BYWEEKNO`, `BYYEARDAY`, `SECONDLY`, `MINUTELY`,
    `HOURLY`) shows only its first occurrence and is marked "repeats in a way Croft can't show yet". It is never guessed.
  - Expansion is bounded: at most 5,000 occurrences per event per query, and never past 2100.
- **Storage (migration 0013).** Per account:
  - `calendar`: path, name, colour, order, sync token, and whether it is shown;
  - `event`: href, etag, UID, the whole object, what waits to be sent with a revision, a conflict mark, and a summary for
    queries (start and end in UTC, whether it repeats, the end of the repeats or open);
  - an `occurrence` cache is not kept: queries expand the repeating events whose range touches the window.

  Every table is scoped by `account_id`.
- **Sync.** The same two-way rules as contacts:
  - this device's changes go first, conditional on the etag, with revisions;
  - a 412 keeps the server's version and marks a conflict copy;
  - a failed listing stops the sync;
  - the server's changes come in batches.
- **The core (FFI v20).**
  - `sync_calendars(account)`.
  - `calendars(account)` and `set_calendar_shown`.
  - `events_between(from_ms, to_ms, zone)`: the occurrences of the shown calendars across all accounts, each with its event id,
    calendar, start and end, all-day, title, place, colour, and the flags.
  - `get_event(account, id)`, which also gives the occurrence's start for repeating events.
  - `save_event`: a new event, or a change to all occurrences. A change to one occurrence is written as a `RECURRENCE-ID`
    override.
  - `delete_event`: all occurrences, or one, which is written as an `EXDATE`.

  The fields are title, start, end, all-day, time zone, place, notes, a repeat preset (none, daily, weekly on the start's
  weekday, monthly on its day, yearly; a rule Croft did not write is kept as it is and shown read-only), and reminders in
  minutes. Attendees are shown and not edited here (invitations are T40).
- **Reminders.** `next_reminders(now, until)` gives the alarms to post. The app schedules them in T39b.
- **Privacy.** No event text, attendee or URL in logs or error strings. `Debug` is redacted.

## Done when
- mail-core tests:
  - parsing and writing round trips, with unknown lines kept;
  - edits that rewrite only what changed;
  - every `DTSTART` form, `DURATION`, all-day events;
  - recurrence against RFC 5545's own examples (the ones Croft expands);
  - `EXDATE` and `RDATE`, and `RECURRENCE-ID` overrides;
  - DST changes in America/New_York and Europe/London;
  - a `VTIMEZONE`-only zone, an unknown zone, and the bounds.
- protocols tests: in-process CalDAV discovery, sync and multiget, and the challenge rule. The CardDAV tests still pass.
- An integration test against Stalwart's CalDAV in CI.
- storage and FFI tests: the query window, a change to one occurrence and to all, deletes, conflicts, and accounts kept apart.
- An independent review, with its findings fixed.
- CI green.
