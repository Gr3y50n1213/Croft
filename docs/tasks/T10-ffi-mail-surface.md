# T10: FFI mail surface

Crate / folder: `core/ffi` (calls `core/engine`, `core/storage`, `core/protocols`)

Depends on: T04 (`EngineHandle`), T05 (discovery), T06 (send), T07 (the first surface). Blocks: T11, T14 to T21 (everything in the app that talks to the core).

**Status: built (API version 3), on branch `task/t10`.** The "Final surface" section at the end is what was built; the "Proposed surface" above it is kept for the reasoning. See "As built" at the very end for where the code differs from the decision.

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

## Final surface (decided by the orchestrator, 11 November 2026)

This replaces the "Proposed surface" above where they differ. Build exactly this; anything else needs a new decision.

- **Runtime.** `Core` owns one tokio multi-thread runtime (2 worker threads) and the `Engine`. UniFFI async functions run on it. Every sync function returns in well under a frame (local SQLite only).
- **Open.** `Core.open(db_path: String, db_key: Option<Vec<u8>>, credentials: Arc<dyn CredentialProvider>) -> Result<Core, CoreError>`. `CredentialProvider` is a UniFFI callback interface implemented by the app: `fn credential(account_id: i64) -> Result<Option<FfiCredential>, ProviderError>` (called each time a connection signs in; the core never stores it) and `fn sign_in_needed(account_id: i64) -> Result<(), ProviderError>`. `FfiCredential` is an object built with `FfiCredential::password(secret: Vec<u8>)` or `FfiCredential::oauth2(user, access_token: Vec<u8>)` (see "As built"); the secrets sit in `Zeroizing` buffers. T11 provides the Android implementation; until then tests use an in-memory one.
- **Discovery.** `async fn discover(email) -> Result<FfiDiscovered, DiscoverError>` plus an optional `DiscoveryObserver` callback interface (`on_probe(probe, state)`) for S03. `async fn verify(discovered, credential, pinned_fingerprint: Option<String>) -> Result<(), VerifyFailure>`; `VerifyFailure` mirrors `VerifyError` as a fieldless enum plus an optional fingerprint, with `message_key()` (the UI owns the wording and translations; the English text from T05 is the default).
- **Accounts.** `add_account(discovered, display_name, pinned_fingerprint) -> AccountId` (stores settings with no secret; starts syncing), `list_accounts() -> Vec<AccountInfo { id, label, email, status }>` (the address is shown on screen but never logged or put in an error) (status: Ok, Syncing, Offline, NeedsSignIn, Error), `remove_account(id)`.
- **Folders and messages.** `list_folders(account) -> Vec<FolderInfo>`; `list_messages(scope, before: Option<RowCursor>, limit: u32) -> MessagePage { rows: Vec<MessageRow>, next: Option<RowCursor> }` where `scope` is `Folder { account, folder_id }` or `UnifiedInbox`. `MessageRow` is shaped for S10: id, account, folder, sender name/address, subject, snippet, date, seen, flagged, has attachments, pending (local change not confirmed). Keyset paging by (date, id), newest first.
- **Observing (option B).** `async fn next_change(after: u64) -> ChangeBatch { seq: u64, folders: Vec<(account, folder_id)>, accounts: Vec<account>, failures: Vec<OpFailure> }`. It returns as soon as anything changed after `after` (or immediately if `after` is old). The app re-queries what it shows. Chosen because the engine already broadcasts changed folder ids and it has no re-entrancy or callback-lifetime risk.
- **Actions.** `mark_seen(ids: Vec<i64>, value) -> Vec<OpId>`, `set_flagged(ids, value)`, `archive(ids)`, `trash(ids)`, `move_to(ids, folder_id)`, `delete_forever(ids)`, `undo(op_id) -> bool`, `set_undo_window_ms(ms)`. Archive and Trash resolve the destination from the account's special-use folder; a missing one is the typed error `NoArchiveFolder` / `NoTrashFolder` (the UI then follows the folder rules in `server-contract.md`). Message ids are local row ids; the core looks up their account and refuses ids of other accounts.
- **Sync and network.** `sync_now(account)`, `set_network_available(bool)` (applies to every account), `signed_in(account)` after the person fixes a password.
- **Send.** Not in T10. Added by T06 part B / T18 with its own version bump.
- **Errors.** Fieldless enums only (as `StorageReason` in API version 2). No `String` that could carry data crosses the boundary in an error.
- **Version.** This surface is `FFI_API_VERSION = 3`.

