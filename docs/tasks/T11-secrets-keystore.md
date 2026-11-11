# T11: Secrets and the Android Keystore

Crate / folder: `core/ffi` (callback interface) + `android/` (keystore code)

Depends on: T01 (`Db::open` with a key, feature `sqlcipher`), T10 (the `Core.open` signature). Blocks: T17 (sign-in), T21 (OAuth tokens).

## Read first
`CLAUDE.md` (security rules), `docs/PLAN.md` (Security section: credentials, local data), `docs/DECISIONS.md` (key is exactly 32 raw bytes; backups are off until this task lands), `android/README.md` (backup rules and `BackupRulesTest`), `docs/store/READINESS.md` (permissions table: no new permission is allowed here), `docs/tasks/T01-sqlite-layer.md`.

## Goal
Passwords, OAuth tokens and the database key live only in the Android Keystore. They never reach SQLite, config files, logs or crash reports, and the Rust core never stores a secret.

## Spec
- **Database key.** On first run generate 32 random bytes, wrap them with a non-exportable AES-256-GCM key in the Android Keystore (`AndroidKeyStore`, StrongBox when available, else TEE), and keep only the wrapped blob in app-private storage. At start the app unwraps it and passes it to `Core.open`. Build the Android library with the `sqlcipher` feature (`bundled-sqlcipher`); the implementer checks that it cross-compiles with cargo-ndk for arm64-v8a and x86_64 and stays 16 KB aligned (`android/tools/check-16kb-alignment.sh`). If it does not, report it as blocked; do not ship an unencrypted database.
- **Account secrets.** A `SecretStore` Kotlin class keeps one entry per `(account_id, kind)` (`password`, `oauth_refresh`, `oauth_access`) in the same wrapped form. Removing an account deletes its entries.
- **Credential callback.** In `core/ffi` define a UniFFI callback interface `CredentialProvider { fn credential(account_id) -> Option<Credential> }` where `Credential` is `Password{user, secret}` or `OAuthToken{access, expires_at}`; also `credential_rejected(account_id)` so the app can mark the account Needs sign-in. The engine calls it per connection attempt (never from the UI path), holds the value in memory only for that connection, and zeroizes it after. The Kotlin implementation reads `SecretStore`. A missing credential becomes `BackendError::Auth`.
- **Never logged.** Secret types implement `Debug` as `[redacted]`; no `Display`. Add a test that formats every secret-bearing type and asserts the fixture secret is absent.
- **Backups.** Keep `allowBackup="false"` and the extraction rules; `BackupRulesTest` must still pass. Record in `docs/DECISIONS.md` whether the encrypted database could be allowed to back up later (it cannot: the key is device-bound).
- **Key loss.** If the keystore key is gone (restore, lock-screen reset) the wrapped blob cannot be opened: the app deletes the database and secrets, and starts as new. Mail is re-synced from the server. This path shows plain text on S01, not a crash.

## Done when
- Rust: `Db::open` with the key opens an encrypted file; with a wrong key returns the existing wrong-key error and leaves the file unchanged; a plain-SQLite reader cannot read it (`cargo test --features sqlcipher`).
- Robolectric: `SecretStore` round trip, delete, and "unknown account returns null". Robolectric's fake `AndroidKeyStore` support is limited, so the implementer confirms what it can do. Where it cannot, put the cipher behind a small `SecretBox` interface, test the envelope format and failure paths with a JVM AES-GCM fake, and test the real keystore class only for its wiring.
- JVM test (host library, T10 surface): a `CredentialProvider` fake hands a password to the engine and a second one returns nothing; the account goes to Needs sign-in and no attempt is counted as Transient.
- Grep-style test: no secret type is `Serialize`, and no table or column in `core/storage/migrations/` is named for a password or token.
- On a device (not checkable here): StrongBox availability, key survival across a reboot and across a screen-lock change, behaviour after "clear data". Reported as unverified.
