// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft

import uniffi.croft_ffi.croftVersion
import uniffi.croft_ffi.ffiApiVersion

/**
 * The FFI API version this app was written against. Bump it together with the Rust constant
 * `FFI_API_VERSION` in core/ffi/src/lib.rs, after adapting to the change that made it necessary.
 */
const val EXPECTED_FFI_API_VERSION: UInt = 3u

/** What the Rust core reports about itself. */
data class RustInfo(val croftVersion: String, val ffiApiVersion: UInt)

/**
 * Loads the Rust library and reads its versions.
 *
 * Fails with [IllegalStateException] when the library speaks a different FFI API version than this
 * app expects, so a mismatch is caught here and not as a crash deep inside a call.
 */
fun loadRustInfo(): RustInfo {
    val api = ffiApiVersion()
    checkFfiApiVersion(api)
    return RustInfo(croftVersion = croftVersion(), ffiApiVersion = api)
}

/** Refuses a library whose FFI API version is not [expected]. Separate so a test can try a mismatch. */
internal fun checkFfiApiVersion(actual: UInt, expected: UInt = EXPECTED_FFI_API_VERSION) {
    check(actual == expected) { "FFI API version $actual, expected $expected" }
}
