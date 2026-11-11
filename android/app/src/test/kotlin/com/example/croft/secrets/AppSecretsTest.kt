// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.secrets

import java.io.File
import kotlin.io.path.createTempDirectory
import org.junit.After
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotEquals
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Before
import org.junit.Test

/** The database key's life: first run, later runs, and what happens when the Keystore key is gone. */
class AppSecretsTest {
    private lateinit var root: File
    private lateinit var dir: File
    private lateinit var db: File
    private val keys = MemoryKeySource()

    @Before
    fun setUp() {
        root = createTempDirectory("croft-appsecrets-test").toFile()
        dir = File(root, "secrets")
        db = File(root, "croft.db")
    }

    @After
    fun tearDown() {
        root.deleteRecursively()
    }

    private fun secrets() = AppSecrets(dir, AesGcmSecretBox(keys), keys, db)

    private fun keyBytes(s: AppSecrets): ByteArray = s.databaseKey().use { k -> k.withBytes { it.copyOf() } }

    private fun makeDatabaseFiles() {
        listOf("", "-wal", "-shm", "-journal").forEach { File(db.path + it).writeText("not really sqlite") }
    }

    @Test
    fun theFirstRunMakesA32ByteKeyAndKeepsOnlyAWrappedCopy() {
        val s = secrets()
        val key = keyBytes(s)
        assertEquals(32, key.size)
        assertTrue(key.any { it != 0.toByte() })
        val files = dir.listFiles()!!.toList()
        assertEquals(listOf("db-key.v1"), files.map { it.name })
        assertFalse(containsBytes(files.single().readBytes(), key))
        assertFalse(s.keyLoss.pending.value)
    }

    @Test
    fun laterRunsUnwrapTheSameKey() {
        val first = keyBytes(secrets())
        makeDatabaseFiles()
        assertArrayEquals(first, keyBytes(secrets()))
        assertTrue(db.exists())
    }

    @Test
    fun aKeyThatIsClosedIsWipedAndTheCopyGivenToTheBlockIsWiped() {
        val key = secrets().databaseKey()
        var leaked: ByteArray? = null
        key.withBytes { leaked = it }
        assertTrue(leaked!!.all { it == 0.toByte() })
        key.close()
    }

    @Test
    fun aLostKeystoreKeyDeletesTheDatabaseAndTheSecretsAndStartsAsNew() {
        val s = secrets()
        val old = keyBytes(s)
        makeDatabaseFiles()
        s.secrets.put(1, SecretKind.PASSWORD, Fixture.PASSWORD.toByteArray())
        keys.lose()

        val fresh = secrets()
        val key = keyBytes(fresh)
        assertEquals(32, key.size)
        assertNotEquals(old.toList(), key.toList())
        listOf("", "-wal", "-shm", "-journal").forEach { assertFalse(File(db.path + it).exists()) }
        assertFalse(fresh.secrets.hasAny())
        assertTrue(fresh.keyLoss.pending.value)
        // The new key is the one kept from now on.
        assertArrayEquals(key, keyBytes(secrets()))
        // The marker survives a new process until the screen has shown it.
        assertTrue(secrets().keyLoss.pending.value)
        fresh.keyLoss.acknowledge()
        assertFalse(fresh.keyLoss.pending.value)
        assertFalse(secrets().keyLoss.pending.value)
    }

    @Test
    fun aWrappedKeyThatCannotBeOpenedIsAlsoALoss() {
        val s = secrets()
        s.databaseKey().close()
        makeDatabaseFiles()
        val blob = File(dir, "db-key.v1")
        blob.writeBytes(blob.readBytes().also { it[20] = (it[20].toInt() xor 1).toByte() })
        val s2 = secrets()
        s2.databaseKey().close()
        assertFalse(db.exists())
        assertTrue(s2.keyLoss.pending.value)
    }

    @Test
    fun dataWithoutItsWrappedKeyIsALoss() {
        makeDatabaseFiles()
        val s = secrets()
        s.databaseKey().close()
        assertFalse(db.exists())
        assertTrue(s.keyLoss.pending.value)
    }

    @Test
    fun aBusyKeystoreDeletesNothingAndThrows() {
        val s = secrets()
        val key = keyBytes(s)
        makeDatabaseFiles()
        s.secrets.put(1, SecretKind.PASSWORD, "x".toByteArray())
        keys.unavailable = true
        try {
            secrets().databaseKey()
            fail("expected UNAVAILABLE")
        } catch (e: SecretBoxException) {
            assertEquals(SecretBoxException.Kind.UNAVAILABLE, e.kind)
        }
        assertTrue(db.exists())
        assertTrue(s.secrets.hasAny())
        assertFalse(secrets().keyLoss.pending.value)
        keys.unavailable = false
        assertArrayEquals(key, keyBytes(secrets()))
    }

    @Test
    fun aLossAlsoDropsTheOldKeystoreKeyEntry() {
        val s = secrets()
        s.databaseKey().close()
        keys.lose()
        secrets().databaseKey().close()
        assertEquals(1, keys.discards)
    }
}
