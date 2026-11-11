# Handoff: working on Croft with Claude Code

You are picking up a project that has a full plan, a full UX spec with wireframes, and the first slice of
tested back-end logic. No app exists yet. This file says what is solid, what is not, and how to start.

## The project in one minute
**Croft** is an open-source email client for people who run their own mail server (Stalwart, Dovecot, Mailcow and similar), and
it also works with Gmail and Outlook. Android is first, then web, then iOS. It is built for phones first and tries to feel as
polished as Gmail, with no account, no analytics and no mail passing through anyone else's servers.

It is made by one person (the owner, who directs the work) with Claude Code doing the building. The planning was done in a long
conversation with Claude and recorded in the files here. **This repository is the source of truth.** Any older copy of the plan
that lives in a document is a reference only. If something here contradicts what the owner tells you, ask them.

| Word | Meaning |
|---|---|
| Core | The Rust library in `core/` that holds all mail logic. The Android, web and iOS apps are thin screens on top of it |
| Companion | An optional server a person runs (AGPL-3.0) that adds web access, search and push. A phone works without one |
| Push gateway | A tiny stateless service that forwards content-free "check mail" pings to Google (FCM) or Apple (APNs) |
| Op, OpQueue | An action a person took (archive, mark read, and so on), applied on the phone at once and sent to the server later |
| Undo window | 3, 5 or 10 s (default 5) during which a move, delete, spam or snooze can still be cancelled. Read and star wait 1.5 s |
| S01 to S60, W01 to W04, T01 | Screen IDs in `design/screens.csv`. Use them in code, tests and issues |
| Moss, Wax | The brand seed colour (`#4F6F52`) and accent (`#B4472E`) |

## First five minutes
1. Unzip, then `cd croft` and run `git init && git add -A && git commit -m "Initial handoff"`.
2. Install the tools under "Set up your machine" below. Without them, tests cannot run.
3. Start Claude Code in this folder (`claude`). It reads `CLAUDE.md` by itself.
4. To let it run by itself, give it `MASTER-PROMPT.md` as the first message. To approve each step yourself, paste "Session 0" from "Prompts to paste" and read its answer before any code is written.
5. Add the licence texts (see `LICENSES.md`) as the first commit. Do this before anything is published.

## What is in this folder
| Path | What |
|---|---|
| `MASTER-PROMPT.md` | The first message for an unattended cloud run. It replaces the "wait for approval" steps in this file |
| `CLAUDE.md` | Rules Claude Code reads every session (architecture, security, standards, commands) |
| `docs/PLAN.md` | Product plan: vision, protocols, features, architecture, security, stack, risks, decisions |
| `docs/ROADMAP.md` | Five phases and four release gates |
| `docs/UX.md`, `docs/ux/*.md` | UX spec: screen inventory, state machines, server interaction contract, swipe actions, controls and long-press menus, motion, haptics, theme picker, accessibility, build order |
| `docs/store/` | Google Play and App Store readiness: requirements and checklists, draft privacy policy, review notes, iOS privacy manifest template |
| `docs/tasks/` | Eight task specs (T01 to T08), each with spec and "done when" |
| `design/wireframes/` | 28 sheets (PNG and SVG), including the swipe anatomy (SW) and long-press menus (LP), covering all 60 phone screens, 4 web screens and the tablet layout |
| `design/screens.csv` | Screen inventory as data (65 rows) |
| `design/tokens.json` | Colour seeds, motion springs, densities, swipe geometry and thresholds, haptics default |
| `core/` | Rust workspace: `storage` (schema v1), `protocols` (types and `MailBackend` trait), `mail-core` (sync planner, operation queue) |
| `test-servers/docker-compose.yml` | GreenMail, Stalwart, Radicale for integration tests |
| `.github/workflows/ci.yml` | fmt, clippy, tests, schema check |

## What was verified
- `cd core && cargo test`: 46 tests pass (37 in mail-core, 4 in protocols, 5 in storage). `cargo clippy --all-targets -- -D warnings` and `cargo fmt --check` are clean.
- `python3 core/storage/tools/check_schema.py`: all checks pass (counters, FTS5 search, queue queries, foreign keys).
- The code builds with the standard library only, on Rust 1.97.

