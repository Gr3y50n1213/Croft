UniFFI surface for Kotlin (and Swift later). The wasm-bindgen surface for the web is not part of this crate yet.

Keep it small and versioned; every change here is reviewed by hand.

## The surface (API version 8)
Free functions: `croft_version()`, `ffi_api_version()` (**bump `FFI_API_VERSION` in `src/lib.rs` on any change**;
the apps check it at start-up), and the text helpers `verify_failure_message_key` / `verify_failure_default_message`
and `discover_error_message_key` / `discover_error_default_message`.

`Core` (one object; `Core.open(db_path, db_key, credentials)`):
- accounts: `discover` (async), `verify` (async; since version 7 it checks against the stored pin of the same account and returns the pin it used, which goes to `add_account`), `add_account(discovered, display_name, pinned, confirmed_fingerprint, credential)` (async; the credential is the new account's, see "Credentials"; since version 7 it takes `confirmed_fingerprint` and returns `AddedAccount { id, updated, update_token }`: an address that is already added with the same IMAP and SMTP servers (host, port, security, user name) is **updated** instead of added again: same id, new settings, the account leaves the needs-sign-in state, and the app stores the credential under that id as for a new account, then calls `commit_account_update(id, token)` (or `rollback_account_update(id, token)`, which returns whether it undid that update, if it cannot store it); other servers are `CoreError::AccountExists`; a trusted certificate pin is never replaced, added to an account that had none or dropped without the person's confirmation of that fingerprint (`CoreError::CertificateChanged`; a pin is never dropped); the sample mailbox never counts), `add_demo_account()` (async, since version 6; see "The sample mailbox"), `commit_account_update`, `rollback_account_update` (async; both since version 7), `list_accounts`, `remove_account` (async), `signed_in` (async)
- reading: `list_folders`, `list_messages(scope, before, limit)` (keyset paging; `MessageScope` is a folder, the unified Inbox, or Starred for one account or all). Since version 5 a `MessageRow` has `thread_count` (messages sharing its conversation key in its folder) and `recipients` (names, else addresses, from the stored `to_json`), and `AccountInfo` has `last_synced_ms` (the newest folder sync)
- observing: `next_change(after)` (async; a numbered change log, see `src/changes.rs`)
- actions: `mark_seen`, `set_flagged`, `archive`, `trash`, `move_to`, `delete_forever`, `undo`, `set_undo_window_ms`
- sync and network: `sync_now`, `set_network_available`
- reading a message (since version 8, T18a): `open_conversation`, `load_body` (async), `save_attachment` (async),
  `allow_remote_content`, `trust_sender`, `untrust_sender`, `trusted_senders`; see "Reading a message". Their errors
  are `ReadError`

Callback interfaces the app implements: `CredentialProvider` (`credential`, `sign_in_needed`) and `DiscoveryObserver` (`on_probe`).
Every callback returns `Result<_, ProviderError>`; `ProviderError` has one fieldless variant, `Failed` (see "Callbacks").
Errors: `CoreError`, `DiscoverError`, `VerifyFailure`, `ReadError`: fixed categories, no free text.

## Reading a message (version 8)
Everything here lives in `src/reading.rs`; the parsing and the sanitiser are in `core/reader` (`croft-reader`).
Message ids are the same row ids as `MessageRow.id`; an id of an unknown message is `ReadError::NoSuchMessage`, and
an attachment id is only accepted together with its own message's id.

| Call | Returns | Notes |
| --- | --- | --- |
| `open_conversation(message_id)` | `Conversation { account, opened_id, messages: [ConversationMessage] }` | Sync, database only, offline. Oldest first. Expand `opened_id` and the newest; collapse the rest. |
| `load_body(message_id, remote_images: RemoteImagesSetting)` | `MessageBody` | Async. Fetches the message once if it is not stored (needs the network then), else works offline. Never marks it read. |
| `save_attachment(message_id, attachment_id, dest_dir)` | the file's path (`String`) | Async. `dest_dir`: the app's own cache directory, absolute. |
| `allow_remote_content(message_id)` | — | "Show images" on one message. Call `load_body` again. |
| `trust_sender(account_id, address)` | — | "Always show images from this sender" (the known-senders list). |
| `untrust_sender(account_id, address)` | `bool` (it was there) | For the S49 list. |
| `trusted_senders(account_id)` | `[String]` (lower case, sorted) | For the S49 list. |

