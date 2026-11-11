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

/**
 * Using `uniffi.croft_ffi.Core` from the app (see also core/ffi/README.md):
 *
 * - Closing the core must happen off the main thread. `Core.close()` (the generated destructor,
 *   also what the cleaner runs) drops the Rust object, which stops the core's runtime and waits
 *   up to 2 seconds for its account tasks to let go of the database, so that a reopen of the same
 *   file finds it free. That wait is bounded and on purpose; run the close on a background
 *   dispatcher (for example `withContext(Dispatchers.IO) { core.close() }`), never from
 *   `onDestroy`, a view model's main-thread hook or a lifecycle callback.
 * - So are `Core.open` (reads and migrates the database and asks the credential provider) and the
 *   callbacks the core makes into the app.
 * - Secrets cross as `ByteArray`: `FfiCredential.password(bytes)`, `FfiCredential.oauth2(user, bytes)`
 *   and the database key given to `Core.open`. `FfiCredential` is a class, not a data class, so it
 *   prints no secret. The app should `fill(0)` each array as soon as the call that took it has
 *   returned and `close()` a credential once it is handed over. The core wipes the copy it holds,
 *   but not UniFFI's lifting buffer or any copy made on the Kotlin side.
 */

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
