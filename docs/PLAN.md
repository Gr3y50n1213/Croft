# Croft — Build Plan

Oct 28, 2026

## Vision and principles

Build an open-source mail app that feels as polished as the newest Gmail, but works with any mail server: a self-hosted Stalwart, Mailcow or Nextcloud box first, Gmail and Outlook second. Android and web ship first; iOS follows on the same core.

- **Server-agnostic by design.** Speak open standards (IMAP, SMTP, JMAP, CalDAV, CardDAV), never one server's private API.
- **One brain, native faces.** All mail logic lives in a shared Rust core. Each platform gets its own native UI so it looks and feels right there.
- **Works alone, better with a companion.** The phone app connects straight to any server. An optional self-hosted companion adds the web app, instant push and fast search.
- **Private by default.** No analytics, no third-party servers in the data path, credentials kept in the OS keystore.
- **Polish is a feature.** Motion, typography and speed get the same priority as protocol support.

## Decisions made

Six decisions anchor the plan; everything below follows from them.

| Decision | Choice | Why |
| --- | --- | --- |
| Shared core | Rust, exposed through UniFFI | One sync and protocol engine for Android, iOS, desktop and the server. UniFFI generates Kotlin and Swift bindings, including async functions. |
| UI | Native per platform | Jetpack Compose on Android, SwiftUI on iOS, a TypeScript web app for browsers |
| Backend | Optional self-hosted companion server, multi-user from v1 | Phones work alone; the companion unlocks the web app, instant push and server-side search for a household or a company |
| Audience | Open source, public | Must self-host in one Docker command and handle personal and business setups |
| Builder | You plus Claude Code | Plan is staged into small, testable milestones an AI coding agent can deliver one at a time |
| v1 scope | Mail + push + smart inbox + calendar/contacts + PGP | Ambitious; the roadmap phases it so a usable mail app ships first |

**Why a companion server, and why optional.** Browsers cannot open IMAP or SMTP connections, so any web app needs a server in the middle. iPhones also cannot keep a background mail connection open, so instant push on iOS needs a server that watches the mailbox and sends a push. Making it optional keeps the barrier low: a person with only a Gmail account installs the app and goes; a business or homelab adds the companion next to its mail server.

## Architecture

One Rust core does all mail work; thin native apps draw the screens; an optional companion server reuses the same core to serve the web app and send push.

*(Architecture diagram not exported. In words: three clients, Android, web and later iOS, each use the one Rust core; the optional Companion server reuses the same core and sits between the web app and the mail server.)*

Phones run in **direct mode** by default, talking to the mail server themselves. When a companion is configured, phones can also sync through it, and it wakes them (Android and iPhone alike) with a content-free push.

**Core modules (Rust workspace)**

- `mail-core`: account, mailbox, thread and message models; the sync engine and offline action queue (every change is queued, applied locally, then confirmed by the server)
- `protocols`: one trait (`MailBackend`) with IMAP, JMAP, SMTP and DAV implementations, so the UI never knows which protocol an account uses
- `crypto`: OpenPGP, Autocrypt, key storage glue
- `storage`: SQLite schema, migrations, full-text index
- `ffi`: the small, stable API the apps call, exported via UniFFI (Kotlin, Swift) and wasm-bindgen (browser, for PGP and MIME work done client-side)

**Data flow.** The UI subscribes to observable queries ("threads in Inbox, newest first"). The core pushes updates whenever sync or a user action changes the database, so screens stay live without polling.

