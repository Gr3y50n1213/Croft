# T39b: The calendar screens on Android

Folders: `android/` (S34, S36, S37, S38, S39, reminder notifications, the bottom bar).

Depends on: T39a (the calendar in the core, FFI v20). Read first: `docs/ux/screens.md` (S34 to S39), the calendar wireframes
(`design/wireframes/14_E1_calendar_1.png`, `15_E2_calendar_2.png`), `docs/tasks/T39a-calendar-core.md`,
`docs/store/READINESS.md` (alarms: no exact alarms). S35 Week (the time grid with drag to move) and S39's "Subscribe to
a calendar link" are T39c; RSVP on S37 is T40.

## Goal
The person can open Calendar from the bottom bar, see their events as an agenda or a month, open one, and create, change or
delete events (one occurrence or all), choose which calendars show, and get a notification at each reminder.

## Spec
- **Bottom bar.** Calendar is enabled where the session has the core's calendars (`session.calendar`). Mail, Calendar and
  Contacts switch between the three destinations; Back from Calendar goes to the Inbox.
- **S34 Agenda** (the default view).
  - The header shows the month and year and a view switch (Agenda, Month).
  - Days are grouped, each with its weekday and date. Today's date is in a filled circle, and a day without events in the
    range says "No events".
  - Each event is a card in its calendar's colour (a bar on the leading edge; the text never relies on colour), with its
    title and time ("All day" for whole days) and its place.
  - A warning icon marks an event that overlaps another one in the same day (its description says "Overlaps another
    event").
  - The list starts at today and loads further days as it scrolls, 60 days at a time.
  - A "New event" button opens S38. Pull to refresh syncs every account's calendars; opening Calendar syncs too.
  - Sync problems are named per account, as in S40.
  - The "time zone unknown" and "repeats in a way Croft can't show yet" flags show as a small note on the card.
- **S36 Month.** A grid of weeks (Monday first, or Sunday first when the locale says so):
  - one dot per event, up to three per day;
  - today has an outline and the selected day is filled;
  - the selected day's events are listed underneath, as in S34;
  - swiping or the arrows change month.
- **S37 Event.**
  - The header is in the calendar's colour, with the title and the date and time.
  - Below it: the place, the organiser and attendees with their answers, the reminders, the calendar, the notes, and the
    status when it is tentative or cancelled.
  - Edit and Delete are in the bar.
  - For a repeating event, Edit and Delete ask "This event / All events". Delete always asks first.
  - Notes for a conflict or a waiting change, as in S41.
- **S38 Event editor.**
  - Title (Save is disabled until there is one), All day, start and end date and time (times in 15-minute steps; moving
    the start keeps the length), repeat (Does not repeat, Every day, Every week, Every month, Every year; a rule Croft
    did not write shows "Custom" and is kept), reminders (none, at the time, 5, 10, 15 or 30 minutes, 1 hour, 1 day
    before; several allowed), the calendar (for a new event), the place and the notes.
  - Close with changes asks "Discard changes?".
  - Saving a change to a repeating event asks "This event / All events" (from S37's choice).
  - Saving stores at once and syncs that account in the background.
- **S39 Calendars.** Calendars grouped by account, each with its colour and a check for whether it is shown. A hidden
  calendar stays in sync. Reached from the calendar header's menu.
- **Reminders.**
  - A WorkManager job (unique, like snooze's) asks the core for the reminders due in the next 24 hours, posts those that
    are due, and plans itself again at the next one, or in 6 hours at the latest.
  - It is planned again when the app starts, after a sync, and after a save.
  - Notifications go on their own channel, "Event reminders", with the title and the time. A tap opens S37.
  - No exact alarm: a reminder can come a few minutes late while the phone sleeps (READINESS row 5).
- **Privacy.** Nothing of an event is logged; the state classes' `toString` leave titles, places and people out.

## Done when
- Unit tests (Robolectric):
  - S34 groups by day, marks today and overlaps, says "No events", and opens S37 and S38;
  - S36 shows dots and the selected day's list;
  - S37 shows the fields, and its Edit and Delete ask for the scope of a repeating event;
  - S38 keeps Save off without a title, snaps to 15 minutes, keeps the length when the start moves, asks before
    discarding, and saves with the right scope;
  - S39 hides and shows a calendar;
  - the reminder job posts due reminders and plans the next.
- New controls (the day header, the event card) have gallery entries.
- CI green.
