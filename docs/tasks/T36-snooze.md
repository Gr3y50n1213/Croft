# T36: Snooze

Folders: `core/` (storage, ffi), `android/` (S29, the Snoozed folder, the wake job).

Depends on: T35. Read first: `docs/ux/server-contract.md` ("Snooze logic"), `docs/ux/screens.md` (S29), `docs/ux/swipe.md` (the Snooze
action), `docs/ux/controls.md` (S21's overflow menu), `docs/PLAN.md` ("Gmail-style smarts").

Pin and follow-up (keywords that sync) are T36b, after this.

## Goal
A message can be put away until a chosen time and comes back to the Inbox, unread, when it is due.

## Spec
- **On this device** (no Companion): snoozing hides the message on this phone until the wake time; other devices still show it, and
  the confirmation says "Snoozed on this phone". The Companion's Croft/Snoozed folder is a later task (OPEN-ITEMS).
- **The core:**
  - `snooze(ids, until_ms)` sets the wake time (it must be in the future, at most a year ahead); `unsnooze(ids)` clears it.
  - Snoozed messages are left out of every list (as the list queries already do), and are listed in the Snoozed folder, soonest
    first (`list_snoozed`).
  - `wake_due_snoozes(now)` returns each message whose time has come to the Inbox: unread (through the queue, like any mark-unread),
    marked "back from snooze" for 24 hours (a new column, migration 0009). It returns them so the app can say so.
  - `next_snooze_wake()` gives the soonest wake time, so the app can plan its wake-up.
  - Snoozes due while the app was closed are applied before the list first draws.
- **Android:**
  - S29, a sheet with the presets Later today (18:00, or in three hours after 15:00), Tomorrow (08:00), This weekend (Saturday 09:00)
    and Next week (Monday 08:00), plus Pick date and time. This weekend is left out on Saturday and Sunday (it would mean
    next week's).
  - Snooze is offered on the swipe action, in the selection toolbar and in S21's overflow menu, with Undo (it unsnoozes).
  - The Snoozed folder in the drawer lists them with their wake time, with Unsnooze.
  - A woken message shows a Snoozed chip for 24 hours, and a notification says "Back in your inbox" (only where notifications
    are on).
  - The wake-up is one WorkManager job set for the soonest wake time: inexact, kept across restarts, with no exact-alarm
    permission and no new permission.
- **Order in the list.** A woken message stays at its date's place in the list (unread, with its chip and the notification), not at
  the top: the list is ordered by the message's date, and a separate sort key is a later change (OPEN-ITEMS).

## Done when
- Storage tests: snooze, unsnooze, the bounds, the Snoozed list, waking (unread through the queue, the back-from-snooze mark, and
  nothing woken early), per-account scoping, migration 0009.
- FFI tests: snooze, list, wake, the next wake time.
- Android unit tests: the S29 presets (each case by time of day and weekday), snooze with Undo, the Snoozed folder, the chip, and
  the wake job's schedule.
- CI green.
