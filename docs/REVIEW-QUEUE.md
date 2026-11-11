# Review queue

Security-sensitive changes for the owner to read by hand (sign-in, encryption, account isolation, the
FFI surface, secrets). Each was first reviewed by a separate agent that did not write it. Newest first.

| Task | Files | What to look at | Agent review |
|---|---|---|---|
| T01 | `core/storage/src/db.rs`, `core/storage/src/error.rs`, `core/storage/src/folders.rs`, `core/storage/src/messages.rs` | How the SQLCipher key is applied (`apply_key`, `validate_key`); the open sequence (read-only version check, no checkpoint on a refused open, hot-journal recovery); that every folder and message query filters `account_id`; that errors print only categories. Known and accepted: `StorageError::Sqlite` is public, so a caller could still print rusqlite's raw message; nothing does today. Later hardening: a composite foreign key so a message's `account_id` must match its folder's | Opus, 2 rounds. Round 1: 2 medium (refused open changed the file with a `-wal`; migration race), 5 low, all fixed. Round 2: 3 low (refusal on the read-write connection, `file:` URIs, hot journal), all fixed |
