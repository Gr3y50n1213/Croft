UniFFI surface for Kotlin (and Swift later). The wasm-bindgen surface for the web is not part of this crate yet.

Keep it small and versioned; every change here is reviewed by hand.

## The surface (API version 1)
- `croft_version() -> String`: version of the core library.
- `ffi_api_version() -> u32`: version of this surface. **Bump `FFI_API_VERSION` in `src/lib.rs` on any
  breaking change** (remove, rename, retype); the apps check it at start-up.
- `Core.open(db_path)`, `Core.list_folders(account_id)`, `FolderInfo`, `CoreError`.

The database is not encrypted yet; the key parameter arrives with the keystore task.

## Lints
The crate inherits the workspace lints, including `unsafe_code = "forbid"`. UniFFI 0.32's macros compile
under it, so there is no exception.

## Building
```
cd core && cargo test -p croft-ffi
cargo run -p croft-ffi --features cli --bin uniffi-bindgen -- --help   # same UniFFI version as the library
```
The Android build (`android/`) runs cargo-ndk and the bindings generator; see `android/README.md`.
