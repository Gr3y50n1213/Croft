// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.secrets

import javax.crypto.KeyGenerator
import javax.crypto.SecretKey

/**
 * A wrapping key held in memory: a plain JCE AES-256 key, so the envelope code of
 * [AesGcmSecretBox] runs for real on the JVM, and the failure paths (key gone, keystore busy) can
 * be switched on by the test. Everything here is made-up test data.
 */
class MemoryKeySource : WrappingKeySource {
    private var key: SecretKey? = null

    /** Set to make every call fail the way a busy or broken keystore does. */
    @Volatile
    var unavailable = false
    var discards = 0

    private fun fresh(): SecretKey = KeyGenerator.getInstance("AES").apply { init(256) }.generateKey()

    @Synchronized
    override fun keyForSealing(): SecretKey {
        if (unavailable) throw SecretBoxException(SecretBoxException.Kind.UNAVAILABLE)
        return key ?: fresh().also { key = it }
    }

    @Synchronized
    override fun keyForOpening(): SecretKey {
        if (unavailable) throw SecretBoxException(SecretBoxException.Kind.UNAVAILABLE)
        return key ?: throw SecretBoxException(SecretBoxException.Kind.KEY_LOST)
    }

    @Synchronized
    override fun discard() {
        key = null
        discards++
    }

    /** The phone restored from a backup, or the lock screen was reset: the key is gone. */
    @Synchronized
    fun lose() {
        key = null
    }

    /** A new key under the same source: blobs made before no longer open (they fail authentication). */
    @Synchronized
    fun replaceKey() {
        key = fresh()
    }
}

/** Fixture secrets (made up, never real). */
object Fixture {
    const val PASSWORD = "fixture-pw-Rk-5527"
    const val TOKEN = "fixture-token-Hs-8841"
}

/** True when [text] holds the secret as text or as the digits of a byte-array dump (every 3 bytes in a row). */
fun leaks(text: String, secret: String): Boolean {
    val bytes = secret.toByteArray()
    if (text.contains(secret)) return true
    if (text.contains(bytes.joinToString(", ")) || text.contains(bytes.joinToString(","))) return true
    return (0..bytes.size - 3).any { i -> text.contains(bytes.slice(i until i + 3).joinToString(", ")) }
}

/** True when [haystack] contains [needle] as a run of bytes. */
fun containsBytes(haystack: ByteArray, needle: ByteArray): Boolean {
    if (needle.isEmpty()) return true
    for (i in 0..haystack.size - needle.size) {
        if ((needle.indices).all { haystack[i + it] == needle[it] }) return true
    }
    return false
}
