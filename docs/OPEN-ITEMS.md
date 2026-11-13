# Open items for the owner

Things only the owner can decide or do. Newest first within each section.

## Urgent
- **Read by hand:** the security-sensitive changes listed in `docs/REVIEW-QUEUE.md` (sign-in, TLS and pinning, SQLCipher key, account isolation, the FFI surface, the patched IMAP library).

- **Maven Central rate-limits this container** (429 Too Many Requests on full Gradle builds). Nothing to allow in
  settings: it is Maven Central's limit on the container's address. If it persists in later sessions, the Android
  build can only be confirmed on GitHub CI or your own machine. See `docs/ENVIRONMENT.md`.

## To check on a phone (T20, not checkable here)
- That the periodic check really runs under Doze and app standby, and how late it is with battery saver on.
- How notifications group, and how the lock screen looks ("N new messages" only), on real phones from several makers.
- That sound and vibration follow the account's channel in system settings.
- Archive, Delete and Mark read from a notification while Croft is closed (the core opens in the background), and Undo within the window.

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

- **Benchmark build type (T14 follow-up):** the `benchmark` build type does not add `src/release/kotlin`, so it probably no longer compiles since the debug-only test hooks moved to `src/debug` with release stubs (T17c). Fix it the same way as `preview` before the macrobenchmark is run again.

## Companion (phase 2)
- **Real passkeys and identity providers are untested (T24, T25).** The Companion's sign-in was tested with a software passkey and a fake OIDC provider only. To try on a real host: passkeys on a phone, a computer and a security key; OIDC against your own Authelia, Authentik, Keycloak or Entra ID (README, "Signing in through your identity provider").
- **An identity provider with a private certificate authority (T25 review):** the Companion trusts the public roots only; a home-lab provider signed by its own CA cannot be used yet. A setting for an extra CA file is a later task if needed.
- **Web compose (T31b):** the Companion does not yet ask the core to copy web drafts to the mail server's Drafts folder
  (`sync_draft_to_server`), and sending through a real SMTP server from the Companion is untested here. Attachments, address
  suggestions and rich text in web compose are still to come.
- **Scheduled send (T33a):** the phone's S25 needs the Companion connection (T34) before it can schedule through it; until then
  scheduling is on the web. The web's Scheduled view offers Cancel only (no "send now" or "change time" yet). Signing a device
  out does not cancel what it scheduled: after a lost device, check Scheduled (a "cancel all scheduled" button may be worth adding).
- **The web app on real browsers (T30):** tested with Chromium and its virtual authenticator only. To try: Safari and Firefox,
  platform passkeys (Touch ID, Windows Hello, a phone) and password managers, and single sign-on with a real provider. The
  Docker base images (`rust`, `node`, `debian`) are not pinned by digest yet.
- **Web settings live in each browser (T32a):** W04's appearance and swipe settings are kept in the browser's storage, not synced
  through the Companion yet (W04 says colours, density and shape should sync across devices). Settings sync is a later task. The
  web About pane uses the same placeholder privacy and source addresses as the phone app (`web/src/lib/settings/links.ts`, see
  "Placeholders from T17c").
- **Push in browsers (T32b):** untested with the real push services (Google's for Chrome, Mozilla's for Firefox, Apple's for
  Safari) and with real notifications (headless Chromium refuses them); try on real browsers and an installed app on a phone.
  A refresh answer lost on a flapping connection still ends the session on the next try (no grace window for a just-rotated
  refresh token yet; a change to the T24 reuse rule that needs its own review).
- **The push gateway needs the owner (T29), before the Play build has push:** a Firebase project for the app (its real application
  id), a service account key for it with permission to send through FCM, and a host for the gateway with a public https address
  (`gateway/README.md`). Real delivery through FCM is untested until then. T34 should register phones with their Companion's VAPID
  key (bound endpoints), and consider `Urgency: high` for new mail (the Companion sends `normal`, which FCM may delay on a sleeping
  phone).
- **Push to a push service on the Companion's own network (T28):** push endpoints reach public addresses only, so a self-hosted ntfy
  that resolves only to a private address (split-horizon DNS, a LAN-only install) gets no pings. If owners need it, add a separate
  `push_allowed_networks` setting (never shared with `mail_allowed_networks`). Real push services (Mozilla, Google and Apple for browsers;
  ntfy and other UnifiedPush distributors) are not tested here.
- **JMAP thread ids are not stable yet (T27):** the Companion names a thread by the lowest email id of the core's conversation view, which treats Trash and Junk differently; the core should expose its stored conversation key so `threadId` never changes (RFC 8621).
