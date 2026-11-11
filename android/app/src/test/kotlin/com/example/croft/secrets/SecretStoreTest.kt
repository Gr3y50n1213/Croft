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
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Before
import org.junit.Test

class SecretStoreTest {
    private lateinit var dir: File
    private val keys = MemoryKeySource()
    private lateinit var store: SecretStore

    @Before
    fun setUp() {
        dir = createTempDirectory("croft-secrets-test").toFile()
        store = SecretStore(dir, AesGcmSecretBox(keys))
    }

    @After
    fun tearDown() {
        dir.deleteRecursively()
    }

    private fun bytes(s: String) = s.toByteArray()

    @Test
    fun roundTripForEveryKind() {
        SecretKind.values().forEach { kind ->
            store.put(7, kind, bytes("v-${kind.name}"))
        }
        SecretKind.values().forEach { kind ->
            assertArrayEquals(bytes("v-${kind.name}"), store.get(7, kind))
        }
    }

    @Test
    fun putReplacesAndDoesNotKeepTheCallersArray() {
        val first = bytes(Fixture.PASSWORD)
        store.put(1, SecretKind.PASSWORD, first)
        first.fill(0)
        assertArrayEquals(bytes(Fixture.PASSWORD), store.get(1, SecretKind.PASSWORD))
        store.put(1, SecretKind.PASSWORD, bytes("second"))
        assertArrayEquals(bytes("second"), store.get(1, SecretKind.PASSWORD))
    }

    @Test
    fun anUnknownAccountOrKindIsNull() {
        assertNull(store.get(404, SecretKind.PASSWORD))
        store.put(1, SecretKind.PASSWORD, bytes("x"))
        assertNull(store.get(1, SecretKind.OAUTH_REFRESH))
        assertNull(store.get(2, SecretKind.PASSWORD))
    }

    @Test
    fun removeDeletesOneEntryAndRemoveAccountDeletesAllOfAnAccount() {
        SecretKind.values().forEach { store.put(1, it, bytes("a")) }
        store.put(2, SecretKind.PASSWORD, bytes("b"))
        store.remove(1, SecretKind.OAUTH_ACCESS)
        assertNull(store.get(1, SecretKind.OAUTH_ACCESS))
        assertTrue(store.has(1, SecretKind.PASSWORD))
        store.removeAccount(1)
        SecretKind.values().forEach { assertFalse(store.has(1, it)) }
        assertArrayEquals(bytes("b"), store.get(2, SecretKind.PASSWORD))
        store.removeAccount(404) // no entries: nothing to do, no error
    }

    @Test
    fun nothingOnDiskHoldsTheSecretInTheClear() {
        store.put(1, SecretKind.PASSWORD, bytes(Fixture.PASSWORD))
        store.put(1, SecretKind.OAUTH_ACCESS, bytes(Fixture.TOKEN))
        val files = dir.listFiles()!!.toList()
        assertEquals(2, files.size)
        files.forEach {
            val content = it.readBytes()
            assertFalse(containsBytes(content, bytes(Fixture.PASSWORD)))
            assertFalse(containsBytes(content, bytes(Fixture.TOKEN)))
            assertFalse(it.name.contains(Fixture.PASSWORD))
        }
    }

    @Test
    fun aDamagedEntryReadsAsNullAndIsDeleted() {
        store.put(1, SecretKind.PASSWORD, bytes(Fixture.PASSWORD))
        val file = dir.listFiles()!!.single()
        file.writeBytes(file.readBytes().also { it[it.size - 1] = (it[it.size - 1].toInt() xor 1).toByte() })
        assertNull(store.get(1, SecretKind.PASSWORD))
        assertFalse(store.has(1, SecretKind.PASSWORD))
    }

    @Test
    fun anEntryCopiedOntoAnotherAccountOrKindDoesNotOpen() {
        store.put(1, SecretKind.PASSWORD, bytes(Fixture.PASSWORD))
        val source = dir.listFiles()!!.single()
        // Pretend someone copies account 1's blob over account 2's password and account 1's refresh token.
        source.copyTo(File(dir, "acct-2.password.v1"))
        source.copyTo(File(dir, "acct-1.oauth_refresh.v1"))
        assertNull(store.get(2, SecretKind.PASSWORD))
        assertNull(store.get(1, SecretKind.OAUTH_REFRESH))
        assertArrayEquals(bytes(Fixture.PASSWORD), store.get(1, SecretKind.PASSWORD))
    }

    @Test
    fun aLostKeyIsReportedAndNothingIsDeleted() {
        store.put(1, SecretKind.PASSWORD, bytes("x"))
        keys.lose()
        try {
            store.get(1, SecretKind.PASSWORD)
            fail("expected KEY_LOST")
        } catch (e: SecretBoxException) {
            assertEquals(SecretBoxException.Kind.KEY_LOST, e.kind)
        }
        assertTrue(store.has(1, SecretKind.PASSWORD))
    }

    @Test
    fun aBusyKeystoreIsReportedAndNothingIsDeleted() {
        store.put(1, SecretKind.PASSWORD, bytes("x"))
        keys.unavailable = true
        try {
            store.get(1, SecretKind.PASSWORD)
            fail("expected UNAVAILABLE")
        } catch (e: SecretBoxException) {
            assertEquals(SecretBoxException.Kind.UNAVAILABLE, e.kind)
        }
        keys.unavailable = false
        assertArrayEquals(bytes("x"), store.get(1, SecretKind.PASSWORD))
    }

    @Test
    fun aFailedSealLeavesTheOldEntry() {
        store.put(1, SecretKind.PASSWORD, bytes("old"))
        keys.unavailable = true
        try {
            store.put(1, SecretKind.PASSWORD, bytes("new"))
            fail("expected UNAVAILABLE")
        } catch (e: SecretBoxException) {
            // expected
        }
        keys.unavailable = false
        assertArrayEquals(bytes("old"), store.get(1, SecretKind.PASSWORD))
        assertFalse(dir.listFiles()!!.any { it.name.endsWith(".tmp") })
    }

    @Test
    fun clearDeletesEveryAccountSecret() {
        store.put(1, SecretKind.PASSWORD, bytes("a"))
        store.put(2, SecretKind.OAUTH_REFRESH, bytes("b"))
        assertTrue(store.hasAny())
        store.clear()
        assertFalse(store.hasAny())
    }
}
