// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The UniFFI bindings generator, pinned to the same UniFFI version as the library. Built only with the `cli` feature.
//!
//! Used by the Android build: `cargo run -p croft-ffi --features cli --bin uniffi-bindgen -- generate ...`.

fn main() {
    uniffi::uniffi_bindgen_main()
}
