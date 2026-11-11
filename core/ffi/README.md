UniFFI surface for Kotlin (and Swift later). The wasm-bindgen surface for the web is not part of this crate yet.

Keep it small and versioned; every change here is reviewed by hand.

## The surface (API version 3)
Free functions: `croft_version()`, `ffi_api_version()` (**bump `FFI_API_VERSION` in `src/lib.rs` on any change**;
the apps check it at start-up), and the text helpers `verify_failure_message_key` / `verify_failure_default_message`
and `discover_error_message_key` / `discover_error_default_message`.

`Core` (one object; `Core.open(db_path, db_key, credentials)`):
- accounts: `discover` (async), `verify` (async), `add_account` (async), `list_accounts`, `remove_account` (async), `signed_in` (async)
- reading: `list_folders`, `list_messages(scope, before, limit)` (keyset paging; `MessageScope` is a folder or the unified Inbox)
- observing: `next_change(after)` (async; a numbered change log, see `src/changes.rs`)
- actions: `mark_seen`, `set_flagged`, `archive`, `trash`, `move_to`, `delete_forever`, `undo`, `set_undo_window_ms`
- sync and network: `sync_now`, `set_network_available`

Callback interfaces the app implements: `CredentialProvider` (`credential`, `sign_in_needed`) and `DiscoveryObserver` (`on_probe`).
Every callback returns `Result<_, ProviderError>`; `ProviderError` has one fieldless variant, `Failed` (see "Callbacks").
Errors: `CoreError`, `DiscoverError`, `VerifyFailure`: fixed categories, no free text.

## Async and the runtime
`Core` owns a tokio multi-thread runtime (2 workers). The exported `async` methods are plain futures that the app's
executor polls; each spawns its work on that runtime and awaits the task. The UniFFI `async_runtime = "tokio"` option is
not used, because it would run the futures on a second runtime of its own. Cancelling a call aborts `next_change`,
`discover` and `verify`; changes to the database (`add_account`, `remove_account`, `signed_in`) always finish.
`Core.open` and the callbacks into the app are the only places that can block: call `open` off the main thread.

**Closing the core must also happen off the main thread.** Dropping `Core` (Kotlin `close()`, or the cleaner) stops the runtime
and waits up to 2 seconds for the account tasks to release the database, so that reopening the same file finds it free.
The wait is bounded and kept on purpose; it is skipped (the runtime is shut down in the background) only when the drop
happens inside an async context. In Kotlin, close on a background dispatcher, never from `onDestroy` or another main-thread hook.

## Credentials
The core never stores a secret. `account.config_json` holds server settings and the pinned certificate fingerprint
(`src/settings.rs`, with a version field). When an account's adapter is built (at `open`, `add_account`, `signed_in`) the core
asks the `CredentialProvider` and passes the answer to `ImapBackend::new`. No answer: the account starts paused and
`sign_in_needed` is called once. For a new account the provider is asked with the new row id before `add_account` returns.

### Callbacks
`DiscoveryObserver.on_probe` runs on a blocking thread of the core (not on a runtime worker), one call at a time and in order,
so an observer that sleeps delays only its own progress reports, never `next_change` or a sync. After the `discover` call is
cancelled nothing more is delivered (a callback already running finishes).

A Kotlin exception thrown in a callback would, with a plain return type, become a Rust panic whose message holds the
exception text. So the callbacks return `Result<_, ProviderError>` (`#[derive(uniffi::Error)] enum ProviderError { Failed }`,
no fields): a thrown exception, of any type, arrives as `Failed` and its text is dropped. The core treats a failing
`credential` as "no credential" (the account is paused and `sign_in_needed` is called) and ignores a failing
`sign_in_needed` or `on_probe`.

## Secrets and what the core cannot wipe
`FfiCredential` is an **object** (Kotlin class), built with `FfiCredential.password(secret: ByteArray)` or
`FfiCredential.oauth2(user, accessToken: ByteArray)`, not an enum with fields. UniFFI turns an enum variant with fields into a
Kotlin `data class`, whose `toString()` prints every property: a `String` field prints the secret, and so does a `ByteArray`
field (Kotlin prints the elements). A Kotlin object has no properties, so it prints as `FfiCredential@1a2b3c`
(`CredentialPrintingTest`). The bytes must be UTF-8; otherwise they count as "no credential" (for `verify`:
`VerifyFailure::WrongPassword`). The object keeps the secret in a `Zeroizing` buffer in Rust, wiped when the object is released
(Kotlin `close()`, or the cleaner); it has no `Clone` or `PartialEq`, and its Rust `Debug` prints `***`. For each use the core
copies the secret once into the protocol layer's wiping credential, so one object can serve several calls. `db_key` is wiped
after the database is opened.

The core cannot wipe these copies, so the app must:
- **UniFFI's lifting buffer.** An argument (the `ByteArray` given to `FfiCredential.password`, `db_key`) crosses as a
  serialized buffer that UniFFI allocates and frees; that buffer is not wiped.
- **The Kotlin side's own copies.** The `ByteArray` the app built or kept, any `String` it made from it (a `String`
  cannot be wiped at all, so build the array from the keystore directly), and whatever the JVM or the keystore library copied.
- **The same for `db_key`** (the 32-byte key given to `Core.open`).

The app should `fill(0)` every `ByteArray` it holds as soon as the call that took it has returned (the constructor of
`FfiCredential`, `Core.open` for `db_key`), and `close()` a credential object once it has been handed over. This narrows the
window; it does not remove the copies above.

## Lints
The crate inherits the workspace lints, including `unsafe_code = "forbid"`. UniFFI 0.32's macros (callback interfaces and
async included) compile under it, so there is no exception.

## Building
```
cd core && cargo test -p croft-ffi
cargo run -p croft-ffi --features cli --bin uniffi-bindgen -- --help   # same UniFFI version as the library
```
The Android build (`android/`) runs cargo-ndk and the bindings generator; see `android/README.md`.
