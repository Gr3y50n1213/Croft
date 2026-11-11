// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.secrets

import android.content.Context
import android.content.pm.PackageManager
import android.os.Build
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import android.security.keystore.StrongBoxUnavailableException
import java.security.KeyStore
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey

/** The Keystore calls [KeystoreKeySource] makes, so the decisions around them run on the JVM. */
internal interface KeystoreOps {
    /** `KeyStore.getKey(alias)`: the key, or null. Null does not prove the key is gone (Android 8 to 11 return it when the service is unreachable). May throw anything the platform throws. */
    fun getKey(alias: String): SecretKey?

    /** `KeyStore.aliases()`: a call that is independent of the key itself. Empty can also mean "the service failed" (Android 8 to 11). */
    fun aliases(): List<String>

    /** True when the key under [alias] can be got and does a tiny encryption. May throw anything the platform throws. */
    fun canaryWorks(alias: String): Boolean

    /** `KeyStore.deleteEntry(alias)`. */
    fun delete(alias: String)

    /** Creates a key under [alias] (replacing an entry of that name). */
    fun generate(alias: String): SecretKey
}

/**
 * The AES-256-GCM wrapping key in the Android Keystore: generated inside the keystore, never
 * exportable, in StrongBox when the phone has it and in the TEE otherwise. It needs no screen
 * unlock (mail syncs in the background), so it is not tied to the lock screen.
 *
 * **A missing key is not taken at its word.** On Android 8 to 11 (keystore1) `aliases()` returns an
 * empty set and `getKey` returns null when the service call fails, so "the alias is not listed"
 * proves nothing alone. A second tiny key, the *canary* (`<alias>.canary`), is made together with the
 * wrapping key. The main key counts as lost ([SecretBoxException.Kind.KEY_LOST]) only when, on
 * every one of [CHECKS] checks spaced by [checkDelaysMs]: `getKey` returns null, `aliases()`
 * answers with a non-empty list that lacks the main alias, and the canary is listed and does an
 * encryption. A keystore that cannot list the canary is a keystore that is not answering.
 * - an empty or failing `aliases()`, an alias that is listed but not returned, a canary that
 *   fails, or any mixed answer is UNAVAILABLE;
 * - main and canary both missing (the whole keystore looks wiped, or it is not answering) is
 *   UNAVAILABLE with [SecretBoxException.weak] set: the caller acts on it only when it persists;
 * - on API below [CONFIRMED_LOSS_SDK] the canary evidence is [SecretBoxException.weak] too, for
 *   the same reason: the caller needs it to persist across several starts and an hour.
 * A `KeyPermanentlyInvalidatedException` (or an API 33 key-specific error) is definite at once
 * ([KeystoreErrors]). Other failures are mapped by [KeystoreErrors].
 *
 * Nothing in here creates a key except [create], and [create] is called by `AppSecrets` only on
 * the first run and in the reset after a declared loss. The waits between checks happen **outside**
 * the monitor: each single check is synchronized, the sleep is not, so a caller that needs the key
 * for something else is not held up by another thread's back-off. The class must not be called from
 * the main thread (a check, with its waits, takes more than half a second).
 */
