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
| 2026-11-11 | T08: `@material/material-color-utilities` 0.3.0 (not 0.4.0), TypeScript 5.9.3, run on Node 22.18+ without a build step | 0.4.0 has imports without `.js` and does not load under plain Node ESM; 0.3.0 gives identical colours for the 2021 spec (checked) |
| 2026-11-11 | T08: all eight presets use the Tonal spot style; generator accepts an optional per-preset style map for later | Neither `tokens.json` nor `theme.md` gives a preset its own style; Tonal spot is the documented default |
| 2026-11-11 | T08: pure black sets `background`, `surface`, `surfaceDim`, `surfaceContainerLowest` to #000000, other roles as dark | `theme.md` only says the darkest surface becomes black and raised containers stay a few tones lighter |
| 2026-11-11 | T08: generated Kotlin `croftMotionScheme()` needs Compose Material 3 1.5 or later (`MotionScheme` is internal in 1.4) | Expressive motion is required by the UX docs; if the app must stay on 1.4, drop that wrapper and keep `CroftSpring` |
| 2026-11-11 | T08: Kotlin package for generated tokens is `com.example.croft.design` | Package name is the owner's decision; placeholder |
| 2026-11-11 | T01: rusqlite 0.41.1 (`bundled`; feature `sqlcipher` → `bundled-sqlcipher`), libsqlite3-sys 0.39.1, serde 1.0.229, serde_json 1.0.154, zeroize 1.9.1 (only with `sqlcipher`), tempfile 3.27 (tests) | Newest releases on crates.io on 11 Nov 2026, checked with `cargo info` |
| 2026-11-11 | `croft-storage` declares `rust-version = "1.89"` (rusqlite 0.41 needs it); the workspace still says 1.85 | Raise the workspace value when the next crate needs it; this container has 1.97 |
| 2026-11-11 | Passing a database key to a build without the `sqlcipher` feature is an error (`EncryptionUnavailable`), not ignored | The spec said the key is only used with the feature; ignoring it would silently store mail unencrypted ("encryption never downgrades silently") |
| 2026-11-11 | Database key is exactly 32 raw bytes, given to SQLCipher as a raw hex key (no passphrase derivation) | The key comes from the platform keystore, so it is already random; raw keys open in about 1 ms |
| 2026-11-11 | `Db::open` checks the version on a read-only connection first, disables checkpoint-on-close until the version is accepted, and turns relative paths into `./path` | Review findings: a refused (too new, wrong key) database must not be changed, even with a leftover `-wal`; bundled SQLite reads `file:` names as URIs |
| 2026-11-11 | A message with no References, Message-ID or usable subject gets thread key `uid:<folder_id>:<uid>` | Otherwise every such message would share one empty thread |
