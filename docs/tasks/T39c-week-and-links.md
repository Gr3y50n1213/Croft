# T39c: The week view and calendar links

Folders: `core/` (protocols, storage, ffi) for calendar links; `android/` for S35 and S39's "Subscribe to a calendar link".

Depends on: T39a (the calendar in the core) and T39b (the calendar screens). Read first: `docs/ux/screens.md` (S35, S39),
the calendar wireframe (`design/wireframes/14_E1_calendar_1.png`), `docs/ux/motion.md` ("Drag and drop"),
`docs/ux/haptics.md`, and T37's public-addresses-only request (`core/protocols/src/oauth/fetch.rs`, `Reach::Internet`).
Calendar links are security-sensitive (a URL the person pastes is fetched): an independent review before merging.

## Goal
The person can see three days at a time as a time grid, create an event by tapping an empty slot, and move an event by
dragging it. They can also add a read-only calendar from a link (an `https:` or `webcal:` address of an iCalendar file),
which Croft fetches again from time to time.

## Spec
- **S35 Week** (a third view beside Agenda and Month).
  - Three days on phones (seven on wide screens, 600 dp and more), with the hours down the side and all-day events in a
    strip at the top.
  - A line marks the current time on today.
  - Events are blocks in their calendar's colour; events that overlap share the width.
  - A tap on an empty slot opens S38 at that time (rounded down to 15 minutes, for an hour).
  - A long press on an event lifts it (Drag start haptic), dragging moves it in 15-minute steps (the Tick haptic at each
    step), and letting go saves the new time. A repeating event asks "This event / All events" first.
  - At Reduced and Off motion the block jumps to each step without animation.
  - TalkBack users get a custom action on each event, "Move", that opens S38's times instead of dragging.
  - Swiping sideways, or the arrows, move by the days shown.
- **Calendar links (core).**
  - `subscribe_calendar(account, url, name)` checks the address:
    - `webcal:` is read as `https:`;
    - only `https:` is accepted;
    - no credentials in the URL, at most 2048 bytes;
    - only public addresses, as for one-click unsubscribe, so a link can never reach the device's or the Companion's
      private network.
  - It fetches the file:
    - one GET, no cookies, no `Authorization`, no redirect followed except up to two to `https:` public addresses;
    - at most 5 MiB, with bounded waits;
    - it must parse as a `VCALENDAR`.
  - The result is stored as a read-only calendar of that account (migration 0014: `calendar.source_url`, `read_only`).
  - Its events are split by `UID` into one object each, so the existing query and expansion work unchanged.
  - A sync of the account fetches its links again: at most once an hour, with `If-None-Match` and `If-Modified-Since`
    when the server gave an `ETag` or `Last-Modified`.
  - Edits to a read-only calendar's events are refused.
  - `unsubscribe_calendar(account, id)` removes it with its events.
  - The URL is kept as given (it may hold a private token), never logged, and shown in full only in S39's detail.
- **S39.** "Subscribe to a calendar link": a dialog with the address and a name, saying it is read-only and that Croft
  fetches it from time to time. Errors are said plainly (not an address Croft can use, not a calendar file, couldn't
  reach it). A linked calendar shows a link icon and can be removed (it asks first).
- **The core (FFI v21).** `subscribe_calendar`, `unsubscribe_calendar`, `CalendarInfo.read_only` and `CalendarInfo.linked`.

## Done when
- protocols tests (in-process HTTPS server):
  - the exact request (no cookies, no credential);
  - redirects, the size cap, the conditional fetch;
  - a private address and `http:` refused before connecting.
- storage and FFI tests: the split by `UID`, a refetch that changes, adds and removes events, read-only edits refused, and
  removal.
- Android tests:
  - S35's layout of overlapping events, a tap on a slot, and a drag that snaps and saves (with the scope question for a
    repeating event), plus the TalkBack Move action;
  - the subscribe dialog and its errors.
- An independent review of the link fetch, with its findings fixed.
- CI green.
