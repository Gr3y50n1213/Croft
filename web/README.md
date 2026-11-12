# Croft web app

The browser app (T30 onwards): SvelteKit and TypeScript, built as a static single-page app that the Companion serves at its own
address (`companion/README.md`, "The web app"). It talks to the Companion's API; it never holds a long-lived secret a script can
read (the refresh token is an `HttpOnly` cookie, the access token lives in memory). Colours, shapes and motion come from the
generated design tokens (`design/generated/css`), read in place. Licence: MPL-2.0.

Screens: W01 to W04 in `docs/UX.md`. Done so far: sign-in (passkey, single sign-on, recovery leading to a new passkey) and the
empty W01 layout (T30).

## Develop

```
cd web
npm ci
npm run dev        # http://localhost:5173, with /api sent to a Companion on 127.0.0.1:8480
```

## Check

```
npm run check      # svelte-check (types)
npm test           # unit tests (node:test) for src/lib
npm run build      # build/ (what the Companion serves)
cd ../companion && cargo build && cd ../web
npm run e2e        # Playwright: a real Companion serving build/, Chromium's virtual authenticator as the passkey
```

Dependencies are pinned exactly and installed without install scripts (`.npmrc`).
