# T17: Onboarding and account screens

Crate / folder: `android/` (plus a demo backend in `core/protocols`)

Depends on: T05 (discovery), T10 (discover, verify sign-in, add account), T11 (secrets), T12 (S08), T13, T16 (the inbox to land on). Blocks: T20 (S07 comes after the first account), T21 (OAuth sign-in button).

## Read first
`docs/ux/build-order.md` (step 7), `docs/ux/screens.md` and `design/screens.csv` (S01 to S09, S44, S45 rows), `docs/ux/state-machines.md` (account connection), `docs/ux/controls.md` (text fields, buttons, banners), `docs/ux/accessibility.md` (plain language), `docs/ux/navigation.md` (onboarding back rule), `docs/store/READINESS.md` (review access, Gmail wording, no new permissions), `docs/tasks/T05-account-discovery.md`, `docs/PLAN.md` (Gmail and Microsoft sign-in), `design/wireframes/` for these screens.

## Goal
S01 Welcome, S02 Add account, S03 Finding your server, S04 Sign in, S05 Server settings, S09 First sync, S44 Accounts, S45 Account, with the real discovery code and a built-in demo mailbox for store review. S06 (Companion) and S08 (built in T12) are only linked: the "I run a Companion" button is shown disabled with "Coming later" until phase 2.

## Spec
- **S02:** Continue stays disabled until the address is valid (validate on blur, live after the first error); shortcuts for Gmail, Outlook, iCloud, Other.
- **S03:** one row per probe (built-in table, autoconfig, SRV, JMAP well-known) with waiting, running, found, skipped from the T10 progress stream; first match wins; failure leads to S05 pre-filled with whatever was learned. Never blocks on the network; Back goes one step back.
- **S04:** password or app password with show/hide toggle and autofill hints; wrong password shows inline in plain words. Gmail and Outlook show the text from `READINESS.md` row 10 (needs an app password or your own OAuth client) and a button that T21 will connect; until T21, the button is hidden. A self-signed certificate asks once to trust it (trust-on-first-use): show the host and the certificate fingerprint, "Trust this server" or Cancel, and on trust pass the fingerprint to the core (`trust_certificate`) so it pins that exact certificate for this server; a later change of certificate shows the prompt again with a stronger warning. No dialog on plain wrong-password.
- **S05:** protocol, host, port, security, username; no unencrypted option except for private-network addresses, with a red warning; "Test connection" shows each step from `verify_sign_in`; advanced options hidden until asked.
- **S09:** skeleton rows; newest 50 messages first, older in the background; "Continue in background" is always there.
- **S44, S45:** status chips, add, reorder, remove (lists what is deleted locally before confirming, then deletes secrets too through T11), name, identities and signature, folder mapping (Trash, Archive, Junk, Sent, Drafts), sync window, server settings (opens S05), account colour.
- **Failure copy:** every discovery or sign-in error has a plain-language message from the T05 reason keys (Couldn't connect to mail.example.org. Check the address, or try again.), technical detail behind a Details link, no raw codes. Keep strings in resources so they can be translated.
- **Demo mailbox ("Try with sample mail"):** a `DemoBackend` implementing `MailBackend` in `core/protocols` that serves a fixed set of fictional messages (people and `example.org` addresses only) from the app, read-only: no socket is ever opened, actions are kept on the device only and the row says "Not saved to server" (contract rule for read-only mailboxes). It has an Inbox with threads, an attachment, a blocked-image message, Sent, Drafts, Trash. The account is marked demo and can be removed. This is what store review uses (`READINESS.md`, review access): every phase-1 screen must be reachable with it.

## Done when
- Rust: `DemoBackend` tests (folders, snapshot, headers, flag change stays local) and a test that constructing and using it does not touch a network connector (inject a connector that panics).
- JVM tests (host library, T10 surface): discovery of three test servers (GreenMail, Dovecot or the in-process IMAP, Stalwart) each ends at S04 with the right settings; one wrong password shows the inline error; one self-signed certificate shows the trust prompt, and after trust the account connects and the pinned fingerprint is stored; a changed certificate prompts again.
- Every failure case (no server found, unreachable host, wrong password, certificate untrusted, TLS version too old, timeout) renders its own message; a test iterates all T05 reason keys and fails if one has no string.
- Compose tests: Continue disabled until a valid address; Back order through S02 to S05 and out of the app on S01; the probe rows show each state; Test connection steps appear in order; offline shows the failure, not a hang.
- Removing an account in S44 deletes its rows and secrets (core test plus `SecretStore` test) and the confirmation lists them.
- Screenshot tests for S01 to S05, S09, S44, S45: compact, medium, expanded, light and dark, font scale 1.0 and 2.0, LTR and RTL.
- Sample mail: a Compose/Robolectric flow starts from S01 and reaches the inbox, a conversation (once T18 exists), Settings and Accounts without network (airplane-mode flag).
- On a device (not checkable here): password manager autofill, the browser-tab hand-off, TalkBack script for "add account" (run by hand before release).
