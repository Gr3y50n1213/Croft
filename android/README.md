Jetpack Compose app, Material 3 Expressive. It currently has one screen (`MainActivity`) that shows the version of the Rust core, proving Rust → UniFFI → Kotlin → Compose. Build the real screens from `docs/ux/` and the wireframes (S01 to S60) in `design/wireframes/`.

## Build
Needs: JDK 17+ (CI and the first setup use 21), the Android SDK (`ANDROID_HOME`) with `platforms;android-37.0`
(compileSdk 37: Material 3 1.5 and Compose 1.12 require it; targetSdk stays 36), `build-tools;36.0.0` and `ndk;29.0.14206865`, Rust with the targets
`aarch64-linux-android` and `x86_64-linux-android`, `perl` and `make` (OpenSSL's crypto is compiled from source, see below),
and `cargo install cargo-ndk --version 4.1.2 --locked`.

```
cd android
./gradlew :app:assembleDebug :app:testDebugUnitTest
tools/check-16kb-alignment.sh app/build/generated/rust/jniLibs   # LOAD segments must be 0x4000 aligned
```

The unit tests include Robolectric tests, which download Android jars from Maven Central on first run. Behind a
proxy or mirror, set `ROBOLECTRIC_REPO_URL` to the Maven repository to use.

What Gradle does (see `app/build.gradle.kts`):
- `buildRustAndroid`: `cargo ndk` builds `libcroft_ffi.so` (release) for `arm64-v8a` and `x86_64` into `app/build/generated/rust/jniLibs`.
- `buildRustHost`: `cargo build -p croft-ffi` for this computer; the JVM unit tests load it through JNA.
- Both Rust builds use the `croft-ffi` feature `sqlcipher-vendored-openssl` (`rustFeatures` in `app/build.gradle.kts`), so the
  database is encrypted in every build the app runs and the JVM tests open a real encrypted file. See "Rust features" below.
- `generateUniffiBindings`: `uniffi-bindgen` (same UniFFI version as the library, `core/ffi`) writes the Kotlin bindings to `app/build/generated/uniffi/kotlin`, package `uniffi.croft_ffi`.
- `design/generated/kotlin/CroftTokens.kt` is compiled in place (never copied) and drives `CroftTheme`.

The application id is `croft.applicationId` in `gradle.properties` (placeholder `com.example.croft`).
The FFI API version the app expects is `EXPECTED_FFI_API_VERSION` in `RustBridge.kt`; it must match
`FFI_API_VERSION` in `core/ffi/src/lib.rs`.

## Rust features
`croft-ffi` (`core/ffi/Cargo.toml`) has three features besides `cli` (the bindings generator):
- `sqlcipher`: the database file is encrypted with SQLCipher, linked against the computer's OpenSSL (`libssl-dev`). Used by
  `cargo test -p croft-ffi --features sqlcipher` (the `sqlcipher` job in CI). Without a SQLCipher feature, `Core.open` refuses
  a database key (`StorageReason::EncryptionUnavailable`) instead of storing mail unencrypted.
- `sqlcipher-vendored-openssl`: `sqlcipher` plus OpenSSL's crypto compiled from source (the `openssl-src` crate). This is what
  Gradle builds for Android (the NDK has no OpenSSL to link) and for the host library, so nothing depends on the build
  computer's OpenSSL. **It needs `perl` and `make` on the build computer** (OpenSSL's `Configure` is a Perl script) and adds
  several minutes to a cold build of each ABI. The licences of what it compiles in are in `LICENSES.md`.
- `cli`: builds the `uniffi-bindgen` binary.

## Secrets (T11)
`com.example.croft.secrets` keeps the database key and the account passwords and tokens wrapped by one Android Keystore key,
in `noBackupFilesDir/croft-secrets`. Use `createAppSecrets(context, dbFile)` (one instance per process; it takes a file lock around
key creation) and `openCore(...)`. When the Keystore key is definitely gone, the database and the wrapped secrets are moved to
`noBackupFilesDir/croft-quarantine/<timestamp>/` (not deleted) and deleted after 7 days and 3 later successful starts. A Keystore
that merely fails is never treated as a loss: `openCore` throws `SecureStorageException` ("Secure storage is not ready; try again",
or "This data was written by a newer Croft") and changes nothing. Add and remove accounts through `AccountSecrets`, which keeps the
order that is safe after a crash.

