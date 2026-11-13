# T32: Web settings (W04), the installable app and push, accessibility

Folder: `web/` (and the Companion's `webapp.rs` if serving needs a change).

Depends on: T30 (sign-in, the shell), T31 (W01, W02), T28 (the push relay). Closes the web part of phase 2.

Built in two pull requests: **T32a** W04 (settings) and **T32b** the PWA with push, and the accessibility sweep.

## Read first
`docs/UX.md` (W04, S43 to S60 for what settings hold, "Keyboard, switch and pointer"), `docs/ux/customisation.md`,
`docs/ux/theme.md`, `docs/ux/swipe.md` (S47, S48), `design/wireframes/27_W4_web_settings.png`, `companion/README.md` (the
passkey, session and push routes).

## Spec (T32a, W04)
- **The settings page** (`/settings`, `/settings/<category>`): a category list and a detail pane (W04), back to the inbox; on
  narrow windows the list, then the detail.
- **Appearance**: mode (system, light, dark, pure black), palette (the token presets), contrast (standard, medium, high),
  density (comfortable, default, compact), roundness (tight, standard, round), list style (container, flat), with a live preview;
  applied at once through the generated token CSS (`data-croft-*` on `<html>`), and before the first paint on the next load.
  Kept in this browser for now (synced through the Companion later, OPEN-ITEMS). Haptics: "Off on the web".
- **Swipe and hover actions** (S47 on the web): the action for each direction and the sensitivity, from the swipe settings types
  (T30b); W01's hover buttons and trackpad swipes follow them.
- **Security**: the passkeys (`GET /api/passkeys`; add one; remove one, never the last), the signed-in devices
  (`GET /api/sessions`; sign one out; this one marked), and sign out everywhere else.
- **Accounts**: the mail accounts (from the JMAP session), read only; adding one stays on the phone app for now.
- **About**: the version (`/version`), the licence (MPL-2.0 for the app, AGPL-3.0 for the Companion), the source.

## Spec (T32b, the PWA, push, accessibility)
- **Installable**: a web manifest (name, icons from the brand, theme colour from the tokens, `display: standalone`), served under
  the page's CSP (`manifest-src 'self'`).
- **A service worker** (`worker-src 'self'`): caches the app shell (the page, `/_app/immutable/`, `/theme.js`, the icons) so the
  app opens offline with an "Offline" banner; never caches or intercepts `/api`, `/jmap`, `/oidc` or `/enrol` (mail and tokens
  are never stored by it).
- **Push in the browser** (Notifications in W04): subscribe with the Companion's VAPID key (`/api/push/vapid`, then
  `POST /api/push/subscriptions`), unsubscribe; the service worker receives the content-free ping and, when an app tab is open,
  tells it to refresh, else shows "New mail" (no content: the ping has none) that opens the app. The subscription belongs to this
  browser's session, so signing out ends it (T28).
- **Accessibility**: `axe-core` on every screen in the end-to-end tests (sign-in, W01, W02 with a compose window, W04's
  categories), every control reachable by keyboard, focus visible.

## Done when
- T32a: unit tests for the appearance settings (stored, applied, defaults); Playwright: the theme changes live and survives a
  reload without a flash; the swipe setting changes the hover buttons; a passkey is added and removed (never the last); another
  session is signed out; axe-core clean on W04.
- T32b: Playwright: the manifest and service worker are served under the CSP and install; the app opens offline with the banner;
  the service worker never touches `/api` or `/jmap`; a push subscription is made and removed; a ping refreshes an open tab.
  An independent review of the service worker and push handling.
- Not checkable here: installing on real devices, push through real browser push services (Mozilla, Google, Apple).
