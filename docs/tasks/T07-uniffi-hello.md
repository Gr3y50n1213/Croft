# T07: UniFFI surface and first Compose screen

Crate / folder: `core/ffi + android`

## Goal
The plan's first task: prove the toolchain end to end.

## Spec
- `core/ffi`: UniFFI proc-macro crate exposing `fn croft_version() -> String` and a small `Core` object (open database, list folders). Generate Kotlin bindings; cargo-ndk builds the `.so` for arm64-v8a and x86_64.
- `android/`: Gradle project with one Compose screen that calls it and shows the result.
- CI builds both. Version the FFI API from the start.

## Done when
- The Android app shows the Rust-provided version string on a device or emulator.
- CI builds the Rust library and the debug APK.
