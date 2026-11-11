# T18: Conversation, compose and Outbox

Crate / folder: `core/protocols`, `core/mail-core`, `core/storage`, `core/ffi`, `android/`

Depends on: T06 (SMTP and Outbox), T09 (JMAP, for the body fetch on both protocols), T10, T13, T16. Blocks: T19 (search needs stored bodies), T20 (reply from a notification).

This is the largest task. It has two parts that can be committed separately; if one session cannot finish both, stop after Part A, report, and the orchestrator splits the rest.

## Read first
`docs/ux/build-order.md` (step 8), `docs/ux/screens.md` (S20, S21, S23, S24, S26 rows), `docs/ux/controls.md` (Send button table, input chips, attachment chip, banners), `docs/ux/state-machines.md` (Outgoing message), `docs/ux/server-contract.md` (Send, Save draft rows), `docs/ux/navigation.md` (compose back, deep links, mailto), `docs/ux/customisation.md` (privacy and inbox rows), `docs/PLAN.md` (safe HTML), `docs/store/READINESS.md` (no READ_CONTACTS), `docs/tasks/T06-smtp-outbox.md`, `CLAUDE.md` security rules.

## Part A: reading (S20, S21)
- Core: add `fetch_body(folder, uid) -> raw bytes` and attachment fetch to `MailBackend` (IMAP BODY.PEEK, JMAP blob download). Parse with `mail-parser`; store text, sanitised HTML and attachment metadata (tables `message_body`, `attachment` exist). Cap size and parts; malformed MIME never panics (fuzz-style test over fixtures).
- Sanitise HTML in the core (choose `ammonia` or a small own allow-list and record it in `docs/DECISIONS.md`): drop scripts, forms, iframes, event handlers, `javascript:` links; strip tracking pixels and remote `src`/`srcset`/CSS `url()` by default, replacing them with a placeholder and a count; `cid:` images resolve to local parts; links carry their real destination.
- Remote content blocked until the person allows it per message or, in settings, for known senders (S49 row, default Ask). The WebView or renderer gets the sanitised string only and has network access disabled while blocked.
- S20: older messages collapsed, newest expanded, action bar on top (archive, delete, mark unread, move), reply bar below. Mark read after the setting's delay (immediately, 2 s, never). S21: "Remote images blocked" bar with Show images, attachment chips (not downloaded, downloading, downloaded, failed, blocked for executables), overflow menu: Mark as unread, Move to, Report spam, Show original (the unsubscribe banner, Snooze and Mute thread are later phases; hide them).
- Threading: conversation built from the stored thread key (JMAP thread id, else References and In-Reply-To).

## Part B: compose and Outbox (S23, S24, S26)
- S23: From identity picker, To, Cc, Bcc input chips with validation (error chip, never colour alone), autocomplete from sent-mail history only (no contacts permission), subject, body, toolbar, attachments through the system file picker (no storage permission), reply, reply all, forward, `mailto:` intent fill. Send button follows the full condition table in `controls.md`.
- Drafts: saved on the device 1 s after typing stops; copied to the server Drafts folder after 5 s (at most every 20 s) and on pause, by APPEND then delete of the old version (contract row). Offline keeps the local draft and copies later. Compose Back with content autosaves with a "Draft saved" snackbar; empty closes.
- Send: writes an Outbox row with `send_at = now + undo send time` (T06), Undo snackbar returns to the draft with the cursor where it was. S26 Outbox lists Queued, Sending, Failed with Retry, Edit (back to draft), Discard (confirm), progress ring while sending.
- S24: no keys exist before phase 3, so build only the policy seam: an `EncryptionPolicy` the compose screen asks before sending. With a fake policy that says "required, recipient X has no key" the S24 sheet names X and offers Send unencrypted or Cancel; Send never proceeds without a choice. The default policy says "not required".
- Not in this task: S25 Schedule send (needs a Companion), S22 invitations, S31 unsubscribe.

**Store rules (from the 11 November audit)**
- Attachments use the system document picker and photo picker. Request no storage permission and no `READ_MEDIA_*` (Play limits them to apps whose core function needs them; **Confirm**). No `READ_CONTACTS`: use the system Contact Picker if device contacts are ever offered.
- Sharing and opening attachments go through a `FileProvider` with a temporary URI grant, never `file://`.
- The `mailto:` and `SEND` intent filters are exported and BROWSABLE: declare only what is handled.
- If a WebView renders HTML mail, turn off JavaScript and file access; remote content stays blocked by default.

## Done when
- Rust: parser tests over fixtures (plain, multipart/alternative, inline cid, nested attachments, broken charset, huge part, truncated MIME); sanitiser tests show script, iframe, event handler, tracking pixel, CSS url() and remote image removed with blocking on and kept with it off; no body text appears in any error string.
- Integration (container Stalwart and GreenMail, feature `integration`): fetch a body and an attachment over IMAP and JMAP; draft APPEND then replace; send with the T06 fake SMTP server, with no network (queued, then sent exactly once on reconnect), and Undo inside the window sends nothing.
- JVM tests (host library): reply, reply all and forward fill the right fields; the Send button condition table, row by row; autosave survives a simulated crash (kill after 1 s: local draft restored; after 5 s: server draft present); Outbox states and actions; `EncryptionPolicy` with the fake never sends silently.
- Robolectric: a `mailto:` intent opens compose filled in; the system picker result attaches a file; Back rules.
- Screenshot tests for S20, S21, S23, S24 (with the fake policy), S26: three widths, light and dark, font scale 1.0 and 2.0, LTR and RTL.
- Semantics tests: banners announce, chips labelled, focus after Send goes to the first row.
- On a device (not checkable here): WebView rendering fidelity, scroll and zoom of real HTML mail, keyboard and IME behaviour in compose, attachment save to a real phone. Reported as unverified.
