// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.secrets

import javax.crypto.Cipher
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec

/**
 * Why a [SecretBox] could not seal or open something. The text is fixed on purpose: no message
 * here ever carries a secret, a context name or a cause's text.
 */
class SecretBoxException(val kind: Kind, val weak: Boolean = false) : Exception(kind.name) {
    // No cause is chained on purpose: a provider's message can name key aliases or sizes.

    // [weak] marks an answer that points at a lost key but is not proof: KEY_LOST on Android 8 to
    // 11 (where the keystore service answers "empty" when it fails), or UNAVAILABLE because the
    // key and the canary both look absent. The caller acts on it only after it has persisted
    // across several app starts over at least an hour (see `LossConfirmation`); until then it is
    // handled like UNAVAILABLE. A weak exception is never raised for the other two kinds.

    enum class Kind {
        /**
         * The wrapping key is definitely gone, on definite evidence only: the platform said the key
         * is permanently invalidated (or, on Android 13 and later, reported a key-specific error that
         * is not transient), or the alias was missing from the keystore's list on every one of
         * several spaced checks while a second, independent "canary" key made with it was listed and
         * usable (see [KeystoreErrors] and [KeystoreKeySource]). On Android 8 to 11 the last kind is
         * [SecretBoxException.weak]: the platform's service reports failure as "nothing there".
         */
        KEY_LOST,

        /**
         * The stored bytes are not a valid envelope (too short, version byte 0) or fail the
         * authentication check ([javax.crypto.AEADBadTagException]): damage, tampering or a blob
         * that belongs to another entry. Nothing else maps here.
         */
        CORRUPT,

        /**
         * The envelope's version byte is higher than this build knows: the data was written by a
         * newer Croft. It is never reset and never deleted; the open fails and the person is told.
         */
        UNSUPPORTED_VERSION,

        /**
         * Everything else: the keystore failed in a way that may pass (busy, restarting, hardware
         * error, or one of the many failures the platform reports as a plain key exception).
         * Nothing may be moved or deleted because of it; the caller retries with backoff.
         */
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
    /**
     * The key to seal with. **Never creates a key**: a missing key is a loss to report (a new key
     * under the same name would destroy the old one, and every secret sealed with it would read as
     * corrupt). Throws [SecretBoxException] like [keyForOpening].
     */
    fun keyForSealing(): SecretKey

    /**
     * Makes the wrapping key (and whatever the source keeps to tell a lost key from an unreachable
     * keystore), replacing an entry of the same name, and returns it. Only `AppSecrets` calls
     * it: on the very first run (no wrapped file of any kind exists) and in the reset after a
     * declared and quarantined key loss. Throws UNAVAILABLE when the key cannot be made.
     */
    fun create(): SecretKey

    /**
     * The key to open with. Throws [SecretBoxException] with KEY_LOST only on definite evidence
     * that it does not exist or cannot be used any more, and with UNAVAILABLE otherwise.
     */
    fun keyForOpening(): SecretKey

    /**
     * Deletes the key. A key loss does not call this (the entry is kept, in case the loss was a
     * wrong call and the quarantined data can be opened again); it is for a future "erase
     * everything".
     */
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
 * The additional authenticated data is the version byte followed by the context, so neither a
 * blob copied onto another entry nor a changed version byte authenticates. Nothing about the
 * secret (not even its length class) is kept outside the ciphertext.
 *
 * Binding the version byte into the AAD changes what version 1 means. That is done in place, on
 * purpose, and not as version 2: no build that wrote a version 1 blob has been released, so no
 * blob made without the version in the AAD exists. Any later change to the format must use a new
 * version byte.
 *
 * Failures are mapped by [errors] (see [KeystoreErrors]): [SecretBoxException.Kind.CORRUPT] only
 * for an authentication failure or this envelope's own format checks, KEY_LOST only on definite
 * evidence, UNSUPPORTED_VERSION for a higher version byte, and UNAVAILABLE for everything else.
 */
class AesGcmSecretBox internal constructor(
    private val keys: WrappingKeySource,
    private val errors: KeystoreErrors,
) : SecretBox {
    constructor(keys: WrappingKeySource) : this(keys, KeystoreErrors())

    override fun seal(plain: ByteArray, context: ByteArray): ByteArray {
        val key = keys.keyForSealing()
        try {
            val cipher = Cipher.getInstance(TRANSFORMATION)
            cipher.init(Cipher.ENCRYPT_MODE, key)
            cipher.updateAAD(aad(context))
            val iv = cipher.iv
            check(iv.size == IV_BYTES) { "unexpected IV size" }
            val sealed = cipher.doFinal(plain)
            val out = ByteArray(1 + IV_BYTES + sealed.size)
            out[0] = VERSION
            iv.copyInto(out, 1)
            sealed.copyInto(out, 1 + IV_BYTES)
            sealed.fill(0)
            return out
        } catch (e: Exception) {
            // Android wraps keystore failures (ProviderException and others) in unchecked exceptions.
            throw SecretBoxException(sealFailure(e))
        }
    }

    override fun open(blob: ByteArray, context: ByteArray): ByteArray {
        if (blob.isEmpty()) throw SecretBoxException(SecretBoxException.Kind.CORRUPT)
        // The version is read as unsigned: a higher one is a newer Croft's data, not damage.
        if ((blob[0].toInt() and 0xff) > (VERSION.toInt() and 0xff)) {
            throw SecretBoxException(SecretBoxException.Kind.UNSUPPORTED_VERSION)
        }
        if (blob.size < 1 + IV_BYTES + TAG_BYTES || blob[0] != VERSION) {
            throw SecretBoxException(SecretBoxException.Kind.CORRUPT)
        }
        val key = keys.keyForOpening()
        try {
            val cipher = Cipher.getInstance(TRANSFORMATION)
            cipher.init(Cipher.DECRYPT_MODE, key, GCMParameterSpec(TAG_BYTES * 8, blob, 1, IV_BYTES))
            cipher.updateAAD(aad(context))
            return cipher.doFinal(blob, 1 + IV_BYTES, blob.size - 1 - IV_BYTES)
        } catch (e: Exception) {
            throw SecretBoxException(errors.classify(e))
        }
    }

    /** Sealing has no authentication step, so an authentication failure cannot be CORRUPT here. */
    private fun sealFailure(e: Exception): SecretBoxException.Kind =
        when (val kind = errors.classify(e)) {
            SecretBoxException.Kind.CORRUPT -> SecretBoxException.Kind.UNAVAILABLE
            else -> kind
        }

    private fun aad(context: ByteArray): ByteArray {
        val out = ByteArray(1 + context.size)
        out[0] = VERSION
        context.copyInto(out, 1)
        return out
    }

    override fun toString(): String = "[redacted]"

    private companion object {
        const val TRANSFORMATION = "AES/GCM/NoPadding"
        const val VERSION: Byte = 1
        const val IV_BYTES = 12
        const val TAG_BYTES = 16
    }
}
