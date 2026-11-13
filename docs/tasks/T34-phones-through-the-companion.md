# T34: Phones through the Companion

Folders: `companion/` and `web/` (signing an app in), `android/` (flavours, S06, S58, push), `core/` (the Companion client, if it
belongs there).

Depends on: T24 and T25 (sign-in), T28 (the push relay), T29 (the gateway for FCM). Closes phase 2.

Built in steps, each its own pull request:
- **T34a** signing a phone in to the Companion (Companion and web);
- **T34b** the Android flavours `play`, `fdroid`, `github` and their checks;
- **T34c** S06 Connect a Companion and S58 Companion and devices on Android;
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

## Spec (T34b to T34d)
Written in this file when each starts.
