# Progress

Newest entry first. Each entry: status, what was built, which checks ran and their results, what is
unverified, the next step.

---

## T23: Companion foundation (12 November 2026)

**Status:** draft pull request #36 on `task/t23` (CI); independent review done, fixes included. Phase 2 begins; the plan is `docs/tasks/PHASE-2.md`.

**What was built.** `companion/`, a Cargo workspace of its own (AGPL-3.0-or-later): settings (TOML plus environment variables, unknown
keys refused, an https public address), the master key file (created with mode 0600, refused when others can read it, never
overwritten), envelope encryption (a random data key per user wrapped by the master key; values sealed under the data key bound to the
user and the field), SQLite storage with forward-only migrations (`meta`, `user`, `user_key`, `audit_log`), master key rotation that
survives a crash at any point, the command line (`init`, `user add`, `user list`, `rotate-master-key`, `serve`, `healthcheck`), and the
HTTP service (`/healthz`, `/version`, strict security headers, a body limit, a time limit, graceful shutdown). A Dockerfile (non-root,
`/data` volume), an example Compose file and a README on the master key. A `companion` CI job: fmt, clippy, tests, the image build.

**Checks.** 17 tests: sealing and opening bound to user and field, a wrong or rotated-away master key opens nothing, rotation keeps
every value, damaged input never opens; migrations once and a newer database refused; every table scoped by user or justified as
global; the settings file and environment; the key file's permissions; init twice keeps the key; a rotation cut short finishes at the
next start; the HTTP headers, the body limit and the health check. Clippy clean.

**Review** (one independent agent): two serious findings (a planted key file could be adopted; the Docker key volume was not
writable) and several smaller ones, fixed with tests (20 tests now); two accepted for now (DECISIONS).

