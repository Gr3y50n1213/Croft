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
- composing and sending (since version 8): drafts, `send`, `undo_send`, the Outbox actions, recipient suggestions, reply and forward fields, identities. Everything is in "Composing and sending" below.

Callback interfaces the app implements: `CredentialProvider` (`credential`, `sign_in_needed`) and `DiscoveryObserver` (`on_probe`).
Every callback returns `Result<_, ProviderError>`; `ProviderError` has one fieldless variant, `Failed` (see "Callbacks").
Errors: `CoreError`, `DiscoverError`, `VerifyFailure`, and (since version 8) `ComposeError`: fixed categories, no free text.

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

## Composing and sending (version 8)

Code: `src/compose.rs` (the calls and records), `src/submit.rs` (the SMTP submitter), `core/mail-core/src/compose.rs` (writing the
message, reply fields), `core/protocols/src/drafts.rs` (the draft copy on the server), tables in `migrations/0006_compose.sql`.
Errors are `ComposeError` (fixed categories; the screens map each to their own text). Every call takes the `account_id` **and** the
id it acts on and refuses an id of another account (`NoSuchDraft`, `NoSuchOutboxItem`, `NoSuchMessage`, `NoSuchIdentity`), so a
screen holding a stale id from another account can never read or change anything there.

### Compose lifecycle (for the screens)

1. **Open compose.** New: `identities(account)` fills the From picker (the default first; there is always at least the account's own
   address and name). Reply, reply all, forward: `reply_draft` / `reply_all_draft` / `forward_draft(account, message_id)` return a
   `ReplyDraft { content, quote_included }`: put `content` in the fields. Nothing is stored yet. `mailto:`: the app fills a
   `DraftContent` itself. Reopening: `list_drafts(account)` after a restart finds what was open; `load_draft(account, id)` gives the
   `FfiDraft` (content, attachments, `server_copy`, `outbox_id`).
2. **Typing.** The recipient chips: `parse_recipient(text)` / `parse_recipients(text)` turn typed or pasted text into `FfiAddress`,
   `is_valid_address(address)` decides the error chip, `suggest_recipients(account, prefix, limit)` gives the autocomplete (see below).
