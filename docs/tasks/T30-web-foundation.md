# T30: The web client's foundation

Folders: `web/` (the app, MPL-2.0 like the other apps) and `companion/` (serving it, and sign-in sessions for browsers).

Depends on: T24 and T25 (sign-in), T27 (the JMAP API). Blocks T31 (the inbox and compose) and T32 (settings, PWA).

Built in two pull requests: **T30a** the app, its serving and sign-in; **T30b** the shared state machines in TypeScript.

## Read first
`docs/PLAN.md` (stack: SvelteKit and TypeScript with an in-house component set on the shared tokens), `docs/UX.md` (W01 to W04, the
web rows of each screen), `docs/ux/state-machines.md`, `docs/ux/swipe.md`, `design/generated/css/croft-tokens.css`,
`companion/README.md` (the sign-in API), `docs/DECISIONS.md` (T24: the enrolment page's CSP; T25: OIDC codes for apps).

## Goal
A person opens their Companion's address in a browser, signs in with a passkey (or single sign-on, or a recovery code), and sees
an empty, signed-in shell drawn with Croft's design tokens. The browser never holds a long-lived secret that a script can read.

## Spec (T30a)
- **The app.** `web/`: SvelteKit (Svelte 5, TypeScript strict) built as a static single-page app (`adapter-static`, a fallback
  page), no server-side code; Vite; `svelte-check`; unit tests with Vitest; end-to-end tests with Playwright (the pre-installed
  Chromium). The generated token CSS (`design/generated/css/croft-tokens.css`) is the only source of colours, type, shape and
  motion; light and dark follow the system.
- **Served by the Companion** at its own origin (no CORS): the built files are read once at start from `web_dir` (a setting; the
  Docker image builds the app and sets it), held in memory, and served by exact path only (no file system access per request, so
  no path tricks). Hashed assets (`/_app/immutable/`) are cached for a year; everything else `no-store`. Paths that are not files
  and not the API (`/api/`, `/oidc/`, `/enrol`, `/healthz`, `/version`) get the app's page. Without `web_dir` the Companion serves
  no app, as before.
- **A strict CSP for the app**: `default-src 'none'`, scripts and styles from `'self'` only (SvelteKit's inline start-up script
  allowed by its hash, computed by the Companion from the page it serves), `connect-src 'self'`, `img-src 'self' data:`,
  `frame-ancestors 'none'`, `base-uri 'none'`, `form-action 'self'`; the other security headers as today.
- **Sessions for browsers.** The sign-in finishes (passkey, recovery, OIDC exchange) take `"web": true`: the refresh token is then
  set as a cookie (`__Host-croft_refresh`, `HttpOnly`, `Secure`, `SameSite=Strict`, `Path=/`, the refresh token's lifetime) and
  not returned in the body; the access token (15 minutes) is returned and kept in memory only. `POST /api/web/refresh` rotates the
  cookie and returns a new access token; it needs the header `X-Croft-Web: 1` (a request a page on another origin cannot send
  without a preflight) and the cookie. `POST /api/web/sign-out` signs the session out and clears the cookie. A refresh token reused
  after rotation revokes the session, as for apps. Plain `http` on this machine (tests, development) uses a cookie without
  `__Host-` and `Secure`.
- **Sign-in screens**: a passkey (with the browser's passkey list: a discoverable credential, no user name needed), single sign-on
  when the Companion has OIDC (PKCE: the verifier in `sessionStorage` only for the round trip, the redirect back to the app's own
  `/oidc/back` page), and a recovery sign-in (user name, TOTP, recovery code), which leads to adding a passkey. Errors in plain
  words, from the API's codes.
- **The signed-in shell**: the user's name, sign out, and an empty W01 layout (sidebar, list, reading pane placeholders) at the
  breakpoints of `docs/UX.md`. The access token is renewed before it expires and after the tab wakes; a failed renewal returns to
  sign-in.
- **CI**: a `web` job (install with the lock file, `svelte-check`, Vitest, build) and the Companion job's end-to-end run (the built
  app served by a real Companion, signed in with Chromium's virtual authenticator).

## Spec (T30b)
- The swipe machine and its geometry (`android/swipe`) ported to TypeScript in `web/src/lib/swipe/`, with the Kotlin tests ported
  one for one (the same cases and property checks), so both platforms keep the same rules (`docs/ux/swipe.md`).

## Done when
- T30a: Companion tests: the app's files served by exact path with the right caching and CSP (the hash matching the page's script),
  the fallback page, nothing outside `web_dir`; the web sign-in finishes set the cookie and return no refresh token; refresh needs
  the header and the cookie, rotates, and a reused cookie revokes the session; sign-out clears it. Playwright against a real
  Companion: enrol a user with a virtual authenticator, sign in to the app with the passkey, see the shell, reload and stay signed in
  (the cookie), sign out. Vitest for the client's token handling. An independent review (sessions in browsers are
  security-sensitive).
- T30b: the ported tests pass in Vitest.
- Not checkable here: real passkey providers (platform authenticators, password managers), Safari and Firefox.