**Companion server.** A Rust service built on the same core. It turns any IMAP/SMTP/DAV server into a JMAP API for clients (the same idea as existing IMAP-to-JMAP bridges such as [jmapper](https://github.com/amaanq/jmapper)), watches mailboxes for push, runs scheduled sends and keeps a server-side search index. Against a native JMAP server such as Stalwart it passes requests through.

## Protocol and server compatibility

The app talks only open standards, so it works with any mail server that speaks them. The core picks the best protocol per account automatically.

| Need | Primary protocol | Fallback | Notes |
| --- | --- | --- | --- |
| Read and sync mail | JMAP (RFC 8620/8621) | IMAP4rev1/rev2 with CONDSTORE, QRESYNC, IDLE | JMAP is faster and syncs by state; IMAP covers every other server |
| Send mail | JMAP EmailSubmission | SMTP submission (port 587/465) |  |
| Live updates | JMAP EventSource / WebSocket | IMAP IDLE | Feeds push notifications |
| Calendar | CalDAV (RFC 4791) | JMAP Calendars where offered |  |
| Contacts | CardDAV (RFC 6352) | JMAP Contacts where offered |  |
| Server filters | ManageSieve | none | Vacation replies, server-side rules |
| Account setup | Autoconfig XML, SRV records (RFC 6186), JMAP well-known URL | Manual host/port entry | Users type only their email address |

**Server support targets for v1.** Each gets an automated test container in CI.

| Server | Path used | Test priority |
| --- | --- | --- |
| Stalwart | JMAP, CalDAV, CardDAV | High |
| Mailcow, Mail-in-a-Box, Mailu, docker-mailserver, iRedMail | IMAP + SMTP (Dovecot/Postfix under the hood); SOGo or Radicale DAV where installed | High |
| Nextcloud (with any of the above) | IMAP + SMTP for mail, Nextcloud CalDAV/CardDAV | High |
| Gmail | IMAP + SMTP with OAuth (XOAUTH2), Google CalDAV/CardDAV | Medium (see OAuth risk) |
| Outlook / Microsoft 365 | IMAP + SMTP with OAuth, CalDAV not offered | Medium |
| Fastmail | JMAP | Medium |
| iCloud, Yahoo, others | IMAP + SMTP with app passwords | Low |

**Gmail and Microsoft sign-in.** Both providers block plain passwords for third-party apps and require OAuth. Google classes full mailbox access as a restricted scope: a public app must pass an independent CASA security assessment and repeat it every year ([EmailEngine scope guide](https://learn.emailengine.app/docs/accounts/gmail/gmail-api-scopes)). An open-source project cannot publish one shared client secret safely either. The plan handles this in three tiers: app passwords work on day one; a bring-your-own OAuth client setting lets power users and companies register their own Google project; and a project-owned verified client is a funded later goal (see Risks). Until that client is verified, Google caps an unverified app at 100 users, so onboarding and both store listings say plainly that Gmail needs an app password or your own client (see `docs/store/READINESS.md`).

## Feature spec for v1

v1 is a complete daily-driver mail app plus calendar and contacts. Anything not listed here waits for v2.

**Mail basics**

- Add accounts by email address only; autodiscovery fills in the rest
- Unified inbox across accounts, plus per-account folders and labels (Gmail labels map to folders)
- Conversation threading, with a per-account toggle for single messages
- Compose with rich text, attachments, inline images, signatures per identity, send-as aliases
- Drafts sync to the server; offline compose queues until online
- Undo send (5 to 30 seconds), scheduled send (companion required when the phone is off)
- Swipe actions, multi-select, keyboard shortcuts on web
- Safe HTML rendering: remote images blocked by default, tracking pixels stripped

**Push notifications**

- With companion: the companion keeps IDLE or JMAP push open and wakes the phone. Android uses UnifiedPush (any distributor, no Google needed) or FCM; iOS uses APNs. FCM and APNs pings pass through the project's thin push gateway (see Review decisions). Push carries only an account ID and a "new mail" ping; the phone fetches the content itself.
- Without companion on Android: periodic sync is the default, and a Companion or UnifiedPush gives instant delivery. The Google Play build has no always-on foreground service, because Android 14 and later require a declared foreground-service type that Play reviews, and none is a clean fit for holding IDLE open. The F-Droid and GitHub builds may offer an opt-in persistent connection.
- Notification actions: archive, delete, mark read, reply.

**Gmail-style smarts** (all computed on device or on the companion, never a third-party service)

- Inbox categories: Primary, Updates, Promotions, Social, Forums, built from headers (List-Id, List-Unsubscribe, Precedence, known senders) plus a small on-device classifier
- Snooze, pin and follow-up reminders. Pin and follow-up are IMAP or JMAP keywords, so they sync across devices. Snooze is a device schedule, because an IMAP keyword carries no value to hold a wake time; with a Companion the message also moves to a Croft/Snoozed folder (a label on JMAP) and is moved back at the wake time, so other clients see it too
- One-tap unsubscribe using RFC 8058 List-Unsubscribe-Post
- Fast full-text search with an on-device index (SQLite FTS5) and server search as fallback
- Smart reply suggestions are a v2 item, optional and local-only

**Calendar and contacts**

- Calendar: day, 3-day, week, month and agenda views; create, edit and RSVP; reminders; multiple calendars with colours
- Invitations in mail (iMIP) show an inline RSVP card
- Contacts: browse, search, edit; autocomplete in compose draws from contacts and recent recipients

**Encryption (PGP)**

- OpenPGP sign, encrypt, decrypt and verify, via the Rust core so it works the same on every platform
- Keys generated or imported on device; private keys never leave it unless the user exports them
- Key discovery via WKD and Autocrypt headers; a clear lock badge in the message view
- S/MIME is a v2 item for business users

## UI, UX and design system

The Android app follows Material 3 Expressive, the language Gmail adopted in 2025: the inbox sits in rounded containers on a tinted background, with pill-shaped swipe actions, a search app bar and springy motion ([9to5Google](https://9to5google.com/2025/09/02/gmail-material-3-expressive-containers/)). Every platform shares one set of design tokens so the brand matches, while components stay native.

**Shared design tokens.** Colour, type scale, shape and motion live in one tokens file (JSON), generated into Compose themes, SwiftUI themes and CSS variables. Colour schemes come from a seed colour via Material Color Utilities, and Android 12+ uses the wallpaper (dynamic colour).

**Per platform.**

- **Android:** Jetpack Compose with Material 3 Expressive components; adaptive layouts (list-detail on tablets and foldables); predictive back; home-screen widget; edge-to-edge.
- **Web:** SvelteKit (TypeScript), with an in-house component set built on the shared tokens. Google's Material Web library is in maintenance mode and lacks Expressive components ([GitHub](https://github.com/material-components/material-web)), so the project owns its components. Three-pane layout on wide screens, full keyboard shortcuts, installable as a PWA with web push.
- **iOS (phase 4):** SwiftUI with iOS-native navigation and Liquid Glass styling, using the same colours and type. It should feel like a good iPhone app, not an Android port.

**Key screens to design first, in order:**

1. Onboarding: enter email, autodiscover, sign in, done
2. Inbox list with categories, swipe actions, unified/per-account switch
3. Conversation view with quoted-text folding and attachment previews
4. Compose as a full screen on phones and a floating window on web
5. Search with filter chips (from, has attachment, date, account)
6. Calendar agenda and week view
7. Settings: accounts, notifications, encryption keys, companion connection

**Quality bar.** 60 fps scrolling with 50,000 messages cached, cold start under 1 second on a mid-range phone, full TalkBack, VoiceOver and screen-reader support, and right-to-left languages from day one.

**Full UX detail.** Swipe gestures, control states, state machines, motion, haptics, the theme picker and a wireframe for every screen are in the [UX.md](UX.md). Build from its screen IDs (S01 to S60) so the code, tests and wireframes use the same names.

## Security and privacy

The app holds people's whole mailbox, so security is designed in from the first commit, not audited in at the end.

- **Credentials:** passwords and OAuth refresh tokens stored only in Android Keystore, iOS Keychain, or the companion's encrypted secrets store. Never logged, never in backups.
- **Local data:** the on-device database is encrypted with SQLCipher, keyed from the OS keystore. Optional app lock with biometrics.
- **Transport:** TLS 1.2+ only, certificate validation always on; self-signed certificates allowed only through an explicit trust-on-first-use prompt that pins the fingerprint (common on homelab servers).
- **Companion sign-in:** passkeys and TOTP two-factor in v1, or the server's OIDC provider (Authelia, Authentik, Keycloak, Entra ID); short-lived access tokens with rotating refresh tokens; login rate limiting and lockout.
- **Multi-user isolation:** every query is scoped to the signed-in user at the data layer, not only in API handlers; admins manage accounts and quotas but cannot read other users' mail; automated tests try cross-user access on every endpoint.
- **Companion storage:** envelope encryption. Each user's mail credentials and cached mail are encrypted with that user's own data key; data keys are wrapped by a master key kept outside the database (a secrets file, Docker secret, or a KMS such as Vault). A stolen database alone reveals nothing.
- **Audit and recovery:** an audit log of sign-ins, new devices and admin actions, visible to each user; remote sign-out of lost devices; encrypted backups.
- **HTML mail:** rendered in a sandboxed WebView/iframe with scripts disabled and a strict content security policy; remote content proxied through the companion when enabled, hiding the reader's IP from trackers.
- **Push payloads:** encrypted end to end (RFC 8291 Web Push encryption) and content-free, so Google, Apple and the push gateway learn nothing about mail.
- **Supply chain:** reproducible builds, signed releases, dependency auditing (cargo-deny, Dependabot), F-Droid builds from source.
- **Process:** threat model written in phase 0, SECURITY.md with a private reporting address, fuzzing of MIME and protocol parsers, and an external security review before 1.0. Security-critical code (auth, crypto, isolation) always gets a line-by-line human review, even when Claude Code wrote it.

## Tech stack

The stack favours mature, permissively licensed Rust crates, several from the Stalwart project. Pin versions when the repo is created; library choices here are the starting point, not a lock-in.

| Layer | Choice | Role |
| --- | --- | --- |
| Async runtime, TLS | tokio, rustls | Networking everywhere |
| IMAP | async-imap (or imap-codec / imap-next) | IMAP sync, IDLE, CONDSTORE/QRESYNC |
| JMAP | [jmap-client](https://docs.rs/jmap-client) (Stalwart) | Full JMAP Core, Mail and WebSocket |
| MIME parsing | mail-parser (Stalwart) | Fast, lenient RFC 5322 / MIME parsing |
| Sending | mail-send or lettre | SMTP submission with XOAUTH2 |
| CalDAV / CardDAV | libdav + calcard (or icalendar) | DAV sync and iCalendar/vCard parsing |
| OpenPGP | rPGP (pgp crate) | Pure Rust, MIT/Apache licensed |
| Local storage | SQLite via rusqlite + SQLCipher, FTS5 | Cache, outbox, search index |
| OAuth | oauth2 crate | Google and Microsoft sign-in with PKCE |
| Bindings | [UniFFI](https://github.com/mozilla/uniffi-rs) + cargo-ndk; wasm-bindgen for web | Kotlin and Swift APIs; WebAssembly for the browser |
| Android UI | Kotlin, Jetpack Compose, Material 3 Expressive, WorkManager, UnifiedPush library | App shell and background sync |
| Web UI | SvelteKit, TypeScript, Material Color Utilities | Web client and PWA |
| iOS UI (later) | SwiftUI, BackgroundTasks, APNs | iPhone and iPad |
| Companion server | Rust: axum, PostgreSQL or SQLite, tantivy search | JMAP API for clients, push relay, scheduled send, search |
| Packaging | Docker image + Compose file, Helm chart later | One-command self-hosting |

## Roadmap and milestones

A usable Android mail app comes first, then the companion and web app, then the bigger v1 features, reaching 1.0 in roughly a year for one developer working with Claude Code.

*See [ROADMAP.md](ROADMAP.md) for the five phases and four release gates.*

Each phase ends in something people can install, so the project stays useful even if later phases slip. Durations are estimates; revisit them after phase 0 shows your real pace.

## Repo, testing, release and governance

One monorepo keeps the core, apps and companion in step: a protocol change and its UI land in the same pull request.

```
/core            Rust workspace
  mail-core/     sync engine, models, state
  protocols/     imap, jmap, smtp, dav adapters
  crypto/        OpenPGP, keystore glue
  storage/       SQLite schema, migrations, search
  ffi/           UniFFI and wasm-bindgen surface
/companion       Rust server (axum)
/android         Gradle project, Compose app
/web             SvelteKit app
/ios             SwiftUI app (phase 4)
/design          tokens, icons, Figma exports
/test-servers    docker-compose: Stalwart, Dovecot+Postfix, Radicale, GreenMail
/docs            user and self-hosting docs
```

**Testing**

- Rust unit tests and property tests for MIME parsing and sync state machines
- Integration tests run the core against real servers in containers on every pull request
- A recorded corpus of awkward real-world mail (bad encodings, huge threads) as regression fixtures
- Compose screenshot tests and Playwright tests for web; manual test passes on a low-end Android phone each release

**CI and release**

- GitHub Actions (or Forgejo Actions on a self-hosted Forgejo): lint, test, build all targets
- Android ships to Google Play, F-Droid and GitHub Releases as three build flavors: Play (FCM push, no persistent foreground service, no self-updater), F-Droid (UnifiedPush, no Google libraries) and GitHub (as F-Droid, signed by the project); web and companion ship as one Docker image. What each store requires is in `docs/store/READINESS.md`
- Semantic versioning; the core's FFI API is versioned so old apps keep working with newer companions

**Licence and community**

- MPL-2.0 for the core and apps, AGPL-3.0 for the companion and push gateway (see Review decisions)
- CONTRIBUTING.md, code of conduct, issue templates, a public roadmap board, a Matrix or Discord room
- Translations through Weblate from the first public beta
- Funding: Open Collective or GitHub Sponsors, earmarked first for the Google security assessment and an external security review

## Building it with Claude Code

You act as product owner and reviewer; Claude Code writes most of the code. That works when every task is small, specified and checked by tests.

**Desktop setup:** Rust (rustup), Android Studio with the NDK, Docker, Node.js, and Claude Code in the repo folder.

1. **Write CLAUDE.md first.** It holds the rules Claude Code reads every session: the architecture (logic lives in the Rust core, UIs stay thin, protocol code only in `protocols/`), coding standards, how to run tests, and security rules (never log secrets, every query scoped to a user).
2. **One milestone per task.** Turn each roadmap item into an issue with a short spec and acceptance tests. Ask Claude Code to plan first, review the plan, then let it build.
3. **Tests are the guardrail.** Integration tests against the container mail servers must pass before anything merges, and Claude Code runs them itself while working.
4. **Keep sessions focused.** One crate or one screen per session, a git branch per task, small commits.
5. **Review what matters by hand.** Read every change to auth, crypto, multi-user isolation and the FFI surface. For big changes, have a fresh Claude Code session review the diff before you do.
6. **Check the look on a real phone.** Screens are generated from the design tokens; judge motion and spacing on a device, not only in previews.

**First task to hand Claude Code:** set up the monorepo from the layout above, a Rust workspace with a UniFFI function called from a Compose screen, the test-server containers, and CI that builds both.

## Risks and open questions

The biggest risk is scope: v1 as listed is several apps' worth of work, so the roadmap ships usable mail long before calendar and PGP.

| Risk | Impact | Mitigation |
| --- | --- | --- |
| Scope too large for one developer | Never ships | Strict phases, each releasable on its own; small Claude Code tasks with tests |
| AI-written code with subtle security bugs | Credential or mail leaks | Human review of security-critical code, fuzzing, cross-user tests, external review before 1.0 |
| Gmail requires a yearly CASA assessment for a shared OAuth client | Easy Gmail sign-in blocked or costly | App passwords plus bring-your-own OAuth client first; fund the assessment later. Until then the store listings say Gmail needs your own client |
| IMAP servers vary wildly in quirks | Sync bugs, lost flags | Container test matrix, quirks table per server, never delete locally without server confirmation |
| Android kills background connections | Missed notifications without companion | UnifiedPush and FCM via companion; periodic sync otherwise; an opt-in persistent connection only in the F-Droid and GitHub builds |
| iOS has no persistent background connection | No instant push without companion | Companion plus thin push gateway; periodic sync otherwise |
| Push gateway is a shared dependency | Push stops if it goes down | Stateless and cheap to run; encrypted pings only; self-hostable |
| Rust FFI adds build complexity | Slower onboarding for contributors | Template project, prebuilt core artifacts in CI, good docs |
| Multi-user companion holds many people's credentials | High-value target | Per-user envelope encryption, passkeys/OIDC, audit log, security review |
| A store rejects the app, or Android developer verification blocks sideloaded builds | Weeks of delay, or GitHub and F-Droid builds that will not install | Work through the Store readiness checklist before each release; give reviewers a sample mailbox; start the 14-day closed test early; register the package name and signing key for developer verification as soon as the programme opens to the project |

## Review decisions

Your answers to the open questions, turned into decisions.

**Name: Croft (chosen 1 November 2026).** A croft is a small piece of land a family works for itself: your own server, your own mail. Use **Croft** as the app name and **Croft Mail** in store listings, since plain "Croft" searches are crowded by Lara Croft games.

| Check | Result (web search, 1 Nov 2026) | Next step |
| --- | --- | --- |
| Mail apps named Croft | None found | none |
| Google Play / App Store | No app called Croft or Croft Mail found | Search both stores directly before publishing |
| US trademarks | [Croft Technology, Inc.](https://trademarks.justia.com/976/16/n-97616692.html) applied in 2022 for a stylized CROFT mark covering immigration and HR software (class 42) | Different field, so low risk; a trademark lawyer's search before 1.0 is worth it |
| Similar names | Lovecroft, a small open-source mailing-list manager | No conflict |
| Domains and GitHub org | Not checked (needs a registrar lookup) | Try croftmail.org, croft.email, croftmail.app and a `croft-mail` GitHub org |

**Visual direction: stone and sealing wax.** Calm, physical, a little warm, and deliberately unlike the usual AI look (purple-blue gradients, glows, sparkle icons, glassy cards).

| Role | Colour | Use |
| --- | --- | --- |
| Seed (primary) | Moss green #4F6F52 | Generates the Material 3 scheme on web and when dynamic colour is off |
| Accent | Sealing-wax red #B4472E | Compose button, unread dot, send; used sparingly |
| Surfaces | Warm paper #F6F3EE light, slate #1B1D1C dark | Backgrounds instead of pure white and black |

On Android 12+ wallpaper colours still take over by default, with a setting to keep the Croft palette. Icons use Material Symbols Rounded. Web text uses Atkinson Hyperlegible Next, a free font built for readability; Android uses the system font.

**Team: you plus Claude Code.** The plan now assumes one developer directing Claude Code (Opus 5.5) on a desktop. See "Building it with Claude Code" for how to run that.

**Companion: multi-user in v1, secure by design.** One instance serves a household or a company, with admin and user roles and strict per-user isolation. Security requirements are in the Security section.

**Licence: MPL-2.0 for the core and apps, AGPL-3.0 for the companion.** MPL keeps changes to project files open but works with the Apple App Store, which GPL apps have struggled with. AGPL makes anyone who runs a modified companion as a service share their changes.

**Push relay: run a thin push gateway, never a mail-watching service.** The question has three possible answers:

| Option | How it works | What the project's server sees | Verdict |
| --- | --- | --- | --- |
| A. No project server | Only UnifiedPush works; Google (FCM) and Apple (APNs) push are impossible on the store builds | Nothing | Too limiting: no instant push on iPhone or most Play Store phones |
| B. Thin push gateway | Your companion watches your mail, encrypts a tiny "check mail" ping and hands it to the project gateway, which forwards it to Google or Apple | Device tokens and timing, never mail or addresses | **Chosen.** Same model Nextcloud and Matrix apps use |
| C. Hosted mail watcher | A project server stores your mail password and watches your inbox for you | Mail credentials for every user | Rejected: one breach exposes everyone |

Option B is needed because only the app's publisher holds the keys Google and Apple require to send pushes to the official app. A companion cannot reach a phone through FCM or APNs on its own. The gateway is open source and self-hostable for anyone who builds their own copy of the app; UnifiedPush users skip it entirely. People without a companion get periodic sync only.
