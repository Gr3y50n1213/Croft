Jetpack Compose app, Material 3 Expressive. It currently has one screen (`MainActivity`) that shows the version of the Rust core, proving Rust → UniFFI → Kotlin → Compose. Build the real screens from `docs/ux/` and the wireframes (S01 to S60) in `design/wireframes/`.

## Build
Needs: JDK 17+ (CI and the first setup use 21), the Android SDK (`ANDROID_HOME`) with `platforms;android-36`,
`build-tools;36.0.0` and `ndk;29.0.14206865`, Rust with the targets
`aarch64-linux-android` and `x86_64-linux-android`, and `cargo install cargo-ndk --version 4.1.2 --locked`.

```
cd android
./gradlew :app:assembleDebug :app:testDebugUnitTest
tools/check-16kb-alignment.sh app/build/generated/rust/jniLibs   # LOAD segments must be 0x4000 aligned
```

What Gradle does (see `app/build.gradle.kts`):
- `buildRustAndroid`: `cargo ndk` builds `libcroft_ffi.so` (release) for `arm64-v8a` and `x86_64` into `app/build/generated/rust/jniLibs`.
- `buildRustHost`: `cargo build -p croft-ffi` for this computer; the JVM unit tests load it through JNA.
- `generateUniffiBindings`: `uniffi-bindgen` (same UniFFI version as the library, `core/ffi`) writes the Kotlin bindings to `app/build/generated/uniffi/kotlin`, package `uniffi.croft_ffi`.
- `design/generated/kotlin/CroftTokens.kt` is compiled in place (never copied) and drives `CroftTheme`.

The application id is `croft.applicationId` in `gradle.properties` (placeholder `com.example.croft`).
The FFI API version the app expects is `EXPECTED_FFI_API_VERSION` in `RustBridge.kt`; it must match
`FFI_API_VERSION` in `core/ffi/src/lib.rs`.
