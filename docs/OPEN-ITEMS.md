# Open items for the owner

Things only the owner can decide or do. Newest first within each section.

## Urgent
- **No git remote.** The project arrived as a zip, so this container has nowhere to push. Work is
  committed locally on branch `claude/croft-build` and is lost if the container is reclaimed. Create an
  empty GitHub repository and attach it to the next session (or give its name) so every task can be pushed.

## Placeholders to replace
- Android application ID / package name: `com.example.croft` (defined once, in `android/gradle.properties` as `croft.applicationId` once the Android project exists).
- iOS bundle ID: `com.example.croft` (not used yet).
- Publisher name, domain, contact and security-report address: `PUBLISHER_NAME`, `example.org`, `security@example.org`.

## Accounts, money, legal (not for Claude to do)
- Trademark search for "Croft". Google Play and Apple developer accounts. Signing keys.
- Items marked **Confirm** in `docs/store/READINESS.md` (export compliance, Gmail device-only assessment, foreground service, closed-test rules).
- Legal review of `docs/store/PRIVACY-POLICY.draft.md`.

## Network
- During this session the owner allowed `dl.google.com`, `maven.google.com`, `www.mozilla.org`, `www.gnu.org` and `docs.rs` on the Default environment. Nothing else is blocked so far.
