# T34: Phones through the Companion

Folders: `companion/` and `web/` (signing an app in), `android/` (flavours, S06, S58, push), `core/` (the Companion client, if it
belongs there).

Depends on: T24 and T25 (sign-in), T28 (the push relay), T29 (the gateway for FCM). Closes phase 2.

Built in steps, each its own pull request:
- **T34a** signing a phone in to the Companion (Companion and web);
- **T34b** the core's Companion client, and S06 Connect a Companion and S58 Companion and devices on Android;
- **T34c** the Android flavours `play`, `fdroid`, `github` and their checks;
- **T34d** push registration (UnifiedPush in `fdroid` and `github`, FCM through the gateway in `play`) and push in place of
  periodic checks.

Syncing a phone's mail through the Companion (instead of straight from the mail server) is not in T34: the phone already
reaches its mail server; the Companion adds push, search and scheduled send (OPEN-ITEMS).

## Read first
`docs/UX.md` (S06, S07, S58), `docs/store/READINESS.md` (flavours, background work, data that leaves the device),
`companion/README.md` (sign-in, tokens, push), `docs/tasks/PHASE-2.md`, RFC 8252 (OAuth 2.0 for native apps) and RFC 7636 (PKCE).

## A decision first: how a phone signs in
A phone signs in through the browser, as RFC 8252 asks of native apps: the app opens the Companion's sign-in page in a Custom Tab,
the person signs in there (passkey, or single sign-on), and the Companion hands a one-time code to the app's redirect,
`<applicationId>:/companion`, a private-use URI scheme; the app redeems it with its PKCE verifier for its own tokens (T24's app
tokens: 15-minute access, rotating refresh, a device the person can sign out).
- Why not a passkey inside the app (Credential Manager): Android only lets an app use passkeys for domains that vouch for it
  (`assetlinks.json` naming the app's signing key), and a self-hosted Companion's domain is chosen after the app is built.
- Why not an App Link (`https://` redirect): the same, Android verifies App Links only for domains named in the app.
- The cost: another app can register the same scheme. PKCE makes a caught code useless to it. An app that starts its own flow
  with the Croft scheme still needs the person to act on the Companion's own page, which says plainly that it connects the Croft
  app on a phone: a passkey asks for the person's verification, and single sign-on starts only from a click on that page (never
  from a link) and makes the provider ask the person to sign in (`prompt=login`), even with a session in the browser (T34a
  review). Every new device shows in the person's list and the audit log, where it can be signed out.

## Spec (T34a)
- The Companion accepts as an app redirect, besides its own pages, exactly the Croft app's schemes (`<applicationId>:/companion`
  for the Play, F-Droid, GitHub and preview builds; placeholders until the real application id).
- `/app/sign-in?app_challenge=..&app_redirect=..` (a web app page): "Connect the Croft app", with passkey and single sign-on.
  A passkey sign-in there ends in a hand-over code for the app, never in a browser session.
- `POST /api/sign-in/passkey/finish` with `app: {challenge, redirect}` answers `{redirect}` (the app's redirect with `code`).
- `POST /api/sign-in/app/sso` `{challenge, redirect}` (from the page, by a click): single sign-on for the app.
- `POST /api/sign-in/app/exchange` `{code, verifier, device}`: the app's tokens, for a code from either way of signing in. Codes
  live 60 s, once, bound to the challenge.
- Done when: Companion tests (the redirect rule, codes once and in time, the verifier, a disabled user, limits), Playwright (the
  page signs in with a virtual passkey and lands on the app's redirect with a code that the test redeems), an independent review.

## Spec (T34b)
- **The core's Companion client** (`core/protocols/src/companion.rs`, so the iPhone app gets it too): the address (`https://host[:port]`,
  or `http` to this machine for tests), a probe (version, single sign-on), the browser sign-in with PKCE (the verifier stays in the
  core), and the app's session: the access token renewed with the refresh token, each new secret handed to the app's saver; the
  person's devices, signing one out, signing this one out; the push calls T34c needs. FFI v14 (`companion_*`, `CompanionLink`).
- **Android**: S06 Connect a Companion (the address, Connect opens the Companion's page in a Custom Tab, the redirect activity brings
  the code back), S58 (the Companion, who is signed in, the signed-in devices with Sign out, Disconnect this phone). The connection's
  secret is the app's own entry in the keystore-backed store, apart from accounts' (the orphan sweep never sees it; erasing everything
  removes it). A redirect with no sign-in waiting is ignored.
- Done when: core tests (addresses, the page address, secrets read back, callbacks without a code), an end-to-end test against a real
  Companion on this machine (probe, sign-in, devices, renewal with the new secret saved, an old secret ending the session, sign-out),
  Android unit tests (the connection's states, ignored redirects, a replaced secret kept, signed out by the Companion), the manifest
  check (the redirect activity allowed only for the app's own scheme), an independent review.
- After the review: a renewal runs to the end even when its caller gives up (the new tokens are never lost to a closed screen);
  one link per process (never a second one from the stored secret); a new secret that could not be stored is kept in memory and
  stored at the next step; a code the Companion refuses leaves the sign-in waiting for the real one; only a 401 ends the session;
  plain `http` to this machine only in debug builds; the Companion client follows the process's address policy; a key loss moves
  the Companion secret aside with the accounts'; an erase and a secret that cannot be kept sign the device out first.

## Spec (T34c and T34d)
Written in this file when each starts.
