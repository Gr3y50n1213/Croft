# T16: Wire the list and actions to the real core

Crate / folder: `android/` (uses `core/ffi` through T10)

Depends on: T04, T10, T11, T14, T15, `core/test-support` (in-process IMAP server from T03). Blocks: T17, T18.

## Read first
`docs/ux/build-order.md` (step 6), `docs/ux/server-contract.md` (all), `docs/ux/state-machines.md` (account connection, optimistic row action), `docs/ux/message-list.md` (folder rules table, optimistic updates), `docs/ux/controls.md` (message row states, snackbar), `docs/tasks/T02-T04-design.md`, `docs/ux/screens.md` (S16, S18, S30, S45 folder mapping, S49 undo time).

## Goal
S10 to S19 run on real data. The `MessageRepository` and `RowActions` interfaces from T14 and T15 are implemented over the T10 surface. Undo, offline, sign-in pause and "a sync never overwrites a pending change" work end to end through the app's code. Also S30 Move to and the undo-time setting.

## Spec
- `CoreMessageRepository`: the paged query and change notifications from T10 mapped to the T14 interface; the account chips from the connection state machine; the Offline banner from the platform connectivity callback calling `set_network_available`.
- `CoreRowActions`: mark read, star, archive, delete, move, spam, delete forever (asks first), each returning an op id. The snackbar Undo calls `undo(op_id)`. Read and star have no snackbar; swiping again reverses them. Bulk actions over 50 messages ask first.
- Folder rules from `message-list.md`: no Trash, no Archive, no Junk folder prompts (ask once, remember in the account's folder mapping), Delete in Trash and Spam is delete forever with a confirmation.
- Failure states: a permanent refusal restores the row with a red edge and a Retry snackbar and the `Rejected` haptic; a transient error keeps the row as left with the clock marker; Auth shows the Needs sign-in banner (the sign-in screen is T17; here the banner button emits an event).
- S30 Move to: search, recent folders, folder tree; no create-new for servers that cannot.
- Undo time (3, 5, 10 s) as a setting that the core reads (S49 stays otherwise for later tasks).
- Replace fake data with the real repository behind a debug switch so the T14 and T15 screenshot tests keep using the fake.

## Done when
All checks run as JVM tests against the host library and the in-process IMAP server (no emulator):
- Archive then Undo inside the window: the server sees no command (assert the server's command log).
- Archive and let the window pass: the message is in the server's Archive folder and the row is gone; the clock marker appears while pending and disappears on confirm.
- Kill and reopen the `Core` (simulating process death) with an op pending: it is sent exactly once after reopening.
- Server set to refuse a move permanently: the row returns with the failed state; server set to reply transiently: the row stays, retries follow the back-off schedule with a fake clock.
- Server rejects the login: the account shows Needs sign-in, its queue is paused, and no attempt is counted as Transient.
- Airplane mode (network flag false): every action works locally, nothing is sent, reconnecting sends in order.
- While a read flag is pending, a server-side flag change on the same message does not change the row; after confirm the server value shows.
- Mark read, archive, star the same message quickly while the server returns the new UID late: the star lands on the moved copy.
- Bulk delete of 51 messages asks first; 50 does not.
- Screenshot tests for S16 and S18 with real-repository state (a few fixed states), light and dark.
- On a device (not checkable here): list feel with a real mailbox, Wi-Fi to cellular changes, battery behaviour of the connection. Reported as unverified.
