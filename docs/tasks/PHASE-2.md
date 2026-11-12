# Phase 2: Companion and web (task plan)

Roadmap phase 2: "Multi-user Companion: JMAP API, push relay, search, Docker image. Web app: three-pane inbox, compose, keyboard
shortcuts, PWA. Passkeys and OIDC sign-in; push via UnifiedPush and the FCM gateway." Gate 3 after it is the public beta.

Browsers cannot open IMAP or SMTP connections, so the web app needs the Companion; the Companion comes first. Each task gets its own
file (Goal, Spec, Done when) before it is built, as the master prompt asks. This is the order and the scope of each; details are in
the task files as they are written.

| Task | Scope | Depends on |
|---|---|---|
| T23 | Companion foundation: the `companion/` workspace (AGPL-3.0), axum service, config, SQLite storage and migrations, per-user envelope encryption under a master key kept outside the database, admin CLI, health endpoint, Docker image, CI | phase 1 |
| T24 | Companion sign-in: users and roles, passkeys (WebAuthn) and TOTP, short-lived access tokens with rotating refresh tokens, rate limiting and lockout, audit log | T23 |
| T25 | OIDC sign-in (Authelia, Authentik, Keycloak, Entra ID) as an alternative to passkeys | T24 |
| T26 | Mail accounts in the Companion: a user's IMAP/SMTP or JMAP accounts, credentials sealed with the user's data key, the core's engine running per account | T24 |
| T27 | JMAP API for clients (Core and Mail) over the accounts of T26, per-user isolation | T26 |
| T28 | Push relay: the Companion watches accounts (IDLE or JMAP push) and sends content-free pings over UnifiedPush and Web Push; the Android F-Droid and GitHub flavours register a UnifiedPush distributor | T26 |
| T29 | Thin push gateway (separate service, AGPL) for FCM; the Play flavour; the FCM flavour's Data safety and manifest checks | T28 |
| T30 | Web client foundation: SvelteKit + TypeScript, design tokens (the generator's CSS output), sign-in to the Companion, the state machines shared with Android | T27 |
| T31 | Web W01 inbox and W02 conversation and compose, keyboard shortcuts table, hover quick actions | T30 |
| T32 | Web W04 settings and Appearance, PWA (installable, offline shell), accessibility check with axe-core | T31 |
| T33 | Companion search (tantivy) and scheduled send | T27 |
| T34 | Phones through the Companion: S58 connection screen, sync mode, push in place of periodic checks | T28 |

Things the owner decides before gate 3 are listed in `docs/OPEN-ITEMS.md` (store accounts, the push gateway's host and its keys,
the Docker Hub namespace).