Records:
- `ConversationMessage { id, account, folder, folder_special_use, sender_name, sender_address, to: [MailAddress],
  cc: [MailAddress], date_ms, subject, snippet, seen, flagged, answered, draft, has_attachments, pending,
  body_state: BodyState (NotLoaded | Loaded), remote_allowed }`. `to`/`cc` and `snippet` are filled once the body was
  loaded (the sync stores headers only). `folder_special_use` is `sent` for the person's own replies (show "You").
- `MailAddress { name: String?, address: String? }`.
- `MessageBody { message_id, text: String?, html_sanitised: String?, remote_allowed, blocked_remote_count,
  trackers_removed, sender_known, truncated, attachments: [AttachmentInfo] }`.
- `AttachmentInfo { id, file_name, mime_type, size, content_id: String?, shown_inline, blocked }`: show chips for
  `shown_inline == false`; `blocked` (programs and scripts: `.exe`, `.apk`, `.bat`, `.js`, `.scr`, `.msi`, `.jar`, ...)
  is the "blocked for executables" chip, never opened; `save_attachment` refuses it with `AttachmentBlocked`.
- `RemoteImagesSetting { Ask (default), KnownSenders, Never }`: the S49/S57 "Remote images" setting, passed on every
  `load_body`.
- `ReadError`: `NoSuchMessage`, `NoSuchAttachment`, `NoSuchAccount`, `AccountUnavailable`, `Offline` (not stored and no
  connection), `SignInNeeded`, `NotOnServer` (moved or deleted elsewhere), `TooLarge { size }` (over 50 MiB),
  `Unreadable`, `ServerUnavailable` (retry), `ServerRefused`, `AttachmentBlocked`, `BadDestination`, `WriteFailed`,
  `InvalidAddress`, `Storage { reason }`, `Internal`. No variant carries text.

### Who decides about remote images
The core does. The app passes the setting; `load_body` loads remote images when:
`Never` → never; `Ask` → the person tapped "Show images" on this message (`allow_remote_content`); `KnownSenders` → as
`Ask`, or the sender's address is on the account's known-senders list (`trust_sender`). `MessageBody.remote_allowed`
says what happened. **What S49 must call**: nothing when the setting changes (it is passed on each `load_body`); for its
known-senders list, `trusted_senders(account_id)` and `untrust_sender(account_id, address)`. The S21 bar offers "Show
images" (`allow_remote_content`, then `load_body` again) and, under `KnownSenders`, "Always from this sender"
(`trust_sender(account, sender_address)`). Hide the bar when `blocked_remote_count == 0` or the setting is `Never`.

### How the app renders a message
- **The WebView gets `html_sanitised` and nothing else** (`loadDataWithBaseURL(null, html, "text/html", "utf-8", null)`).
  Never the raw message, never the text pasted into HTML by the app. With `html_sanitised == None`, show `text` as plain
  text (no HTML).
- **JavaScript off** (`javaScriptEnabled = false`), **file and content access off** (`allowFileAccess = false`,
  `allowContentAccess = false`), no `addJavascriptInterface`, no zoom-out tricks needed.
- **Network off while remote content is blocked**: when `remote_allowed` is false call `blockNetworkLoads = true`
  (and `blockNetworkImage = true`). Only when it is true allow image loads; the HTML then only names `https` images (the
  core upgraded `http`). Inline (`cid:`) images are already in the HTML as `data:` URIs built by the core (PNG, JPEG,
  GIF, WebP, BMP only; never SVG), so they show with the network off.
