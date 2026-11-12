# T26: Mail accounts in the Companion

Crate / folder: `companion/` (and, where the core needs a small public hook, `core/ffi`).

Depends on: T24 (users and sessions). Blocks T27 (the JMAP API serves these accounts) and T28 (push watches them).

## Read first
`docs/PLAN.md` (Security: "Credentials", "Companion storage", "Multi-user isolation"), `docs/tasks/PHASE-2.md`,
`companion/README.md`, `docs/DECISIONS.md` (the T23 to T25 rows), and the core's account API: `core/ffi/src/core.rs` (`Core::open`,
`discover`, `verify`, `add_account`, `remove_account`, `signed_in`), `core/ffi/src/credentials.rs` (`CredentialProvider`).

## Goal
A signed-in Companion user adds their own mail accounts (IMAP/SMTP or JMAP, with a password), and the Companion runs the same Rust
core for them that the phones run, with that user's mail and credentials sealed under that user's own key. Nothing of one user's mail
or credentials is reachable through another user, and a stolen database reveals nothing.

## Spec
- **One core per user.** Each user with accounts gets a `croft_ffi::Core` over their own SQLCipher database
  (`data_dir/users/<user id>/mail.db`). Its key is derived from the user's data key (HKDF-SHA256, a fixed context string), never stored.
  It is opened when the user first needs it and kept open while the server runs (T28 will watch these accounts). Each core has its own
  small runtime; one per user is accepted for households and small companies (DECISIONS), revisited if it does not scale.
- **Credentials.** The Companion's `CredentialProvider` for a user reads the account's password from a table in the Companion's own
  database, sealed with the user's data key and bound to (user, account) in the associated data; `save_tokens` (OAuth rotation) seals the
  new secret the same way; `sign_in_needed` marks the account. Nothing in the mail database or the logs holds a credential.
- **API** (full sessions only, every call scoped to the caller):
  - `POST /api/accounts/discover` (`{email}`) → the core's discovery result.
  - `POST /api/accounts/verify` (`{discovered, password, accept_pin?}`) → ok, or the core's `VerifyFailure` (wrong password, a
    certificate to confirm with its fingerprint, unreachable).
  - `POST /api/accounts` (the same, with a display name) → the new account (the password is sealed only after the core accepts it).
  - `GET /api/accounts` → the caller's accounts (id, address, kind, state); `DELETE /api/accounts/{id}` → removes it and its sealed
    credential.
  - Passwords are taken in request bodies only, wiped after use, and never returned.
- **What T26 leaves out.** OAuth providers (Gmail, Outlook) through the Companion need a browser redirect to the Companion rather than
  the device's loopback; they come later (DECISIONS). The demo account is not offered.
- **Re-enrolment and disabling** do not touch mail accounts; a disabled user's core stops (no syncing for a disabled user).
- **Container.** The image builds SQLCipher (rusqlite `bundled-sqlcipher`, linked against the system's OpenSSL) and ships the OpenSSL
  runtime library; the CI image build covers it.

## Done when
- With the core's in-process test servers (IMAP and SMTP from `core/test-support`, the JMAP test server), a user adds an account through
  the API, sees it listed, and removes it; a self-signed server's certificate is confirmed by fingerprint first.
- Isolation: user B cannot list, verify into, use or remove user A's accounts; B's core never opens A's database (a test with B's key
  on A's file fails to open it).
- No password in the Companion's database in the clear, in the mail database, in an answer or in the audit log; a wrong master key opens
  neither the credentials nor the mail database.
- The cross-user test of T24 covers the new routes.
- Not checkable here: real mail servers (covered by the core's own integration tests), many users on one host (memory and threads).
