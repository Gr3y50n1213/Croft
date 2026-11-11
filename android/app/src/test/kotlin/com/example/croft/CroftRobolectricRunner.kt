// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft

import org.junit.runners.model.FrameworkMethod
import org.robolectric.RobolectricTestRunner
import org.robolectric.internal.bytecode.InstrumentationConfiguration

/**
 * Robolectric with the generated UniFFI bindings and JNA shared with the plain tests.
 *
 * Robolectric loads the app's classes in its own class loader. Without this, a Gradle run (one
 * test JVM for every class) held two copies of `uniffi.croft_ffi`, each with its own table of
 * callback objects, over one native library. The library keeps a single table of callback entry
 * points: whichever copy registered last received every callback, with handles that only the other
 * copy knew ("UniffiHandleMap: Invalid handle"). The credential provider then looked like it had
 * no credential, and the plain tests that depend on callbacks failed or timed out depending on
 * which class had run before them. Sharing the bindings and JNA with the parent loader leaves one
 * copy and one table. (Running each class alone, or with plain JUnit, never showed it.)
 */
class CroftRobolectricRunner(testClass: Class<*>) : RobolectricTestRunner(testClass) {
    override fun createClassLoaderConfig(method: FrameworkMethod): InstrumentationConfiguration =
        InstrumentationConfiguration.Builder(super.createClassLoaderConfig(method))
            .doNotAcquirePackage("uniffi.croft_ffi")
            .doNotAcquirePackage("com.sun.jna")
            .build()
}