- **Blocked images** keep their box: their `src` is a 1×1 grey GIF (`croft_reader::BLOCKED_IMAGE_PLACEHOLDER`).
- **Links**: the HTML keeps each link's real destination (`https`, `http`, `mailto`, `tel`) with
  `rel="noopener noreferrer"`. Handle taps in `shouldOverrideUrlLoading`: show the real URL before opening it in the
  browser (or the composer for `mailto:`); never navigate the WebView itself.
- **Mark as read** after the setting's delay with `mark_seen([id], true)`: `load_body` never does it.
- **Attachments**: `save_attachment(id, attachment_id, context.cacheDir/...)`, then share the returned file through the
  app's `FileProvider` with a temporary URI grant (store rule), never a `file://` URI.

What the sanitiser keeps and removes, the limits and the tracking-pixel rule are in `core/reader/src/sanitize.rs` and
`docs/DECISIONS.md` (T18a rows).

## The sample mailbox (version 6)
`add_demo_account() -> AccountInfo` adds "Try with sample mail" for store review (T17): about twenty fictional messages
(people and addresses at `example.org` / `example.com` only) in Inbox, Sent, Drafts, Trash and Archive, served from memory by
`DemoBackend` (`core/protocols/src/demo/`). It takes no credential and no secret, the `CredentialProvider` is never asked about it,
and it **never opens a connection** (no server adapter is built for it; `set_network_available` does not apply to it and its
status is never Offline). Calling it again returns the same account: there is at most one. `AccountInfo.is_demo` is true for it;
the app shows "Not saved to server" on its rows from that flag (rows have no flag of their own).

Read, star, move, archive, trash and delete forever work and are confirmed at once by the in-memory copy, so a sync agrees with
them. At `Core.open` the copy is rebuilt from what the device stored, so a sync after a restart undoes nothing either. The account
row stores only `{"version":1,"kind":"demo"}` in `config_json` (no migration); only that exact shape on the sample address counts,
and server settings refuse any record with a `kind` key. Sending, once the app's engine has an Outbox, sends nothing: the engine's
sample-mailbox submitter files the copy in the sample Sent folder and stores its row on the device before the Outbox lets go, so
a restart cannot lose it. `add_demo_account` also starts a stored sample mailbox that is not running. `remove_account` removes it
and all its rows like any other account (it waits for an `add_demo_account` in progress); `signed_in` returns at once for it. Tests: `src/tests/demo.rs` (with a provider that records any call, checked at the end of each test), the backend's own tests,
and `core/engine/tests/demo.rs` (sync, every action and a send on a runtime with no I/O driver).

## The test server (feature `test-server`, test only)
`--features test-server` adds the object `TestImapServer` (`src/test_server.rs`): the in-process IMAP server of `core/test-support`
(TLS on 127.0.0.1 with a fresh self-signed certificate, one made-up login) with calls to start it with N messages, stop it, inject a
message, change flags as another client would, read its command log, make a folder read-only (a permanent refusal), answer
sign-ins with "unavailable" (transient) or "wrong password", turn `UIDPLUS` and `MOVE` off, and cut connections. It is for the host
library the Kotlin JVM tests load (`android/mailtests`) and **never for the Android library**: `android/app/build.gradle.kts` keeps it
out of `rustFeatures` and `android/tools/check-release-outputs.sh` fails when a release `.so` carries its symbols. Run its own test
with `cargo test -p croft-ffi --features test-server`.

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
(`src/settings.rs`, with a version field). When the adapter of a stored account is built (at `open`, `signed_in`) the core
asks the `CredentialProvider` and passes the answer to `ImapBackend::new`. No answer: the account starts paused and
`sign_in_needed` is called once. A **new** account's credential is an argument of `add_account`: the core builds the first
connection from it and never asks the provider about a new account. The app stores that credential under the returned id right
after the call returns (and nothing when it fails), so the provider can answer for that id from then on. Account ids are never
reused (`core/storage/src/accounts.rs`), so an id carries no secrets of an earlier account.

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
