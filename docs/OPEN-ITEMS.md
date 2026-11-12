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
- Android application ID / package name: `com.example.croft` (Google Play refuses `com.example`, and the ID can never change after the first Play upload: pick it before then, usually a reversed domain you own such as `org.yourdomain.croft`) (defined once, in `android/gradle.properties` as `croft.applicationId` once the Android project exists).
- iOS bundle ID: `com.example.croft` (not used yet).
- Publisher name, domain, contact and security-report address: `PUBLISHER_NAME`, `example.org`, `security@example.org`.

## Placeholders from T17c
- **Privacy policy address.** S60 and S01 open `https://example.org/croft/privacy` (`AboutLinks.PRIVACY_POLICY_URL` in `android/app/src/main/kotlin/com/example/croft/ui/accounts/Links.kt`). It is a placeholder: host the policy (`docs/store/PRIVACY-POLICY.draft.md`) at a stable public address and change that one constant before the first store upload.
- **Source link.** S60's "Source code" opens `https://example.org/croft/source` (`AboutLinks.SOURCE_URL`, same file). It is a placeholder: set the real address. It may be the GitHub repository page if you accept that the account name in that address appears in the app (and so in every build); otherwise use an address of your own domain.
- **Licence texts.** S60's licence screen lists each library with its licence name; it does not bundle the full licence texts. Check with the licence review whether the stores' and the licences' notice rules need them in the app.

## Left out of S45 for T18 and later
S45 (Account) has no entry yet for these, because the core has no storage for them (no disabled placeholders, Apple 2.1):
- **Identities and signatures** (T18, compose): the core's `identity` table exists, with no FFI to read or write it.
- **Sync window** (how far back to sync): no core setting yet.
- **Server settings** (opens S05 pre-filled): the core cannot read an account's server settings back, and cannot change them (needs a core call to read and one to update a stored account; the S05 screen exists).
- The account name and colour live in the app (`croft-accounts.json`); the Inbox, the drawer and the account switcher still show the core's label and the avatar colours they have; use `AccountPrefs.nameOf` and `colourOf` there when they are revisited.

## Accounts, money, legal (not for Claude to do)
- **Launch plan (owner, 11 Nov 2026):** soft launch on F-Droid first, then the official launch on Google Play and the Apple App Store. No store accounts yet; they are needed before the official launch (see `docs/ROADMAP.md` "Store lead times": the Play closed test needs 12 testers for 14 days, and Google and Apple account checks take days to weeks). F-Droid needs no paid account, but submitting means a merge request to F-Droid's data repository on GitLab (a free GitLab account, the owner's), and a real application ID instead of `com.example.croft`.
- Trademark: a preliminary web search (not legal advice) found no email product named Croft; the closest is Croft Technology, Inc. with live US marks in class 42 (not email). See `docs/store/TRADEMARK-PRELIM.md`. A lawyer's clearance search is still advised before the official launch.
- Google Play and Apple developer accounts. Signing keys.
- The preview build (`:app:assemblePreview`, CI artifact `croft-preview-apk`, `docs/TESTING-ON-A-PHONE.md`) is signed with the Android debug key and has the id suffix `.preview`. It is for testing only and cannot be updated to a store build. The real release still needs the owner's own signing key, kept outside the repository (`android/README.md`, "Release build").
- Items marked **Confirm** in `docs/store/READINESS.md` (export compliance, Gmail device-only assessment, foreground service, closed-test rules).
- Legal review of `docs/store/PRIVACY-POLICY.draft.md`.

## Network
- During this session the owner allowed `dl.google.com`, `maven.google.com`, `www.mozilla.org`, `www.gnu.org`, `docs.rs` and `downloads.gradle.org` on the Default environment.
