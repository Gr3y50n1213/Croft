// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.secrets

import android.os.Build
import android.security.keystore.KeyPermanentlyInvalidatedException
import androidx.annotation.RequiresApi
import javax.crypto.AEADBadTagException

/** What an `android.security.KeyStoreException` (Android 13 and later) says, without the class. */
internal class PlatformKeystoreError(val transient: Boolean, val numericCode: Int)

/**
 * Decides what a failure from the Android Keystore means. The platform reports many transient
 * failures (the keystore service restarting, a busy secure element, a hardware error) as
 * [java.security.UnrecoverableKeyException], a plain [java.security.InvalidKeyException] or
 * [javax.crypto.IllegalBlockSizeException]; on Android 8 to 11 a failed connection to the service
 * even makes `getKey` return null. None of those is evidence that the key is gone. The mapping,
 * which decides whether data is moved aside or kept, is therefore:
 *
 * - [SecretBoxException.Kind.KEY_LOST]: a [KeyPermanentlyInvalidatedException] anywhere in the cause
 *   chain; or, from Android 13 (API 33), an `android.security.KeyStoreException` anywhere in the
 *   chain that is not transient and whose code names the key itself (does not exist, corrupted).
 *   (The third kind of evidence, an alias missing on several spaced checks, is gathered by
 *   [KeystoreKeySource].)
 * - [SecretBoxException.Kind.CORRUPT]: only [AEADBadTagException], the authentication failure.
 * - [SecretBoxException.Kind.UNAVAILABLE]: everything else, including every other exception type.
 *
 * The platform lookup is a parameter so that the rules run on the JVM with fake exceptions.
 */
internal class KeystoreErrors(
    private val sdkInt: Int = Build.VERSION.SDK_INT,
    private val platformError: (Throwable) -> PlatformKeystoreError? = ::androidKeystoreError,
) {
    /** The kind of [SecretBoxException] to raise for [failure]. Never UNSUPPORTED_VERSION. */
    fun classify(failure: Throwable): SecretBoxException.Kind {
        val chain = causeChain(failure)
        if (chain.any { it is KeyPermanentlyInvalidatedException }) return SecretBoxException.Kind.KEY_LOST
        if (sdkInt >= Build.VERSION_CODES.TIRAMISU && chain.any { isKeySpecific(platformError(it)) }) {
            return SecretBoxException.Kind.KEY_LOST
        }
        if (failure is AEADBadTagException) return SecretBoxException.Kind.CORRUPT
        return SecretBoxException.Kind.UNAVAILABLE
    }

    private fun isKeySpecific(error: PlatformKeystoreError?): Boolean =
        error != null && !error.transient && error.numericCode in KEY_SPECIFIC_CODES

    internal companion object {
        // android.security.KeyStoreException.ERROR_KEY_DOES_NOT_EXIST and ERROR_KEY_CORRUPTED. The
        // other codes (user authentication, permission, uninitialised keystore, keymint and
        // system errors, usage errors, validity window, expired operation) say nothing about the
        // key being gone. The numbers are copied because the class does not exist before API 33.
        const val ERROR_KEY_DOES_NOT_EXIST = 6
        const val ERROR_KEY_CORRUPTED = 7
        val KEY_SPECIFIC_CODES = setOf(ERROR_KEY_DOES_NOT_EXIST, ERROR_KEY_CORRUPTED)

        /** [failure] and its causes, outermost first (stops on a cycle). */
        fun causeChain(failure: Throwable): List<Throwable> {
            val out = ArrayList<Throwable>()
            var t: Throwable? = failure
            while (t != null && out.none { it === t }) {
                out += t
                t = t.cause
            }
            return out
        }
    }
}

/** The real lookup. Only called from API 33 on (the class does not exist before). */
internal fun androidKeystoreError(t: Throwable): PlatformKeystoreError? =
    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) readKeyStoreException(t) else null

@RequiresApi(Build.VERSION_CODES.TIRAMISU)
private fun readKeyStoreException(t: Throwable): PlatformKeystoreError? =
    (t as? android.security.KeyStoreException)?.let {
        PlatformKeystoreError(transient = it.isTransientFailure, numericCode = it.numericErrorCode)
    }
