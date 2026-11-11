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

## How to run things
```
cd core && cargo test                       # unit tests (no network needed)
cd core && cargo fmt --all --check && cargo clippy --all-targets -- -D warnings
python3 core/storage/tools/check_schema.py  # applies migrations to SQLite and checks triggers, FTS5, queue queries
docker compose -f test-servers/docker-compose.yml up -d   # servers for integration tests (when Docker is available)
```

## Where things stand
Done and tested: schema v1, sync planner, operation queue (per-kind delays, account back-off, offline hold, UID re-pointing), protocol vocabulary and the `MailBackend` trait.
Everything else is in `docs/tasks/`, starting with T01. The first build environment could not reach
crates.io, so no external crate has been compiled yet: expect to fix version and API details when you add them.
