# Progress

Newest entry first. Each entry: status, what was built, which checks ran and their results, what is
unverified, the next step.

---

## T13 and T14: controls, haptics, navigation, Inbox screens (11 November 2026)

**Status:** merged into `main` (pull requests #3 and #4) after GitHub CI passed. Device-only items are unverified (ripple and haptic feel, scroll speed with 5,000 rows, TalkBack order).

**What was built:** T13: every shared control in every state (`ui/components/`), one `Haptics` interface (levels, 40 ms guard), a debug-only gallery, the stored theme applied at start and Settings → Appearance reachable. T14: the Inbox on a fake repository shaped like the core's FFI surface: S10 (paging, sticky dates, fast scroller, "N new" pill), S11 selection, S12 drawer, S13 account switcher, S17 pull to refresh, S18 offline and sign-in banners, S19 inbox zero; scroll position survives rotation and process death; a build-only macrobenchmark module.

**Checks:** neither task's tests could run in the build container (Maven Central refused one Robolectric jar). On GitHub CI: T13 green on its first run; T14 had 21 failures of 1,260 on its first run: 2 real screen bugs (the compose button had no name for screen readers; the "N new" pill faded in for one frame with animations off) and 19 test assumptions, all fixed without weakening a test (details in `docs/tasks/T14-message-list.md`).

**Also:** the flaky outbox test was `after_a_refused_sign_in_remove_and_add_does_not_make_the_task_spin_and_signing_in_sends`: it read an empty outbox list as "copy filed in Sent", which is not what the engine promises. Reproduced at about 31% under load; 0 in 450 runs after the test fix. The engine was correct.

**Next:** design tokens follow-up (amber colour, wait times, pull threshold and drawer width into `tokens.json`), then T15 (swipe engine).

---

## T12: Theme engine and Appearance screens (11 November 2026)

**Status:** done except: the screens are built and tested but not yet reachable from the app (T13 adds navigation); device-only items (feel of the cross-fade, wallpaper colours on a real Android 12+ phone, the system file and photo pickers).

**What was built:** `android/.../theme/` (settings, JSON export and import with a version field, DataStore, runtime colour schemes from Google's Material Color Utilities Java port, pure black, contrast levels, cross-fade with reduced-motion variants) and screens S50, S51, S52, S53 and S08. Optional fonts Inter ("Croft Sans") and Atkinson Hyperlegible (SIL OFL 1.1).

**Checks:** the agent ran 388 Android tests with 0 failures (144 contrast, 120 + 32 screenshots with Roborazzi, behaviour, JSON, golden schemes), but only by putting one Robolectric jar from the old mirror download on the classpath locally (not committed). The orchestrator built the merged app and compiled its tests. **GitHub CI (pull request #2) is the real test run.**

---

## T11: Secrets and the Android Keystore (11 November 2026)

**Status:** done except device-only checks (StrongBox, reboot, screen-lock change, "clear data", real keystore1 behaviour on Android 8 to 11).

**What was built:** the mail database is encrypted on the phone (SQLCipher with OpenSSL 3.6.3 built from source for Android, 16 KB aligned). Its key and all account secrets are wrapped by a non-exportable AES-256-GCM key in the Android Keystore (StrongBox when present), stored in `noBackupFilesDir`. Key loss is declared only on definite evidence (a canary key, repeated checks, and on Android 8 to 11 three starts over an hour); even then data is moved to a quarantine folder, restored automatically if the alarm was false, and deleted only after 7 days and 3 good starts. FFI API version 4: a new account's credential is passed to `add_account` itself; a keystore that is not ready is a temporary failure, not "needs sign-in". Account ids are never reused.

**Checks run and seen passing (by the orchestrator):** Rust fmt, clippy on 1.97 and 1.99, `cargo test` 712 passed, `croft-ffi` with SQLCipher 52 passed, `check_schema.py` ok; merged app (T11 + T12) builds and its tests compile. The agent ran 148 Android tests under Gradle with 0 failures (one large Robolectric jar excluded by a local, uncommitted init script because Maven Central refused it). **GitHub CI (pull request #2) is the real test run.**

**Review:** Opus, two rounds. Round 1: 2 high (a temporary keystore error could wipe all data; a new account's password could reach another account's server), 2 medium, 3 low. Round 2: 1 high (on Android 8 to 11 a keystore outage could still look like a lost key, and could replace the real key), 2 medium, 4 low. All fixed with tests. The first CI run found two failing tests; the cause was two copies of the Rust bindings under Robolectric, fixed with a custom test runner. The next CI run found a real race: a stale "needs sign-in" notice could arrive after the person signed in; the FFI layer now checks the engine's current state (20 of 20 runs pass).

**Known flaky test (resolved, see T13 and T14 entry):** an outbox test in `core/engine/tests/outbox.rs` has failed once each in two different sessions' runs, never reproduced and its name not captured. Next time it fails, capture the name and find the cause.

---

## Session 1 summary (11 November 2026) — read this first

**Update, first GitHub CI run (pull request #1):** all six jobs pass on GitHub (core, SQLCipher, schema, design tokens, integration against Stalwart/GreenMail/Dovecot, and Android `assembleDebug` + 27 JVM unit tests with libraries from Maven Central itself). CI found two problems the container could not: a clippy lint new in Rust 1.99, and an Android screen test still expecting FFI API version 1. Both fixed. This settles the "mirror" concern below for the Android tests.

**Where things stand:** stage 0 and stage 1 are done (T01–T06), plus T07 and T08 (stage 2, "done except" device checks), and from phase 1 the JMAP adapter (T09) and the FFI surface the app will use (T10). Stage 3 (the Android screens, T11–T21) has task files but has not started: every one of those tasks needs new Android libraries from Maven Central, which rate-limits this container (429), so nothing there could be built and tested here.

**Checks on the final branch (`claude/croft-build`, run by the orchestrator):** `cargo fmt --check` and both `cargo clippy -D warnings` clean; `cargo test` 703 passed, 0 failed; storage with SQLCipher 146 passed; `check_schema.py` all ok; design tokens 60 of 60. Container suites against real servers: Stalwart (JMAP) 6/6, GreenMail and Dovecot (IMAP) 9/9, SMTP to GreenMail 6/6, discovery 3/3.

**Not verified:** anything on an Android device or emulator (none here); the Android unit tests through Gradle (Maven Central 429; they ran once with libraries from a mirror, against the rules, see `docs/ENVIRONMENT.md`); GitHub CI has never run; the iOS side.

**Next session, first:** (1) attach a git remote so work can be pushed (`docs/OPEN-ITEMS.md`); (2) check Maven Central: `curl -sI https://repo.maven.apache.org/maven2/junit/junit/4.13.2/junit-4.13.2.jar` must answer 200, then run `cd android && ./gradlew --refresh-dependencies :app:assembleDebug :app:testDebugUnitTest`; (3) start T11 (keystore), then T12–T21 in order.

**How the work was done:** the orchestrator wrote the designs it is meant to keep (migrations 0002/0003, following a moved message, the FFI surface, the JMAP id map) and delegated the rest to subagents, one git worktree per task. Every security-sensitive change was reviewed by a separate Opus agent before it was merged: 13 reviews found 2 high, 13 medium and many low problems, all fixed (`docs/REVIEW-QUEUE.md`). The tool could choose a model and an effort level per subagent (Sonnet for building, Opus for reviews and the sync engine; Haiku was not needed). Its built-in "isolated worktree" option did not work because the session started outside the repository, so worktrees were made by hand.

---

## T09: JMAP adapter (11 November 2026)

**Status:** done. Not done: resuming `Email/changes` from the stored state after a restart (the first snapshot after a restart is a full read); push (EventSource); keep-alive connections; `$junk`/`$notjunk` keywords for spam (the trait has no flag for it yet, same as IMAP).

**What was built:** `croft_protocols::jmap`: `JmapBackend` implementing `MailBackend` for Stalwart, Fastmail and other JMAP servers, with its own small HTTP client (same TLS and pinning policy as IMAP and SMTP); a persistent map from JMAP ids to per-folder numbers (migration 0005) so nothing above the protocols crate knows which protocol an account uses; `JmapSubmitter` for sending through EmailSubmission. Stalwart v1.1.0 in `test-servers/` with a setup script that creates two users.

**Checks run and seen passing (by the orchestrator):** on the merged branch fmt, clippy (both), `cargo test` **703 passed, 0 failed**, storage with `sqlcipher` 146, `check_schema.py` ok. **All container suites run together by the orchestrator against fresh containers: Stalwart 6 of 6, IMAP (GreenMail, Dovecot) 9 of 9, SMTP to GreenMail 6 of 6, discovery 3 of 3.**

**Review:** Opus, one round: 1 high (after a proxy error such as 502 or 504, a JMAP send was retried and could be delivered twice), 2 medium (stale numbers after the id map was reset; a retried move lost the new number), 5 low; all fixed with tests. The fixes were checked against Stalwart by the orchestrator, not re-reviewed.

---

## T10: FFI mail surface, API version 3 (11 November 2026)

**Status:** done except: the Kotlin tests were not run through Gradle (Maven Central rate-limits this container). They were compiled by Gradle offline and run with plain JUnit against the real Rust library: 21 of 21 passed (agent's run; not repeated by the orchestrator). Sending is not on the surface yet.

**What was built:** `core/ffi`: `Core.open(path, key, credentials)` owning a 2-thread runtime and the engine; discovery with progress, sign-in check, accounts (add, list with status, remove), folders, a paged message list (one folder or the unified inbox), change notifications (`next_change`), actions with Undo (archive, trash, move, delete forever, mark read, star), sync now, network on/off, signed in again. Credentials come from the app through a callback each time a connection signs in and are never stored; a credential is an opaque object that wipes its secret when released and never prints it. Errors are fixed enums.

**Checks run and seen passing (by the orchestrator):** after merging with T06 part B, fmt, clippy (both), `cargo test` **601 passed, 0 failed**, `check_schema.py` ok. The merge needed a small fix (the FFI layer now maps the outbox change kinds part B added).

**Review:** Opus, one round: 3 medium (the credential printed its secret in Kotlin; a discovery callback ran on a core worker thread and could freeze syncing; documentation overstated how secrets are wiped), 3 low; all fixed. The fix changed the credential into an object (not re-reviewed).

**Next:** T11 (keystore), needs Gradle with new libraries.

---

## T06 part B: the outbox in the engine (11 November 2026)

**Status:** done. T06 is now complete. Not done: refreshing an OAuth token for sending (with T21); creating a missing Sent folder is only unit-tested (the test server always has one); a non-UTF-8 message cannot be saved in Sent (documented limit).

**What was built:** `MailSubmitter` (send, file a copy in Sent) in `core/protocols/src/submit.rs`, with an SMTP + IMAP implementation. The engine runs a send loop per account: Undo window, back-off, offline and sign-in pause shared with the action queue, a copy in Sent filed exactly once (Message-ID lookup first), "may not have been sent" after a crash, never resent automatically. `EngineHandle`: send, undo send, outbox list, retry, edit back to draft, discard, replace the sender after a new password. A message with a `Bcc:` header (also hidden behind bare carriage returns), or without a Message-ID, is refused before it is stored.

**Checks run and seen passing (by the orchestrator):** fmt, clippy, `cargo test` 563 passed in each of 7 full runs; **one run out of 8 had one failing test that could not be reproduced** (the machine was under heavy load with three builds at once; the failing test's name was not captured). `check_schema.py` all ok.

**Review:** Opus, one round: 2 medium (a Bcc header hidden by bare carriage returns would be revealed to all recipients; the account task could spin at full CPU after a sign-in failure and re-adding the account), 4 low (including replacing a "mark other accounts offline" workaround with a proper per-account take). All fixed with tests.

---

## T04: Sync engine (11 November 2026)

**Status:** done except IDLE (push from the server while the app is open): wake-ups use the next due action, a 15-minute timer and a nudge on every action. The place to add IDLE is marked in `core/engine/src/worker.rs`. Within an account, actions run one at a time (the contract allows four in parallel; a later speed-up).

**What was built:** new crate `core/engine` (`croft-engine`): one task per account; `EngineHandle` for the screens (mark read, star, move, delete forever, Undo, change notifications, online/offline, sync now), every action one local SQLite transaction that never waits for the network. Folder sync with the planner, chunked so a big sync does not freeze the screen. Follows a moved message to its new UID (from the server's answer, or by Message-ID when the server cannot say), and handles a server renumbering a folder (UIDVALIDITY change) without acting on the wrong message.

**Checks run and seen passing (by the orchestrator):** fmt, clippy, `cargo test` 520 passed, 0 failed, three runs in a row on the merged branch; storage with `sqlcipher` 135 passed; `check_schema.py` all ok. 58 engine tests: 47 against a scripted fake server, 5 against the real IMAP adapter and the in-process IMAP server, plus resilience and task-loop tests.

**Review:** Opus, three rounds. Round 1: 1 high (an interrupted folder reset could leave a folder showing almost nothing until the server changed), 2 medium, 3 low. Round 2 (of the fixes): 2 medium (an interrupted reset could make a "delete forever" hit the other copy of a duplicated message; a check computed too early), 2 low. All fixed with tests that failed before the fix. Round 3's fixes were not reviewed again.

**Next:** T06 part B (outbox in the engine), T10 (FFI surface for the app).

---

## T06: SMTP send and Outbox, part A (11 November 2026)

**Status:** part A done; part B (wiring the outbox into the engine so "send with no network queues and sends later exactly once" runs end to end) is not done yet. Its plan is at the end of `docs/tasks/T06-smtp-outbox.md`.

**What was built:** `croft_protocols::smtp`: an SMTP submission client written directly on tokio and rustls (TLS on 465, STARTTLS on 587, AUTH PLAIN/LOGIN/XOAUTH2, pinned certificates, same TLS policy as IMAP). `lettre` was not used: it cannot take a custom certificate verifier, so pinning would be impossible. `croft_mail_core::outbox`: the outbox state machine (queued, sending, submitted, filing in Sent, sent, failed; Undo, retry, edit back to draft, discard, abandon). A message whose sending was interrupted after the server may have accepted it is never resent automatically; it fails with "May not have been sent; check Sent before retrying". Migration 0004 rebuilds the `outbox` table (envelope, new states). A fake SMTP server in `core/test-support`.

**Checks run and seen passing (by the orchestrator):** fmt, clippy, `cargo test` 442 passed, 0 failed on the merged branch; `check_schema.py` all ok. The agent ran 6 SMTP-to-GreenMail container tests (send, then read back over IMAP, including Bcc and a body line that is just a dot) on its own copy of the servers; not re-run by the orchestrator.

**Review:** Opus, one round: no high or medium; 5 low fixed (message bytes and addresses in `Debug`, missing STARTTLS now pauses the account, abandon for a stuck sending row, outbox actions check the account, a schema comment).

**Next:** T06 part B after T04 is merged.

---

## T05: Account discovery (11 November 2026)

**Status:** done. Not checked here: the real DNS resolver and HTTPS client against the live internet (this container's DNS answers nothing; both were tested against local DNS and TLS servers), hickory's system configuration on Android, autoconfig files recorded from real providers (fixtures are written to Mozilla's format).

**What was built:** `croft_protocols::discovery`: built-in table (Gmail, Outlook, iCloud, Fastmail), autoconfig over HTTPS on the person's own domain only, RFC 6186 SRV, JMAP well-known; probes in parallel, earliest source in that order wins. Then `verify`: a certificate check with no credentials first, then sign-in. Each failure has its own plain message (wrong password, needs an app password, untrusted certificate with its fingerprint, certificate changed, connection intercepted, certificate invalid, cannot connect, and others). The local part of the address never leaves the device in a URL, a DNS name or a header; no third-party lookup.

**Checks run and seen passing (by the orchestrator):** fmt, clippy (with and without `integration`), `cargo test` 344 passed, 0 failed across the workspace. Container tests (GreenMail via autoconfig, Dovecot via SRV with TLS and STARTTLS: 3 of 3; T03's 9 of 9) were run by the agent on its own copy of the servers, not re-run by the orchestrator.

**Review:** Opus, two rounds. Round 1: 1 medium (Croft offered "trust this certificate" for Gmail and other big providers, where a self-signed certificate means interception), 2 low, 3 info. Fixing it also fixed a clash: T03 had changed certificate errors to `Auth`, which discovery would have shown as "wrong password". Round 2: 2 low, 3 info; fixed except one kept on purpose (a self-signed certificate is offered trust whatever name it was made for).

**Next:** T06 (in progress).

---

## T03: IMAP adapter (11 November 2026)

**Status:** done. Not covered by a container: UIDVALIDITY change on a real server (only the in-process server), CONDSTORE-only servers (only in-process), XOAUTH2 success (only in-process; no container offers it), the public certificate path against a real public server (no internet mail server reachable from here).

**What was built:** `croft_protocols::imap`: `ImapBackend` implementing `MailBackend` with async-imap 0.12.0 (vendored and patched, see below), rustls 0.23 (ring), TLS 1.2+ only, certificate checks always on, self-signed only through a pinned SHA-256 fingerprint, plaintext only for tests on loopback. The trait gained `MoveOutcome` (new UIDs), `find_by_message_id`, `capabilities`. `core/test-support`: an in-process IMAP server for fast tests. `test-servers/`: GreenMail 2.1.15 and Dovecot 2.4.5, pinned.

**Checks run and seen passing (by the orchestrator):** fmt, clippy (with and without `integration`), `cargo test` (protocols: 29 unit, 45 in-process, 1 log-capture, 2 vendored-crate guards), release-profile tests, and the container suite **9 of 9, three runs in a row** against fresh GreenMail and Dovecot containers.

**Review:** Opus, one round. 1 high: the IMAP library logged the LOGIN line with the password at trace level, and a release build could switch that on; fixed by vendoring async-imap with all four logging calls removed and its buffer cap lowered from 512 MiB to 64 MiB, plus a compile-time log-level guard. 2 medium: a wrong password could be classed as temporary (lockout risk); a hostile server could force about 500 MB of memory. 4 low. All fixed with tests. The agent saw occasional TLS "network error" failures in the container suite (also before its changes); the orchestrator's three runs were clean.

**Next:** T05 (discovery) fixes, then T06.

---

## T07: UniFFI surface and first Compose screen (11 November 2026)

**Status:** done except: (1) not shown on a device or emulator (none exists here: no KVM); (2) the Android unit tests passed only with libraries a subagent fetched from a mirror of Maven Central, which broke the "no mirrors" rule (see `docs/ENVIRONMENT.md`); they could not be re-run from Maven Central itself because it rate-limits this container; (3) the GitHub CI job has never run.

**What was built:** `core/ffi` (UniFFI 0.32.2): `croft_version()`, `ffi_api_version()` (now 2), `Core.open(path)`, `Core.list_folders(account)`, `FolderInfo`, `CoreError` with a fixed `StorageReason`. `android/`: Gradle 9.6.0 (wrapper and distribution checksums match the official ones), AGP 9.5.1, Kotlin 2.4.21, Compose BOM 2026.09.00 with Material 3 1.5.0-rc01, compileSdk 37, targetSdk 36, minSdk 26. cargo-ndk 4.1.2 builds `libcroft_ffi.so` for arm64-v8a and x86_64; all LOAD segments are 16 KB aligned. One Compose screen shows the Rust version, themed with the generated design tokens (`CroftTokens.kt` compiled for the first time, unchanged). The app excludes all data from cloud backup and device transfer. CI has an `android` job.

**Checks run and seen passing (by the orchestrator):** Rust fmt, clippy, `cargo test` (5 ffi tests among 176); `./gradlew --offline :app:assembleDebug` builds the APK (17.6 MB). Earlier, with mirror-fetched libraries, 7 JVM tests passed (4 calls into the real Rust library, 3 Robolectric screen tests).

**Review:** Opus, one round: 1 medium (device transfer could copy the database), 5 low/info; all fixed. FFI surface listed in `docs/REVIEW-QUEUE.md`.

**Next:** re-run `./gradlew --refresh-dependencies :app:assembleDebug :app:testDebugUnitTest` once Maven Central answers; then stage 3 screens.

---

## T02: Persist the operation queue (11 November 2026)

**Status:** done.

**What was built:** the offline action queue now survives a kill. `core/mail-core/src/persist.rs` (`OpStore` trait, `LocalEffect`, `PersistentQueue`) and `core/storage/src/op_store.rs` (SQLite implementation). Each action and its on-screen effect (read flag, star, hidden-while-moving, pending count, copy of a moved message) are written in one transaction. Migrations 0002 (hidden rows, "waiting for a new UID" table) and 0003 (folder counters leave out hidden rows) were designed by the orchestrator; see `docs/tasks/T02-T04-design.md`.

**Checks run and seen passing (by the orchestrator):** fmt, clippy, `cargo test` 171 (63 mail-core, 4 protocols, 104 storage), 107 storage tests with `sqlcipher`, `check_schema.py` all ok (29 checks).

**Review:** one round by a separate Opus agent: 3 medium, 3 low (a moved copy shown while a later move was pending; a failed move unhiding a row another move still hides; a row left hidden after a folder reset; op ids reused after restart; repoint to the same address deleting the row; unreadable rows dropped silently). All fixed, each with a test that failed before the fix. The fixes were not re-reviewed (they do not touch account scoping).

**Left for T04 (written in the design note):** during a full folder sync, clear `local_hidden` and `pending_ops` on rows with no unfinished operation; after a UIDVALIDITY reset of the *source* folder of a "waiting for new UID" entry, do not remove a row that now belongs to a different message.

**Next:** T04 sync engine.

---

## T01: SQLite connection layer (11 November 2026)

**Status:** done.

**What was built:** `core/storage`: `Db::open(path, key)` with WAL, foreign keys, forward-only migrations each in their own transaction, `SchemaTooNew` for a newer database, optional SQLCipher (`--features sqlcipher`). Repositories for accounts, folders, messages (list newest first without snoozed or hidden rows, upsert from server headers without overwriting a pending local change, flags, delete) and settings. Every folder and message query filters `account_id`. All 17 Python schema checks are ported to Rust tests.

**Checks run and seen passing (by the orchestrator):** fmt, clippy (with and without `sqlcipher`), `cargo test` 98 tests (37 mail-core, 4 protocols, 57 storage), 60 storage tests with `sqlcipher`, `check_schema.py` 17/17. Test suite run three times in a row: stable.

**Review:** two rounds by a separate Opus agent; every finding fixed (see `docs/REVIEW-QUEUE.md`). Listed for the owner to read by hand.

**Unverified:** read-only file permissions (the container runs as root, which ignores them). Behaviour on a real Android file system.

**Next:** T02 (persist the queue), already built on top of this.

---

## T08: Design token generator (11 November 2026)

**Status:** done except: the generated Kotlin and Swift files have not been compiled (no `kotlinc` or `swiftc` here). The Kotlin file is compiled for the first time when the Android project exists (T07/stage 3).

**What was built:** `design/generator/` (TypeScript, run by Node 22) reads `design/tokens.json` and writes `design/generated/` (Kotlin, CSS, Swift, TypeScript): 8 presets × light/dark/pure black × 3 contrast levels = 72 colour schemes of 48 roles, from Google's Material Color Utilities; motion springs, shapes, spacing, swipe and haptics values. `design/CLAUDE.md` holds the token rules.

**Checks run and seen passing (by the orchestrator, not only the agent):** `npm ci && npm run generate && npm test`: 60 of 60 tests pass; re-generating leaves `design/generated/` unchanged.

**Unverified:** Kotlin and Swift compile; the contrast acceptance test (build-order step 2, not this task).

**Next:** compile `CroftTokens.kt` inside the Android app.

---

## Stage 0: environment, baseline, licences, instruction file (11 November 2026)

**Status:** done.

**What was done**
- Unzipped the handoff, ran `git init`, made the "Initial handoff" commit on `main`, then started work on branch `claude/croft-build`.
- Wrote the global instruction file `~/.claude/CLAUDE.md` (full text below). The same "How to work" section is already in the repository's `CLAUDE.md`, word for word.
- Checked the container and recorded it in `docs/ENVIRONMENT.md`. Started the Docker daemon. Installed the Android SDK, platform 36, build-tools 36.0.0 and NDK r29. No emulator is possible (no KVM).
- The owner allowed five blocked hosts on the environment during the session (Google's Android downloads, the two licence sites, docs.rs).
- Downloaded `LICENSE-MPL-2.0` and `LICENSE-AGPL-3.0` from the official addresses. First lines checked: "Mozilla Public License Version 2.0" and "GNU AFFERO GENERAL PUBLIC LICENSE / Version 3, 19 November 2007".

**Checks run and seen passing:** `cargo test` 46 of 46; `check_schema.py` 17 of 17.

**Not done / unverified:** nothing can be pushed, because there is no git remote (see `docs/OPEN-ITEMS.md`).

**Next:** stage 1, tasks T01 to T06.

**The global instruction file, as written (`/root/.claude/CLAUDE.md`):**

```
## How to work
You are the orchestrator. Plan the work, then delegate
parts of it to subagents instead of doing everything yourself.

For each subagent, write a full, self-contained brief:
- the goal and why it matters
- the exact files, commands and context it needs
- clear steps, spelled out plainly
- what to report back, and what "done" means

Pick the cheapest model that can do the job well:
- haiku: searches, lookups, simple edits
- sonnet: normal coding and writing
- opus: hard bugs, reviews, tricky design
- fable: only the hardest calls, or work you'd do yourself

Set effort to match: low for simple, medium for normal,
high for tricky. Run independent jobs in parallel.

Check every subagent's work before you build on it,
and tell me plainly if anything failed.
If your tool can't choose models per subagent, use the
closest option it has and tell me what it can't do.
```
