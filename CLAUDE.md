# Croft: rules for every Claude Code session

Croft is an open-source email client (Android first, then web, then iOS) for self-hosted mail servers
of any software.

Read first, in this order: `HANDOFF.md`, then `docs/PLAN.md` (product, architecture, security, stack),
`docs/ROADMAP.md`, and `docs/tasks/README.md` (the next tasks). For anything with a screen, read
`docs/UX.md` or the matching file in `docs/ux/`. Screen IDs (S01 to S60, W01 to W04, T01 in
`design/screens.csv`) are the names to use in code, tests and issues. Wireframes are in `design/wireframes/`.
Design values are in `design/tokens.json`. Store rules are in `docs/store/READINESS.md`: read it before adding a permission,
a background service, an account feature or any data that leaves the device.

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

If you were started with `MASTER-PROMPT.md` (an unattended run), it also sets what to build, how to decide
without the owner, and the progress files to keep.

## Architecture
- All mail logic lives in the Rust core (`core/`). UIs stay thin: they draw state and send intents.
- Protocol code only in `core/protocols/`. Nothing outside it knows whether an account is IMAP or JMAP.
  Everything else talks to the `MailBackend` trait.
- Every change a person makes is applied locally first and queued in `OpQueue` (`core/mail-core/src/ops.rs`),
  then confirmed by the server. Undo cancels a queued operation; it never reverses a finished one.
  Moves, deletes, spam and snooze wait the Undo window; read, unread and star wait 1.5 s. A confirmed move re-points
  later ops on that message to its new UID. The full contract is in `docs/ux/server-contract.md`.
- `core/mail-core` is pure and synchronous (no clock, no I/O). Time is passed in. Keep it that way so it
  stays testable without a network.
- The database schema is in `core/storage/migrations/`. Migrations are forward-only; never edit a released one.

## Security rules (read changes to these by hand, always)
- Never log secrets: passwords, OAuth tokens, private keys, message bodies, full addresses in error strings.
- Secrets live only in the platform keystore. They never enter SQLite, config JSON or crash reports.
- Every query is scoped by `account_id`. A new table without `account_id` must be justified in
  `every_table_has_an_account_scope_or_is_global` (core/storage/src/lib.rs).
- Remote content (images, tracking pixels) is blocked until the person allows it.
- Encryption never downgrades silently: if a recipient has no key and encryption is required, ask.

## Code standards
- Rust: `cargo fmt`, `cargo clippy` clean, no `unsafe` (workspace forbids it), no `unwrap`/`expect` outside tests.
- Public items are documented (`missing_docs` is on).
- Errors from the network are `BackendError::{Transient, Auth, Permanent}`: the split drives retry vs fail vs sign-in.
- Small commits with the task ID in the message, one crate or one screen per task.
- Store rules: every build follows Google Play's rules; Apple's rules apply only to the Apple builds, but keep
  behaviour consistent across platforms (prefer a design that satisfies both over platform-only behaviour).
- The repository is meant to be public. Commits, files and docs never contain the owner's name, handle, email,
  session links or any real key or password. Test credentials are made up and marked as such.

## How to run things
The checks that must pass before every commit (also in `.github/workflows/ci.yml`):
```
cd core && cargo fmt --all --check
cd core && cargo clippy --all-targets -- -D warnings
cd core && cargo clippy -p croft-ffi --features cli --all-targets -- -D warnings
cd core && cargo test                                  # unit and in-process tests, no network needed
cd core && cargo test -p croft-storage --features sqlcipher
python3 core/storage/tools/check_schema.py             # applies migrations to SQLite and checks triggers, FTS5, queue queries
cd design/generator && npm ci && npm run generate && npm test   # tokens; git diff must show no change in design/generated
```
Integration tests against real servers (Docker):
```
docker compose -f test-servers/docker-compose.yml up -d --wait greenmail dovecot stalwart
bash test-servers/stalwart/setup.sh                    # Stalwart starts empty; this creates the test users
cd core && cargo test -p croft-protocols --features integration
docker compose -f test-servers/docker-compose.yml down -v
```
Android (needs `ANDROID_HOME`, NDK r29, cargo-ndk; see `android/README.md`):
```
cd android && ./gradlew :app:assembleDebug :app:testDebugUnitTest
```
Parallel work in one container: each task gets its own git worktree; container tests then need their own
compose project name (`-p`) and host ports (the tests read `CROFT_TEST_*_PORT`). Remove a worktree once merged:
build folders fill the disk. Never check out the same branch in two worktrees (for example `claude/croft-build`
in the main folder and in a task worktree): a push from one leaves the other's files stale.

## Where things stand
Progress, task by task, is in `docs/PROGRESS.md` (newest first); choices in `docs/DECISIONS.md`; what waits on the
owner in `docs/OPEN-ITEMS.md`; security-sensitive code to read by hand in `docs/REVIEW-QUEUE.md`.
Done and tested: storage (SQLite, SQLCipher, migrations 0001-0004), persisted operation queue, IMAP adapter (vendored
async-imap), JMAP adapter, account discovery, SMTP sender and outbox in the engine, sync engine (`core/engine`), FFI
surface v3 for the app, design tokens, first Android screen. Next tasks are T11-T21 in `docs/tasks/` (the Android app).