**Not verified here:** the Docker image builds on CI only (this container's build sandbox cannot reach crates.io); running it on a real
host behind a real reverse proxy.

---

## T22: JMAP accounts in the app (12 November 2026)

**Status:** draft pull request #34 on `task/t22` (CI); independent review done, fixes included. FFI API version 12 (no type the app reads changed).

**What was built.** A server that discovery finds offering JMAP (and that is not a well-known provider's) becomes a JMAP account:
its own stored settings (session URL, login, pin; told apart by a `jmap` key, so earlier settings read exactly as before), the JMAP
adapter (T09) behind the account's backend over the persistent id map, `EmailSubmission` for sending (the server files the sent copy),
the same states as IMAP (no credential, storage not ready, needs sign-in, unusable), and the same rules for adding: a sign-in check
with the trust question for a self-signed certificate, the same address and server is signing in again, another server is
`AccountExists`, a pin never replaced unconfirmed. On Android, S03 no longer stops a JMAP-only server: S04 shows "host, JMAP" and the
account is added. Drafts of JMAP accounts stay on the device for now (DECISIONS).

**Checks.** Rust: 7 FFI tests against the in-process JMAP server (trust then add, sync, mark read, keeps no secret; sending with
EmailSubmission; a restart keeps the same rows; wrong password; signing in again; a self-signed server with a wrong password says
wrong password; settings shapes), the whole FFI suite (172) and clippy. Integration (CI, Stalwart): two JMAP accounts added through the
FFI, one sends to the other, the message arrives. JVM: the app's onboarding backend adds a JMAP-only account and its Inbox shows.
Robolectric: a JMAP-only lookup reaches S04 saying JMAP. Two older tests that used a JMAP-only server as "refused input" now use
input that is still refused (no server at all).

**Review** (one independent agent): six findings, all fixed (REVIEW-QUEUE), with tests: no request before the trust question, a
changed pin is `CertificateChanged`, an IMAP account on a JMAP-publishing server signs in again over IMAP, S05 settings stay IMAP, a raw
OAuth token is refused for JMAP.

**Not verified:** a real Fastmail or self-hosted Stalwart account on a phone.

---

## T21: bring-your-own OAuth for Gmail and Outlook (12 November 2026)

**Status:** merged to `main` through pull request #33 (all CI jobs green; independent review done, fixes included). **FFI API version 12.**

**What was built.** (1) *Core* (`croft_protocols::oauth`): the provider table (Google: `accounts.google.com`, `oauth2.googleapis.com`, scope
`https://mail.google.com/`; Microsoft: the v2.0 endpoints of `login.microsoftonline.com/common`, `IMAP.AccessAsUser.All`, `SMTP.Send`,
`offline_access`), PKCE `S256`, a random `state`, the authorisation URL, the check of the redirect (another flow's `state`, a missing
code and `access_denied` are typed errors), the token requests and the reading of their answers (`invalid_grant` is `Auth`, a slow or
failing endpoint is `Transient`), and a loopback listener on `127.0.0.1` that receives the redirect itself. Token requests go only to
the provider's token host (or, for tests, loopback). (2) *FFI*: `oauth_begin`, `oauth_finish`, `oauth_cancel`, `oauth_client_id`;
`FfiCredential.oauth_stored`; `CredentialProvider.save_tokens`. An OAuth account gets a token session shared by IMAP and SMTP: the
access token is refreshed a minute before it expires and once more when the server refuses it, then the account needs a sign-in.
(3) *Android*: S04 "Or sign in with your own Google/Microsoft client" for Gmail and Outlook: client ID (and the Desktop client's secret
for Google), the help (where to register, which client type, the redirect to copy for Microsoft, Google's 7-day limit for projects
left in Testing), the provider's page in a Custom Tab, a waiting line with Cancel, plain messages for each failure. The secret is kept
like a password (`AccountSecrets.addOAuthAccount`, `SecretKind.OAUTH_REFRESH`). *Signing in again*: the Needs sign-in banner and a new Sign in
button on S45 open the sign-in flow for that account's address (it did nothing before T21, for password accounts too); finishing it
there is the core's "sign in again" (same id, mail kept, paused changes go on).

**Decisions that depart from the task** (DECISIONS): loopback redirect for both providers instead of a redirect activity; Google
needs a "Desktop app" client with its own secret; a pending sign-in does not survive the process being stopped.

**Checks so far.** Rust: 16 unit tests (RFC 7636 vector, URLs, redirects, token answers, loopback), 5 in-process tests (code exchange,
rotation, `invalid_grant`, slow endpoint, XOAUTH2 with a good and an expired token), 9 FFI tests (servers that are not the provider's refused; the whole sign-in with a simulated
browser, sync over XOAUTH2, no token in the database file, refresh on a refused token with the rotated secret handed back, refresh before
expiry, revoked sign-in to needs-sign-in, cancel, access denied, bad client id).

**Not verified (needs real accounts and a phone):** the real Google and Microsoft consent pages, whether Google's token endpoint
accepts a Desktop client's code without the secret, Microsoft accepting any loopback port, the browser hand-off and the return to
Croft on real phones, refresh-token lifetime.

---

## T20: background check and notifications (12 November 2026)

**Status:** merged to `main` through pull request #31 (all CI jobs green; review done). **FFI API version 11.**

**What was built.** (1) *What counts as new* (`croft_mail_core::notify`, pure): unread, in the Inbox, above the account's "notified up to"
marker under the Inbox's UIDVALIDITY. The first check (or one after the Inbox was renumbered) only sets a baseline, so the mail already
there never notifies; the sample mailbox never notifies; the marker moves only after the app says it posted. The marker lives in the
`setting` table (`notify.marker.<account>`) and goes with the account. (2) *Engine and FFI*: `sync_once(account, deadline)` runs due
changes and a full sync within the deadline (a timeout is a flag, not an error) and returns sender, subject and date of each new
message; `mark_notified(account, uids)`. Problems are flags (`transient_failure`, `needs_sign_in`). (3) *Android*: one WorkManager
periodic work (`croft-mail-check`, every 30 minutes by default; 15 min, 1 h, 2 h or manual; network required; optional battery-not-low;
no expedited work, no foreground service, no exact alarm, no boot receiver of our own). The worker opens the process's core when cold
and checks each account in turn within 8 minutes; it retries only on a transient failure. S32 notifications: one channel per account,
a group with a summary, sender and subject only, lock screen private with a public "N new messages", up to three buttons (Archive,
Delete, Mark read, Reply). Archive and Delete leave a quiet notice with Undo for the Undo window. A tap opens the message with the
Inbox under it, through a non-exported activity-alias. One "Sign in again" notification per account, not one per run. S07 after the
first real account (the system prompt only on Android 13 and later, only there; a calm banner with a Settings link if refused; Send
test; Not now). S46: master switch, per-account switches, frequency, battery option, lock-screen option, buttons, Send test, a link
to the system's battery settings (never `REQUEST_IGNORE_BATTERY_OPTIMIZATIONS`). Erase and account removal clear the settings,
notifications and channels. (4) *Manifest*: `POST_NOTIFICATIONS`; WorkManager's `FOREGROUND_SERVICE` and `SystemForegroundService`
removed; the allowlist names the WorkManager job service and diagnostics receiver; the release check now refuses store-forbidden
permissions and foreground service types outright.

**Checks.** Rust: the decision table (first sync, already read, other numbering, below the marker, advance only after posting,
duplicate delivery, sample mailbox), `sync_once` against the in-process IMAP server (baseline, exactly the new unread mail, offered
until marked, deadline with a hanging server), FFI (flags, removal forgets the marker, version 11). Android (on CI): the runner (two
accounts, a paused one notified once, retry only on transient, nothing posted twice, deadline, muted accounts, refused permission),
the worker through `TestListenableWorkerBuilder`, the periodic request (unique name, interval, constraints, not expedited; KEEP,
UPDATE, cancel for manual), notifications through `ShadowNotificationManager` (channels, summary, no body text, public version, actions
and their intents, tap URL, refused permission), the action handler (fake action layer, Undo), the intent gate, S07 (no prompt below
API 33, one prompt and the banner on 33+, skip) and S46 behaviour, screenshots of S07, S07 refused and S46 (three widths, light and
dark, font 1.0 and 2.0, RTL). The release manifest check and its tests pass locally.

**Not verified (needs a phone):** Doze and app standby, the real delay under battery saver, grouping and the lock screen on several
makers' skins, channel vibration (OPEN-ITEMS). The READINESS item "confirm with a pre-launch review" stays open.

---

## T19: search (12 November 2026)

**Status:** merged to `main` through pull request #29 (all CI jobs green; review done, see REVIEW-QUEUE). **FFI API version 10.**

**What was built:** (1) *Query language* (`croft_mail_core::search`): words (prefix), `"phrases"`, `from:`, `to:`, `subject:`, `is:unread`, `is:starred`, `has:attachment`, `in:`, `before:`, `after:`; the FTS5 text is only quoted strings, so no input raises an FTS error; the only error is a typed bad date. (2) *Local search* (`croft_storage::search`): bm25 with the subject first, then date, then id; scoped by account; Trash and Junk only with `in:`; matches marked for highlighting; a note when bodies were not all downloaded. (3) *Server search*: `MailBackend::search` for IMAP (`UID SEARCH` with quoted criteria; a refusal is "unsupported") and JMAP (`Email/query` AND filter); the engine stores headers of hits the device lacked, so every hit is a normal row. (4) *FFI*: `search_local`, `search_server` (flags for unsupported, offline, sign-in, failed), ranges in UTF-16 units; the sample mailbox searches on the device only. (5) *Android* S27 and S28: 150 ms pause, local results then the servers' merged by id, typing again cancels the server request, notes at the end of the list, recent searches with Undo and Clear (on the device only), filter and account chips, Back from results to S27 to the Inbox, highlighted matches in the row component.

**Checks:** Rust: parser (every operator, hostile input), storage (ranking stability, account scoping, index triggers, body only once stored, every filter, hostile queries), IMAP in-process (sender, subject, body, flags, unsupported, non-ASCII), engine (stored hits, unsupported, offline), FFI (ranges, local, server bringing in a new message, sample mailbox), integration: GreenMail search passed here, Stalwart (JMAP) passed on CI. The FFI test found that the account wrapper did not forward `search` (every real account answered "unsupported"); fixed. JVM (`:mailtests`): 8 controller tests including the real core offline. Robolectric and screenshots (S27, S28, empty, offline; two widths, light and dark, font 1.0 and 2.0, one RTL) on CI.

**Not done:** non-ASCII words are not sent to IMAP servers (no literal support yet); recent searches are one list for all accounts; perceived speed on a mailbox of tens of thousands of messages, keyboard and highlight legibility are unverified on a device.

---

## T18b: conversation and message screens (12 November 2026)

**Status:** draft pull request #22 on `task/t18b` (CI only; not merged). T18 Part A on Android: S20 Conversation and S21 Message,
on the core of T18a (FFI 9) and the compose screens of T18d (merged, including its review fixes). No Rust change.

**Built.** A tap on a row of any folder opens S20. Messages oldest first; older ones collapsed to sender, date and snippet (a tap or
the TalkBack action "Expand message" opens them); the opened message and the newest expanded. Action bar: archive, delete (Delete
forever with a confirmation in Trash and Spam), mark unread, overflow with Mark as unread, Move to (S30 sheet) and Report spam (Not
spam in Spam). They run through the Inbox's own `RowActions` on the messages of the opened folder, close the screen and hand the Inbox
the Undo notice. Reply bar: Reply, Reply all (only when more than one other person is on the message, as docs/UX.md says), Forward,
through `ComposeStart`. Mark read after the S49 delay counted from when the body is shown; `load_body` never marks. The sample mailbox
shows "Not saved to server". Offline: "Not downloaded yet. It will load when you're online." with Retry, retried by itself when the
account changes. Every `ReadError` has plain-language copy (`strings_reading.xml`, `ReadCopyTest`).
S21: plain text in a selectable `Text`; HTML in one hardened WebView fed `html_sanitised` only; "Remote images blocked" bar (count, Show
images, Always for this sender under Known senders, a sentence and nothing that loads under Never); a quiet note when only the text
version is shown; attachment chips in every state; executables labelled "Blocked for your safety" and never saved or opened;
attachments open through a FileProvider (see below). S49: Remote images and Mark as read rows, and the known senders list.

**Decisions.** Mark-read default is After 2 s (docs/ux/customisation.md), not Immediately. Dark theme: HTML mail is never recoloured; it
sits on a light card (docs/UX.md); the simple-mail inversion is not built. Zoom: no rule in the docs; the page fits the width, pinch
zoom on, no zoom buttons, text size follows the system and S52 text size. Show original, unsubscribe, Snooze, Mute thread, Print and
Block sender are not offered (no FFI call or later phase), no disabled placeholders. The reading font applies to plain text only.
`data:` and `about:` requests pass `shouldInterceptRequest` in both modes (they never leave the device).

**WebView settings (exact).** `javaScriptEnabled=false`, `allowFileAccess=false`, `allowContentAccess=false`,
`allowFileAccessFromFileURLs=false`, `allowUniversalAccessFromFileURLs=false`, `blockNetworkLoads=!remote_allowed`,
`blockNetworkImage=!remote_allowed`, `safeBrowsingEnabled=true`, geolocation off, `saveFormData=false`, `domStorageEnabled=false`,
`databaseEnabled=false`, multiple windows off, `mixedContentMode=NEVER_ALLOW`, `cacheMode=LOAD_NO_CACHE`, `forceDark=OFF`,
`textZoom` from the font scale; no `addJavascriptInterface`, no `WebChromeClient`; `shouldOverrideUrlLoading` always true (both
overloads); `shouldInterceptRequest` answers 403 with an empty body for everything while blocked and for everything but `https` when
allowed. Links: http, https and mailto show a dialog with the real address (hidden characters written out) and open only on confirm
(browser chooser; mailto in Croft's compose); anything else is ignored.

**FileProvider.** `androidx.core.content.FileProvider`, authority `<applicationId>.attachments`, `exported=false`,
`grantUriPermissions=true`; `res/xml/shared_attachment_paths.xml` has one entry: `<cache-path name="shared_attachments"
path="shared-attachments/"/>`. The core saves into `cacheDir/shared-attachments/`; the viewer gets `ACTION_VIEW` + MIME + a chooser with
`FLAG_GRANT_READ_URI_PERMISSION` only. The folder is emptied at app start, when the viewer returns and when the screen is left.
`check-release-manifest.py` now fails on any provider with `grantUriPermissions` that is not allowlisted by authority, not exported
`false`, or with another paths file (`uri-grant-providers`). T18d's own-provider check covers this authority (test in `SharedAttachmentsTest`).

**Checks.** Locally (host Rust 1.99 through `RUSTUP_TOOLCHAIN`): `:mailtests` reading tests against the in-process IMAP server and the
sample mailbox (conversation, body not marked read until `mark_seen`, remote blocked, Show images, allowed, Never wins, known senders,
attachment saved into the folder, a blocked executable refused through a draft appended to Drafts, every `ReadException` variant has a
`ReadProblem`) and `ReadingRulesTest` (links, requests, reply-all rule) pass; `python3 -m unittest discover -s android/tools` passes;
the app and its unit tests compile. Robolectric and screenshot tests cannot run here (Maven Central refuses the container), so they ran on CI only.

**CI.** First run (`c1b4ec3`): 6 of 7 green; `android` had 13 failed tests, 2 of them T18d's (fixed by its `210f52f`, merged) and 11 of mine (merged semantics on three nodes, Safe Browsing asserted on a recording settings object because Robolectric's is a stub, the FileProvider static path cache cleared per test, a folder out of view in the Move to sheet, a state change with a paused clock). Second run (`123393f`): all 7 jobs green (`design`, `schema`, `core`, `sqlcipher`, `integration`, `android`, `android-release`).

**Unverified (needs a device).** How real HTML mail renders and scrolls inside the conversation (WebView under Robolectric draws
nothing; the screenshots show an empty light card), pinch zoom and nested scrolling, the chooser and real viewer apps, TalkBack reading
order, the WebView's own Safe Browsing interstitial.

**Next.** Tablet list and detail (T01), simple-mail dark inversion, Show original when the core has a call, per-message Save and Share
from the chip menu.

---

## T18a: reading in the core (11 November 2026)

**Status:** draft pull request #18 on `task/t18a` (CI only; not merged). Part A of T18, core side only: no screens.
**FFI API version 9** and migration **0007** (T18c merged first with version 8 and migration 0006; `claude/croft-build`
is merged into this branch).

**Opus review round 3: 6 high (DoS), 2 medium, 3 low; fixed by a work budget inside the parser** (12 November).
The shape scan's prediction is replaced by measurement: the vendored html5ever (new change 3 in its `CROFT-PATCHES.md`)
and a newly vendored ammonia 4.2.4 (`core/vendor/ammonia`, MIT OR Apache-2.0, notices kept) charge every token, every
step over the stack of open elements and the formatting list, every node, every rebuilt attribute byte, the DOM's
sibling searches and the CSS and URL work to a per-thread budget (`html5ever::budget`): **30 000 000 units and 8 MiB of
rebuilt attributes** per document. When it is spent the parser stops at once and the message shows as text. Input is
normalised first (CR, CRLF to LF; NUL to U+FFFD); the serialiser writes into a buffer that refuses more than 8 MiB; the
3 s deadline is a backstop that cancels the parser (its thread ends within milliseconds); one sanitiser thread per
process (a caller waits for it within its own deadline, then shows text); a deadline miss or a busy sanitiser is
`html_pending` (stored, retried on the next open, never a mark), only deterministic outcomes are `too_complex`. L1: the
scan counts implied `tr`/`colgroup`. L2: size-only folding only when neither copy is fetched. L3: one fetch and sanitise
per message (a per-message lock in the engine). Release, x86, every input to 4 MiB with the budget alone (no shape
scan): worst 0.31 s and 92 MB; newsletters render (a 4 MiB MJML newsletter uses 16.5 million units). Table in
`docs/DECISIONS.md`; regression test per finding (`tests/sanitize.rs`, `tests/sanitizer_thread.rs`,
`engine/tests/body.rs`, `storage` reading tests), charge-point tests that fail when a charge is removed
(`tests/budget.rs`, checked by removing two charges), and a guard test on the vendored sources. The S60 licence list was
regenerated with `gen_licences.py` (Gradle ran offline this time). Not re-reviewed.

**Opus review round 2: 3 high (DoS), 2 medium, 5 low; all fixed** (12 November; N6 accepted and documented). N1:
the shape scan never pops on end tags inside quoted values, comments, CDATA or bogus comments, and void names nest once
`<svg`/`<math` appeared. N2: a node budget for rebuilt formatting elements, 1 500 000 (0.31 s and 125 MB at the cap;
the legacy test layout counts 1.17 million and still renders). N3: html5ever vendored with a one-pass escape and no
logging of token text (`core/vendor/html5ever/CROFT-PATCHES.md`). D: sanitising runs on its own thread with a 3 s
deadline (30 s in debug builds); past it, or past 8 MiB of markup, the message shows as text and a stored
`too_complex` mark stops any retry. N4/N5: browser-style dimension reading and CSS functions in sizes count as
pixels. N7: `getrandom`. N8: every Cf character stripped, 11 more launcher extensions. N9: copies fold only with the
same SHA-256 (or the same server size before both are fetched). N10: `styles_removed` counts kept elements only. Every
reviewer input now falls back in under 1 ms, or runs in under 0.4 s (release, x86; `limits_bench.rs`).

**Opus review: 4 high (hang or memory), 2 medium, 5 low; all fixed** (12 November). H1/H2: a byte scan that can only
over-count (`croft_reader::shape`) measures tags, depth, the parser's cost (sum of depths) and attributes per tag before
any parser runs; limits 50 000 tags, depth 2 048, cost 40 000 000, 256 attributes per tag; worst case found inside them
0.22 s (release, x86), the review's 47-58 s inputs refused in under 1 ms, newsletter-shaped HTML (including a legacy
layout at depth 1 210) still renders. H3: references deduplicated with a set and cut to 100 from each end (also in the
IMAP and JMAP header code). H4: a 21 MiB budget for `data:` URIs per document. M1: pixels decided inside the ammonia
pass on the output tree (marker attribute per `img`, random tokens for remote addresses), sizes in any unit, opacity,
srcset candidates. M2: names trimmed again after cutting; `save_attachment` re-checks the final name. L1: inline images
typed by their bytes. L2: more executable types (local HTML/SVG/MHT/CHM, macro Office files, `.apkm`), superscript
device names, blank-looking characters. L3: only true copies (same Message-ID, sender and date) fold in a conversation.
L4: existence checks read no blob. A regression test per finding; the timings are in `core/reader/tests/limits_bench.rs`
(ignored; run in release). T18c's `reply_all_draft` and Sent-folder suggestions now get To and Cc from every message
whose body was loaded (test `a_loaded_body_gives_reply_all_its_to_and_cc_and_its_quote`); messages never opened still
have none. The S60 licence list (`open-source-licences.json`) gains the 20 new crates: its Rust entries were
regenerated with `gen_licences.py`'s own `rust_entries()` (Gradle cannot run offline here; the Kotlin inputs did not change).

**What was built:** (1) `MailBackend::fetch_body(folder, uid) -> RawMessage { Found(bytes), Gone, TooLarge { size } }`:
IMAP `EXAMINE` + `RFC822.SIZE` + `BODY.PEEK[]`, JMAP `blobId` download from the session's `downloadUrl` (session origin
only), the sample mailbox serves `message_source` (and a stand-in for a message whose source was not kept), the app's
account backend forwards it; a default answer for adapters that have none. 50 MiB limit, checked before downloading.
(2) New crate `croft-reader` (pure): `render` parses with mail-parser 1.0 (caps on size, parts, depth, text, HTML,
inline images), sanitises the HTML with ammonia configured for mail (see `sanitize.rs` and `docs/DECISIONS.md`), lists
attachments with safe names and executable marks; `part_content`; `remote_allowed`. (3) Migration 0007 and
`storage::reading`: the fetched message (`message_raw`), body counts, attachment details, `message.remote_allowed`,
`trusted_sender`, a Message-ID index, and the conversation query (Sent in, Trash/Junk/Drafts only when opened from
there, one copy per Message-ID, widened through Message-IDs); a confirmed move takes all of it along. (4)
`EngineHandle::fetch_body`: fetch, parse on a blocking thread, store; never marks read; offline and sign-in cases
typed. (5) FFI (`core/ffi/src/reading.rs`): `open_conversation`, `load_body`, `save_attachment`, `allow_remote_content`,
`trust_sender`, `untrust_sender`, `trusted_senders`, records `Conversation`, `ConversationMessage`, `MailAddress`,
`MessageBody`, `AttachmentInfo`, enums `BodyState`, `RemoteImagesSetting`, error `ReadError`; README section "Reading a
message" (how the app renders: sanitised string only, JavaScript and file access off, network off while blocked).
Kotlin: only `EXPECTED_FFI_API_VERSION` and its test literal.

