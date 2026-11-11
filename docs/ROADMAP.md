# Roadmap

Rough estimates for one developer directing Claude Code. Each phase ends in something people can install.
Revisit the durations after phase 0 shows the real pace.

| Phase | Name | About | Contents |
|---|---|---|---|
| 0 | Foundations | 6 weeks | Monorepo, CI, test-server containers, design tokens. Rust core skeleton called from a Compose screen through UniFFI. Data model, SQLite schema, sync state machine design. |
| gate 1 | | | Core reads a real inbox from Stalwart and Dovecot |
| 1 | Android mail MVP | 3 months | Autodiscovery, IMAP and JMAP sync, SMTP send, offline outbox. Inbox, conversation view, compose, swipe actions, periodic sync. App passwords; bring-your-own OAuth for Gmail and Outlook. |
| gate 2 | | | Daily-driver alpha: the team uses it on their own mail. Start the store paperwork now (see below) |
| 2 | Companion and web | 3 months | Multi-user Companion: JMAP API, push relay, search, Docker image. Web app: three-pane inbox, compose, keyboard shortcuts, PWA. Passkeys and OIDC sign-in; push via UnifiedPush and the FCM gateway. |
| gate 3 | | | Public beta on F-Droid, Google Play and Docker Hub. Needs the Play closed test finished and the store items in `docs/store/READINESS.md` done |
| 3 | Smart inbox, calendar, PGP | 4 months | Categories, snooze, one-tap unsubscribe, on-device search index. Calendar and contacts on Android and web (CalDAV, CardDAV). OpenPGP sign and encrypt, WKD and Autocrypt key discovery. |
| gate 4 | | | 1.0 after an external security review |
| 4 | iOS and v2 | after 1.0 | SwiftUI app on the same core, APNs push through the Companion. S/MIME, local smart replies, desktop apps on the same core. |

## Store lead times
These take calendar time, not build time, so they start before the gate that needs them (details and sources in `docs/store/READINESS.md`).

| Start by | Item | Lead time |
|---|---|---|
| Phase 0 | Trademark search for "Croft"; Play developer account and identity check; Apple Developer Program | Days to weeks |
| Phase 0 | Android developer verification: register package names and signing keys | Days |
| Gate 2 | Play closed test: at least 12 testers opted in for 14 days (personal accounts created after 13 Nov 2023) | At least 2 weeks |
| Gate 2 | Google OAuth verification and security assessment for the shared Gmail sign-in; until done, the app is capped at 100 new users | Weeks to months; get a quote |
| Phase 4 | Apple default-mail-app entitlement request (Account Holder); test it in TestFlight early | Apple decides |
