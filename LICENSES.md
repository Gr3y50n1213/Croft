# Licences

| Part | Licence |
|---|---|
| `core/`, `android/`, `web/`, `ios/`, `design/` | MPL-2.0 |
| `companion/` and the push gateway | AGPL-3.0-or-later |

The canonical licence texts (`LICENSE-MPL-2.0`, `LICENSE-AGPL-3.0`) still have to be added to the repo
root from https://www.mozilla.org/MPL/2.0/ and https://www.gnu.org/licenses/agpl-3.0.txt. They were not
copied in from memory because licence text has to be verbatim. Add them as the first small commit. The AGPL text is at the address above (`curl -fsSL https://www.gnu.org/licenses/agpl-3.0.txt -o LICENSE-AGPL-3.0`).
For the MPL, open https://www.mozilla.org/MPL/2.0/, follow its plain-text link and save the file as `LICENSE-MPL-2.0`.
Check the first lines afterwards ("Mozilla Public License Version 2.0" and "GNU AFFERO GENERAL PUBLIC LICENSE Version 3"). Put the MPL header comment in new source files when the first ones are added.

## Third-party code in the Android app's native library
The app's Rust library (`libcroft_ffi.so`) is built with SQLCipher and a copy of OpenSSL's crypto compiled from source.
Both ship in the app, so their licence notices belong in the app's open-source licences screen (a later task adds it) and
in any release's notice file. The two below are C code that Rust crates compile in; the Rust crates' own licences are in
`core/Cargo.lock`'s dependency tree.

| Component | Where it comes from | Licence |
|---|---|---|
| OpenSSL 3.x (the `libcrypto` part only) | `openssl-src` crate, built by `croft-ffi`'s feature `sqlcipher-vendored-openssl` | Apache-2.0 |
| SQLCipher (Community Edition, an encrypted fork of SQLite) | bundled by `libsqlite3-sys` / `rusqlite` with the `bundled-sqlcipher-vendored-openssl` feature | BSD-style licence (Zetetic LLC; keep its copyright notice and disclaimer with every binary distribution) |

SQLite itself is in the public domain. The versions are the ones in `core/Cargo.lock` (`openssl-src`, `libsqlite3-sys`).

## Fonts bundled in the Android app
| Font | Use | Licence |
|---|---|---|
| Inter 3.x (shown as "Croft Sans"), Copyright 2020 The Inter Project Authors | Optional interface font | SIL Open Font License 1.1 |
| Atkinson Hyperlegible, Copyright 2020 Braille Institute of America, Inc. | Optional interface and reading font | SIL Open Font License 1.1 |
Licence texts: `android/app/src/main/assets/licenses/`.
