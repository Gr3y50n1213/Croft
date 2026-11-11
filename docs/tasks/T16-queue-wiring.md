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

## Result (session notes, T16)

**What ran and what did not.** The Rust checks all pass (`cargo fmt --check`, clippy with 1.99 and with `--features cli`, and with `--features test-server`; `cargo test`;
`cargo test -p croft-ffi --features sqlcipher`: 59 tests; `cargo test -p croft-ffi --features test-server`: 56). Under Gradle: `:app:assembleDebug`, `:app:assembleRelease`
(with the keep-rule and mapping checks), `:app:bundleRelease`, `:app:compileDebugUnitTestKotlin`, `:swipe:test` (48) and the new **`:mailtests:test` (22 tests, all pass, about 50 s)**
ran. **`:app:testDebugUnitTest` cannot start in the build container**: Maven Central answers HTTP 429 for `org.robolectric:nativeruntime-dist-compat:1.0.19` (as for T13 to T15).
Nothing was worked around. So **none of the Robolectric tests of this task has been run** (the screens' tests below), and no screenshot golden was recorded; GitHub CI is the first real run of those.
That is why the done-when lines that are about the data path are tested in a plain JVM module, `:mailtests`, which needs no Robolectric (see "Where things are").

### What was built
- **FFI version 5** (see `T10-ffi-mail-surface.md`, "API version 5"): `MessageRow.thread_count` and `.recipients`, `AccountInfo.last_synced_ms`, `MessageScope::Starred`, and the `test-server` feature with `TestImapServer`.
- **`CoreMessageRepository`** (`mail/CoreMessageRepository.kt`) and **`CoreRowActions`** (`mail/RowActions.kt`): the T14 and T15 interfaces over `Core`. The repository maps the core's fixed errors to `MailError`, keeps which call made which operation
  (so a failure notice can name its messages and Retry can repeat it), and applies the folder rules (special-use folder, else the folder the person chose, else it refuses with the account).
- **The app opens the core off the main thread** (`mail/MailRuntime.kt`, called from `MainActivity` in a `LaunchedEffect`): `createAppSecrets`, `openCore` with the T11 key, `SecretStoreCredentials`. **With no account in the core the app shows the fake sample mailbox** (T14's 5,000 rows) as before, because onboarding is T17; that also
  is what shows when the secure storage cannot be opened (nothing is changed; T17 owns the "try again" screen). The open core and the account secrets are in `MailRuntime.core` for T17. A session made once stays until the process ends, so an account added by T17 needs `MailRuntime` to build a new session (T17's job).
- **S30 Move to** (`ui/mail/MoveToSheet.kt`, rules in `MoveRules`): search, recent destinations (kept per account), the folder tree (indented by `/`), no "create new" (the core cannot create a folder). Reached from the selection bar's overflow menu (`SelectionAction.MOVE_TO`, one account at a time), a swipe on Move to (the card parks at 40 percent behind the sheet and goes or returns with the answer) and the row's own accessibility action.
  **`MOVE_TO` is selectable in the swipe picker now** (`SwipeAction`, as T15 noted); the three tests that said otherwise were changed (`SwipeMachineTest`, `SwipeRulesTest`, `SwipeSettingsScreenTest`).
- **Folder rules**: an account with no Archive (or Trash) folder gets the question "which folder do you use?" the first time (the same sheet), the answer is remembered (`FolderMapping`, `noBackupFilesDir/croft-folders.txt`), and Archive can be declined for the account, which hides it. No Spam folder: Mark as spam stays hidden (the doc). Delete in Trash and Spam is delete forever with the confirmation (T14/T15, unchanged).
  What is **not** offered: "set the deleted flag and expunge" for a missing Trash, and "create an Archive folder" (the core has no create-folder call). The mapping is the app's own store (there is no S45 yet); it does not reach the core.
- **S18 states**: the account chip and banners come from `AccountStatus` through the repository (Offline, Needs sign-in, Server problem = `Error`). The Offline state follows the platform connectivity callback (`mail/ConnectivityMonitor.kt`, `MailSession.followConnectivity`) which calls `set_network_available`. The Needs sign-in banner button and the account switcher's Fix raise `MailEvent.SignInRequested(account)` on `MailSession.events` (the sign-in screen, T17, listens; while nothing listens the old "account screens come next" note shows).
- **Failures**: a failure notice from the core makes the controller mark the named messages that are in the list with the red edge (`MessageRowState.failed`, from T13), play the `Rejected` haptic and show a snackbar "Couldn't finish that change. The mail is back where it was." with Retry, which asks the repository to run the same operations again, message by message.
- **S49 undo time**: `ui/mail/InboxSettingsScreen.kt` (3, 5 or 10 s; Settings, "Inbox and categories"), `mail/MailSettings.kt` (`croft-mail.json`). `CroftApp` hands it to the core (`set_undo_window_ms`) at start and on every change and to the snackbars (`MailSession.undoWindowMs`). The rest of S49 stays for later.
- **Permissions**: the app had **no `INTERNET` permission** (nothing needed it until the core synced). `INTERNET` and `ACCESS_NETWORK_STATE` (for the connectivity callback) are in the manifest and in `tools/manifest-allowlist.json` (with reasons); `docs/store/READINESS.md`'s permission table needs the two rows (that file was outside this task's list, so it is not edited).

### Where things are
- Rust: `core/ffi/src/{types,list,core,lib,test_server}.rs`, `core/ffi/Cargo.toml` (feature), tests in `core/ffi/src/tests/{local,server}.rs`.
- Gradle: `android/app/build.gradle.kts` (`rustFeatures` must not name `test-server`, `hostFeatures` does; `generateUniffiBindings` now reads the **Android** library, so the app's bindings cannot contain the test object; `generateUniffiHostBindings` makes the test bindings), `android/mailtests/` (new module), `android/settings.gradle.kts`.
- Kotlin: `android/app/src/main/kotlin/com/example/croft/mail/` (`CoreMessageRepository`, `FolderMapping`, `MailRuntime`, `MailSession`, `MailSettings`, `ConnectivityMonitor`), `ui/mail/` (`MoveToSheet`, `InboxSettingsScreen`, `InboxController` and `InboxScreen` changes), `ui/nav/` (route and settings entry).
- Tests: `android/mailtests/src/test/kotlin/com/example/croft/mail/` (`QueueWiringTest` 12, `CoreMessageRepositoryTest` 8, `FolderMappingTest` 2; the kit is `MailTestKit.kt`) and, for Robolectric, `app/src/test/.../ui/mail/` (`CoreStatesTest` 4, `MoveToTest` 5, `InboxSettingsScreenTest` 1, `MoveRulesTest` 4, `ListRulesTest` +1, `CoreScreenshotTest` 8 pictures, kit `CoreStateKit.kt`) and `.../mail/MailSettingsTest.kt` (3, plain JUnit; needs the app's test classpath, so it also waits for CI).
- **Why `:mailtests` is a module**: the host library of the JVM tests now carries the in-process server (`TestImapServer`), the app's bindings do not. The module compiles the app's `mail/` data layer in place (all of it except the four files that need Android: `MailSession.kt`, `MailSettings.kt`, `ConnectivityMonitor.kt`, `MailRuntime.kt`) with bindings generated from the host library; it has no Android types, so it runs without Robolectric. Two sets of bindings in one JVM would fight over the native library's callback tables (UniFFI keeps one pointer per callback interface, the last writer wins), which is why the test bindings are not added to the app's test classpath. `:app:testDebugUnitTest` depends on `:mailtests:test`, like `:swipe:test`.

### Done-when, line by line
| Line | Test, and what it ran on |
|---|---|
| Archive then Undo inside the window: the server sees no command | `QueueWiringTest.archiveThenUndoInsideTheWindow_theServerSeesNoCommand`: asserts the server's command log holds no MOVE, COPY, STORE, EXPUNGE or APPEND, after the window and a sync. **Ran, passes.** |
| Archive and let the window pass: in the server's Archive folder, row gone; clock while pending, gone on confirm | `archiveAndLetTheWindowPass_theMessageIsInArchive_andTheRowStaysGone` (server Archive holds the message, Inbox shrank, the row never returns, the Archive list shows it) and `aReadChangeShowsTheClockWhilePending_andItGoesWhenTheServerConfirms` (the marker is the row's `pending`). A move hides its row, so the clock is shown with a read or star. **Ran, pass.** |
| Kill and reopen the `Core` with an op pending: sent exactly once | `killAndReopenTheCoreWithAnOpPending_isSentExactlyOnce` (the screen's change loop is cancelled, the core closed, reopened on the same file; one MOVE or COPY in the whole log). **Ran, passes.** |
| Permanent refusal: the row returns with the failed state; transient: the row stays, retries follow the back-off with a fake clock | `aPermanentRefusalPutsTheRowBackWithTheFailure_andIsNotRetried` (a read-only Inbox answers `NO [READ-ONLY]`; the notice names the message and says `REJECTED`; no second try after 6 s; Retry works once the server allows it) and `aTransientAnswerKeepsTheRowGone_andTheChangeGoesOutWhenTheServerIsBack` (`NO [UNAVAILABLE]` at sign-in: no failure, not Needs sign-in, delivered by the first back-off retry on the real clock). **The fake clock cannot reach the core through the FFI** (it uses the system clock), so the schedule (5 s, 30 s, 2 min, 10 min, 1 h, 8 attempts) is covered by `core/engine/tests/fake_backend.rs`, `with_the_server_unreachable_every_action_works_locally_and_retries_back_off`, as before. The red edge itself is a screen matter: `CoreStatesTest.s18_aRefusedChange...` (**not run**). |
| Server rejects the login: Needs sign-in, queue paused, no attempt counted as Transient | `aRefusedLoginShowsNeedsSignIn_pausesTheQueue_andSigningInSendsItOnce` (status, `sign_in_needed` once, nothing sent, the change waits, signing in sends it once) **ran, passes**. The attempt count is not visible from Kotlin: the Rust test `a_sign_in_refused_with_a_change_waiting_pauses_the_account_without_counting_an_attempt` reads `op_queue.attempts == 0`. **Ran, passes.** |
| Airplane mode: every action works locally, nothing is sent, reconnecting sends in order | `airplaneMode_everyActionWorksLocally_nothingIsSent_andReconnectingSendsInOrder` (read, archive and star offline; nothing sent for 3 s past every window; online: the archive and the star arrive, and the read flag is sent before the move). **Ran, passes.** |
| While a read flag is pending, a server-side flag change does not change the row; after confirm the server value shows | `whileAReadFlagIsPending_aServerSideFlagChangeDoesNotChangeTheRow_...`. **Ran, passes.** (Both flags keep their local value while any change of the message is pending; this is `messages::upsert`.) |
| Mark read, archive, star quickly while the server returns the new UID late: the star lands on the moved copy | `readArchiveStarQuickly_theStarLandsOnTheMovedCopy` and `...WithoutUidplus_theNewUidComesLate_...` (the server's `UIDPLUS` is turned off, so no COPYUID: the new UID is found later by Message-ID). **Ran, pass.** The engine's own cases with a fake backend are in `core/engine/tests/fake_backend.rs`. |
| Bulk delete of 51 asks first; 50 does not | The rule is the screen's: `ListRulesTest.bulkArchiveOrDeleteOfMoreThanFiftyAsksFirst_andDeleteForeverAlwaysDoes` (T14, **not run here**, unchanged). `QueueWiringTest.aBulkOfFiftyAndOneGoesThroughTheQueueAsOneCall` runs 51 trashed through the real core (**ran, passes**). |
| Screenshot tests for S16 and S18 with real-repository state, light and dark | `CoreScreenshotTest`: 4 fixed states (S16 after a swipe with Undo; S18 offline with the pending clock; S18 needs sign-in; S18 refused change with the red edge and Retry) x light and dark = 8 pictures, through the app's real `CoreMessageRepository` over a scripted `Core` (a `Core` subclass with no Rust behind it). **Compiled, not run; no golden recorded.** *Not a live core*: the app's test classpath has no `TestImapServer` (see above), so the states are scripted in the shape the Rust core returns; the live path is `:mailtests`. |
| On a device | Not checkable here: list feel with a real mailbox, Wi-Fi to cellular changes, the battery cost of the connection (the core keeps its connection and syncs on its own schedule; no wake lock, no foreground service). **Unverified.** |

### Security points for the reviewer
- `core/ffi/src/list.rs`: two new SQL statements (`starred_rows`, and the correlated subquery in `COLUMNS`); every statement filters on `account_id` (the subquery on `t.account_id = message.account_id`); all values are bound parameters; the recipients come from `to_json` through `serde_json` and never fail the list. `MessageRow` now holds addresses in `recipients`; its `Debug` still prints ids and the date only (test `recipients_come_from_the_stored_to_list_...` checks it).
- `core/ffi/src/types.rs`: the new fields; `AccountInfo`'s `Debug` prints `last_synced_ms` (a time), still no label or address.
- `core/ffi/src/test_server.rs`, `core/ffi/Cargo.toml` (`test-server`), `android/app/build.gradle.kts` (`rustFeatures` check, `hostFeatures`, bindings from the Android library), `android/tools/check-release-outputs.sh` (symbols of the test server must not be in any release `.so`: checked with `llvm-nm -D` and a text search; the check was run against the Android `.so` files built here (none found) and against the host library (60 test symbols found), so it does tell the two apart). The server listens on 127.0.0.1 only and its password is made up.
- `android/app/src/main/AndroidManifest.xml` and `android/tools/manifest-allowlist.json`: **new permissions** `INTERNET` and `ACCESS_NETWORK_STATE` (normal permissions, no user data). `docs/store/READINESS.md` still has to list them.
- `android/app/src/main/kotlin/com/example/croft/mail/MailRuntime.kt`: where the core is opened (database key and credential provider from T11, off the main thread). A failure falls back to the sample mailbox and keeps no error text. `MailSession.events` carries an account id only.
- `mail/FolderMapping.kt`: `noBackupFilesDir/croft-folders.txt` holds account ids, role names and folder ids; no names, addresses or mail text.
- `mail/CoreMessageRepository.kt`: the core's errors become fixed `MailError` values; no string from the core is kept. It remembers message ids and operation ids of the last 200 calls in memory (for Retry) and the account of up to 20,000 listed messages.

### Choices to copy into `docs/DECISIONS.md`
| Date | Choice | Why |
|---|---|---|
| 2026-11-11 | The app's Kotlin bindings are generated from the Android library; the host library (JVM tests) also has the `test-server` feature, and only the module `:mailtests` compiles bindings made from it. | The Android library must not know the test object, and a binding for a function the shipped library lacks would fail the contract check at load. One set of bindings per JVM, because UniFFI keeps one vtable pointer per callback interface. |
| 2026-11-11 | The real-server tests are a plain JVM module (`:mailtests`) that compiles the app's `mail/` data layer in place, not tests in `app/src/test`. | They run without Robolectric (HTTP 429 from Maven Central blocks it here) and `:app:testDebugUnitTest` depends on them. |
| 2026-11-11 | `MessageScope::Starred` leaves out Trash and Junk. | A deleted message that was starred should not show as starred; the T14 fake already did this. |
| 2026-11-11 | The folder the person chose for Archive or Trash, and the recent Move to folders, are the app's own (`croft-folders.txt`), not in the core. | The core has no folder mapping; S45 does not exist yet. |
| 2026-11-11 | The app declares `INTERNET` and `ACCESS_NETWORK_STATE`. | The core opens its own sockets; the connectivity callback needs the second. |
| 2026-11-11 | `MOVE_TO` is selectable in the swipe picker; Snooze still is not. | S30 exists now. |
| 2026-11-11 | No account in the core: the app shows the sample mailbox. | Onboarding is T17. |

### Not done
- **Recipients for the Sent folder are empty in the app**: the sync does not store the To header (`to_json` is never written; `RemoteHeader` has no recipients). The FFI field, the mapping and the screen are done and tested with a value written into the database. The follow-up is in `core/protocols` (the IMAP ENVELOPE and the JMAP mapping), `core/storage` (`messages::upsert`) and `core/mail-core` test helpers; it was outside this task's list.
- A Kotlin test with a **fake clock** for the back-off (the core cannot take one through the FFI; the engine test covers the schedule).
- Running any Robolectric test, recording any golden picture, and the device checks above.
- "Create new" in S30, the "set the deleted flag and expunge" choice for a missing Trash, and the S45 screen for the folder mapping.
- A new session when the first account is added at run time (T17).
- Starred for "all accounts" in the drawer (it lists the first account's Starred in the unified Inbox, like the other mailboxes), a Starred count (the folder counters have none), and the "Select all N" offer in Starred.
- `docs/store/READINESS.md` (the two permission rows) and `.github/workflows/ci.yml` (a step for `cargo test -p croft-ffi --features test-server`; `:mailtests:test` is already part of `:app:testDebugUnitTest`) are outside this task's list.

### First CI run (PR #9): 6 failures of 1849, and what each was

The first real run of the T16 Robolectric tests was GitHub CI: 1849 tests, 6 failed, 1 skipped. **All six were test assumptions; no app or FFI code changed.** No test was deleted, skipped, `@Ignore`d or loosened. The Maven Central 429 for `nativeruntime-dist-compat` was bypassed for the local run only (the jar was fetched from another Maven Central mirror and served to Gradle by an init script outside the repository), so the whole of `:app:testDebugUnitTest` ran here after the fixes (all pass), and the four screen tests were repeated 8 times under CPU load without a failure.

| Test | Root cause | Kind | Fix |
|---|---|---|---|
| `RustBridgeTest.apiVersionIsTheOneTheAppExpects`, `.aMismatchedApiVersionIsRefused` | Not a host-versus-Android mismatch: both libraries are version 5 (`FFI_API_VERSION`, `EXPECTED_FFI_API_VERSION`). The tests still said `4u` and `"expected 4"` from before T16's bump to 5. The message in the failure ("FFI API version 2, expected 5") shows the check itself works. | Test assumption (stale literal) | `5u`; the message check uses `EXPECTED_FFI_API_VERSION`. |
| `CoreScreenshotTest[S16_UNDO-false]`, `[S18_FAILED-false]`, `CoreStatesTest.s18_theOfflineBanner...`, `MoveToTest.anAccountWithoutAnArchiveFolder...` | The first `waitForRow(10)` timed out, with the list still on its skeleton, although the repository had returned the rows. `CoreFixture` built the repository with `Dispatchers.Default` as its `io`. Compose's `createComposeRule` (the v1 rule) runs effects on an unconfined test dispatcher, so after `withContext(Default)` the screen's coroutine resumed **on the background thread** and wrote the list's state there, off the main thread, while the test waited for the frame: a different test failed on each run (the same four classes, a different one each time, 1 to 2 of 4 per run locally; reproduced). In the app the effect runs on the real main dispatcher and resumes on main, so this is not an app bug. | Test assumption (the fixture's thread) | `CoreFixture` uses `Dispatchers.Unconfined` (the scripted core is in memory, as the T14 fake was). The mapping, folder rules and failure handling are still the app's own. |
| `CoreStatesTest.s18_needsSignInShowsItsBanner...` (flaked once locally under load; passed on CI) | The test's fake sign-in screen listened with `events.first()`, which stops listening as soon as the event arrives; the screen then checks `hasEventListener` to decide whether to show the placeholder note, and sometimes saw no listener. | Test assumption (a one-shot listener) | The listener collects until it is cancelled, like a screen would. |