## Backup and device transfer
The mail database must never be copied off the phone. The manifest sets `android:allowBackup="false"`,
`android:dataExtractionRules="@xml/data_extraction_rules"` (Android 12+: `allowBackup="false"` alone does not stop
device-to-device transfer) and `android:fullBackupContent="@xml/backup_rules"` (Android 11 and older). Both files are in
`app/src/main/res/xml/` and exclude every domain (`root`, `file`, `database`, `sharedpref`, `external`) from cloud backup
and device transfer. `BackupRulesTest` checks the manifest attributes and the rules. Keep it that way when adding storage.

## Supply chain
- Plugins resolve from `google()` and `mavenCentral()` only (no Gradle plugin portal).
- The Gradle wrapper is pinned: `distributionSha256Sum` is in `gradle/wrapper/gradle-wrapper.properties`. To update it,
  run `gradle wrapper --gradle-version X --gradle-distribution-sha256-sum <official sha256>` with that Gradle version and
  compare the checksum with `https://services.gradle.org/distributions/gradle-X-bin.zip.sha256`.
- Gradle calls cargo, cargo-ndk and the bindings generator with `--locked`, so `core/Cargo.lock` is used as committed.

## Components, gallery and haptics (T13)
- Controls live in `app/src/main/kotlin/com/example/croft/ui/components/` (buttons, extended FAB, Send, chips, text fields,
  switch, radio and checkbox rows, message row, attachment chip, banner, snackbar, destructive dialog, bottom sheet, menu,
  icon button). Every control takes an `interactionSource`, which is how the gallery shows pressed, focused and hovered
  controls next to each other, and how the 3 dp focus ring (2 dp gap) follows keyboard focus.
- The component gallery is in `app/src/debug/` and is not part of a release build (`src/release/` has a stub with the same
  API). In a debug build open Settings, then Component gallery. A row of chips at the top overrides the animation level for the
  gallery alone; a line at the bottom shows the haptic events the controls asked for.
- Haptics: `com.example.croft.haptics.Haptics` (semantic events). `CroftTheme` provides the real one: `AndroidHaptics`
  (`HapticFeedbackConstantsCompat`) behind `LevelFilteredHaptics` (level from S53, 40 ms guard). Compose's own platform haptic
  feedback is switched off in `CroftTheme`, so nothing vibrates around the interface. Tests use `RecordingHaptics`
  (`app/src/test/.../haptics/`).
- Animation level Off: `MaterialTheme.motionScheme` is `OffMotionScheme` (every spring a jump), so Material's own components are
  instant too. Ripples and state layers are press feedback and keep the platform's behaviour.
- Navigation: `ui/nav/CroftNavHost` (Home, which is the Inbox since T14, Settings, Appearance, gallery in debug builds). Back is
  `BackHandler`, the system back-callback API.
- Debug-only source sets: `src/testDebug/` holds the tests that need the gallery.

## The real core behind the Inbox (T16)
- `MainActivity` opens the core **off the main thread** (`mail/MailRuntime`: `createAppSecrets`, `openCore`, `SecretStoreCredentials`, T11)
  and builds the `MailSession` over it. With no account in the core the app still shows the fake sample mailbox (onboarding is T17); the
  open core is kept in `MailRuntime.core` for T17.
- `mail/CoreMessageRepository` implements `MessageRepository` over `uniffi.croft_ffi.Core` (FFI API version 6); `CoreRowActions` implements
  `RowActions`; `FolderMapping` remembers, per account, the folder chosen for Archive or Trash when the server has none (asked once) and the
  recent Move to folders. S30 is `ui/mail/MoveToSheet`; S49 (Undo time) is `ui/mail/InboxSettingsScreen` with `mail/MailSettings`
  (`croft-mail.json`); the Undo time is handed to the core (`set_undo_window_ms`) at start and at every change.
- Connectivity: `mail/ConnectivityMonitor` (`registerDefaultNetworkCallback`) calls `set_network_available`. The manifest now declares
  `INTERNET` and `ACCESS_NETWORK_STATE` (and `tools/manifest-allowlist.json` allows them).