## What was NOT verified (check these first)
1. **No external crate has ever been compiled.** The build environment could not reach crates.io. The dependency choices in `docs/PLAN.md` are a starting point; versions and APIs (async-imap, rusqlite, UniFFI, jmap-client, rPGP) must be checked when added.
2. **`test-servers/docker-compose.yml` has never been run.** Image names, tags and ports come from each project's documentation. Dovecot + Postfix is still to add.
3. **Licence texts are missing.** Add `LICENSE-MPL-2.0` and `LICENSE-AGPL-3.0` verbatim from the official sources (see `LICENSES.md`). The licence choice itself is in the plan.
4. **Nothing Android has been built.** No Gradle project, no SDK, no device testing. Motion values, haptics and layout numbers in the UX docs are starting points to judge on a real phone.
5. **Wireframes show structure, not final visuals.** Colours, icons and copy come from the tokens and the real components.
6. **Spring "settles in" times** in `docs/ux/motion.md` are estimates from the stiffness and damping numbers.
7. **Store rules** in `docs/store/READINESS.md` were read from Google's and Apple's pages on 5 November 2026. They change often, so re-check the sources before each release.
8. The schema, planner and queue are designed from the plan, not from running against a real server. Expect IMAP quirks to change details (T03, T04).

## Set up your machine
Rust via rustup (stable), Docker, Node.js, Android Studio with the NDK (needed from T07), Python 3 (schema check), and Claude Code started in this folder.
```
cd core && cargo test && cargo clippy --all-targets -- -D warnings
python3 core/storage/tools/check_schema.py
```
Both should pass before you change anything. If they do not, fix that first.

## How to work with Claude Code here
This section and "Prompts to paste" are for a person approving each step. For an unattended run, use `MASTER-PROMPT.md` and skip both.

- One task per session, one git branch per task, small commits. Ask for a plan first and approve it before it writes code.
- Start each session by telling it to read `CLAUDE.md` and the task file.
- Tests are the guardrail. Do not merge with red tests. Integration tests against the container servers must pass from T03 onward.
- Read by hand: anything touching auth, crypto, account isolation, the FFI surface, and secrets handling. For large changes, have a fresh session review the diff first.
- Check look and motion on a real phone, not only previews.

## Prompts to paste
**Session 0, sanity check (no code changes):**
> Read CLAUDE.md and HANDOFF.md. Run the tests, clippy and the schema check and tell me the result. Then read docs/PLAN.md, docs/ux/server-contract.md and docs/tasks/T01-sqlite-layer.md and, in a short list, tell me (1) anything you could not understand or that contradicts something else, (2) anything in T01 that looks wrong or missing given what is in core/, and (3) your plan for T01. Do not change any files.

**Session 1, first task:**
> Read CLAUDE.md, then docs/tasks/T01-sqlite-layer.md. Make a plan for it and wait for my approval before writing code. Use a branch called t01-sqlite-layer. When you add dependencies, check the current versions and API on docs.rs rather than relying on memory, and tell me which versions you chose.

**Before each later task:** same prompt with the next task file.

## Order of work
T01 to T06 finish the Rust back end for IMAP mail. T07 proves the Rust to Android bridge. T08 generates theme code from `design/tokens.json`. After T08, follow `docs/ux/build-order.md` steps 2 to 13 (theme engine, component gallery, message list, swipe engine, queue wiring, onboarding, compose and search, calendar, settings, web, tablet, store readiness). The companion server and web app are phase 2 in `docs/ROADMAP.md`.

## Store rules (read before building Android or iOS)
`docs/store/READINESS.md` lists what Google Play and the App Store require, as of 5 November 2026, and what it changed in the plan.
The ones that shape code from day one: target Android 16 (API 36) with no orientation or resizability locks; build native libraries
for 16 KB pages; handle Back only with the system back-callback API; no exact-alarm or contacts permission; in-app account deletion
(S58) plus a web page; a built-in sample-mail mode (S01) for review; content-free push; an iOS privacy manifest.
Items marked **Confirm** there could not be verified (export compliance, Gmail device-only assessment, foreground-service need,
trademark search for "Croft"). Settle them before the first submission.

## Open items for the owner (not for Claude Code to decide)
- Android package name, iOS bundle ID, domain and legal publisher name (developer verification and the store accounts need them).
- Trademark search for "Croft"; paid Google Play and Apple developer accounts.
- Items marked **Confirm** in `docs/store/READINESS.md` (export compliance, Gmail device-only assessment, foreground service, closed-test rules for organisation accounts).
- Privacy policy review (`docs/store/PRIVACY-POLICY.draft.md` has placeholders and needs a lawyer).

## Decisions already made (do not reopen without a reason)
Name Croft (store name "Croft Mail"). Rust core with native UIs. Android first, then web, then iOS. MPL-2.0 for core and apps, AGPL-3.0 for the companion and push gateway. Multi-user companion from v1. Thin push gateway, never a hosted mail watcher. Moss green seed `#4F6F52`, Wax red accent `#B4472E`, paper `#F6F3EE`. Details and reasons are in `docs/PLAN.md` under "Review decisions".
