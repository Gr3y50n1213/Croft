// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft

import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import uniffi.croft_ffi.FfiCredential

/**
 * `FfiCredential` is an object, not an enum with fields, because UniFFI generates a Kotlin data
 * class for an enum variant with fields, and a data class's `toString()` prints its properties:
 * a `String` field prints the secret and so does a `ByteArray` field (Kotlin prints the array's
 * elements). This test fails if the type ever becomes a data class again.
 */
class CredentialPrintingTest {
    private val secret = "fixture-secret-Wm8-2290"

    private fun assertNoSecret(text: String) {
        val bytes = secret.toByteArray()
        assertFalse(text, text.contains(secret))
        // Nor its bytes the way an array dump shows them, with or without spaces.
        assertFalse(text, text.contains(bytes.joinToString(", ")))
        assertFalse(text, text.contains(bytes.joinToString(",")))
        // Nor any three consecutive bytes of it.
        for (i in 0..bytes.size - 3) {
            assertFalse(text, text.contains(bytes.slice(i until i + 3).joinToString(", ")))
        }
    }

    @Test
    fun passwordToStringDoesNotContainTheSecret() {
        val text = FfiCredential.password(secret.toByteArray()).toString()
        assertNoSecret(text)
        assertTrue(text, text.contains("FfiCredential"))
    }

    @Test
    fun oauth2ToStringDoesNotContainTheToken() {
        val text = FfiCredential.oauth2("alice", secret.toByteArray()).toString()
        assertNoSecret(text)
        assertTrue(text, text.contains("FfiCredential"))
    }

    @Test
    fun theCallersArrayCanBeWipedRightAfterTheConstructor() {
        val bytes = secret.toByteArray()
        val credential = FfiCredential.password(bytes)
        bytes.fill(0)
        assertTrue(bytes.all { it == 0.toByte() })
        assertNoSecret(credential.toString())
        credential.close()
    }
}
