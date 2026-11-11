// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.secrets

import java.security.GeneralSecurityException
import java.security.InvalidKeyException
import java.security.UnrecoverableKeyException
import javax.crypto.AEADBadTagException
import javax.crypto.BadPaddingException
import javax.crypto.Cipher
import javax.crypto.IllegalBlockSizeException
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec

/**
 * Why a [SecretBox] could not seal or open something. The text is fixed on purpose: no message
 * here ever carries a secret, a context name or a cause's text.
 */
class SecretBoxException(val kind: Kind) : Exception(kind.name) {
    // No cause is chained on purpose: a provider's message can name key aliases or sizes.

    enum class Kind {
        /** The wrapping key is definitely gone (restore to a new phone, lock-screen reset, removed key). */
        KEY_LOST,

        /** The stored bytes are not a valid envelope or fail the authentication check (damage, tampering, wrong entry). */
        CORRUPT,

        /** The keystore failed in a way that may pass (busy, restarting, hardware error). Nothing must be deleted because of it. */
        UNAVAILABLE,
    }
}

/**
 * Seals small secrets into a blob that is safe to keep in app-private storage, and opens them
 * again. The real one is [AesGcmSecretBox] over an Android Keystore key; tests use the same class
 * over an in-memory key.
 *
 * [context] names what the blob is for (for example the account and kind). It is bound into the
 * authentication, so a blob copied onto another entry does not open.
 */
interface SecretBox {
    /** Returns the blob for [plain]. Does not keep or change [plain]; the caller wipes it. */
    fun seal(plain: ByteArray, context: ByteArray): ByteArray

    /** Returns the secret in [blob]. The caller wipes the result. Throws [SecretBoxException]. */
    fun open(blob: ByteArray, context: ByteArray): ByteArray
}

/** Where [AesGcmSecretBox] gets its AES-256 key: the Android Keystore in the app, memory in tests. */
interface WrappingKeySource {
    /** The key to seal with. Creates it on first use. Throws [SecretBoxException] (UNAVAILABLE) when that fails. */
    fun keyForSealing(): SecretKey

    /** The key to open with. Throws [SecretBoxException] with KEY_LOST when it does not exist any more. */
    fun keyForOpening(): SecretKey

    /** Forgets the key (used when the database and the secrets are reset after a key loss). */
    fun discard()
}

/**
 * AES-256-GCM envelope:
 *
 * ```
 * byte 0        format version (1)
 * bytes 1..12   the 12-byte IV the cipher chose (the Keystore requires randomised IVs)
 * bytes 13..    ciphertext followed by the 16-byte GCM tag
 * ```
 *
 * The context is the additional authenticated data. Nothing about the secret (not even its
 * length class) is kept outside the ciphertext.
 */
class AesGcmSecretBox(private val keys: WrappingKeySource) : SecretBox {
    override fun seal(plain: ByteArray, context: ByteArray): ByteArray {
        val key = keys.keyForSealing()
        try {
            val cipher = Cipher.getInstance(TRANSFORMATION)
            cipher.init(Cipher.ENCRYPT_MODE, key)
            cipher.updateAAD(context)
            val iv = cipher.iv
            check(iv.size == IV_BYTES) { "unexpected IV size" }
            val sealed = cipher.doFinal(plain)
            val out = ByteArray(1 + IV_BYTES + sealed.size)
            out[0] = VERSION
            iv.copyInto(out, 1)
            sealed.copyInto(out, 1 + IV_BYTES)
            sealed.fill(0)
            return out
        } catch (e: GeneralSecurityException) {
            throw classify(e)
        } catch (e: RuntimeException) {
            // Android wraps keystore failures (ProviderException and others) in unchecked exceptions.
            throw SecretBoxException(SecretBoxException.Kind.UNAVAILABLE)
        }
    }

    override fun open(blob: ByteArray, context: ByteArray): ByteArray {
        if (blob.size < 1 + IV_BYTES + TAG_BYTES || blob[0] != VERSION) {
            throw SecretBoxException(SecretBoxException.Kind.CORRUPT)
        }
        val key = keys.keyForOpening()
        try {
            val cipher = Cipher.getInstance(TRANSFORMATION)
            cipher.init(Cipher.DECRYPT_MODE, key, GCMParameterSpec(TAG_BYTES * 8, blob, 1, IV_BYTES))
            cipher.updateAAD(context)
            return cipher.doFinal(blob, 1 + IV_BYTES, blob.size - 1 - IV_BYTES)
        } catch (e: GeneralSecurityException) {
            throw classify(e)
        } catch (e: RuntimeException) {
            throw SecretBoxException(SecretBoxException.Kind.UNAVAILABLE)
        }
    }

    private fun classify(e: GeneralSecurityException): SecretBoxException {
        val kind = when (e) {
            is AEADBadTagException, is BadPaddingException, is IllegalBlockSizeException ->
                SecretBoxException.Kind.CORRUPT
            // KeyPermanentlyInvalidatedException is an InvalidKeyException.
            is InvalidKeyException, is UnrecoverableKeyException -> SecretBoxException.Kind.KEY_LOST
            else -> SecretBoxException.Kind.UNAVAILABLE
        }
        return SecretBoxException(kind)
    }

    override fun toString(): String = "[redacted]"

    private companion object {
        const val TRANSFORMATION = "AES/GCM/NoPadding"
        const val VERSION: Byte = 1
        const val IV_BYTES = 12
        const val TAG_BYTES = 16
    }
}
