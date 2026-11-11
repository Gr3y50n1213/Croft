# T20: Periodic background sync and notifications

Crate / folder: `android/` (plus a small pure helper in `core/mail-core` and a `sync_once` call in `core/engine` and `core/ffi`)

Depends on: T04, T10, T11, T16, T17 (S07 follows the first account). Quick-reply from a notification needs T18. Blocks: phase 2 push (UnifiedPush and the FCM gateway reuse the notification code).

## Read first
`docs/store/READINESS.md` (rows 4 and 5, permissions table, open items on foreground service: **the Play build has no always-on foreground service and no exact alarms**), `docs/PLAN.md` (push section: periodic sync is the default without a Companion), `docs/ux/screens.md` (S07, S32, S46 rows), `docs/ux/customisation.md` (Notifications rows), `docs/ux/haptics.md` (notification vibration belongs to the channel), `docs/ux/state-machines.md` (account connection: Paused; push health), `docs/ux/navigation.md` (deep links), `docs/ux/accessibility.md`.

## Goal
New mail arrives while the app is closed, using only WorkManager periodic work, with S32 basic notifications, the S07 explanation before the system permission prompt, and a minimal S46.

## Spec
- **Work.** One `PeriodicWorkRequest` (unique name, `KEEP`), default every 30 minutes, choices 15 min (the WorkManager minimum), 30 min, 1 h, 2 h, manual only; constraint `NETWORK_CONNECTED`, optional "only when battery is not low". No expedited work, no `setForeground`, no `AlarmManager` exact alarms, no `JobScheduler` directly, no boot receiver of our own (WorkManager restores itself).
- **Worker.** Gets the process-wide `Core` (the same instance the UI uses; open it if the process is cold), calls a new `sync_once(account, deadline)` for each enabled account in turn (syncs, drains due queue operations, returns the list of new messages), and stops cleanly before the deadline (8 minutes). It returns `retry` only on Transient, never loops, and records last-synced time. Accounts in Needs sign-in or Paused are skipped; a paused account posts one "Needs sign-in" notification, not one per run.
- **What counts as new.** A pure function in `core/mail-core`: a message is notifiable if it is unread, in the Inbox, newer than the account's stored "notified up to" marker, and not part of the first sync (S09 backfill never notifies) or the demo mailbox. The marker advances only after the notification is posted; a message already read on another device is not notified. The marker lives in the `setting` table.
- **S32.** Channel per account (sound and vibration are the person's, set in system settings, we do not override), grouped per account with a summary; one line each: sender and subject only (nothing from the body); lock-screen visibility private by default with a public version saying "N new messages"; up to three chosen actions from Archive, Delete, Mark read, Reply, each a `BroadcastReceiver` calling the same core actions as T16 (Archive and Delete honour the undo window; the notification updates). Tap opens `croft://message/<account>/<id>` with the inbox as synthetic back stack. Reply opens compose (T18) filled in.
- **S07.** Shown after the first account: explains in plain words why, then asks for `POST_NOTIFICATIONS` on Android 13 and later only at that moment (no prompt below 13); then the method list. Only "Periodic sync" is selectable; UnifiedPush and Google push show "Coming with the Companion" disabled. Send test posts a real notification through the same code path. Skipping is allowed and the app works without it. Denied permission shows a calm banner with a Settings link, never a repeated prompt. S07 states honestly that battery saver can delay checks.
- **S46 (minimal).** Master switch, per account switch, check frequency, action buttons (pick up to three; the customisation table says two and the screens table says three: use three and record it), Send test. Quiet hours and push-method health wait for phase 2.
- **Manifest.** Only the permissions in the READINESS table are allowed. Inspect the merged manifest: if WorkManager's `SystemForegroundService` entry or any `FOREGROUND_SERVICE_*` permission is merged in, remove it with `tools:node="remove"` and record in `docs/DECISIONS.md` (the READINESS item "confirm with a pre-launch review" stays open).

**Store rules (from the 11 November audit)**
- WorkManager only: no foreground service, no exact alarms.
- Do not use `REQUEST_IGNORE_BATTERY_OPTIMIZATIONS` (Play restricts it; **Confirm**). Link to the system battery settings instead.
- Ask for `POST_NOTIFICATIONS` after S07 only, and the app must work fully if it is refused.
- Push payloads are content-free (required on iOS, same on the Play flavour). Locally built notifications may show text; offer a lock-screen privacy option.
- An FCM flavour adds Google libraries: it changes the Data safety answer (device ID) and its merged manifest must be checked. F-Droid and UnifiedPush builds must not carry them.

## Done when
- Rust: the notifiable-message function over a table of cases (first sync, already read, other folder, older than marker, marker advance only after post, duplicate delivery); `sync_once` against the in-process IMAP server returns exactly the new messages and respects the deadline (fake clock).
- Robolectric with `work-testing` (`TestListenableWorkerBuilder`): the worker syncs two accounts with a fake `Core`, skips a paused one, returns success, returns retry on Transient and posts nothing twice on a re-run; the periodic request has the unique name, the chosen interval and constraints, and uses no expedited flag.
- Robolectric notification tests (`ShadowNotificationManager`): channel per account, group summary, content has no body text, lock-screen public version, chosen actions and their intents, tap intent URL, action receiver calls the fake action layer and updates the notification; permission denied posts nothing and shows the banner; no prompt below API 33 (`@Config(sdk = ...)`).
- Merged-manifest test (parse the merged manifest from the build): permissions are a subset of INTERNET, POST_NOTIFICATIONS, RECEIVE_BOOT_COMPLETED (and later USE_BIOMETRIC, CAMERA); no `SCHEDULE_EXACT_ALARM`, `USE_EXACT_ALARM`, `FOREGROUND_SERVICE*` or `READ_CONTACTS`; no service with a foreground service type.
- Screenshot tests for S07 and S46 (three widths, light and dark, font scale 1.0 and 2.0, LTR and RTL) and a semantics test for the permission rationale.
- On a device (not checkable here): that checks really run under Doze and app standby, the actual delay on a phone with battery saver, notification grouping and lock-screen look on real OEM Android skins, vibration from the channel. Reported as unverified.
