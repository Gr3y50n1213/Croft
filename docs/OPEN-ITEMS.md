# Open items for the owner

Things only the owner can decide or do. Newest first within each section.

## Urgent
- **No git remote.** The project arrived as a zip, so this container has nowhere to push. Work is
  committed locally on branch `claude/croft-build` and is lost if the container is reclaimed. Create an
  empty GitHub repository and attach it to the next session (or give its name) so every task can be pushed.

- **Read by hand:** the security-sensitive changes listed in `docs/REVIEW-QUEUE.md` (sign-in, TLS and pinning, SQLCipher key, account isolation, the FFI surface, the patched IMAP library).

- **Maven Central rate-limits this container** (429 Too Many Requests on full Gradle builds). Nothing to allow in
  settings: it is Maven Central's limit on the container's address. If it persists in later sessions, the Android
  build can only be confirmed on GitHub CI or your own machine. See `docs/ENVIRONMENT.md`.

## Placeholders to replace
- Android application ID / package name: `com.example.croft` (defined once, in `android/gradle.properties` as `croft.applicationId` once the Android project exists).
- iOS bundle ID: `com.example.croft` (not used yet).
- Publisher name, domain, contact and security-report address: `PUBLISHER_NAME`, `example.org`, `security@example.org`.

## Accounts, money, legal (not for Claude to do)
- **Launch plan (owner, 11 Nov 2026):** soft launch on F-Droid first; no store accounts yet. F-Droid needs no paid account, but submitting means a merge request to F-Droid's data repository on GitLab (a free GitLab account, the owner's), and a real application ID instead of `com.example.croft`.
- Trademark search for "Croft". Google Play and Apple developer accounts. Signing keys.
- Items marked **Confirm** in `docs/store/READINESS.md` (export compliance, Gmail device-only assessment, foreground service, closed-test rules).
- Legal review of `docs/store/PRIVACY-POLICY.draft.md`.

## Network
- During this session the owner allowed `dl.google.com`, `maven.google.com`, `www.mozilla.org`, `www.gnu.org`, `docs.rs` and `downloads.gradle.org` on the Default environment.
