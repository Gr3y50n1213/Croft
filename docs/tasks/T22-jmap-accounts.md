# T22: JMAP accounts in the app

Crate / folder: `core/ffi` (settings, account start, verify, submit), `android/` (onboarding only where it must change)

Depends on: T09 (the JMAP adapter, `JmapBackend`, `JmapSubmitter`), T10, T11, T17 (S02 to S05, S09). Last gap of roadmap phase 1
("IMAP and JMAP sync"): until now `add_account` refuses settings without IMAP, and nothing starts a JMAP account.

## Read first
`docs/PLAN.md` (the protocol table: JMAP is preferred where a server offers it), `docs/tasks/T09-jmap-adapter.md`, `core/protocols/src/jmap/`
(what the adapter needs: session URL, credential, trust), `core/ffi/src/settings.rs` (the stored settings and their version),
`core/ffi/src/credentials.rs` (`AccountBackend`), `core/ffi/src/submit.rs`, `docs/ux/screens.md` (S03 to S05), `CLAUDE.md` security rules.

## Goal
An account on a JMAP server (Stalwart, Fastmail, any server that publishes `/.well-known/jmap`) can be added from S02 by its address and
a password, syncs, sends and works offline exactly like an IMAP account. Nothing outside `core/protocols` knows which protocol it uses.

## Spec
- **Which protocol.** When discovery finds a JMAP session URL, the account uses JMAP (mail and sending); otherwise IMAP and SMTP as now.
  A server that offers both gets JMAP. A JMAP account never stores or uses IMAP or SMTP settings. The provider table's servers stay
  IMAP (none of them publishes JMAP to third-party clients with a password today), so Gmail, Outlook and OAuth are unchanged.
- **Stored settings.** A new settings version (the reader still reads every earlier one) with `jmap: { session_url, username }` and the
  pinned fingerprint, as for IMAP. The account row's `protocol` is `Jmap`. No secret in the settings.
- **Starting the account.** The account's backend wraps a `JmapBackend` built from the stored settings and the credential the provider
  answers, with the same states as IMAP (no credential, storage not ready, ready, unusable) and the same sign-in-needed handling. The
  submitter is the `JmapSubmitter` (it files the sent copy itself). The JMAP id map is the persisted one (T09).
- **Verify and add.** `verify` checks a JMAP account by opening its session with the credential (wrong password, certificate untrusted or
  changed are the same typed failures as IMAP); `add_account` stores and starts it; adding the same address with the same JMAP server is
  signing in again, another server is `AccountExists`, a pin is never replaced unconfirmed: the same rules as IMAP.
- **Android.** S03 already shows the JMAP probe. S04 shows the JMAP server in the account card. S05 (manual settings) stays IMAP and
  SMTP in this task: typing a JMAP URL by hand is phase 2.
- **FFI.** Bump `FFI_API_VERSION` only if a type the app reads changes.

## Done when
- Rust: a JMAP account added through the FFI against the in-process JMAP server (`croft-test-support`) syncs its Inbox, marks read,
  moves, sends (EmailSubmission) and files the sent copy, and survives a restart of the core (settings read back, id map kept).
- Rust: a wrong password is `VerifyFailure::WrongPassword`; a self-signed server asks for trust and works pinned; re-adding the address
  signs in again; settings of every earlier version still read; a JMAP account's settings hold no secret.
- Rust integration (CI, Stalwart in Docker): add, sync and send against Stalwart through the FFI.
- JVM (host library): the onboarding backend adds a JMAP account found by discovery and its Inbox shows.
- On a device (not checkable here): a real Fastmail or Stalwart account. Reported as unverified.
