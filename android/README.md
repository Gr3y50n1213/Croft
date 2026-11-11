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