- **Tests against a real server, as plain JVM tests: the module `:mailtests`** (`./gradlew :mailtests:test`, also run by
  `:app:testDebugUnitTest`). It compiles the app's `mail/` data layer in place (not the four files that need Android: `MailSession`, `MailSettings`, `ConnectivityMonitor`, `MailRuntime`) with Kotlin bindings
  generated from the **host** library, which is built with the `test-server` feature of `croft-ffi` and so has `TestImapServer`. The
  app's own bindings are generated from the **Android** library (`generateUniffiBindings`), so the app cannot call, or even know,
  the test object; `tools/check-release-outputs.sh` checks that the release `.so` files have no test-server symbols. Why a module and
  not `app/src/test`: it runs without Robolectric (Maven Central's rate limit blocked that download in the build container).
- Screen tests of the Inbox over the real repository with a scripted core are in `app/src/test/.../ui/mail/` (`CoreStateKit`, `CoreStatesTest`,
  `CoreScreenshotTest`, `MoveToTest`).

## Message list on fake data (T14)
- Screens S10 to S19 are `ui/mail/` (`InboxScreen`; the state and logic are in `InboxController`, which only talks to a
  `MessageRepository`). Settings is reached from the drawer.
- `mail/MessageRepository` is the interface the screens use, shaped like the T10 surface (`list_messages` with `RowCursor`
  keyset paging, `next_change`, the actions). `mail/FakeMessageRepository` implements it in memory (5,000 rows by default in the
  app, any number in tests; `injectNewMail`, `setOffline`, `setAccountStatus`, `holdQueries` for tests and the debug app). T16
  swaps in the core by building a `MailSession` with its own repository (`MainActivity`); nothing in `ui/mail/` changes. The fields
  the T10 row does not have yet are listed in the KDoc of `MessageRowData` and `AccountInfo`.
- The place in a list is a `ListAnchor` (message id, date, dp offset), saved by `rememberSaveable` and, per folder, in
  `noBackupFilesDir/croft-list-anchors.json` (`FileAnchorStore`). Never a pixel position.
- Tests: `app/src/test/.../ui/mail/` (`InboxScreenTest`, `InboxRestorationTest`, `ListMotionTest`, `ListPartsTest`, screenshots
  `InboxScreenshotTest` for S10 to S19) and `.../mail/` (the fake repository). Test names start with the screen ID.
- Macrobenchmark (device only): the `:macrobenchmark` module and the app's `benchmark` build type (release-like, debug-signed,
  profileable). With a device attached: `./gradlew :macrobenchmark:connectedBenchmarkAndroidTest`. It scrolls the 5,000-row Inbox
  and records frame timing; there is no result yet.

## Release build (T16b)
- `./gradlew :app:assembleRelease :app:bundleRelease` makes `app-release-unsigned.apk` and `app-release.aab`: R8 and resource
  shrinking on, not debuggable, no signing config, and no dependency-info block (`dependenciesInfo` off; F-Droid rejects it).
- No signing keys are in the repository. The owner signs afterwards, with a key kept outside it: for the APK, `zipalign -P 16 -p 4`
  is already satisfied, then `apksigner sign --ks <keystore> app-release-unsigned.apk`; for the AAB, `jarsigner -keystore <keystore>
  app-release.aab <alias>` (Play re-signs with its own key when Play App Signing is on). F-Droid builds and signs from source.
- Keep rules: `app/proguard-rules.pro`, one reason per rule. `checkReleaseKeepRules` fails the build before R8 if the UniFFI
  (`uniffi.croft_ffi.**`) and JNA (`com.sun.jna.**`) rules are missing; `checkReleaseMapping` fails after R8 if its `mapping.txt`
  shows either package renamed. This does not replace a device run: start the release build, sign in and sync by hand before any upload.
