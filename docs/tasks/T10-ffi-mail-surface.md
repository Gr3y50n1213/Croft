# T10: FFI mail surface (proposal)

Crate / folder: `core/ffi` (calls `core/engine`, `core/storage`, `core/protocols`)

Depends on: T04 (`EngineHandle`), T05 (discovery), T06 (send), T07 (the first surface). Blocks: T11, T14 to T21 (everything in the app that talks to the core).

**Status: proposal.** The orchestrator owns this surface and reviews every line of it. The implementer builds what the orchestrator finalises from this section. Anything marked "choice" is not decided yet; do not guess, ask.

## Read first
`CLAUDE.md` (architecture and security rules), `core/ffi/README.md`, `core/ffi/src/lib.rs`, `docs/tasks/T02-T04-design.md`, `docs/ux/server-contract.md`, `docs/ux/state-machines.md` (account connection, outgoing message), `android/README.md` (how the host library is loaded in JVM tests).

## Goal
Everything a screen needs, in a small versioned UniFFI surface. The UI sends intents and draws state; it never waits for the network.

## Proposed surface
- **Open.** `Core.open(db_path, db_key: Option<Vec<u8>>, credentials: Arc<dyn CredentialProvider>)`. The key is 32 bytes from the platform keystore (T11). `CredentialProvider` is a callback interface defined in T11. Errors are fixed enums, never strings with data.
- **Accounts.** `add_account(discovery_result, display_name) -> AccountId`, `list_accounts() -> Vec<AccountInfo>` (with the status chip state from `docs/ux/state-machines.md`), `remove_account(id)` (returns the list of what is deleted locally for S44), `set_account_color`.
- **Discovery and sign-in.** `discover(email) -> stream of probe progress` (S03 shows waiting, running, found, skipped) and a final `DiscoveryResult`; `verify_sign_in(settings, credential)` reporting each step (S05 Test connection); a failure is a typed enum with a plain-language message key and, for an untrusted certificate, the fingerprint to show for trust-on-first-use. `trust_certificate(account_or_draft, fingerprint)`.
- **Folders and messages.** `list_folders(account)` with unread counts; a message list query `(scope: folder | unified inbox, sort, page_size)` returning rows already shaped for S10 (sender, subject, preview, time, flags, thread count, pending marker); `get_message(id)` for T18.
- **Observing the list.** Choice, decide before coding:
  - A. UniFFI callback interface `ListObserver { on_change(change_set) }` registered per query. Pro: no polling, precise diffs. Con: callbacks arrive on a Rust thread, the app must hop to the main thread; lifetime bugs if the observer is not unregistered (give `ListHandle` a `close()` and drop guard).
  - B. A polling cursor: `list.page(offset, limit)` plus a single `subscribe_changes() -> ChangeCursor` the app awaits (`next_change() -> ChangedFolderIds`) then re-queries. Pro: simplest, no re-entrancy, matches the engine's `watch`/broadcast of changed folder ids. Con: re-query cost on every change; the app diffs itself.
  - Recommendation: B for the first version (it matches what T04 already emits), with the page size and anchor kept in the cursor so scroll position survives.
- **Actions with Undo.** `mark_seen`, `set_flagged`, `archive`, `delete`, `move_to`, `mark_spam` take a list of message ids and return `OpId`s. `undo(op_id)` returns whether it could still cancel. `undo_window_ms` is a setting read by the engine, not passed per call.
- **Sync and state.** `sync_now(account)`, `account_state(account)` stream (Connecting, Idle, Syncing, Offline, Needs sign-in, Server problem, Paused), `set_network_available(bool)` from the platform's connectivity callback, `sign_in_again(account, credential_changed)`.
- **Send.** `send(draft) -> OpId` (Outbox row with `send_at`), `undo_send(op_id)`, `outbox() -> rows`, `retry`, `edit_back_to_draft`, `discard` (T06 and T18).
- **Threading.** Every function returns at once; long work is an async function exposed through UniFFI's async support. No call blocks on the network.

## Versioning rule
Any removal, rename or type change bumps `FFI_API_VERSION` in `core/ffi/src/lib.rs` and `EXPECTED_FFI_API_VERSION` in `android/.../RustBridge.kt` in the same commit; additions also bump it while the surface is pre-1.0. The app refuses to start on a mismatch.

## Done when
- `cd core && cargo test -p croft-ffi` passes.
- Kotlin JVM tests in `android/app/src/test` (host library through JNA, as T07) cover: open with a key and reopen; add, list and remove an account against the in-process IMAP server in `core/test-support`; list folders; observe a change after a fake server delivers mail; `mark_seen` then `undo` sends nothing; `discover` against a recorded-fixture server; a wrong password gives the typed Auth error.
- A test proves no ffi function returns a string containing a password, token or message body (fixture secrets are searched for in all error values).
- The version-mismatch check has a test.
- `cargo fmt --all --check` and `cargo clippy --all-targets -- -D warnings` are clean.
