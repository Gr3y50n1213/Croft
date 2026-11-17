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
- Snooze (T36): how late the wake-up job runs under Doze (WorkManager, no exact alarm), and that "Back in your inbox" appears once.

## Placeholders to replace
- Android application ID / package name: `com.example.croft` (Google Play refuses `com.example`, and the ID can never change after the first Play upload: pick it before then, usually a reversed domain you own such as `org.yourdomain.croft`) (defined once, in `android/gradle.properties` as `croft.applicationId` once the Android project exists).
- The Companion's list of the app's sign-in redirects, `APP_SCHEMES` in `companion/src/app_signin.rs` (`com.example.croft` and
  `com.example.croft.preview`): change it together with the application ID (T34a).
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
- The preview build (`:app:assemblePlayPreview`, the Play flavour, CI artifact `croft-preview-apk`, `docs/TESTING-ON-A-PHONE.md`) is signed with the Android debug key and has the id suffix `.preview`. It is for testing only and cannot be updated to a store build. The real release still needs the owner's own signing key, kept outside the repository (`android/README.md`, "Release build").
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
- **Phones and the Companion (T34b):** connecting was tested against a real Companion on this machine (the core's client) and with
  unit tests (the app's state), not on a phone: try a Custom Tab sign-in with a passkey and with single sign-on, and the way back to
  the app. A Companion behind a private certificate authority or a self-signed certificate cannot be connected yet (the phone trusts
  the public roots only); an option to trust it, as for mail servers, is a later task. If Android ends the app while the browser
  is in front (common on phones short of memory), the sign-in waiting in the core is lost and the way back does nothing: connect
  again (a message for this is a later task). S58 has Disconnect but not yet "Delete my Companion data" and "Unregister this
  device" (READINESS row 7): with push, in T34d. The FFI hands the secret across in buffers the core cannot wipe (Kotlin wipes
  its copies).
- **Accepted in T34e:** the Companion trusts the provider to honour `prompt=login` and `max_age=0` (it does not check the ID
  token's `auth_time`). Deleting needs the browser on a phone (S58 opens the Companion's page).
- **The deletion page (T34e):** `docs/store/DELETING-YOUR-DATA.draft.md` is the page the store listings give for deleting data
  (Google Play asks for a "Delete account URL"). Host it beside the privacy policy and put its address in the policy. Each
  Companion's own deletion page is `<its address>/delete`.
- **Push on phones (T34d):** untested on a phone. Try UnifiedPush (fdroid or github build with ntfy) and, once the publisher has a
  Firebase project and a gateway (below), the Play build. Until it is tried the periodic check stays as the person set it; with
  push working it could run less often. Accepted for now:
  - a ping that comes while a check runs is dropped (KEEP), and the mail waits for the next change or the periodic check;
  - a replayed ping can cause back-to-back checks (the gateway caps FCM at 60 an hour per device; UnifiedPush has no cap);
  - an erase that races a ping in flight has the same window as the periodic check;
  - the Firebase Installations id stays on the phone after Unregister;
  - S58 does not name the UnifiedPush distributor in use.
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

## Smart inbox (phase 3)
- **Snooze is on this phone only (T36).** Other devices keep showing a snoozed message. Snooze that follows the person across devices
  needs the Companion (a `Croft/Snoozed` folder it moves mail to and back at the wake time, as `docs/ux/server-contract.md` describes);
  a later task. The web app keeps Snooze hidden until then (the rule in `docs/ux/controls.md`).
- **A woken message stays at its date's place (T36).** The list is ordered by the message's date, so a message back from snooze is
  unread with a Snoozed chip and a notification, but not at the top. Putting it at the top needs a separate sort key in the list query.
- **Pin and follow-up keyword names (T36b).** `$Pinned` and `$FollowUp` were chosen without checking the IANA "IMAP and JMAP
  Keywords" registry (not reachable from this container). If the registry, or a recent RFC on keywords, names others for the same
  meaning, switch to them (reading both for a while).
- **Follow-up reminders (T36b).** A follow-up is a mark only: no "remind me at" time and no "if nobody replies" check yet.
- **Pin and Follow up on the web and as swipe actions (T43).**
- **Unsubscribe and forged senders (T37).** The core does not verify DKIM. S31 shows everything that will happen and never deletes
  the account's own or sent mail, but a message forging a contact's address could still offer to delete that contact's older mail
  (shown with the address). Reading the user's own server's `Authentication-Results` (DKIM or DMARC pass) and requiring the From
  domain to match would close this; it needs the header stored at fetch.
- **Unsubscribe email identity (T37).** The email goes from the account's default identity, not the alias the list mail was sent to.
- **Contacts on another host (T38a).** CardDAV is only looked for on the account's mail host. A setting for a CardDAV address
  elsewhere (Nextcloud beside a separate mail server) is needed, with care: the mail password would go there too.
- **Contacts for Google and Microsoft accounts, JMAP Contacts (RFC 9610), groups and photos (T38a).** Not synced yet.
- **Favourite contacts and photos on S41 and S42 (T38b).** The wireframe's star and "Add photo" wait for the core to store them.
  S41's Invite waits for the calendar (T39).
- **Calendar repeats Croft does not expand (T39a).** `BYHOUR`, `BYMINUTE`, `BYSECOND`, `BYWEEKNO`, `BYYEARDAY` and
  `HOURLY` or finer rules show only their first occurrence. `VTIMEZONE`s with irregular rules (not a yearly `BYDAY` in
  `BYMONTH`) fall back to their standard offset. JMAP Calendars, tasks (`VTODO`) and attachments are not synced.
  Reminders more than five weeks before their event are not posted; only the first 1,000 `EXDATE`s of an event are read;
  an object that cannot be read is fetched again at every sync. A change that loses to the server's is dropped (marked
  on the item), not kept as a copy.
