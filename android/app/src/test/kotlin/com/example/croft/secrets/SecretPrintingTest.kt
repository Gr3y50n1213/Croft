// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.secrets

import java.io.File
import java.io.Serializable
import kotlin.io.path.createTempDirectory
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Before
import org.junit.Test

/**
 * Every Kotlin class that holds or touches a secret prints as `[redacted]`, and none of them can
 * be serialized or written out by accident. Each is built holding the fixture secrets.
 */
class SecretPrintingTest {
    private lateinit var dir: File

    @Before
    fun setUp() {
        dir = createTempDirectory("croft-printing-test").toFile()
    }

    @After
    fun tearDown() {
        dir.deleteRecursively()
    }

    @Test
    fun everySecretHoldingClassPrintsRedactedAndNeverTheFixture() {
        val keys = MemoryKeySource()
        val box = AesGcmSecretBox(keys)
        val store = SecretStore(dir, box)
        store.put(1, SecretKind.PASSWORD, Fixture.PASSWORD.toByteArray())
        store.put(1, SecretKind.OAUTH_ACCESS, Fixture.TOKEN.toByteArray())
        val appSecrets = AppSecrets(File(dir, "app"), box, keys, File(dir, "croft.db"))
        val databaseKey = appSecrets.databaseKey()
        val signIns = SignInNeeded().apply { mark(1) }
        val provider = SecretStoreCredentials(store, signIns) { "alice" }

        val printed = mapOf(
            "DatabaseKey" to databaseKey,
            "SecretStore" to store,
            "AppSecrets" to appSecrets,
            "AesGcmSecretBox" to box,
            "KeystoreKeySource" to KeystoreKeySource("croft.test", FakeKeystore(), KeystoreErrors(), emptyList(), {}),
            "KeyLossNotice" to appSecrets.keyLoss,
            "SignInNeeded" to signIns,
            "SecretStoreCredentials" to provider,
            "AccountSecrets" to AccountSecrets(store, signIns),
            "Quarantine" to Quarantine(File(dir, "q"), { 0L }),
            "QuarantineRestore" to appSecrets.quarantineRestore,
            "LossConfirmation" to LossConfirmation(File(dir, "suspect"), { 0L }),
        )
        printed.forEach { (name, obj) ->
            val text = "$obj / ${obj.toString()} / $name: $obj"
            assertEquals(name, "[redacted]", obj.toString())
            listOf(Fixture.PASSWORD, Fixture.TOKEN).forEach { assertFalse("$name leaks", leaks(text, it)) }
        }
        databaseKey.close()
    }

    @Test
    fun theBoxExceptionAndItsKindsCarryNoText() {
        SecretBoxException.Kind.values().forEach {
            val e = SecretBoxException(it, weak = true)
            assertEquals(it.name, e.message)
            assertNull(e.cause)
            assertFalse(leaks(e.toString(), Fixture.PASSWORD))
        }
        SecureStorageException.Reason.values().forEach {
            val e = SecureStorageException(it)
            assertEquals(it.name, e.message)
            assertNull(e.cause)
        }
    }

    @Test
    fun noSecretHoldingClassIsSerializable() {
        listOf(
            DatabaseKey::class.java,
            SecretStore::class.java,
            AppSecrets::class.java,
            AesGcmSecretBox::class.java,
            SecretStoreCredentials::class.java,
        ).forEach { assertFalse(it.name, Serializable::class.java.isAssignableFrom(it)) }
    }
}
