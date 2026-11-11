# T01: SQLite connection layer

Crate / folder: `core/storage`

## Goal
Add the database connection on top of the schema that already exists.

## Spec
- Add `rusqlite` (feature `bundled`). Put SQLCipher behind a cargo feature `sqlcipher` (`bundled-sqlcipher`); `Db::open(path, key: Option<&[u8]>)` issues `PRAGMA key` only with that feature.
- Enable `PRAGMA foreign_keys = ON`, `journal_mode = WAL`.
- On open: read `PRAGMA user_version`, call `croft_storage::pending(version)`, run each migration in its own transaction, then set `user_version`. A database newer than the app returns `SchemaTooNew` and is never touched.
- Repositories (plain functions or small structs, every query takes an `account_id`): accounts, folders, messages (list by folder newest first excluding snoozed, upsert from `RemoteHeader`, update flags, delete), settings (JSON by key).
- Port every check in `tools/check_schema.py` to Rust tests against an in-memory database.

## Done when
- `cargo test` passes, including the ported schema checks.
- A test opens a database at version 0, migrates it, reopens it, and gets no second migration.
- A test proves `SchemaTooNew` is returned for `user_version = latest + 1` and the file is unchanged.
- No query in the crate lacks an `account_id` filter except on `account` itself and `setting`.
