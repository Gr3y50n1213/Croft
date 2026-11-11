# Progress

Newest entry first. Each entry: status, what was built, which checks ran and their results, what is
unverified, the next step.

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
