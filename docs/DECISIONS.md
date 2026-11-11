# Decisions

One line per choice made without the owner: the decision, the reason, the date. Dependency versions too.

| Date | Decision | Reason |
|---|---|---|
| 2026-11-11 | Repository root is the unzipped `croft/` folder; work happens on branch `claude/croft-build`, `main` holds only "Initial handoff" | Master prompt: never push to the default branch; owner merges |
| 2026-11-11 | Android SDK: compile and target API 36, build-tools 36.0.0, NDK 29.0.14206865 (newest r29, stable line) | `docs/store/READINESS.md` asks for API 36; NDK r28+ builds 16 KB-page-aligned libraries by default |
| 2026-11-11 | Licence texts fetched from https://www.mozilla.org/media/MPL/2.0/index.txt and https://www.gnu.org/licenses/agpl-3.0.txt | The plain-text links named in `LICENSES.md` |
| 2026-11-11 | The async sync engine (T04) goes in a new crate `core/engine`, not in `core/mail-core` | `CLAUDE.md` (which wins) says mail-core stays pure and synchronous; T04's task file says mail-core. Details in `docs/tasks/T02-T04-design.md` |
| 2026-11-11 | Migration 0002 adds `message.local_hidden` and table `op_awaiting_uid` | Moves hide the row until the server confirms (never delete locally first); the "waiting for a new UID" mark must survive a restart (T02). See the design note |
| 2026-11-11 | `MailBackend::move_to` returns the new UIDs (`MoveOutcome`); added `find_by_message_id` and `capabilities` | T04 needs the COPYUID answer to re-point later operations, and Message-ID search when a server has no UIDPLUS |
| 2026-11-11 | Engine runs one operation at a time per account in its first version | Simplest correct order; the contract's "up to four in parallel" is a later speed-up |
| 2026-11-11 | Parallel subagents work in hand-made git worktrees under `/home/user/croft/wt/` | The agent tool's own worktree option could not see the repository, because the session started outside it |