- `tools/check-release-outputs.sh [bundletool.jar]` (after the build) checks unsigned, not debuggable, no dependency blob, the
  manifest allowlist, 16 KB alignment (`check-16kb-alignment.sh`, `zipalign -c -P 16`, and the AAB's `PAGE_ALIGNMENT_16K` with bundletool).
- Manifest allowlist: `tools/manifest-allowlist.json` (permissions and exported components, each with a reason). A new permission or
  exported component fails `tools/check-release-manifest.py` until it is added there with its justification; tests are in
  `tools/test_check_release_manifest.py` (`python3 -m unittest discover -s tools`). CI job: `android-release`.
- The application ID is defined once, in `gradle.properties` (`croft.applicationId`); the Kotlin `package` lines and
  `design/generated/kotlin/CroftTokens.kt` carry the same name and move in one commit when it changes.

## Open-source licence list (S60)
The list S60 shows is the checked-in `app/src/main/assets/licenses/open-source-licences.json`. After any change to `core/Cargo.lock` or
`gradle/libs.versions.toml`, regenerate it from the repository root with `python3 android/tools/gen_licences.py` (needs `cargo` and a warm
Gradle cache; it runs `./gradlew --offline`). `python3 android/tools/gen_licences.py --check` and `tools/test_gen_licences.py` (CI job
`android-release`) fail when the file no longer matches those two inputs.

## Swipe engine (T15)
`:swipe` is a plain Kotlin JVM module (no Android or Compose types): the swipe state machine, the settings types and the pill
geometry, with property tests (kotest-property) that run as ordinary JVM tests: `./gradlew :swipe:test` (also a dependency of
`:app:testDebugUnitTest`, so one command and CI cover both). The app's gesture layer, the row host, S47 and S48 are in
`app/src/main/kotlin/com/example/croft/ui/swipe/`; what a swipe, a menu item or a TalkBack action does to a row goes through the
`RowActions` interface (`mail/RowActions.kt`), which T16 implements on the core. A device benchmark of the swipe is
`macrobenchmark/.../SwipeBenchmark.kt` (not run in CI).

## Compose and the Outbox (T18d)
- **Entry point for the reading screens (S20, S21):** open compose with a `ComposeStart` (`mail/compose/ComposeModels.kt`): `ComposeStart.Reply(account, messageId)`, `ReplyAll(account, messageId)`, `Forward(account, messageId)`, `New(account?)`, `Fill(ComposeFill)` (what a `mailto:` link or a share fills in), `Draft(account, draftId, cursor?)`. In the navigation host the function is `openCompose(start)` (`ui/nav/AppNavigation.kt`); the Inbox's own Reply row action is already wired. Compose returns a `ComposeClosed` (nothing, draft saved, discarded, sent with the Undo window) and the Inbox shows the snackbar.
- **Logic and screens apart.** `mail/compose/` is plain Kotlin (no Android): `ComposeController` (fields, chips, autosave, Send rules, Back rules), `AttachmentIntake` (picker copy), `OutboxController`, `EncryptionPolicy` (S24 seam), `Mailto`. It is compiled into `:mailtests` and tested there on the real core. `ui/compose/` holds the screens (S23, S24 sheet, S26) and the Android parts (`ComposeFiles`, `ComposeIntents`).
- **Attachments.** The system document picker (`OpenMultipleDocuments`) and the photo picker (`PickMultipleVisualMedia`) give a `content:` URI with a temporary read grant; no storage or media permission exists. `AttachmentIntake` checks the declared size against `compose_limits().max_attachment_bytes` first, copies the content to `cacheDir/compose-attach` under a generated name (`att-` and random hex, created new, copy capped at the limit), calls `add_draft_attachment_file(account, draft, path, cleanName, mime)` and deletes the copy. `MailRuntime` creates and empties that folder and registers it with `Core.setAttachmentDirectory` right after `Core.open`.
- **Intent filters** (on `MainActivity`, listed in `tools/manifest-allowlist.json` with their actions and the one BROWSABLE scheme; `check-release-manifest.py` fails on any other): `VIEW` and `SENDTO` with `mailto:`, `SEND`, `SEND_MULTIPLE`. A link or a share only fills compose: `attach=` in a link is ignored, `file:` streams are dropped.
- **Encryption.** `EncryptionPolicy` (default `NoEncryptionPolicy`: not required) is asked by `ComposeController.requestSend`; a missing key opens S24 and the message goes only after "Send unencrypted". `MailSession.encryptionPolicy` is where a real policy plugs in; tests use `RequiredKeyMissingPolicy`.
