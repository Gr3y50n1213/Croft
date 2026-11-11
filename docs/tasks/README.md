# Task specs for Claude Code

One task per session, one branch per task, plan first. Do them in this order unless told otherwise.
Check `CLAUDE.md` rules before starting and read the linked UX docs for any screen you touch.

| Task | What | Where |
|---|---|---|
| [T01](T01-sqlite-layer.md) | SQLite connection layer | core/storage |
| [T02](T02-persist-queue.md) | Persist the operation queue | core/mail-core + core/storage |
| [T03](T03-imap-adapter.md) | IMAP adapter | core/protocols |
| [T04](T04-sync-engine.md) | Sync engine | core/engine |
| [T05](T05-account-discovery.md) | Account discovery | core/protocols |
| [T06](T06-smtp-outbox.md) | SMTP send and Outbox | core/protocols + core/mail-core |
| [T07](T07-uniffi-hello.md) | UniFFI surface and first Compose screen | core/ffi + android |
| [T08](T08-design-tokens.md) | Design token generator | design + android |
| [T09](T09-jmap-adapter.md) | JMAP adapter | core/protocols |
| [T10](T10-ffi-mail-surface.md) | FFI mail surface (proposal; the orchestrator finalises it) | core/ffi |
| [T11](T11-secrets-keystore.md) | Secrets and Android Keystore, database key | core/ffi + android |
| [T12](T12-theme-engine-appearance.md) | Theme engine and Appearance screens (build-order step 2) | android |
| [T13](T13-component-gallery.md) | Component gallery and `Haptics` (step 3) | android |
| [T14](T14-message-list.md) | Message list on fake data (step 4) | android |
| [T15](T15-swipe-engine.md) | Swipe engine, long-press menus, S47 and S48 (step 5) | android |
| [T16](T16-queue-wiring.md) | List and actions wired to the real core (step 6) | android + core/ffi |
| [T16b](T16b-release-build.md) | Release build, F-Droid metadata, manifest allowlist, 16 KB check | android + CI |
| [T17](T17-onboarding-accounts.md) | Onboarding, accounts, sample mailbox (step 7) | android + core/protocols |
| [T18](T18-conversation-compose-outbox.md) | Conversation, compose, Outbox (step 8, part 1) | core + android |
| [T19](T19-search.md) | Search, local then server (step 8, part 2) | core + android |
| [T20](T20-background-sync-notifications.md) | Periodic sync and notifications | android + core |
| [T21](T21-byo-oauth.md) | Bring-your-own OAuth for Gmail and Outlook | core/protocols + android |

T09 to T21 cover the rest of roadmap phase 1 (Android mail MVP) and end at its gate, the daily-driver alpha. T12 to T19 follow `docs/ux/build-order.md` steps 2 to 8. Order: T09 and T12 can start as soon as T08 is done; T10 and T11 gate everything that talks to the core; T13 to T15 need only fake data and can run beside T10 to T11. Every UI task ends with a line "On a device (not checkable here)": those items are reported as unverified, never claimed.

After T21, phase 2 (Companion, push gateway, web app) follows; it has no task files yet. Build-order steps 9 to 13 (calendar, contacts, security settings, web, tablet, store readiness) also wait.
