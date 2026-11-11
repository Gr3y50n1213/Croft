// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.secrets

import java.security.Key
import java.util.concurrent.atomic.AtomicInteger
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey

/**
 * A wrapping key held in memory: a plain JCE AES-256 key, so the envelope code of
 * [AesGcmSecretBox] runs for real on the JVM, and the failure paths (key gone, keystore busy) can
 * be switched on by the test. Like the real source it never makes a key while sealing: [create]
 * does, and [creates] counts the calls. A source starts with a key unless [withKey] is false.
 * Everything here is made-up test data.
 */
class MemoryKeySource(withKey: Boolean = true) : WrappingKeySource {
    private var key: SecretKey? = if (withKey) fresh() else null

    /** Set to make every call fail the way a busy or broken keystore does. */
    @Volatile
    var unavailable = false

    /** Makes a lost key look like weak evidence (Android 8 to 11), not like proof. */
    @Volatile
    var weakLoss = false
    var discards = 0
    var creates = 0

    private fun fresh(): SecretKey = KeyGenerator.getInstance("AES").apply { init(256) }.generateKey()

    @Synchronized
    override fun keyForSealing(): SecretKey = keyForOpening()

    @Synchronized
    override fun keyForOpening(): SecretKey {
        if (unavailable) throw SecretBoxException(SecretBoxException.Kind.UNAVAILABLE)
        return key ?: throw SecretBoxException(SecretBoxException.Kind.KEY_LOST, weak = weakLoss)
    }

    @Synchronized
    override fun create(): SecretKey {
        if (unavailable) throw SecretBoxException(SecretBoxException.Kind.UNAVAILABLE)
        creates++
        return fresh().also { key = it }
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

/**
 * A [KeystoreOps] whose answers a test scripts: each call takes the next step from its queue (or
 * repeats [default] when the queue is empty). A step is a value or an exception to throw.
 */
internal class FakeKeystore : KeystoreOps {
    /** What `getKey` returns or throws, in order. */
    val getKeySteps = ArrayDeque<Result<SecretKey?>>()
    var getKeyDefault: Result<SecretKey?> = Result.success(null)

    /** What `aliases()` returns or throws, in order. */
    val aliasSteps = ArrayDeque<Result<List<String>>>()
    var aliasesDefault: Result<List<String>> = Result.success(emptyList())

    /** What `canaryWorks` returns or throws, in order. */
    val canarySteps = ArrayDeque<Result<Boolean>>()
    var canaryDefault: Result<Boolean> = Result.success(false)

    val getKeyCalls = AtomicInteger()
    val aliasCalls = AtomicInteger()
    val canaryCalls = AtomicInteger()
    val generated = mutableListOf<String>()
    val deleted = mutableListOf<String>()
    var generateFails: Throwable? = null

    private fun fresh(): SecretKey = KeyGenerator.getInstance("AES").apply { init(256) }.generateKey()

    override fun getKey(alias: String): SecretKey? {
        getKeyCalls.incrementAndGet()
        return (getKeySteps.removeFirstOrNull() ?: getKeyDefault).getOrThrow()
    }

    override fun aliases(): List<String> {
        aliasCalls.incrementAndGet()
        return (aliasSteps.removeFirstOrNull() ?: aliasesDefault).getOrThrow()
    }

    override fun canaryWorks(alias: String): Boolean {
        canaryCalls.incrementAndGet()
        return (canarySteps.removeFirstOrNull() ?: canaryDefault).getOrThrow()
    }

    override fun delete(alias: String) {
        deleted += alias
    }

    override fun generate(alias: String): SecretKey {
        generateFails?.let { throw it }
        generated += alias
        return fresh()
    }
}

/**
 * A [SecretBox] that fails the way a test says before (or instead of) delegating, to run
 * [AppSecrets] against each kind of failure without a real keystore. [opens] and [seals] count
 * the calls that reached it.
 */
class ScriptedBox(private val inner: SecretBox) : SecretBox {
    /** Failures for the next `open` calls, in order; an empty queue delegates. */
    val openFailures = ArrayDeque<SecretBoxException.Kind>()
    val sealFailures = ArrayDeque<SecretBoxException.Kind>()
    var opens = 0
    var seals = 0

    /** Makes the scripted KEY_LOST and UNAVAILABLE failures weak evidence (Android 8 to 11). */
    @Volatile
    var weak = false

    private fun failure(kind: SecretBoxException.Kind) = SecretBoxException(
        kind,
        weak && (kind == SecretBoxException.Kind.KEY_LOST || kind == SecretBoxException.Kind.UNAVAILABLE),
    )

    /** Makes every seal take this long (to hold the lock in a race test). */
    @Volatile
    var sealDelayMs = 0L

    override fun seal(plain: ByteArray, context: ByteArray): ByteArray {
        seals++
        if (sealDelayMs > 0) Thread.sleep(sealDelayMs)
        sealFailures.removeFirstOrNull()?.let { throw failure(it) }
        return inner.seal(plain, context)
    }

    override fun open(blob: ByteArray, context: ByteArray): ByteArray {
        opens++
        openFailures.removeFirstOrNull()?.let { throw failure(it) }
        return inner.open(blob, context)
    }
}

/**
 * A key whose bytes cannot be read: the JCE cipher asks for them at `init`, and this throws
 * [failure] there, the way a Keystore key fails when it is used. (Kotlin does not check exception
 * types, so a checked [java.security.GeneralSecurityException] can be thrown from here.)
 */
class FailingKey(private val failure: Throwable) : SecretKey {
    override fun getAlgorithm() = "AES"

    // "RAW": the provider must accept the key; the failure then comes from reading its bytes.
    override fun getFormat(): String = "RAW"

    override fun getEncoded(): ByteArray = throw failure
}

/** A key source that hands out [key] for both purposes. */
class FixedKeySource(private val key: Key) : WrappingKeySource {
    override fun keyForSealing(): SecretKey = key as SecretKey

    override fun create(): SecretKey = key as SecretKey

    override fun keyForOpening(): SecretKey = key as SecretKey

    override fun discard() {}
}
