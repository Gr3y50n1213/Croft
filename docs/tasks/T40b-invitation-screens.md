# T40b: The invitation card and answering from the event

Folders: `android/` only (the core is T40a, FFI v22).

Depends on: T40a (invitations in the core). Read first: `docs/ux/screens.md` (S22, S37), the reading wireframe
(`design/wireframes/11_C1_reading.png`, S22), T37's banner (`ui/reading/UnsubscribeParts.kt`) for how a per-message card
loads, and `docs/tasks/T40a-invitations-core.md` for what the core decides.

## Goal
A message with an invitation shows S22's card in the conversation: what it is, when and where, who organises it and who
is invited, a clash with the person's own events, and Yes, Maybe and No. Answering mails the organiser and adds the
event to the calendar the person picks. S37 offers the same answers for an event that invites the person.

## Spec
- **S22 card** (above the messages, like T37's banner), from `invitation(message_id, zone)`:
  - "Sam Rivera invited you" (a request), "Sam Rivera shared an event" (a publish), "Sam Rivera cancelled this event", or
    "Sam Rivera accepted / declined / might come" (a reply: no buttons).
  - The title, the date and time (or the date for a whole day; "Repeats" when it does), the place, and the people
    ("Sam, Mira and you", and "and 12 more").
  - A clash in an amber row: "Conflicts with Standup 12:00" (the first; "and 2 more").
  - Notes, when they apply: the organiser is not the sender ("Sent by x on behalf of y": anyone can write an organiser);
    a newer version is already in the calendar; the event in the calendar has another organiser; the person is not on
    the guest list (answering adds them).
  - Yes, Maybe, No (the current answer selected) when `can_respond`; for a publish, "Add to calendar". A calendar choice
    (chips) when the account has more than one calendar Croft can write to.
  - "Your reply is sent to the organiser and the event is added to your calendar." under the buttons.
  - A cancellation: "Remove from calendar" when `can_remove`, else a note why not.
  - After answering: the card shows the new answer; a snackbar says what happened, plainly for each problem (cannot
    send, no calendar, outdated, another organiser).
  - The reply's subject and text are written in the person's language ("Accepted: Lunch with Sam").
- **S37**: when `can_respond`, a row "Going?" with Yes, Maybe, No (the current answer selected); answering changes it in
  the calendar and mails the organiser, with a snackbar.
- Nothing is sent or stored when the card is shown; only a tap acts.
- Screen readers: the card is one group with a heading; the buttons say which answer is chosen.

## Done when
- Robolectric tests: the card for a request (title, time, place, people, clash, buttons) and nothing called on open; Yes
  calls the core with the chosen calendar and the localized wording, and the card then shows Yes; the notes (not the
  sender, outdated, another organiser); a publish's Add; a cancellation's Remove (and none when it cannot); S37's
  buttons call `respond_to_event`.
- CI green.