class KeystoreKeySource internal constructor(
    private val alias: String,
    private val ops: KeystoreOps,
    private val errors: KeystoreErrors,
    private val checkDelaysMs: List<Long>,
    private val sleep: (Long) -> Unit,
    private val sdkInt: Int = Build.VERSION.SDK_INT,
) : WrappingKeySource {
    constructor(context: Context, alias: String = DEFAULT_ALIAS) :
        this(alias, AndroidKeystoreOps(context.applicationContext), KeystoreErrors(), DEFAULT_CHECK_DELAYS_MS, Thread::sleep)

    private val canaryAlias = alias + CANARY_SUFFIX

    override fun keyForSealing(): SecretKey = find()

    override fun keyForOpening(): SecretKey = find()

    @Synchronized
    override fun create(): SecretKey = try {
        // The canary first: a crash between the two leaves a canary without a main key, which the
        // next first run or reset replaces; the other order would leave a main key whose absence
        // could never be told from an outage.
        ops.generate(canaryAlias)
        ops.generate(alias)
    } catch (e: Exception) {
        throw SecretBoxException(SecretBoxException.Kind.UNAVAILABLE)
    }

    @Synchronized
    override fun discard() {
        try {
            ops.delete(alias)
            ops.delete(canaryAlias)
        } catch (e: Exception) {
            throw SecretBoxException(SecretBoxException.Kind.UNAVAILABLE)
        }
    }

    /** What one check found. */
    private sealed interface Check {
        class Found(val key: SecretKey) : Check

        /** Main absent, canary listed and usable: the evidence of a lost key. */
        object Lost : Check

        /** Main and canary both look absent (or the list is empty): not provable either way. */
        object Absent : Check

        /** Anything else: failure, mixed answer, listed but not returned. */
        object Unsure : Check
    }

    /**
     * The key, or [SecretBoxException]: KEY_LOST when the platform says the key is dead or the
     * checks agree it is gone; UNAVAILABLE for anything undecided. Never returns null and never
     * creates a key.
     */
    private fun find(): SecretKey {
        var lost = 0
        var absent = 0
        for (attempt in 0 until CHECKS) {
            // Outside the monitor: other callers are not held up by this back-off.
            if (attempt > 0) sleep(checkDelaysMs.getOrElse(attempt - 1) { checkDelaysMs.lastOrNull() ?: 0L })
            when (val result = checkOnce()) {
                is Check.Found -> return result.key
                is Check.Lost -> lost++
                is Check.Absent -> absent++
                is Check.Unsure -> Unit
            }
        }
        // Definite platform evidence is thrown out of checkOnce; everything below is the outcome of
        // three clean checks that all pointed one way.
        if (lost == CHECKS) throw SecretBoxException(SecretBoxException.Kind.KEY_LOST, weak = sdkInt < CONFIRMED_LOSS_SDK)
        if (lost + absent == CHECKS) throw SecretBoxException(SecretBoxException.Kind.UNAVAILABLE, weak = true)
        throw SecretBoxException(SecretBoxException.Kind.UNAVAILABLE)
    }

    /** One look at the keystore, under the monitor. */
    @Synchronized
    private fun checkOnce(): Check {
        try {
            ops.getKey(alias)?.let { return Check.Found(it) }
        } catch (e: Exception) {
            if (errors.classify(e) == SecretBoxException.Kind.KEY_LOST) {
                throw SecretBoxException(SecretBoxException.Kind.KEY_LOST)
            }
            return Check.Unsure
        }
        // Null: the alias is gone, or the service did not answer. Ask something independent.
        val listed = try {
            ops.aliases()
        } catch (e: Exception) {
            return Check.Unsure
        }
        if (listed.isEmpty()) return Check.Absent // an answer that cannot be told from a failure
        if (alias in listed) return Check.Unsure // listed but not returned: the service is struggling
        if (canaryAlias !in listed) return Check.Absent
        return try {
            if (ops.canaryWorks(canaryAlias)) Check.Lost else Check.Unsure
        } catch (e: Exception) {
            Check.Unsure
        }
    }

    override fun toString(): String = "[redacted]"

    companion object {
        /** The alias of the wrapping key. One key wraps the database key and every account secret. */
        const val DEFAULT_ALIAS = "croft.wrapping.v1"

        /** Added to the wrapping key's alias for the canary key. */
        const val CANARY_SUFFIX = ".canary"

        /** How many times in a row the evidence must agree before it counts. */
        internal const val CHECKS = 3

        /** Waits before the second and the third check. */
        internal val DEFAULT_CHECK_DELAYS_MS = listOf(100L, 500L)

        /** From Android 12 (API 31) the keystore2 service reports failures as errors, not as an empty answer. */
        internal const val CONFIRMED_LOSS_SDK = 31
    }
}

/** The platform's Keystore. */
internal class AndroidKeystoreOps(private val context: Context) : KeystoreOps {
    private fun keyStore(): KeyStore = KeyStore.getInstance(PROVIDER).apply { load(null) }

    override fun getKey(alias: String): SecretKey? = keyStore().getKey(alias, null) as? SecretKey

    override fun aliases(): List<String> = keyStore().aliases().toList()

    override fun canaryWorks(alias: String): Boolean {
        val key = keyStore().getKey(alias, null) as? SecretKey ?: return false
        val cipher = Cipher.getInstance("AES/GCM/NoPadding")
        cipher.init(Cipher.ENCRYPT_MODE, key)
        return cipher.doFinal(ByteArray(1)).isNotEmpty()
    }

    override fun delete(alias: String) = keyStore().deleteEntry(alias)

    override fun generate(alias: String): SecretKey {
        val strongBox = Build.VERSION.SDK_INT >= Build.VERSION_CODES.P &&
            context.packageManager.hasSystemFeature(PackageManager.FEATURE_STRONGBOX_KEYSTORE)
        return try {
            generate(alias, strongBox)
        } catch (e: RuntimeException) {
            // The feature flag said yes but the secure element refused: use the TEE. The class
            // is tested by value, not in a catch clause, so API 26 and 27 never load it.
            if (strongBox && e is StrongBoxUnavailableException) generate(alias, false) else throw e
        }
    }

    private fun generate(alias: String, strongBox: Boolean): SecretKey {
        val spec = KeyGenParameterSpec.Builder(
            alias,
            KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT,
        )
            .setKeySize(256)
            .setBlockModes(KeyProperties.BLOCK_MODE_GCM)
            .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
            // The cipher picks a fresh IV for every seal; a fixed IV is refused.
            .setRandomizedEncryptionRequired(true)
            .apply {
                if (strongBox && Build.VERSION.SDK_INT >= Build.VERSION_CODES.P) setIsStrongBoxBacked(true)
            }
            .build()
        val generator = KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, PROVIDER)
        generator.init(spec)
        return generator.generateKey()
    }

    override fun toString(): String = "[redacted]"

    private companion object {
        const val PROVIDER = "AndroidKeyStore"
    }
}
