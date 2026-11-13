// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft

import java.io.File
import kotlin.io.path.createTempDirectory
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Test
import uniffi.croft_ffi.Core
import uniffi.croft_ffi.CoreException
import uniffi.croft_ffi.StorageReason
import uniffi.croft_ffi.croftVersion
import uniffi.croft_ffi.ffiApiVersion

/**
 * Kotlin to Rust through the generated bindings, on the JVM: the host build of libcroft_ffi is
 * found through the jna.library.path system property that the Gradle build sets.
 */
class RustBridgeTest {
    @Test
    fun versionIsTheCargoVersion() {
        assertEquals(System.getProperty("croft.cargoVersion"), croftVersion())
    }

    @Test
    fun apiVersionIsTheOneTheAppExpects() {
        assertEquals(EXPECTED_FFI_API_VERSION, ffiApiVersion())
        assertEquals(13u, ffiApiVersion())
        assertEquals(RustInfo(croftVersion(), EXPECTED_FFI_API_VERSION), loadRustInfo())
    }

    @Test
    fun aMismatchedApiVersionIsRefused() {
        checkFfiApiVersion(EXPECTED_FFI_API_VERSION)
        try {
            checkFfiApiVersion(2u)
            fail("expected an IllegalStateException")
        } catch (e: IllegalStateException) {
            assertTrue(e.message, e.message!!.contains("expected $EXPECTED_FFI_API_VERSION"))
        }
    }

    @Test
    fun aFreshDatabaseHasNoFolders() {
        val dir = createTempDirectory("croft-test").toFile()
        try {
            val core = Core.open(File(dir, "croft.db").path, null, NoCredentials())
            assertTrue(core.listFolders(1L).isEmpty())
            core.close()
        } finally {
            dir.deleteRecursively()
        }
    }

    @Test
    fun aFileThatIsNotADatabaseIsAStorageErrorWithoutThePath() {
        val dir = createTempDirectory("croft-test").toFile()
        try {
            val file = File(dir, "jane-doe-mail.db")
            file.writeBytes(ByteArray(4096) { 'x'.code.toByte() })
            try {
                Core.open(file.path, null, NoCredentials())
                fail("expected a CoreException")
            } catch (e: CoreException.Storage) {
                assertEquals(StorageReason.NOT_A_DATABASE, e.reason)
                assertFalse(e.toString(), e.toString().contains("jane-doe"))
            }
        } finally {
            dir.deleteRecursively()
        }
    }
}
