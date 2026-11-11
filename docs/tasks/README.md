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

After T08 the UI work follows `docs/ux/build-order.md` steps 2 to 12.