- **OpenPGP in compose and reading (T41c).**
  - The subject is not protected (no "protected headers"): it travels in the clear.
  - Encrypted mail with Bcc is refused rather than sent as a separate copy to each Bcc recipient.
  - Inline PGP (`-----BEGIN PGP MESSAGE-----` in a plain text body) and Autocrypt gossip headers are not read.
  - Encrypted mail cannot be searched (nothing decrypted is indexed), and its snippet is the outer part's.
  - An encrypted draft copied to the server opens again only in Croft on a device with the same key.
  - The outer headers (subject, addresses, date) are not covered by any signature: the screens must not present them as
    signed.
  - Decrypting a large message holds it in memory (up to 50 MiB) while it is shown.
- **OpenPGP keys (T41b).**
  - The outgoing Autocrypt header never says `prefer-encrypt=mutual` yet: the setting comes with the screens (T42).
  - A WKD lookup is made only when asked (`lookup_key`); compose (T41c) and the screens (T42) decide when to ask.
  - Keys found in a WKD or imported are not refreshed or checked again for revocation; a revoked key is seen only when a new
    copy arrives. Certificates are not merged: a newer copy replaces the stored one (unless the stored one is revoked), so
    a new subkey or expiry arrives only with a full copy.
  - Secret key bytes passed to and from the app's `KeyVault` cross uniffi in a plain buffer that uniffi frees without
    wiping (one copy each way). Rust wipes its own copies; a wiping buffer type would close this.
- **OpenPGP (T41a).**
  - rPGP uses the `rsa` crate 0.9, which has a known timing side channel in RSA decryption (RUSTSEC-2023-0071, "Marvin";
    no fixed release yet). It matters only for decrypting with an RSA key, needs many timed decryptions, and keys Croft
    makes are not RSA. Watch for `rsa` 0.10 and update rPGP when it moves.
  - Croft makes v4 keys and sends SEIPDv1; v6 keys and SEIPDv2 (RFC 9580) wait until common readers take them.
  - No dependency audit (`cargo audit` or `cargo deny`) runs in CI yet; it would have flagged the item above.
  - From the review, left for later: key flags and expiry in a direct-key signature are not read when a user ID binding
    exists (it matters for v6 keys and some Sequoia keys); a restored backup is not checked for the "key overwriting"
    attack (v4 CFB protection does not authenticate the public part); rPGP's own buffer for SEIPDv1 (up to 50 MiB) is not
    wiped; IDEA, CAST5, 3DES and Blowfish are still read when decrypting old mail.
  - For T41b: an Autocrypt key may name other addresses than its `addr`: file it under `addr` only. A signed message
    someone re-encrypted to the person shows a good signature: the screen must say "signed by", not "encrypted by".
- **Invitations (T40a).**
  - Croft mails its own reply to every invitation it answers. For an event a CalDAV server placed in the calendar by
    itself (server-side scheduling), that server may send a reply too: the organiser then gets the same answer twice.
    Telling reliably whether a server schedules would close this.
  - Replies to the person's own invitations (`METHOD:REPLY`) are shown ("Sam accepted") but not written into the stored
    event's attendee list. `COUNTER`, `REFRESH` and `ADD` are not handled.
  - A `.ics` file opened from outside the app (docs/ux/navigation.md) does not open the event with answer buttons yet.
  - Forged invitations: as for unsubscribe, the core does not check DKIM. The card says when the organiser is not the
    sender, and a cancellation only ever removes an event of the same organiser.
- **CardDAV against Radicale (T38a).** Only Stalwart's CardDAV is in the CI integration job; Radicale (in `test-servers/`) needs a
  config and an address book created before it can be tested.