**Checks:** in this container (Rust 1.99, the CI's stable; the default 1.97 cannot build mail-parser 1.0): `cargo fmt --check`,
`cargo clippy --all-targets` and `-p croft-ffi --features cli`, `cargo test` (all crates: reader 33 tests including the
fuzz-style test, storage 153, engine, protocols, ffi 98), `croft-storage` with `sqlcipher` (clippy and 156 tests),
`croft-ffi` with `sqlcipher` (102) and `test-server` (clippy and 99), `check_schema.py`: all pass. Integration
(`cargo test -p croft-protocols --features integration`) against GreenMail, Dovecot and Stalwart under compose project
`t18a` on remapped ports: all pass, including the new `body_integration.rs` (a body and its attachment, byte for byte, on
all three servers, still unread afterwards). `smtp_integration.rs` reads `CROFT_GREENMAIL_*_PORT`, not
`CROFT_TEST_GREENMAIL_*_PORT` (older naming; it passed with those set). Containers stopped. Gradle was not run here
(Maven Central refuses this container): the Android jobs run on CI. Found by testing: 100 000 nested `div`s took 340 s
to sanitise, hence the depth limit; the fuzz test found that an unknown CSS function name (`--x--url(...)`) passed the
first CSS filter, hence the function allow-list.

**Not done / not verified:** the JMAP thread id is not used for conversations (the sync does not store it, and JMAP
accounts cannot be added from the app yet): conversations use References/In-Reply-To on both protocols. No screens
(S20, S21 are Part A's app half, for the Android agent). The review fixes were not re-reviewed. Rendering fidelity in a real WebView is unverified. "Show original" has its data
(`message_raw`, `headers_raw`) but no FFI call yet.
## Preview build for phone testing (12 November 2026)

**Status:** draft pull request `task/preview` (CI only; not merged).

**What was built:** a `preview` build type in `android/app/build.gradle.kts`: `initWith(release)` (R8 and resource shrinking, not debuggable), signed with the Android debug key (made by the build tools, never committed), `applicationIdSuffix = ".preview"`, app name "Croft Preview" (`app/src/preview/res/values/strings.xml`). The release stubs (`src/release`: `TestHooks`, `DebugGallery`) are shared with the preview source set in place (a source set is named after its build type, so `initWith` alone does not bring them; `src/debug` is never included). `checkPreviewSourceSets` fails the build if that wiring is lost, and `checkPreviewMapping` and the keep-rules check apply to preview too. The Rust library is the same release build for both ABIs. CI: the `android-release` job also runs `:app:assemblePreview`, runs `tools/check-release-outputs.sh` with `CROFT_VARIANT=preview` (debug-key signature, not debuggable, manifest allowlist, 16 KB alignment, no test-server code, no debug test hooks) and uploads `croft-preview-apk` (30 days). The guide for the owner is `docs/TESTING-ON-A-PHONE.md`. No new permission, no new dependency.

**Unverified:** the Gradle build was not run here (Maven Central refuses the container); CI is the check. The guide marks the reading and compose steps as depending on the next update.

---
## T18d: compose and Outbox screens (12 November 2026)

**Status:** draft pull request `task/t18d` (CI only; not merged). T18 Part B on Android: S23 Compose, S24 Encryption check (the policy seam), S26 Undo send and Outbox, on the core's compose calls (FFI API version stays 8). No Rust change. No new permission, no new library.

**What was built:** (1) *Logic apart from the screens* (`mail/compose/`, plain Kotlin, compiled into `:mailtests`): `ComposeController` (From, To, Cc, Bcc chips through the core's `parse_recipients` and `is_valid_address`, autosave: device 1 s after typing stops, server 5 s after and at most every 20 s, and on pause; the Send button's table from `check_draft`; Back rules; Send with the questions in front of it), `AttachmentIntake`, `OutboxController`, `EncryptionPolicy`, `Mailto`, `FakeComposeBackend`. (2) *Screens:* S23 (From picker over every identity of every account, chips with a menu, Cc and Bcc behind a toggle, suggestions from `suggest_recipients` only, subject, plain-text body, attachment chips, a quiet "Saved / Saved on this device / Waiting for connection" line, Offline and Sign in banners), the S24 sheet, S26 (Queued, Sending with a progress ring, Failed with the reason from `outbox_reason_key`; Retry, Edit, Discard). (3) *Wiring:* FAB, the row's Reply action, `mailto:`/`SEND`/`SENDTO`/`SEND_MULTIPLE` intent filters on `MainActivity` (`singleTop`), Outbox in the drawer, "Draft saved" and the Undo snackbar on the Inbox (Undo calls `undo_send` and reopens compose on the draft with the cursor), focus to the first row after Send, local drafts at the top of a Drafts folder. (4) `Core.setAttachmentDirectory` is called right after `Core.open` on `cacheDir/compose-attach`, emptied at each start.

**Entry points for S20 and S21:** `ComposeStart.Reply(account, messageId)`, `ReplyAll(...)`, `Forward(...)` passed to the navigation host's `openCompose`. See `android/README.md`, "Compose and the Outbox".

**Attachments:** picker (system document picker or photo picker, no permission) gives a `content:` URI; the declared size is checked against `compose_limits().max_attachment_bytes` before anything is copied; the copy goes to `cacheDir/compose-attach/att-<random hex>`; `add_draft_attachment_file` reads it; the copy is deleted whatever the result.

**Manifest check:** `check-release-manifest.py` now also checks the intent filters of an exported component against `actions` and `browsable-schemes` in the allowlist (new tests in `test_check_release_manifest.py`).

**Checks that ran here:** `:mailtests:test` (all: the 21 earlier files plus `ComposeLogicTest`, `ComposeFlowTest`, `OutboxTest`) on the real core and the in-process IMAP server; `:app:compileDebugKotlin` and `:app:compileDebugUnitTestKotlin`; `python3 -m unittest discover -s tools`. **Not run here:** Robolectric and screenshot tests (Maven Central refuses this container). They ran on CI: all 7 jobs green on `7a86eaa` (the first CI run had 43 failures, all from a test fixture whose constructor parameter hid the compose backend; fixed, then two tests that matched two nodes).

**Existing tests:** none deleted or weakened. `Rig` in `MailTestKit.kt` got an optional `smtp` flag (default off), so the earlier tests are unchanged. The Reply row action keeps its "comes later" snackbar when the screen is given no `onReply` (the existing `SwipeSemanticsTest` still sees it).

**Opus review: 3 high (intent crashes), 6 medium, 8 low; all fixed** (head after the review: see the pull request). H1 to H3: `ComposeIntents.parse` is total (each extra read on its own with its type checked, `BadParcelableException` and any `RuntimeException` give an empty compose). M1: own-provider and own-MediaStore `content:` URIs refused (`ComposeFiles.isForeign`, DECISIONS). M2: chips show `Name <address>` (middle ellipsis keeps the end; the chip menu shows the address). M3: files from a share are pending jobs Send waits for. M4: `ComposeRetained` (ViewModel) keeps compose open across a rotation. M5: an empty snippet never matches, the drafts read is dropped if the folder changed, Select all takes only shown rows; the row cannot carry the Message-ID without an FFI change (not done, DECISIONS). M6: typed recipient text becomes a chip at Send and Back. L1: copies run on workers with a 30 s stall timeout, Back cancels them, each pending file has Cancel. L2: recipients capped (100 from a link or share, the core's limit in compose). L3: a snackbar says when shared text was cut. L4: names lose every Cf character, chips cut in the middle. L5: a link or share while compose is open saves it first ("Draft saved"). L6: Back keeps a share that was not typed in (existing tests adapted: a `Fill` left alone now ends as a saved draft; new and reply left alone still leave nothing). L7: one copy at a time against the live totals. L8: removing an attachment has Undo (the removal is final after 5 s, at Send and at Back). Info: `ComposeState` and results print no subject, body or name.

**Gaps and choices:** (a) The Drafts folder list does not carry a Message-ID, so a server row is hidden only when it has the same subject and start of text as a local draft (`LocalDraftMatch`); a missed copy shows twice, never hides other mail. Local drafts are shown above the folder's list and open in compose; a server-only draft opens nothing yet (needs the reading calls). (b) Undo after the window: the core still takes back a message whose first send failed and is waiting to retry; the "too late" answer is the core's own and is tested in Rust. (c) Account switching in From starts a new draft in that account and is offered only while there is no attachment. (d) Schedule send (S25): long-press on Send says it is not available. (e) The attachment chip's tap removes the file (new optional `tapLabelOverride` on `AttachmentChip`). (f) Screenshots are not committed as goldens. (g) The "may have been sent" state could only be tested on the in-memory backend (the host library has no SMTP server); the Rust tests cover the core side. Unverified on a device: keyboard and IME behaviour, the system pickers, a real share.

## T18c: compose and sending in the core (11 November 2026)

**Status:** draft pull request `task/t18c` (CI only; not merged). Part B of T18, core side only: no screens. **FFI API version 8** (T18a may also bump it; whoever merges second takes the next number).

**What was built:** (1) *Sending for real accounts.* `Core::open` now opens the engine with an Outbox (three connections to the database file: queue, Outbox, compose). Every account with an SMTP server gets an `AccountSubmitter` (`core/ffi/src/submit.rs`): SMTP to the stored server, the credential asked from the app's `CredentialProvider` at each send and dropped, the copy in Sent over the account's IMAP connection (Message-ID looked up first, folder created if missing; Gmail files its own). The SMTP pin rule is in `settings.rs` and tested. (2) *Drafts* (migration 0006, `core/storage/src/drafts.rs`): stored locally, a stable Message-ID, attachments in the database (caps; a path from the app is read only if it is an absolute plain file), the copy on the server is look up, APPEND with `\Draft`, delete the old (`core/protocols/src/drafts.rs`), paced to 20 s, repeated later when offline. (3) *Outbox actions:* `send`, `undo_send`, `list_outbox`, `retry_outbox_item`, `edit_outbox_item`, `discard_outbox_item`, with `OutboxReason` categories instead of text; the Outbox row carries the draft id. (4) *Suggestions* from a `sent_recipient` table (and the Sent folder's stored To/Cc). (5) *Reply, reply all, forward* fields; *identities*. (6) A message writer (`core/mail-core/src/compose.rs`): 7-bit output, no typed text can add a header line, Bcc only in the draft copy. The calls and the compose lifecycle for the Android screens are in `core/ffi/README.md`. Kotlin: only `EXPECTED_FFI_API_VERSION` and the literal in `RustBridgeTest.kt`.

**Never twice, at each crash point:** the Outbox rules of T06 are unchanged (row marked `sending` on disk before the server is contacted; a row found `sending` after a restart is Failed "may not have been sent" and is never resent by itself). On top: a draft with an Outbox message is locked; the Outbox row carries the draft id, so no crash separates them; Undo/Edit/Discard clear the draft's link before the Outbox acts and run under one lock with the check for sent messages, so a crash reads as taken back and never as sent. Tests: `core/ffi/src/compose/tests/sending.rs` (delivered once; offline then once; Undo sends nothing; second Send refused, also from four threads; `AcceptThenDrop`; killed while going out; restart inside the window; crash between the Outbox write and the draft's mark; crash in the middle of an Undo, both ways).

**Checks:** `cargo fmt --check`, both clippy runs (and `--features test-server`), `cargo test` (all crates), `cargo test -p croft-storage --features sqlcipher`, `cargo test -p croft-ffi --features sqlcipher`, `check_schema.py`: pass here. Integration (GreenMail in Docker, own compose project `t18c` and ports): `croft-ffi --features integration compose::tests::integration` (draft APPEND then replace then delete on a server that starts with only an INBOX; offline send delivered once; Undo sends nothing), `croft-protocols` `drafts_integration` and `smtp_integration`: pass. Not run here: the Android build and `:app` tests (Maven Central refuses the container); the Dovecot and Stalwart parts of the integration job.

**Opus review: 2 medium, 9 low; all fixed.** M1: an attachment path could read any file the app can read; now one folder is registered with `set_attachment_directory` (right after `Core.open`), nothing outside it or through a link is read, the open does not follow links or wait, and the opened file is checked (regular, one name, same inode, size). M2: Edit of a message that may have been sent gives the draft a new Message-ID (the old one only to delete the old server copy). L1: the sent-message check runs before any account starts. L2: an engine with no Outbox says so (`outbox_entries` is `None`) and the check does nothing. L3: `retry_outbox_item` takes `confirmed` and refuses a maybe-sent row without it. L4: Gmail files its own copy only for the exact SMTP hosts with a Gmail IMAP host. L5: the draft copy is found and deleted among `\Draft` messages only. L6: header lines stay short (encoded words for unbroken words, RFC 2231 pieces for long file names). L7: invisible and direction-changing characters make an address invalid (no IDNA). L8: the Drafts folder is noted before the first append. L9: attachment bytes are size-checked first and the README tells the app to check before the call. The flaky test searched for the letters "bcc", which the random Message-ID can contain; it now checks the same property precisely (no `bcc:` header line, hidden address absent) and was run 300 times.

**Not done / gaps:** the sync does not store To and Cc (`RemoteHeader` has none), so suggestions start empty and reply all is only as complete as the stored header block (needs T18a's body); the quote is empty until bodies are stored; plain text only; no scheduled send (S25), no S24 policy; punycode for non-ASCII domains; the synced server copy of a draft appears in the Drafts folder list next to the local draft (the app should hide the row with the same Message-ID); Bcc recipients are not in the copy filed in Sent; the attribution line is English; JMAP accounts (the FFI adds IMAP accounts only).
## T17c: accounts, About, sample mail on the core (11 November 2026)

**Status:** draft pull request `task/t17c` (#15, CI only; not merged). The last part of T17: S44 Accounts, S45 Account, S60 About and diagnostics, and "Try with sample mail" on the core's own sample account. No core or FFI change (FFI API version stays 6), no new permission, no new dependency.

**Sample mail:** S01's button calls `add_demo_account()` (no network, no password) and opens the Inbox over the core session. The app's session is now always over the core, with or without accounts, so the sample account persists and a restart opens the Inbox. Rows of the sample account say "Not saved to server" (`AccountInfo.is_demo` turned into `MessageRowData.localOnly`; the core flags no message rows). The old Kotlin fake-data Inbox (`FakeMessageRepository`, `FakeData`) cannot be reached in the running app any more: it stays for tests and screenshots; while the core cannot be opened the session is an empty stand-in (`MailSession.coreUnavailable`, never drawn) and S01 says so.

**S44:** one row per account (name, address, status chip from the core: OK, Syncing, Needs sign-in, Offline, Server problem; the sample mailbox is labelled and shows no address), Add account (the S02 flow, back to S44 when done; also wired to the Inbox's account switcher, "Manage accounts" goes to S44), Move up / Move down (order kept by the app in `croft-accounts.json`), Remove (lists the messages and folders with counts, queued changes, drafts and outbox, and the saved password; then removes). The last account removed returns to S01.

**Remove order:** the core first (`Core.removeAccount` stops the account and waits for a message being handed over), then the saved secret, then the app's own entries (the owner's decision after review; the first version deleted the secret first). A crash between the steps is finished by the sweep at the next start. An id is not reused while the database exists; an erase deletes the database and everything keyed by an id together.

**S45:** name (app-side), status, colour (six theme roles, app-side), folder mapping for Trash, Archive, Junk, Sent and Drafts (`FolderMapping`; a folder the server marks is shown and used, the person chooses only where there is none), Remove, Erase all data. **Left out, listed in `docs/OPEN-ITEMS.md` for T18:** identities and signatures, sync window (no core storage), server settings (the core cannot read them back, so S05 could not be pre-filled).

**S60:** version, privacy policy (view intent; the address is a **placeholder**, `AboutLinks.PRIVACY_POLICY_URL`, also linked on S01), source link (also a placeholder, `AboutLinks.SOURCE_URL`), open-source licences, Copy diagnostics (versions, account count and states only), Erase all Croft data (confirmation with the list, then every account through the core and its secret, the secret store, the app's files, the settings stores back to defaults, then S01), and the "not affiliated with Google, Microsoft or Apple" note. **Licence list:** a checked-in JSON made by `android/tools/gen_licences.py` from `cargo metadata` (Rust crates), the release runtime classpath and POMs (Kotlin libraries) plus OpenSSL, SQLCipher, SQLite and the fonts; CI (`test_gen_licences.py`) fails when `Cargo.lock` or `libs.versions.toml` changed without a new list. It shows names and licence names, not full licence texts (open item).

**CI:** all 7 jobs green on `b21d36e`. It took five runs: the first had 10 failing Robolectric tests, all in my new or adapted tests (the activity could not open the core on a simulated device; dialog list items matched without their bullet; one "OK" label matched twice; the sample rows were not in the visible part of the unified list and the row merges its children, so its tag is in the unmerged tree; a fixture that ignored hidden folders; the folder chooser's list is below the fold of a small test window). Each was a test or fixture assumption; the one product change was S45 keeping the chosen folders in screen state.

**Tests:** `:mailtests` (real core, run locally: all pass): 7 new tests (sample mail with `set_network_available(false)`, Inbox rows and unified Inbox, flag change stays local, rows say not saved, removal plan, removal order and ids never reused, erase everything, diagnostics with no address or host, account prefs) and the existing 37. Plain JUnit (CI): removal and erase over the real core and a real `SecretStore` (`AccountRemovalTest`), the licence list. Robolectric (CI only; the Robolectric jar cannot be downloaded here): `AccountsScreensTest` (S44 chips, remove list and confirm deletes the secret, cancel, failure, last account returns to S01, reorder, add opens S02, S45 name and colour, folder mapping saves, remove from S45, S60 version and note, privacy and source links fire view intents, licences list, diagnostics on the clipboard, reset erases after confirmation, sample-mail flow with the network off from S01 to the Inbox, Settings and Accounts, a failing sample open) and 27 screenshots (S44, S45, S60: compact and expanded, light and dark, font 1.0 and 2.0, plus one right-to-left). Main and test sources compile locally (`compileDebugKotlin`, `compileDebugUnitTestKotlin`).

**Existing tests adapted:** `OnboardingTestKit.fakeSession` lost its `usesFakeData = false` argument (the flag is now `coreUnavailable`, false by default). `MainActivityTest` (HelloScreenTest.kt) used to reach the Inbox through the old fake-data sample; now "Try with sample mail" adds the core's sample account, and a simulated device has no usable AndroidKeyStore, so the test runs with `InMemorySecretsRule` (an in-memory wrapping key through the test-only hook `MailRuntime.secretsFactory`); what it checks is unchanged. Two new tests there: the full sample flow on the real core with the network off, and a restart that opens the Inbox because the sample account exists. Nothing weakened.

**Unverified:** TalkBack on the new screens, the real Keystore, the privacy address.

**Opus review of `eaa1e90`: 2 high, 3 medium, 5 low; all fixed** (CI on `eaa1e90` itself was green). H1: "Erase all" left the T11 quarantine, which the next start restored; now `AppSecrets.eraseEverything()` deletes the quarantine, the wrapped key, the notices and the Keystore wrapping key, and a single removal deletes the account's quarantined secret files. M3 (decision): erase also closes the core and deletes the database, so the next start is a fresh install. H2: the test hook is `TestHooks` in `src/debug` with a release stub, a source test and a release-mapping check. M2 (decision): the core removes the account first, then the secret, then the app's entries, with a durable removal mark and a start-up sweep that finishes a cut-short removal (it deliberately leaves an earlier database's quarantined secrets alone, see `docs/DECISIONS.md`); the removal dialog says a message being handed over may still go out; a test on the real core and an in-process server shows nothing is sent after a removal with a queued change (no Outbox message: the FFI has no send call yet). M1: folder choices and prefs are removed with the account (waited for, inside the non-cancellable path), S45 marks the account gone before it asks, and the sweep prunes both. L1: a failure after the core removed the account is not "couldn't remove". L2: the list says "saved sign-in" for tokens. L3: the licence list covers both ABIs and the release Rust features and maps BSD only from exact names. L4: a failed account read at start shows a retry screen, not S01. L5: the restart test closes and reopens the core, the erase test checks the swipe store, the no-network claim is commented as resting on the core's `DemoBackend` tests. The source link is now a placeholder (`example.org`), no owner handle is in the tree, and T17d (FFI v7) is merged in.

**Round 2 of the review (head `6cd1b3a`): 7 low, fixed.** N1: an erase deletes the quarantine first, so a kill at any later step cannot leave an older database to be restored (tested at each step with the real core and secret store). N2: the erase checks that everything is gone before it discards the Keystore key, otherwise it stops with the erase-failed message and a retry finishes it. N3: `closeCore` is `MailRuntime.forgetCore()`, and a failed erase rebuilds the session. N4: a single removal and the sweep never touch the quarantine (round 1's purge and removal marks are gone, with the reason in `DECISIONS.md`). N5: the release check fails when the R8 mapping is missing. N6: the lambda-defined order assertion is removed (the order is tested in `SecretStoreCredentialsTest` through `AccountSecrets`), the queued-change test goes through `AccountOps.remove`, and the "ids are never reused" claims now say "while the database exists" with erase tests that delete the database as production does (ids start again at 1). N7: tests for the S45 name write-back, the real erase through the activity and the sweep at start. Nits: list positions are swept, and only the core's "destroyed" `IllegalStateException` is mapped.

**Survives an erase:** only the four DataStore files (`croft-theme.json`, `croft-swipe.json`, `croft-mail.json`, `croft-accounts.json` in `filesDir/datastore`) holding default values (DataStore keeps its file open and is reset through its own API); no Croft file is left in the no-backup folder (`AppFiles`, tested) and the Keystore wrapping key is deleted.


---

## T17d: sign-in hardening (11 November 2026)

**Status:** draft pull request #14, `task/t17d` (CI only; not merged). Fixes the low-severity findings the two T17b reviews left. **FFI API version 7** (v7 is not merged yet, so the review fixes changed its surface: `add_account` takes `accept_changed_certificate` and returns `AddedAccount { id, updated }`, `verify` returns the pin it used, new `rollback_account_update`, new `CoreError::AccountExists` and `CoreError::CertificateChanged`); the T17c branch does not bump the version, so 7 stays.

**What was built:** (1) `add_account` decides what an address that is already added means: same email (not the sample mailbox) with the same IMAP host, port and user name updates that account (same id; new settings and pin under the provider rule; the task restarts with the new credential and leaves the needs-sign-in state), another server or unreadable settings is `AccountExists`. One lock covers add, re-add and remove. The app's own shortcut is gone; `AccountSecrets` replaces the stored password under the same id, and keeps an existing account if storing fails. (2) `croft_protocols::clean_host` (trim, brackets, trailing dot, lower case) is used by `connect_client`, `provider_for_host` and the same-server check. (3) A stored pin on a provider's host is ignored at start (the account is not refused). (4) `verify` builds its settings in `prepare`, which a test calls with a pin on provider hosts. (5) `SERVER_DOMAINS` gains `outlook.office.com` and `messagingengine.com`; `lookup` reads one table that a test checks against `provider_for_host`; a Rust test reads `ProviderHosts.kt` as text and compares it with `SERVER_DOMAINS`; the single pin's doc says it must not be applied to a provider's SMTP host. (6) The L1 test holds the first attempt in a non-cancellable wait. (7) Nothing in the sign-in path reads the database on the main thread any more. (8) `openServerSettings` does nothing while adding.

**Checks:** `cargo fmt --check`, both clippy runs, `cargo test` (all crates), `cargo test -p croft-storage --features sqlcipher`, `cargo test -p croft-ffi --features sqlcipher` and `--features test-server`, `check_schema.py`: all pass in the build container. `:mailtests` (40 tests, Rust host library through JNA, in-process IMAP server): all pass. Checked by breaking the code: removing `with_host_provider` from `prepare` fails the new `verify` test; removing an entry from `ProviderHosts.kt` fails the drift test; dropping the sign-in-state clear or the sample-mailbox check fails the re-add tests; removing the `adding` guard fails the `openServerSettings` test; the old flow (`dec4299`) fails the new L1 test, 5 runs out of 5 (its second attempt's password is wiped).

**Not run here:** the `:app` unit tests (no Android SDK, Maven Central refuses the container), so `FailureCopyTest`, `SecretStoreCredentialsTest` and the Compose tests ran only on CI. The integration job (Docker servers) ran only on CI.

**Opus review: 1 high, 5 low; all fixed.** High (H1): a re-add could swap or drop a self-hosted account's trusted pin without the changed-certificate warning (the interceptor's certificate got the ordinary first-use prompt, or, when it chained to an authority, the pin was dropped silently). Now `verify` with no pin uses the stored pin of the same address and servers as the expected certificate, so another certificate is `CertificateChanged` (the pin mode accepts exactly one fingerprint and skips the chain checks, so an authority-signed certificate is a change too) and the app shows the stronger warning; `verify` returns the pin it used and the app passes that to `add_account`; `add_account` refuses to replace or drop a stored pin unless `accept_changed_certificate` is true (`CertificateChanged` otherwise), and the app sets it only for the certificate the person confirmed on the changed-certificate warning in that attempt. `a_stored_pin_follows_the_latest_sign_in` asserted the bad behaviour and is replaced by five tests (different pin refused, no pin refused, replaced with the flag, same pin accepted, no effect without a stored pin), a server test with the in-process server (`a_stored_pin_makes_another_certificate_a_change_and_not_a_first_use`) and three flow tests in `:mailtests` (changed warning then new pin stored; first-use trust never sets the flag; a pin that appears between the check and the add is asked about). L1: `add_account` returns `AddedAccount { id, updated }` and `AccountSecrets` uses it (no list read before the call). L2: option (b): the core keeps what a re-add replaced, and `rollback_account_update(id)` puts it back; when the keystore write fails the app rolls the update back and shows its own message (`SignInNotSavedException`, "the password worked, but could not be saved; the account still uses its old password"). Option (a) (write settings only after the password is stored) needs a two-step call in the core and was not simple. L3: a failed or lost start task goes through the same restore as any failed start (`restore`). L4: addresses are compared with full Unicode lower-casing (`same_address`). L5: "same server" is the IMAP host, port, security mode and user name and the SMTP server (host, port, security, user name; both or neither), so a re-add cannot repoint sending. Tests: `readd.rs` (14), the server test above, `:mailtests` 43 (all pass here). Broken to check: without the pin refusal the two pin tests fail; without the stored-pin lookup in `verify` the server test fails; with the flag always true two flow tests fail. Not tested: L3's failure branch (nothing in the tests can make a validated start fail). `theSavedStateNeverHoldsThePassword...` removes the first flow's account before its second flow (a pinned account for the same address now makes the next check a pin comparison).

**Round 2: 3 low, fixed** (and the nits). Low 1: an update has a token (`AddedAccount.update_token`, from a counter); `rollback_account_update(id, token)` undoes only that update and returns whether it did, and `commit_account_update(id, token)` (called by the app after it stored the password) clears the undo note, so a later sign-in's settings can never be undone by an earlier one's failure (`a_rollback_undoes_only_the_update_it_names`, `a_commit_clears_the_undo_of_that_update_only`). Low 2: `accept_changed_certificate: bool` became `confirmed_fingerprint: Option<String>`, which must equal `pinned_fingerprint` to replace a pin, and a pin is never dropped by a re-add. `with_the_acknowledgement_the_pin_is_replaced_or_dropped` was split: the replace half stays (with the fingerprint now), the drop half became "refused, with or without a confirmation" (`a_pin_is_never_dropped_not_even_with_a_confirmation`): stricter, not weaker. Low 3 (decision in `docs/DECISIONS.md`): an account stored without a pin that is shown a self-signed certificate gets `CertificateChanged` from `verify`, and adding the pin needs the confirmation; `the_acknowledgement_changes_nothing_when_no_pin_is_stored_or_for_a_new_account` asserted that a pin could be added to such an account freely and is replaced by `a_pin_added_to_an_account_that_had_none_needs_the_confirmation` (the new-account half stays), plus a server test with the in-process server; `the_same_address_and_server_updates_the_account_and_keeps_its_id` now passes the confirmation for the pin it adds. Nits: after a rollback the account is put back in the needs-sign-in state it had; if the rollback did not happen (stale token or failure) the app shows the ordinary storage failure, not "the account still uses its old password"; `Attempt.Trust` carries the attempt's id and `trustServer` ignores a question of another attempt; on S05 the confirmation given for "Test connection" carries to "Add account" for the same fingerprint, so the warning shows once. Tests: `readd.rs` 18, server tests, `:mailtests` 45. Broken to check: a rollback that ignores the token fails the race test; no needs-sign-in restore fails its test; dropping the "no pin" rule in `verify` fails the server test; no attempt id check fails the stale-question test (3 of 3 runs); no carry fails the S05 test.

**Notes:** the label of an existing account is kept on a re-add (the person may have renamed it); the sample mailbox is never matched; user names are compared exactly, the address and the host without regard to case. `SecretStoreCredentialsTest` and the core's id test added one address twice and now use two addresses (the same address again is a re-add). `ui/onboarding/FailureCopy.kt` got one line for the new key. The SMTP side of the pin rule is only documented: nothing sends through the stored SMTP settings yet.

---

## T17b: sign-in screens (11 November 2026)

**Status:** draft pull request `task/t17b` (CI only; not merged). Part of T17: S01 Welcome, S02 Add account, S03 Finding your server, S04 Sign in, S05 Server settings, S09 First sync, the failure copy and the trust prompt. S44, S45, S60 and the demo mailbox (`DemoBackend`) are other parts of T17.

**What was built:** a fresh install starts at S01; once an account is added the app shows the Inbox on the real core. The flow is one state machine (`OnboardingFlow`, plain Kotlin) over an `OnboardingBackend` interface that wraps the core's `discover`, `verify` and `add_account` and the secret store (`AccountSecrets.addPasswordAccount`). S03 shows four rows (known providers, autoconfig, DNS service records, JMAP) from the core's progress callbacks; the first match goes to S04; nothing found goes to S05 filled in with a guess. S04 shows the Gmail and Outlook app-password text of `READINESS.md` row 10 and no OAuth button until T21. A self-signed certificate asks once (host and fingerprint, "Trust this server" or Cancel), then verifies and adds again with `pinned_fingerprint`; a changed certificate asks again with a stronger warning. S05 has protocol, host, port, security, username, a password for the test, and "Advanced options" (outgoing server). "Test connection" shows its steps in order. S09 shows skeleton rows, polls the account until a folder has synced, and has "Continue in background" at all times. Every error of `discover` and `verify`, and of `add_account`, has its own plain-language text in `strings_onboarding.xml`, with technical lines behind a Details link.

**The password:** typed into a field of S04 or S05 only (`remember`, never saved state, never a navigation argument); turned into UTF-8 bytes on Sign in; the flow owns the bytes and wipes them when the attempt ends (success, failure, Back, Cancel, close). They are kept longer only while the trust question waits. Then they go into `FfiCredential` (a copy, wiped) and into `AccountSecrets.addPasswordAccount`, which stores them in `SecretStore` under the new account id. Not logged, not in `OnboardingState`, not in the saved snapshot (a test checks this).

**Hooks:** "Try with sample mail" goes through `onTrySampleMail` in `ui/nav/AppNavigation.kt`, which for now opens the existing fake-data Inbox. "I run a Companion" is hidden in every build (there is no store build flag yet: `OnboardingFlags.COMPANION_ENTRY`).

**Tests:** 10 JVM tests of the flow over the real core and the in-process IMAP server (`:mailtests`, all pass in the build container: trust prompt, pinned fingerprint used by the first sync, wrong password, changed certificate, Test connection steps, no password in the snapshot); a JVM test that walks every key of `core/ffi/src/error.rs` and every variant of the generated bindings and fails if one has no string; Compose/Robolectric tests of the screens (disabled Continue, Back order, Back leaves the app on S01, probe row states, offline, wrong password inline, trust prompt, Test connection steps, S09); 54 screenshot tests (compact and expanded, light and dark, font 1.0 and 2.0, plus one right-to-left picture per screen). The Robolectric and screenshot tests cannot run in the build container (Maven Central refuses the Robolectric download), so they run on CI.

**Not done:** unencrypted connections for private-network addresses (the core has no such mode and refuses it for real hosts; S05 offers TLS and STARTTLS only); the privacy-policy and licences links on S01 (S60 does not exist yet); discovery of three real test servers from Kotlin (needs the container servers; the Rust `integration` job covers discovery); the sample-mail choice is not remembered across restarts; no "add another account" entry point yet (S44).

**Unverified:** TalkBack, password-manager autofill and the real Keystore on a device.

**Opus review: 1 high, 1 medium, 4 low; all fixed.** High: changing the port or security on S05 dropped the provider, so an interceptor's certificate of `imap.gmail.com` could be trusted. The core now treats a listed provider's own server host as that provider's (`provider_for_host`, used by `verify` and `add_account`: a pin is never accepted, `add_account` refuses it with `BadSettings`), and S05 keeps the provider whenever the host is a provider's. Medium: the account add (core call and storing the password) now runs to its end even if the screen goes away, Back waits for it, and a second sign-in for an address that already has an account finds it instead of adding another. Low: each attempt owns its password and a cancelled one cannot wipe the next; a late probe callback of an older lookup is ignored; the security choices on S05 are disabled while a test runs; `onFinished` fires once. T17a (core sample mailbox, FFI version 6) is merged in; the app's fake-data session flag is now `MailSession.usesFakeData`, apart from the core's `AccountInfo.is_demo`.

---

## T17a: sample mailbox in the core (11 November 2026)

**Status:** in a draft pull request for CI (not merged). The screens are T17b's.

**What was built:** `DemoBackend` and `DemoSubmitter` in `core/protocols/src/demo/`: 20 fictional messages (people and addresses at `example.org` / `example.com` only) in Inbox (a four-message thread across Inbox and Sent, an attachment, an HTML newsletter with a remote image), Sent, Drafts, Trash and Archive, dated relative to the time the mailbox is added. Read, star, move, archive and delete are confirmed by the in-memory copy and stay on the device; after a restart the copy is rebuilt from the stored rows. Sending files the copy in the sample Sent folder and sends nothing. FFI API version 6: `add_demo_account()` (at most one) and `AccountInfo.is_demo`; the marker lives in `config_json` (no migration); `remove_account` deletes all its rows. Kotlin: expected version 6, test records updated.

**Checks:** `cargo fmt`, both clippy runs, `cargo test` (whole workspace), the SQLCipher tests and `check_schema.py` in the container. New tests: 15 backend tests (folders, snapshot, headers, flag change stays local, moves, resume, filing a copy once, every address at an example domain, `Debug` without content, every call on a runtime without an I/O driver, a source scan for socket and lookup APIs, and the control that a tokio socket call does panic there), 3 engine tests (full sync, every action and a send on a runtime without an I/O driver, later syncs keep the changes; a sent message survives a restart before any sync, also before the first sync), 7 FFI tests (with a credential provider that records any call, checked at the end: add once, list, star and move survive a sync and a restart, remove deletes every row, a stored sample mailbox that is not running is started, a look-alike marker is not the sample mailbox), plus settings tests for the exact marker shape; a mutation check confirmed the restart test fails if the copy is rebuilt from the built-in set. Android tests not run here (Maven Central refuses this container); GitHub CI runs them.

**Review:** an independent review found 1 medium and 6 low issues, all fixed: a message sent from the sample mailbox is now stored in the local Sent folder before the Outbox lets go of it (it could vanish if the app was killed before the next sync); the no-network claims are narrower and the source check stricter (`std::net` and name lookups are not caught by the runtime check, so the source check covers them; `data_lines` moved out of `smtp`); the demo marker must have the exact shape and the sample address; `add_demo_account` and `remove_account` share a lock; a stored sample mailbox that is not running is started; `Debug` prints counts only; the FFI test's provider records calls instead of panicking. New tests for each; mutation checks confirmed the restart-after-send and not-running tests fail without the fixes.

**Known gaps:** no body fetch exists in `MailBackend` yet, so the attachment and the remote image are in the message sources (`DemoBackend::message_source`) but not yet shown; T18 will need them. The app's engine has no Outbox yet, so sending from the sample mailbox is tested in the engine only. A message written in the sample mailbox keeps only its header after a restart.

---

## T15 and T16: swipe engine; Inbox on the real core (11 November 2026)

**Status:** T15 merged (pull request #8). T16 in pull request #9.

**T15:** pure swipe state machine in a plain-Kotlin `:swipe` module (48 tests), gestures and visuals for S14 to S16, S47 and S48, long-press menus on drawer mailboxes, TalkBack custom actions. First CI run: 4 failures of 1823; one real bug (after "Mark all as read" from the drawer, the drawer stayed open and covered the Undo button), a gallery entry drawn off-screen, a test timing assumption and a DataStore test race; all fixed. The amber star on a selected row now meets 3:1.

**T16:** the Inbox and actions run on the real core when an account exists (the sample mailbox stays until onboarding, T17). FFI API version 5 (thread count, recipients, last-synced time, Starred scope). S30 Move to. A host-only `test-server` feature lets 22 Kotlin tests drive a real in-process IMAP server (all pass in the build container). Permissions `INTERNET` and `ACCESS_NETWORK_STATE`. Opus security review: nothing serious, 5 low items fixed (a compile-time guard so the test server can never be built for Android, a stronger release check, build inputs, CI lint, committed Python cache files).

**Known gap:** Sent rows show no recipients yet: the sync does not store To addresses (follow-up in `core/protocols`, `core/storage`).

**Process note:** the main folder and a task worktree briefly had the same branch checked out; a push from one left the other's files stale. Nothing was lost; `CLAUDE.md` now forbids it.

---

## T16b: release build; design-token follow-up; store audit (11 November 2026)

**Status:** merged (pull requests #6 and #7) after GitHub CI passed, including the new `android-release` job.

**What was built:** a `release` build type (R8, resource shrinking, not debuggable, unsigned; signing is the owner's), F-Droid-safe (`dependenciesInfo` off), keep rules for the Rust bindings checked before and after R8, a release-manifest allowlist check (no app permissions, only the launcher exported), 16 KB alignment checks on the APK and the app bundle. The amber status colour, wait times, pull threshold and wide-screen width moved into `design/tokens.json`. A read-only store audit (`docs/store/AUDIT-2026-11-11.md`) updated `READINESS.md` and wrote store rules into T17, T18, T20, T21; S60 (About, privacy policy link) added to T17.

**Owner item that blocks the first Google Play upload:** the real application ID (see `docs/OPEN-ITEMS.md`).

**Next:** T15 (swipe engine), in progress.

---

## T13 and T14: controls, haptics, navigation, Inbox screens (11 November 2026)

**Status:** merged into `main` (pull requests #3 and #4) after GitHub CI passed. Device-only items are unverified (ripple and haptic feel, scroll speed with 5,000 rows, TalkBack order).

**What was built:** T13: every shared control in every state (`ui/components/`), one `Haptics` interface (levels, 40 ms guard), a debug-only gallery, the stored theme applied at start and Settings → Appearance reachable. T14: the Inbox on a fake repository shaped like the core's FFI surface: S10 (paging, sticky dates, fast scroller, "N new" pill), S11 selection, S12 drawer, S13 account switcher, S17 pull to refresh, S18 offline and sign-in banners, S19 inbox zero; scroll position survives rotation and process death; a build-only macrobenchmark module.

**Checks:** neither task's tests could run in the build container (Maven Central refused one Robolectric jar). On GitHub CI: T13 green on its first run; T14 had 21 failures of 1,260 on its first run: 2 real screen bugs (the compose button had no name for screen readers; the "N new" pill faded in for one frame with animations off) and 19 test assumptions, all fixed without weakening a test (details in `docs/tasks/T14-message-list.md`).

**Also:** the flaky outbox test was `after_a_refused_sign_in_remove_and_add_does_not_make_the_task_spin_and_signing_in_sends`: it read an empty outbox list as "copy filed in Sent", which is not what the engine promises. Reproduced at about 31% under load; 0 in 450 runs after the test fix. The engine was correct.

**Next:** design tokens follow-up (amber colour, wait times, pull threshold and drawer width into `tokens.json`), then T15 (swipe engine).

---

## T12: Theme engine and Appearance screens (11 November 2026)

**Status:** done except: the screens are built and tested but not yet reachable from the app (T13 adds navigation); device-only items (feel of the cross-fade, wallpaper colours on a real Android 12+ phone, the system file and photo pickers).

**What was built:** `android/.../theme/` (settings, JSON export and import with a version field, DataStore, runtime colour schemes from Google's Material Color Utilities Java port, pure black, contrast levels, cross-fade with reduced-motion variants) and screens S50, S51, S52, S53 and S08. Optional fonts Inter ("Croft Sans") and Atkinson Hyperlegible (SIL OFL 1.1).

**Checks:** the agent ran 388 Android tests with 0 failures (144 contrast, 120 + 32 screenshots with Roborazzi, behaviour, JSON, golden schemes), but only by putting one Robolectric jar from the old mirror download on the classpath locally (not committed). The orchestrator built the merged app and compiled its tests. **GitHub CI (pull request #2) is the real test run.**

---

## T11: Secrets and the Android Keystore (11 November 2026)

**Status:** done except device-only checks (StrongBox, reboot, screen-lock change, "clear data", real keystore1 behaviour on Android 8 to 11).

**What was built:** the mail database is encrypted on the phone (SQLCipher with OpenSSL 3.6.3 built from source for Android, 16 KB aligned). Its key and all account secrets are wrapped by a non-exportable AES-256-GCM key in the Android Keystore (StrongBox when present), stored in `noBackupFilesDir`. Key loss is declared only on definite evidence (a canary key, repeated checks, and on Android 8 to 11 three starts over an hour); even then data is moved to a quarantine folder, restored automatically if the alarm was false, and deleted only after 7 days and 3 good starts. FFI API version 4: a new account's credential is passed to `add_account` itself; a keystore that is not ready is a temporary failure, not "needs sign-in". Account ids are never reused.

**Checks run and seen passing (by the orchestrator):** Rust fmt, clippy on 1.97 and 1.99, `cargo test` 712 passed, `croft-ffi` with SQLCipher 52 passed, `check_schema.py` ok; merged app (T11 + T12) builds and its tests compile. The agent ran 148 Android tests under Gradle with 0 failures (one large Robolectric jar excluded by a local, uncommitted init script because Maven Central refused it). **GitHub CI (pull request #2) is the real test run.**

**Review:** Opus, two rounds. Round 1: 2 high (a temporary keystore error could wipe all data; a new account's password could reach another account's server), 2 medium, 3 low. Round 2: 1 high (on Android 8 to 11 a keystore outage could still look like a lost key, and could replace the real key), 2 medium, 4 low. All fixed with tests. The first CI run found two failing tests; the cause was two copies of the Rust bindings under Robolectric, fixed with a custom test runner. The next CI run found a real race: a stale "needs sign-in" notice could arrive after the person signed in; the FFI layer now checks the engine's current state (20 of 20 runs pass).

**Known flaky test (resolved, see T13 and T14 entry):** an outbox test in `core/engine/tests/outbox.rs` has failed once each in two different sessions' runs, never reproduced and its name not captured. Next time it fails, capture the name and find the cause.

---

## Session 1 summary (11 November 2026) — read this first

**Update, first GitHub CI run (pull request #1):** all six jobs pass on GitHub (core, SQLCipher, schema, design tokens, integration against Stalwart/GreenMail/Dovecot, and Android `assembleDebug` + 27 JVM unit tests with libraries from Maven Central itself). CI found two problems the container could not: a clippy lint new in Rust 1.99, and an Android screen test still expecting FFI API version 1. Both fixed. This settles the "mirror" concern below for the Android tests.

**Where things stand:** stage 0 and stage 1 are done (T01–T06), plus T07 and T08 (stage 2, "done except" device checks), and from phase 1 the JMAP adapter (T09) and the FFI surface the app will use (T10). Stage 3 (the Android screens, T11–T21) has task files but has not started: every one of those tasks needs new Android libraries from Maven Central, which rate-limits this container (429), so nothing there could be built and tested here.

**Checks on the final branch (`claude/croft-build`, run by the orchestrator):** `cargo fmt --check` and both `cargo clippy -D warnings` clean; `cargo test` 703 passed, 0 failed; storage with SQLCipher 146 passed; `check_schema.py` all ok; design tokens 60 of 60. Container suites against real servers: Stalwart (JMAP) 6/6, GreenMail and Dovecot (IMAP) 9/9, SMTP to GreenMail 6/6, discovery 3/3.

**Not verified:** anything on an Android device or emulator (none here); the Android unit tests through Gradle (Maven Central 429; they ran once with libraries from a mirror, against the rules, see `docs/ENVIRONMENT.md`); GitHub CI has never run; the iOS side.

**Next session, first:** (1) attach a git remote so work can be pushed (`docs/OPEN-ITEMS.md`); (2) check Maven Central: `curl -sI https://repo.maven.apache.org/maven2/junit/junit/4.13.2/junit-4.13.2.jar` must answer 200, then run `cd android && ./gradlew --refresh-dependencies :app:assembleDebug :app:testDebugUnitTest`; (3) start T11 (keystore), then T12–T21 in order.

**How the work was done:** the orchestrator wrote the designs it is meant to keep (migrations 0002/0003, following a moved message, the FFI surface, the JMAP id map) and delegated the rest to subagents, one git worktree per task. Every security-sensitive change was reviewed by a separate Opus agent before it was merged: 13 reviews found 2 high, 13 medium and many low problems, all fixed (`docs/REVIEW-QUEUE.md`). The tool could choose a model and an effort level per subagent (Sonnet for building, Opus for reviews and the sync engine; Haiku was not needed). Its built-in "isolated worktree" option did not work because the session started outside the repository, so worktrees were made by hand.

---

## T09: JMAP adapter (11 November 2026)

**Status:** done. Not done: resuming `Email/changes` from the stored state after a restart (the first snapshot after a restart is a full read); push (EventSource); keep-alive connections; `$junk`/`$notjunk` keywords for spam (the trait has no flag for it yet, same as IMAP).

**What was built:** `croft_protocols::jmap`: `JmapBackend` implementing `MailBackend` for Stalwart, Fastmail and other JMAP servers, with its own small HTTP client (same TLS and pinning policy as IMAP and SMTP); a persistent map from JMAP ids to per-folder numbers (migration 0005) so nothing above the protocols crate knows which protocol an account uses; `JmapSubmitter` for sending through EmailSubmission. Stalwart v1.1.0 in `test-servers/` with a setup script that creates two users.

**Checks run and seen passing (by the orchestrator):** on the merged branch fmt, clippy (both), `cargo test` **703 passed, 0 failed**, storage with `sqlcipher` 146, `check_schema.py` ok. **All container suites run together by the orchestrator against fresh containers: Stalwart 6 of 6, IMAP (GreenMail, Dovecot) 9 of 9, SMTP to GreenMail 6 of 6, discovery 3 of 3.**

**Review:** Opus, one round: 1 high (after a proxy error such as 502 or 504, a JMAP send was retried and could be delivered twice), 2 medium (stale numbers after the id map was reset; a retried move lost the new number), 5 low; all fixed with tests. The fixes were checked against Stalwart by the orchestrator, not re-reviewed.

---

## T10: FFI mail surface, API version 3 (11 November 2026)

**Status:** done except: the Kotlin tests were not run through Gradle (Maven Central rate-limits this container). They were compiled by Gradle offline and run with plain JUnit against the real Rust library: 21 of 21 passed (agent's run; not repeated by the orchestrator). Sending is not on the surface yet.

**What was built:** `core/ffi`: `Core.open(path, key, credentials)` owning a 2-thread runtime and the engine; discovery with progress, sign-in check, accounts (add, list with status, remove), folders, a paged message list (one folder or the unified inbox), change notifications (`next_change`), actions with Undo (archive, trash, move, delete forever, mark read, star), sync now, network on/off, signed in again. Credentials come from the app through a callback each time a connection signs in and are never stored; a credential is an opaque object that wipes its secret when released and never prints it. Errors are fixed enums.

**Checks run and seen passing (by the orchestrator):** after merging with T06 part B, fmt, clippy (both), `cargo test` **601 passed, 0 failed**, `check_schema.py` ok. The merge needed a small fix (the FFI layer now maps the outbox change kinds part B added).

**Review:** Opus, one round: 3 medium (the credential printed its secret in Kotlin; a discovery callback ran on a core worker thread and could freeze syncing; documentation overstated how secrets are wiped), 3 low; all fixed. The fix changed the credential into an object (not re-reviewed).

**Next:** T11 (keystore), needs Gradle with new libraries.

---

## T06 part B: the outbox in the engine (11 November 2026)

**Status:** done. T06 is now complete. Not done: refreshing an OAuth token for sending (with T21); creating a missing Sent folder is only unit-tested (the test server always has one); a non-UTF-8 message cannot be saved in Sent (documented limit).

**What was built:** `MailSubmitter` (send, file a copy in Sent) in `core/protocols/src/submit.rs`, with an SMTP + IMAP implementation. The engine runs a send loop per account: Undo window, back-off, offline and sign-in pause shared with the action queue, a copy in Sent filed exactly once (Message-ID lookup first), "may not have been sent" after a crash, never resent automatically. `EngineHandle`: send, undo send, outbox list, retry, edit back to draft, discard, replace the sender after a new password. A message with a `Bcc:` header (also hidden behind bare carriage returns), or without a Message-ID, is refused before it is stored.

**Checks run and seen passing (by the orchestrator):** fmt, clippy, `cargo test` 563 passed in each of 7 full runs; **one run out of 8 had one failing test that could not be reproduced** (the machine was under heavy load with three builds at once; the failing test's name was not captured). `check_schema.py` all ok.

**Review:** Opus, one round: 2 medium (a Bcc header hidden by bare carriage returns would be revealed to all recipients; the account task could spin at full CPU after a sign-in failure and re-adding the account), 4 low (including replacing a "mark other accounts offline" workaround with a proper per-account take). All fixed with tests.

---

## T04: Sync engine (11 November 2026)

**Status:** done except IDLE (push from the server while the app is open): wake-ups use the next due action, a 15-minute timer and a nudge on every action. The place to add IDLE is marked in `core/engine/src/worker.rs`. Within an account, actions run one at a time (the contract allows four in parallel; a later speed-up).

**What was built:** new crate `core/engine` (`croft-engine`): one task per account; `EngineHandle` for the screens (mark read, star, move, delete forever, Undo, change notifications, online/offline, sync now), every action one local SQLite transaction that never waits for the network. Folder sync with the planner, chunked so a big sync does not freeze the screen. Follows a moved message to its new UID (from the server's answer, or by Message-ID when the server cannot say), and handles a server renumbering a folder (UIDVALIDITY change) without acting on the wrong message.

**Checks run and seen passing (by the orchestrator):** fmt, clippy, `cargo test` 520 passed, 0 failed, three runs in a row on the merged branch; storage with `sqlcipher` 135 passed; `check_schema.py` all ok. 58 engine tests: 47 against a scripted fake server, 5 against the real IMAP adapter and the in-process IMAP server, plus resilience and task-loop tests.

**Review:** Opus, three rounds. Round 1: 1 high (an interrupted folder reset could leave a folder showing almost nothing until the server changed), 2 medium, 3 low. Round 2 (of the fixes): 2 medium (an interrupted reset could make a "delete forever" hit the other copy of a duplicated message; a check computed too early), 2 low. All fixed with tests that failed before the fix. Round 3's fixes were not reviewed again.

**Next:** T06 part B (outbox in the engine), T10 (FFI surface for the app).

---

## T06: SMTP send and Outbox, part A (11 November 2026)

**Status:** part A done; part B (wiring the outbox into the engine so "send with no network queues and sends later exactly once" runs end to end) is not done yet. Its plan is at the end of `docs/tasks/T06-smtp-outbox.md`.

**What was built:** `croft_protocols::smtp`: an SMTP submission client written directly on tokio and rustls (TLS on 465, STARTTLS on 587, AUTH PLAIN/LOGIN/XOAUTH2, pinned certificates, same TLS policy as IMAP). `lettre` was not used: it cannot take a custom certificate verifier, so pinning would be impossible. `croft_mail_core::outbox`: the outbox state machine (queued, sending, submitted, filing in Sent, sent, failed; Undo, retry, edit back to draft, discard, abandon). A message whose sending was interrupted after the server may have accepted it is never resent automatically; it fails with "May not have been sent; check Sent before retrying". Migration 0004 rebuilds the `outbox` table (envelope, new states). A fake SMTP server in `core/test-support`.

**Checks run and seen passing (by the orchestrator):** fmt, clippy, `cargo test` 442 passed, 0 failed on the merged branch; `check_schema.py` all ok. The agent ran 6 SMTP-to-GreenMail container tests (send, then read back over IMAP, including Bcc and a body line that is just a dot) on its own copy of the servers; not re-run by the orchestrator.

**Review:** Opus, one round: no high or medium; 5 low fixed (message bytes and addresses in `Debug`, missing STARTTLS now pauses the account, abandon for a stuck sending row, outbox actions check the account, a schema comment).

**Next:** T06 part B after T04 is merged.

---

## T05: Account discovery (11 November 2026)

**Status:** done. Not checked here: the real DNS resolver and HTTPS client against the live internet (this container's DNS answers nothing; both were tested against local DNS and TLS servers), hickory's system configuration on Android, autoconfig files recorded from real providers (fixtures are written to Mozilla's format).

**What was built:** `croft_protocols::discovery`: built-in table (Gmail, Outlook, iCloud, Fastmail), autoconfig over HTTPS on the person's own domain only, RFC 6186 SRV, JMAP well-known; probes in parallel, earliest source in that order wins. Then `verify`: a certificate check with no credentials first, then sign-in. Each failure has its own plain message (wrong password, needs an app password, untrusted certificate with its fingerprint, certificate changed, connection intercepted, certificate invalid, cannot connect, and others). The local part of the address never leaves the device in a URL, a DNS name or a header; no third-party lookup.

**Checks run and seen passing (by the orchestrator):** fmt, clippy (with and without `integration`), `cargo test` 344 passed, 0 failed across the workspace. Container tests (GreenMail via autoconfig, Dovecot via SRV with TLS and STARTTLS: 3 of 3; T03's 9 of 9) were run by the agent on its own copy of the servers, not re-run by the orchestrator.

**Review:** Opus, two rounds. Round 1: 1 medium (Croft offered "trust this certificate" for Gmail and other big providers, where a self-signed certificate means interception), 2 low, 3 info. Fixing it also fixed a clash: T03 had changed certificate errors to `Auth`, which discovery would have shown as "wrong password". Round 2: 2 low, 3 info; fixed except one kept on purpose (a self-signed certificate is offered trust whatever name it was made for).

**Next:** T06 (in progress).

---

## T03: IMAP adapter (11 November 2026)

**Status:** done. Not covered by a container: UIDVALIDITY change on a real server (only the in-process server), CONDSTORE-only servers (only in-process), XOAUTH2 success (only in-process; no container offers it), the public certificate path against a real public server (no internet mail server reachable from here).

**What was built:** `croft_protocols::imap`: `ImapBackend` implementing `MailBackend` with async-imap 0.12.0 (vendored and patched, see below), rustls 0.23 (ring), TLS 1.2+ only, certificate checks always on, self-signed only through a pinned SHA-256 fingerprint, plaintext only for tests on loopback. The trait gained `MoveOutcome` (new UIDs), `find_by_message_id`, `capabilities`. `core/test-support`: an in-process IMAP server for fast tests. `test-servers/`: GreenMail 2.1.15 and Dovecot 2.4.5, pinned.

**Checks run and seen passing (by the orchestrator):** fmt, clippy (with and without `integration`), `cargo test` (protocols: 29 unit, 45 in-process, 1 log-capture, 2 vendored-crate guards), release-profile tests, and the container suite **9 of 9, three runs in a row** against fresh GreenMail and Dovecot containers.

**Review:** Opus, one round. 1 high: the IMAP library logged the LOGIN line with the password at trace level, and a release build could switch that on; fixed by vendoring async-imap with all four logging calls removed and its buffer cap lowered from 512 MiB to 64 MiB, plus a compile-time log-level guard. 2 medium: a wrong password could be classed as temporary (lockout risk); a hostile server could force about 500 MB of memory. 4 low. All fixed with tests. The agent saw occasional TLS "network error" failures in the container suite (also before its changes); the orchestrator's three runs were clean.

**Next:** T05 (discovery) fixes, then T06.

---

## T07: UniFFI surface and first Compose screen (11 November 2026)

**Status:** done except: (1) not shown on a device or emulator (none exists here: no KVM); (2) the Android unit tests passed only with libraries a subagent fetched from a mirror of Maven Central, which broke the "no mirrors" rule (see `docs/ENVIRONMENT.md`); they could not be re-run from Maven Central itself because it rate-limits this container; (3) the GitHub CI job has never run.

**What was built:** `core/ffi` (UniFFI 0.32.2): `croft_version()`, `ffi_api_version()` (now 2), `Core.open(path)`, `Core.list_folders(account)`, `FolderInfo`, `CoreError` with a fixed `StorageReason`. `android/`: Gradle 9.6.0 (wrapper and distribution checksums match the official ones), AGP 9.5.1, Kotlin 2.4.21, Compose BOM 2026.09.00 with Material 3 1.5.0-rc01, compileSdk 37, targetSdk 36, minSdk 26. cargo-ndk 4.1.2 builds `libcroft_ffi.so` for arm64-v8a and x86_64; all LOAD segments are 16 KB aligned. One Compose screen shows the Rust version, themed with the generated design tokens (`CroftTokens.kt` compiled for the first time, unchanged). The app excludes all data from cloud backup and device transfer. CI has an `android` job.

**Checks run and seen passing (by the orchestrator):** Rust fmt, clippy, `cargo test` (5 ffi tests among 176); `./gradlew --offline :app:assembleDebug` builds the APK (17.6 MB). Earlier, with mirror-fetched libraries, 7 JVM tests passed (4 calls into the real Rust library, 3 Robolectric screen tests).

**Review:** Opus, one round: 1 medium (device transfer could copy the database), 5 low/info; all fixed. FFI surface listed in `docs/REVIEW-QUEUE.md`.

**Next:** re-run `./gradlew --refresh-dependencies :app:assembleDebug :app:testDebugUnitTest` once Maven Central answers; then stage 3 screens.

---

## T02: Persist the operation queue (11 November 2026)

**Status:** done.

**What was built:** the offline action queue now survives a kill. `core/mail-core/src/persist.rs` (`OpStore` trait, `LocalEffect`, `PersistentQueue`) and `core/storage/src/op_store.rs` (SQLite implementation). Each action and its on-screen effect (read flag, star, hidden-while-moving, pending count, copy of a moved message) are written in one transaction. Migrations 0002 (hidden rows, "waiting for a new UID" table) and 0003 (folder counters leave out hidden rows) were designed by the orchestrator; see `docs/tasks/T02-T04-design.md`.

**Checks run and seen passing (by the orchestrator):** fmt, clippy, `cargo test` 171 (63 mail-core, 4 protocols, 104 storage), 107 storage tests with `sqlcipher`, `check_schema.py` all ok (29 checks).

**Review:** one round by a separate Opus agent: 3 medium, 3 low (a moved copy shown while a later move was pending; a failed move unhiding a row another move still hides; a row left hidden after a folder reset; op ids reused after restart; repoint to the same address deleting the row; unreadable rows dropped silently). All fixed, each with a test that failed before the fix. The fixes were not re-reviewed (they do not touch account scoping).

**Left for T04 (written in the design note):** during a full folder sync, clear `local_hidden` and `pending_ops` on rows with no unfinished operation; after a UIDVALIDITY reset of the *source* folder of a "waiting for new UID" entry, do not remove a row that now belongs to a different message.

**Next:** T04 sync engine.

---

## T01: SQLite connection layer (11 November 2026)

**Status:** done.

**What was built:** `core/storage`: `Db::open(path, key)` with WAL, foreign keys, forward-only migrations each in their own transaction, `SchemaTooNew` for a newer database, optional SQLCipher (`--features sqlcipher`). Repositories for accounts, folders, messages (list newest first without snoozed or hidden rows, upsert from server headers without overwriting a pending local change, flags, delete) and settings. Every folder and message query filters `account_id`. All 17 Python schema checks are ported to Rust tests.

**Checks run and seen passing (by the orchestrator):** fmt, clippy (with and without `sqlcipher`), `cargo test` 98 tests (37 mail-core, 4 protocols, 57 storage), 60 storage tests with `sqlcipher`, `check_schema.py` 17/17. Test suite run three times in a row: stable.

**Review:** two rounds by a separate Opus agent; every finding fixed (see `docs/REVIEW-QUEUE.md`). Listed for the owner to read by hand.

**Unverified:** read-only file permissions (the container runs as root, which ignores them). Behaviour on a real Android file system.

**Next:** T02 (persist the queue), already built on top of this.

---

## T08: Design token generator (11 November 2026)

**Status:** done except: the generated Kotlin and Swift files have not been compiled (no `kotlinc` or `swiftc` here). The Kotlin file is compiled for the first time when the Android project exists (T07/stage 3).

**What was built:** `design/generator/` (TypeScript, run by Node 22) reads `design/tokens.json` and writes `design/generated/` (Kotlin, CSS, Swift, TypeScript): 8 presets × light/dark/pure black × 3 contrast levels = 72 colour schemes of 48 roles, from Google's Material Color Utilities; motion springs, shapes, spacing, swipe and haptics values. `design/CLAUDE.md` holds the token rules.

**Checks run and seen passing (by the orchestrator, not only the agent):** `npm ci && npm run generate && npm test`: 60 of 60 tests pass; re-generating leaves `design/generated/` unchanged.

**Unverified:** Kotlin and Swift compile; the contrast acceptance test (build-order step 2, not this task).

**Next:** compile `CroftTokens.kt` inside the Android app.

---

## Stage 0: environment, baseline, licences, instruction file (11 November 2026)

**Status:** done.

**What was done**
- Unzipped the handoff, ran `git init`, made the "Initial handoff" commit on `main`, then started work on branch `claude/croft-build`.
- Wrote the global instruction file `~/.claude/CLAUDE.md` (full text below). The same "How to work" section is already in the repository's `CLAUDE.md`, word for word.
- Checked the container and recorded it in `docs/ENVIRONMENT.md`. Started the Docker daemon. Installed the Android SDK, platform 36, build-tools 36.0.0 and NDK r29. No emulator is possible (no KVM).
- The owner allowed five blocked hosts on the environment during the session (Google's Android downloads, the two licence sites, docs.rs).
- Downloaded `LICENSE-MPL-2.0` and `LICENSE-AGPL-3.0` from the official addresses. First lines checked: "Mozilla Public License Version 2.0" and "GNU AFFERO GENERAL PUBLIC LICENSE / Version 3, 19 November 2007".

**Checks run and seen passing:** `cargo test` 46 of 46; `check_schema.py` 17 of 17.

**Not done / unverified:** nothing can be pushed, because there is no git remote (see `docs/OPEN-ITEMS.md`).

**Next:** stage 1, tasks T01 to T06.

**The global instruction file, as written (`/root/.claude/CLAUDE.md`):**

```
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
```
