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
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey

/** The Keystore calls [KeystoreKeySource] makes, so the decisions around them run on the JVM. */
internal interface KeystoreOps {
    /** `KeyStore.getKey(alias)`: the key, or null. Null does not prove the key is gone (Android 8 to 11 return it when the service is unreachable). May throw anything the platform throws. */
    fun getKey(alias: String): SecretKey?

    /** `KeyStore.aliases()`: a call that is independent of the key itself. */
    fun aliases(): List<String>

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
 * A missing key is not taken at its word. `getKey` returning null can be a keystore service that
 * is not reachable, so the key counts as absent ([SecretBoxException.Kind.KEY_LOST], or a new key
 * on first use) only when `KeyStore.aliases()` succeeds and lacks the alias on [CHECKS] tries in a
 * row, spaced by [checkDelaysMs]. Any failure or mixed answer is UNAVAILABLE. Other failures are
 * mapped by [KeystoreErrors].
 */
class KeystoreKeySource internal constructor(
    private val alias: String,
    private val ops: KeystoreOps,
    private val errors: KeystoreErrors,
    private val checkDelaysMs: List<Long>,
    private val sleep: (Long) -> Unit,
) : WrappingKeySource {
    constructor(context: Context, alias: String = DEFAULT_ALIAS) :
        this(alias, AndroidKeystoreOps(context.applicationContext), KeystoreErrors(), DEFAULT_CHECK_DELAYS_MS, Thread::sleep)

    @Synchronized
    override fun keyForSealing(): SecretKey {
        try {
            return find() ?: generate()
        } catch (e: SecretBoxException) {
            // The entry is there and is itself the dead key: it cannot be used for anything, so a
            // new key replaces it. Any other failure means "not now".
            if (e.kind == SecretBoxException.Kind.KEY_LOST) return generate()
            throw e
        }
    }

    @Synchronized
    override fun keyForOpening(): SecretKey = find() ?: throw SecretBoxException(SecretBoxException.Kind.KEY_LOST)

    @Synchronized
    override fun discard() {
        try {
            ops.delete(alias)
        } catch (e: Exception) {
            throw SecretBoxException(SecretBoxException.Kind.UNAVAILABLE)
        }
    }

    /**
     * The key; null when it is confirmed absent; [SecretBoxException] KEY_LOST when the platform
     * says the key is dead; UNAVAILABLE for anything undecided.
     */
    private fun find(): SecretKey? {
        var absent = 0
        for (attempt in 0 until CHECKS) {
            if (attempt > 0) sleep(checkDelaysMs.getOrElse(attempt - 1) { checkDelaysMs.lastOrNull() ?: 0L })
            try {
                ops.getKey(alias)?.let { return it }
                // Null: the alias is gone, or the service did not answer. Ask something else.
                absent = if (alias in ops.aliases()) 0 else absent + 1
            } catch (e: Exception) {
                if (errors.classify(e) == SecretBoxException.Kind.KEY_LOST) {
                    throw SecretBoxException(SecretBoxException.Kind.KEY_LOST)
                }
                absent = 0
            }
        }
        if (absent == CHECKS) return null
        throw SecretBoxException(SecretBoxException.Kind.UNAVAILABLE)
    }

    private fun generate(): SecretKey = try {
        ops.generate(alias)
    } catch (e: Exception) {
        throw SecretBoxException(SecretBoxException.Kind.UNAVAILABLE)
    }

    override fun toString(): String = "[redacted]"

    companion object {
        /** The alias of the wrapping key. One key wraps the database key and every account secret. */
        const val DEFAULT_ALIAS = "croft.wrapping.v1"

        /** How many times in a row the alias must be missing before the key counts as gone. */
        internal const val CHECKS = 3

        /** Waits before the second and the third check. */
        internal val DEFAULT_CHECK_DELAYS_MS = listOf(100L, 500L)
    }
}

/** The platform's Keystore. */
internal class AndroidKeystoreOps(private val context: Context) : KeystoreOps {
    private fun keyStore(): KeyStore = KeyStore.getInstance(PROVIDER).apply { load(null) }

    override fun getKey(alias: String): SecretKey? = keyStore().getKey(alias, null) as? SecretKey

    override fun aliases(): List<String> = keyStore().aliases().toList()

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