## As built (T10 implementation notes)

Code: `core/ffi/src/` (`core.rs` the object, `changes.rs` the change log, `credentials.rs`, `settings.rs`, `list.rs`, `error.rs`, `types.rs`; tests in `src/tests/`). Kotlin: `android/app/src/test/kotlin/com/example/croft/MailSurfaceTest.kt`.

Differences from the final surface above, and why:
- **`mark_seen` and `set_flagged` return nothing, not `Vec<OpId>`.** `EngineHandle::mark_seen` / `set_flagged` return `()` (the queue may cancel the change against its opposite, so there is not always an operation). The undo for these is the opposite call inside the 1.5 s window. Moves and deletes return the ids. If the screens need ids for flag changes, the engine must return them first.
- **`signed_in` is `async`** (it asks the app for the credential on a blocking thread, which must not happen on the UI thread). It returns `NoCredential` when the provider still has none.
- **`ChangeBatch` has an extra field `everything: bool`**: true when the caller fell off the 512-entry ring, the engine channel lagged, or `after` is from another run. Tuples are not UniFFI types, so `folders` is `Vec<FolderRef { account, folder_id }>`.
- **Failure reasons are a fixed enum** (`FailureReason`), mapped from the engine's reason strings, so no engine text crosses.
- **Archive, Trash and Move to skip a message that is already in the destination** (no error); a batch may span accounts and is validated (every id exists, every destination exists) before the first change.
- **`list_accounts` never reports `Syncing`**: the engine has no "syncing" signal yet. Statuses come from the core (`NeedsSignIn`, `Error` for unusable stored settings, `Offline` from `set_network_available`, else `Ok`).
- **The keyset message query lives in `core/ffi/src/list.rs`**, not in `core/storage`, because T10 may not edit `messages.rs`. It reads `message` and `folder` only, filters on `account_id` in every statement, and should move to `croft_storage::messages` when that file is next open.
- **`Core.open` is synchronous** (a constructor); it reads and migrates the database and calls the provider, so the app calls it off the main thread.
- `VerifyFailure::UntrustedCertificate` and `CertificateChanged` carry the certificate fingerprint (a public SHA-256) as a string; every other error value is fieldless or holds an enum.

Known limits: a stale `SignInNeeded` change from before `signed_in` can flip the chip back for one cycle (the engine emits it asynchronously). Credentials are read when an adapter is built, so an OAuth token that expires pauses the account (`sign_in_needed`) until the app calls `signed_in` with a fresh one.

Security review follow-up (still API version 3, which is unmerged):
- **`FfiCredential` is an object, not an enum, and its secrets are `Vec<u8>` (Kotlin `ByteArray`).** The review asked for `ByteArray` fields in the enum. That does not work: Kotlin prints a data class's array property with its elements (`Password(secret=[102, 105, ...])`), which `CredentialPrintingTest` showed on the first run. A UniFFI object has no Kotlin properties, so it prints `FfiCredential@hash`. It is built with `FfiCredential.password(bytes)` / `FfiCredential.oauth2(user, bytes)`; the bytes sit in `Zeroizing` buffers wiped when the object is released; the core copies them once per use into the protocol layer's wiping credential. Bytes that are not UTF-8 count as "no credential" (`verify` answers `WrongPassword`). No `Clone` or `PartialEq`. T11 must still never log or print a credential, and should `fill(0)` its arrays and `close()` the object. Test: `CredentialPrintingTest`.
- **Callbacks return `Result<_, ProviderError>`** (`ProviderError::Failed`, fieldless), so a Kotlin exception no longer becomes a panic with the exception text. `Failed` from `credential` is "no credential".
- **`DiscoveryObserver::on_probe` runs on `spawn_blocking`** (`blocking_recv` loop), and stops after the call is cancelled. Tests: `a_slow_observer_does_not_delay_next_change_or_sync`, `nothing_is_delivered_after_discover_is_cancelled`.
- **Dropping `Core` can block up to 2 s** (bounded on purpose, so the database is free for a reopen): close it off the main thread (README, `RustBridge.kt`).
- The README lists the copies of secrets the core cannot wipe (UniFFI's lifting buffer, the Kotlin side's copies, `db_key`).
- `rustSources` in `android/app/build.gradle.kts` now lists `ffi`, `engine`, `mail-core`, `protocols`, `storage` and `vendor` (the patched async-imap) plus `Cargo.toml` and `Cargo.lock`; `test-support` is a dev-dependency and is not needed for the library builds.
