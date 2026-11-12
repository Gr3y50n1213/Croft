# T24: Companion sign-in

Crate / folder: `companion/`

Depends on: T23. Blocks T26 (mail accounts belong to a signed-in user), T30 (the web client signs in here).

## Read first
`docs/PLAN.md` (Security: "Companion sign-in", "Multi-user isolation", "Audit and recovery"), `docs/tasks/PHASE-2.md`,
`docs/tasks/T23-companion-foundation.md`, `companion/README.md`, `docs/DECISIONS.md` (the T23 rows, including what was accepted for now).

## Goal
People sign in to their own Companion with a passkey, with TOTP as a second factor where a passkey cannot be used, and get short-lived
access tokens that the web app and the phones use. An admin can manage users but never act as them.

## Spec
- **Enrolment.** `user add` prints a one-time enrolment link (random token, stored only as a hash, single use, 24 hours). The link opens
  a minimal server-rendered page (no framework, a strict CSP allowing only the Companion's own script) that registers a **passkey**
  (WebAuthn, user verification required, a discoverable credential preferred) and shows the TOTP secret as a QR code and text, with ten
  recovery codes (shown once, stored hashed). A user can register more passkeys while signed in.
- **Sign-in.** A passkey (WebAuthn assertion) signs in. A client that cannot use a passkey signs in with the user name, a TOTP code and a
  recovery-grade check (a recovery code once, which then asks for a new passkey). TOTP alone never signs in.
- **Tokens.** An access token (random 32 bytes, stored hashed, 15 minutes) and a refresh token (random, stored hashed, 30 days, rotated
  on every use). Reuse of a rotated refresh token revokes that whole sign-in (token family) and is audited. Each sign-in is a device
  the user can see and sign out remotely.
- **Limits.** Failed sign-ins are rate limited per client address and per user; after 10 failures in 15 minutes the user is locked for
  15 minutes (audited, and the user sees it). Constant-time comparisons for every secret.
- **Roles.** An admin can list, disable and re-enrol users and see the instance's audit log; an admin can never read another user's
  data or act as them. A disabled user's tokens stop working at once.
- **Isolation.** Every handler takes the user from the token and passes it to the store; every store query is scoped by that user id.
  A test helper calls every endpoint as user A with user B's identifiers and expects a refusal.
- **Audit.** Sign-ins (success and failure), new devices, sign-outs, enrolments, admin actions. Each user can read their own entries.
- **HTTP.** The slow-client protection accepted in T23 is revisited here: a header-read timeout and a connection limit, now that sign-in
  endpoints are exposed.

## Done when
- WebAuthn registration and assertion pass with a software authenticator in tests (made-up keys); a wrong origin, a replayed challenge
  and a counter going backwards are refused.
- TOTP: RFC 6238 test vectors; a used code is not accepted twice; clock drift of one step either way is accepted.
- Tokens: expiry, rotation, reuse detection revoking the family, remote sign-out, disabled user refused at once.
- Lockout and rate limits hold under a scripted burst; constant-time comparison is used for every token and code.
- The cross-user test covers every endpoint.
- No token, code, secret or passkey private material appears in a log, an error or the audit log.
- Not checkable here: real authenticators (phones, security keys, platform passkeys) in real browsers. Reported as unverified.
