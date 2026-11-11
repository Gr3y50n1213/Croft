// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.secrets

import javax.crypto.Cipher
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotEquals
import org.junit.Assert.fail
import org.junit.Test

/** The envelope format and the failure paths of [AesGcmSecretBox], over an in-memory JCE key. */
class SecretBoxTest {
    private val keys = MemoryKeySource()
    private val box = AesGcmSecretBox(keys)
    private val plain = Fixture.PASSWORD.toByteArray()
    private val context = "ctx-a".toByteArray()

    private fun failure(block: () -> Unit): SecretBoxException.Kind {
        try {
            block()
        } catch (e: SecretBoxException) {
            return e.kind
        }
        fail("expected a SecretBoxException")
        error("unreachable")
    }

    @Test
    fun roundTrip() {
        val blob = box.seal(plain, context)
        assertArrayEquals(plain, box.open(blob, context))
    }

    @Test
    fun theEnvelopeIsVersionThenIvThenCiphertextAndTag() {
        val blob = box.seal(plain, context)
        assertEquals(1.toByte(), blob[0])
        assertEquals(1 + 12 + plain.size + 16, blob.size)
        assertFalse("plaintext must not appear", containsBytes(blob, plain))
    }

    @Test
    fun everySealUsesAFreshIv() {
        val a = box.seal(plain, context)
        val b = box.seal(plain, context)
        assertNotEquals(a.slice(1..12), b.slice(1..12))
        assertNotEquals(a.toList(), b.toList())
    }

    @Test
    fun aChangeAnywhereInTheBlobIsDetected() {
        val blob = box.seal(plain, context)
        for (i in blob.indices) {
            val damaged = blob.copyOf().also { it[i] = (it[i].toInt() xor 0x01).toByte() }
            assertEquals("byte $i", SecretBoxException.Kind.CORRUPT, failure { box.open(damaged, context) })
        }
    }

    @Test
    fun aTruncatedOrEmptyBlobIsCorrupt() {
        val blob = box.seal(plain, context)
        assertEquals(SecretBoxException.Kind.CORRUPT, failure { box.open(blob.copyOf(blob.size - 1), context) })
        assertEquals(SecretBoxException.Kind.CORRUPT, failure { box.open(ByteArray(0), context) })
        assertEquals(SecretBoxException.Kind.CORRUPT, failure { box.open(blob.copyOf(20), context) })
    }

    @Test
    fun aHigherVersionIsUnsupportedNotCorruptAndNotAReset() {
        val blob = box.seal(plain, context)
        for (version in listOf(2, 3, 0x7f, 0x80, 0xff)) {
            val newer = blob.copyOf().also { it[0] = version.toByte() }
            assertEquals("version $version", SecretBoxException.Kind.UNSUPPORTED_VERSION, failure { box.open(newer, context) })
        }
        // Even when the rest is too short to be an envelope of this version.
        assertEquals(SecretBoxException.Kind.UNSUPPORTED_VERSION, failure { box.open(byteArrayOf(2), context) })
        // Version 0 never existed: damage.
        assertEquals(SecretBoxException.Kind.CORRUPT, failure { box.open(blob.copyOf().also { it[0] = 0 }, context) })
    }

    @Test
    fun theVersionByteIsPartOfTheAuthenticatedData() {
        val key = keys.keyForSealing()
        fun sealWithAad(aad: ByteArray): ByteArray {
            val cipher = Cipher.getInstance("AES/GCM/NoPadding")
            cipher.init(Cipher.ENCRYPT_MODE, key)
            cipher.updateAAD(aad)
            val sealed = cipher.doFinal(plain)
            return byteArrayOf(1) + cipher.iv + sealed
        }
        // The documented format: AAD = version byte, then the context.
        assertArrayEquals(plain, box.open(sealWithAad(byteArrayOf(1) + context), context))
        // The context alone (no version byte) does not authenticate.
        assertEquals(SecretBoxException.Kind.CORRUPT, failure { box.open(sealWithAad(context), context) })
        // Another version byte in the AAD does not either.
        assertEquals(SecretBoxException.Kind.CORRUPT, failure { box.open(sealWithAad(byteArrayOf(0) + context), context) })
    }

    @Test
    fun aBlobDoesNotOpenForAnotherContext() {
        val blob = box.seal(plain, context)
        assertEquals(SecretBoxException.Kind.CORRUPT, failure { box.open(blob, "ctx-b".toByteArray()) })
    }

    @Test
    fun aBlobDoesNotOpenUnderAnotherKey() {
        val blob = box.seal(plain, context)
        keys.replaceKey()
        assertEquals(SecretBoxException.Kind.CORRUPT, failure { box.open(blob, context) })
    }

    @Test
    fun aMissingKeyIsKeyLostAndABusyKeystoreIsUnavailable() {
        val blob = box.seal(plain, context)
        keys.lose()
        assertEquals(SecretBoxException.Kind.KEY_LOST, failure { box.open(blob, context) })
        keys.unavailable = true
        assertEquals(SecretBoxException.Kind.UNAVAILABLE, failure { box.open(blob, context) })
        assertEquals(SecretBoxException.Kind.UNAVAILABLE, failure { box.seal(plain, context) })
    }

    @Test
    fun theSealDoesNotChangeOrKeepTheCallersArray() {
        val copy = plain.copyOf()
        box.seal(plain, context)
        assertArrayEquals(copy, plain)
    }

    @Test
    fun exceptionTextHoldsNoSecretOrContext() {
        val e = SecretBoxException(SecretBoxException.Kind.CORRUPT)
        assertEquals("CORRUPT", e.message)
        assertEquals(null, e.cause)
    }
}