3. **Autosave on the device.** About 1 s after typing stops: `save_draft(account, draft_id, content)`. First call with
   `draft_id = None` returns `SavedDraft { id, message_id, saved_ms }`; pass `id` from then on. It only stores (fast, synchronous).
   The draft's `message_id` never changes. Addresses are stored as typed, invalid ones included (they show as error chips after a
   restart too). After each save, `check_draft(account, id)` returns the facts of the Send button table in `docs/ux/controls.md`:
   `problems` (`NoRecipient`, `InvalidRecipient`, `AttachmentsTooLarge`: any of them means Send is disabled, with the reason),
   `invalid_recipients` (the chips to mark), `subject_empty` and `body_empty` (both true: Send asks "Send without subject or text?"),
   `attachments_bytes` and `size_limit_bytes` (25 MB; the server's own limit shows up at send time as `OutboxReason::TooLarge`).
   Offline and signed-out never disable Send. `compose_limits()` has the numbers (recipients, subject, body, attachments).
4. **Copy to the server.** 5 s after typing stops, at most every 20 s while typing goes on, and with `force = true` when the app is
   paused or compose is closed: `sync_draft_to_server(account, id, force)` (async). It appends the new version to the server's
   Drafts folder (the one in the folder mapping, else the one the server marks `\Drafts`, else it creates `Drafts`) and then
   deletes the old copy, found by the draft's Message-ID, so the server always has a copy and never two. The core paces it too: less
   than 20 s after the last copy of this draft the answer is `Deferred` unless `force`. The result is a `DraftSyncOutcome`
   (`Saved`, `AlreadyCurrent`, `Deferred`, `Offline`, `NotApplicable` for the sample mailbox, `TryLater`, `NeedsSignIn`, `Refused`);
   none is an error: the draft is safe on the device. **Offline or failing, the core copies it later by itself**: when
   `set_network_available(true)` is called and at the next start. An empty draft is not copied.
5. **Attachments.** **First register the attachment folder, once, right after `Core.open`:**
   `set_attachment_directory(path)` with an absolute path to an existing folder that holds only the copies the system picker's
   results are written to (for example `File(cacheDir, "compose-attach")`, created by the app). The core's database must not be inside
   it (`BadAttachmentDirectory`): never pass the app's data folder. Then `add_draft_attachment_file(account, id, path, name,
   mime_type)` (async) with the path of such a copy, or `add_draft_attachment_bytes(account, id, name, mime_type, bytes)` (async).
   They are stored in the draft (encrypted with the database). Limits: `MAX_ATTACHMENT_BYTES` 40 MB each and in total, 50 files; over
   25 MB is accepted so the chip can warn (Send is disabled by `check_draft`). **From a path the core reads only that file**: the
   path must be under the registered folder (`NoAttachmentDirectory` before one is registered), absolute, with no `..` and **no
   link anywhere below the folder**, and name a plain file with one name (not a link, folder, pipe, device or hard link), checked by
   size before it is read; the file is opened without following links and without waiting, examined again once open, and the read
   stops at the cap whatever the size says. Anything else, the database and the files of `/proc` included, is
   `AttachmentUnreadable` / `AttachmentTooLarge`. The name shown and sent is `name`, never the path's. Delete the cache file after the
   call returns. **Bytes** cross the FFI already in memory: check `compose_limits().max_attachment_bytes` before reading a file into
   memory (the core checks the size first, but the copy has been made by then), and use the file variant for anything large.
   `remove_draft_attachment(account, id, attachment_id)`.
6. **Send.** `send(account, id)` returns `SendStarted { outbox_id, send_at_ms }`. The message is written (its Message-ID is the
   draft's; Bcc recipients get it but are not in it), put in the Outbox with `send_at = now + the undo window`
   (`set_undo_window_ms`, the same setting as for moves), and sent when the window ends and the network allows, **exactly once**.
   Show the "Sending... Undo" snackbar until `send_at_ms`. The draft is **locked** (`InOutbox` from `save_draft`, `send`,
   `discard_draft`, attachment calls) until the message is sent or taken back. Refused with `NoRecipient`, `InvalidAddress`,
   `CannotSend` (no SMTP server in the account's settings).
7. **Undo.** `undo_send(account, outbox_id)` inside the window: nothing is sent, the message leaves the Outbox, and the returned
   draft id is editable again: reopen compose with `load_draft` (restoring the cursor is the app's job). `TooLate` once the window has
   closed or sending began; `NoSuchOutboxItem` if it is already gone.
8. **After the send.** Nothing for the app to do. When the server has taken the message the core deletes the draft, deletes its
   copy in the server's Drafts folder, files the copy in Sent (Gmail does that itself), and remembers the recipients for the
   suggestions. Each Outbox change (queued, sending, sent, failed, taken back) arrives in `next_change` as the account id in
   `ChangeBatch.accounts`: re-run `list_outbox` / `list_drafts`.
9. **Outbox screen (S26).** `list_outbox(account)`: `OutboxItem { id, state (Queued, Sending, Failed), send_at_ms, attempts, reason,
   may_have_been_sent, draft_id, subject, recipient_count, to_preview }`. `reason` is an `OutboxReason`, a fixed category, never the
   server's words (they can hold an address): `outbox_reason_key(reason)` is the key to translate, `outbox_reason_default_message`
   the English text. Actions: `retry_outbox_item(account, id, confirmed)` (for `may_have_been_sent` the call is **refused** with
   `NeedsConfirmation` unless `confirmed` is true: show "This message may already have been sent" (suggest a look in Sent) with
   Retry anyway and Cancel, and pass `true` only for Retry anyway; for any other Failed row pass `false`), `edit_outbox_item(account, id) -> draft_id`
   (back to the draft; not while Sending: `WrongState`. A message that was handed to the server at least once comes back with a
   **new Message-ID**, so the corrected message is not taken for the old one; the old draft copy on the server is deleted), `discard_outbox_item(account, id)` (ask first; drops the message and its
   draft). For "notify once" on a Failed row, remember the ids you have notified.
10. **Discard.** `discard_draft(account, id)`: gone from the lists at once; the copy on the server is deleted in the background.

### Suggestions
`suggest_recipients(account, prefix, limit)` (at most 20): addresses of mail the person **sent from this account** whose address or
display name (or a word of it) starts with `prefix`, most used first. There is no device-contacts source and no permission. The
history is written when a message sent through `send` has been accepted by the server (not when it is queued, and not when Undone),
and also read from the To and Cc of the Sent folder's stored messages. **Known gap:** the sync does not store `to_json` and `cc_json`
for any message yet (`RemoteHeader` carries no recipients), so an existing Sent folder adds nothing until the sync is extended; the
history starts with the first message sent through the app.

### Reply, reply all, forward
`reply_draft` / `reply_all_draft` / `forward_draft(account, message_id)` take a stored message (the row id of this account). To is
the sender, or `Reply-To` when the stored header block has one; the subject gets `Re:` or `Fwd:` once (`RE:`, `Fw:`, `FWD:` are
recognised); `In-Reply-To` and `References` are set for a reply (`References` is the original's, when its header block is stored,
plus its Message-ID). Reply all adds the original's To and Cc, **never the person's own addresses** (the account's and every
identity's) and never twice. Answering a message the person wrote goes to its recipients. `DraftContent.identity` is set to the identity
the original was sent to, if it was one of them. The quote and the forwarded text are taken from the stored plain text of the body
(`message_body.text_plain`) when it is stored; **until the reading calls (T18a) store bodies, the quote is empty** and
`quote_included` is false (a forward then has only the original's From, Date, Subject and To lines: fetch the body first). The
original's To and Cc are read from the stored header block (`message_body.headers_raw`) when there is one, else from the message's
`to_json` and `cc_json`; today that is usually nothing (see the gap above), so reply all is only as complete as that. The attribution
line ("On ..., X wrote:") and the forward header are English text built by the core; a translated version is a later change.
Forwarded attachments are not carried over yet.

### Sending for a real account: what the core does
- The engine is opened with an Outbox (`Engine::open_with_outbox`), and every account with an SMTP server in its stored settings gets
  a submitter (`AccountSubmitter`): SMTP to the stored server, the copy in Sent over the account's IMAP connection (found by
  Message-ID first, so never twice). An account without an SMTP server can read but `send` is `CannotSend`. Only Gmail files its
  own copy: the SMTP host must be exactly `smtp.gmail.com` or `smtp.googlemail.com` and the IMAP host Gmail's too (a relay such as
  `smtp-relay.gmail.com` files nothing).
- **The credential** is the same one IMAP uses, from the app's `CredentialProvider`, and is asked for **at each send** (not kept):
  it lives in a wiping string only while the message goes out. No credential means the account is paused as "needs sign-in" and the
  message waits; a busy keystore (`ProviderError::NotReady`) is a retry.
- **The certificate pin.** The account has one pin. It applies to SMTP only when the cleaned SMTP host equals the cleaned IMAP
  host **and** that host is not a provider's (`provider_for_host`). Otherwise SMTP uses normal certificate verification, and a
  provider's SMTP host (`smtp.gmail.com`) never takes a pin. (`StoredSettings::smtp_config`, `src/settings.rs`, with its tests.)
  Consequence: an account whose SMTP server is another host with a self-signed certificate cannot send.
- The sample mailbox sends through its own submitter (a copy in the sample Sent folder, nothing leaves the device); its drafts are
  never copied to a server (`NotApplicable`).
- The database now has three connections from the core (engine queue, Outbox, compose), all with the same key.

### Known gaps
Plain-text body only (no rich text); addresses are checked for structure and for invisible or direction-changing characters, but
non-ASCII domain names are not converted (no IDNA); only IMAP draft copies exist (JMAP accounts are not added by the FFI yet); no scheduled send (S25); no encryption policy (S24 is the app's seam); a draft in the Drafts
folder list (a synced server copy) is a second row next to the local draft with the same Message-ID, and the app should hide the
synced one; the Bcc recipients are not in the copy filed in Sent; a crash between the server append and the local record can leave
one extra old copy of a draft on the server until the next save, or until the draft is sent or discarded (the folder is noted before
the copy is made, so a copy whose record failed is deleted too).

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
