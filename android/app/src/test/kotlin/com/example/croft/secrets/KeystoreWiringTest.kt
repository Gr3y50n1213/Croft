// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.secrets

import android.content.Context
import androidx.test.core.app.ApplicationProvider
import com.example.croft.CroftRobolectricRunner
import java.io.File
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertSame
import org.junit.Assert.assertTrue
import org.junit.Assume.assumeTrue
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.annotation.Config

/**
 * The real Keystore classes on a simulated device. Robolectric's support for `AndroidKeyStore` is
 * limited and varies by version, so these tests check the wiring that does not depend on it (where
 * the files go, that no raw exception escapes) and run the round trip only when the simulated
 * keystore can make the key (otherwise that test is skipped, not failed). What the hardware
 * does (StrongBox, TEE, reboots) can only be checked on a phone.
 */
@RunWith(CroftRobolectricRunner::class)
@Config(sdk = [35])
class KeystoreWiringTest {
    private val context: Context get() = ApplicationProvider.getApplicationContext()

    @Before
    fun forgetEarlierInstances() = ProcessSecrets.clearForTests()

    @Test
    fun theAppSecretsLiveInTheNoBackupFolder() {
        val db = File(context.noBackupFilesDir, "croft.db")
        val secrets = createAppSecrets(context, db)
        // One instance per process and folder.
        assertSame(secrets, createAppSecrets(context, db))
        val dir = secretsDirectory(context)
        assertEquals(File(context.noBackupFilesDir, "croft-secrets"), dir)
        assertTrue(dir.isDirectory)
        assertEquals("[redacted]", secrets.toString())
        // Nothing under the backed-up folders.
        assertFalse(dir.absolutePath.startsWith(context.filesDir.absolutePath + File.separator))
    }

    @Test
    fun aKeystoreProblemIsASecretBoxExceptionNeverARawOne() {
        val source = KeystoreKeySource(context, "croft.test.wiring.a")
        try {
            source.keyForOpening()
        } catch (e: SecretBoxException) {
            assertTrue(e.kind == SecretBoxException.Kind.KEY_LOST || e.kind == SecretBoxException.Kind.UNAVAILABLE)
            assertEquals(null, e.cause)
        }
        // No key made yet: either "lost" (no entry) or "unavailable" (no provider here) - never a crash.
    }

    @Test
    fun roundTripWithTheKeystoreKeyWhenTheSimulatedKeystoreCanMakeOne() {
        val source = KeystoreKeySource(context, "croft.test.wiring.b")
        val box = AesGcmSecretBox(source)
        val plain = Fixture.PASSWORD.toByteArray()
        val blob = try {
            source.create() // sealing never makes a key
            box.seal(plain, "ctx".toByteArray())
        } catch (e: SecretBoxException) {
            assumeTrue("this Robolectric has no usable AndroidKeyStore: ${e.kind}", false)
            return
        }
        assertNotNull(blob)
        assertFalse(containsBytes(blob, plain))
        assertArrayEquals(plain, box.open(blob, "ctx".toByteArray()))
        // Dropping the key makes the blob unopenable: the key-loss path.
        source.discard()
        try {
            box.open(blob, "ctx".toByteArray())
            throw AssertionError("expected KEY_LOST")
        } catch (e: SecretBoxException) {
            assertEquals(SecretBoxException.Kind.KEY_LOST, e.kind)
        }
    }
}
