# T21: Bring-your-own OAuth for Gmail and Outlook

Crate / folder: `core/protocols` (OAuth flow, XOAUTH2), `core/ffi`, `android/`

Depends on: T03 and T06 (XOAUTH2 on IMAP and SMTP), T05 (the provider table), T10, T11 (keystore and credential callback), T17 (S04). Last task of roadmap phase 1.

## Read first
`docs/PLAN.md` ("Gmail and Microsoft sign-in", Risks row on CASA), `docs/store/READINESS.md` (row 10 wording, Google OAuth verification, the 100-user cap), `docs/ux/screens.md` (S04: OAuth opens a browser tab, never an embedded view), `docs/ux/state-machines.md` (Needs sign-in), `docs/ux/accessibility.md` (plain language), `CLAUDE.md` (security rules), `docs/tasks/T11-secrets-keystore.md`.

## Goal
A person who registers their own OAuth client with Google or Microsoft pastes its client id into Croft and signs in with Authorization Code + PKCE. Croft ships no shared client id or secret. Tokens live only in the keystore.

## Spec
- **Core flow** (`oauth2` crate, rustls): build the authorisation URL (code challenge S256, random `state`, redirect URI), parse and validate the redirect (state must match, `error=access_denied` becomes a plain message), exchange the code, refresh, and detect revocation (`invalid_grant` is `BackendError::Auth`). Provider table: Google `https://mail.google.com/` scope, `accounts.google.com` and `oauth2.googleapis.com`; Microsoft `IMAP.AccessAsUser.All`, `SMTP.Send` and `offline_access` on the v2 endpoint of `login.microsoftonline.com`. The implementer confirms current endpoints, scopes and accepted redirect forms from each provider's own documentation and records them in `docs/DECISIONS.md`. Only those hosts are contacted (extends T05's "nothing to other hosts" rule).
- **Public client only.** No client secret is stored or requested. If a provider's client type needs a secret, pick the client type that does not (Android or iOS/desktop type for Google, "mobile and desktop" for Microsoft) and describe it in the help text.
- **Token handling.** Extend T11's `CredentialProvider` with refresh-token read and `save_tokens(account, tokens)` so the core refreshes in memory and hands rotated tokens back for the keystore; the core never persists them. The client id is not a secret and is kept in the `setting` table per provider. Bump `FFI_API_VERSION` and `EXPECTED_FFI_API_VERSION`. Access tokens are refreshed shortly before expiry and on an `Auth` reply, once, before declaring Needs sign-in.
- **XOAUTH2** for IMAP `AUTHENTICATE` and SMTP (`lettre`); the same string format for both; no token in any log or error.
- **App side.** Chrome Custom Tabs (or the default browser) through AndroidX Browser; never a `WebView`. A redirect `Activity` with a custom-scheme or claimed-link filter receives the result and hands it to the core. Handle the person closing the tab (cancel, no error toast), process death during the flow (state survives in the core's pending-flow record for 10 minutes), and a second launch.
- **Setup help.** A "Use your own client" section in S04 (and S45 for re-sign-in) with a plain-language guide: where to register, which client type, the redirect value to copy (copy button), and that a Google project left in Testing status issues refresh tokens that expire after 7 days, so the person should publish it (implementer **confirms** this from Google's page before writing it). Wording follows `READINESS.md` row 10; the shared-client path is not built, and "Gmail" and "Outlook" appear as words only, no logos.
- **Re-sign-in.** Needs sign-in on an OAuth account starts the flow again for that account and resumes the paused queue.

## Done when
- Rust unit tests: PKCE challenge matches the RFC 7636 appendix B vector; `state` mismatch, missing code and `access_denied` redirects are rejected with the right typed error; the authorisation URL contains the expected scope, challenge method and redirect and no secret.
- Rust tests against an in-process fake token endpoint (in `core/test-support`): code exchange, refresh with rotation, `invalid_grant` giving `Auth`, a slow endpoint giving `Transient`; a test that requests to any other host fail.
- XOAUTH2 test: the in-process IMAP and the fake SMTP server accept `AUTHENTICATE XOAUTH2` with a valid token and refuse an expired one; on refusal the engine refreshes once and retries once.
- JVM tests (host library): the full flow with the fake endpoint stores tokens only through the fake `CredentialProvider` (assert no token in the database file, settings, or any log capture).
- Robolectric: the Custom Tabs launch intent carries the built URL; a grep test proves no `WebView` class is used in the OAuth code; the redirect Activity forwards the result; cancel and a stale `state` show the plain messages.
- Screenshot tests for the S04 OAuth variants (Gmail, Outlook, help section, error): three widths, light and dark, font scale 1.0 and 2.0, LTR and RTL.
- On a device (not checkable here): the real Google and Microsoft consent pages, the browser hand-off on real phones, refresh-token lifetime with a real account. Reported as unverified.
