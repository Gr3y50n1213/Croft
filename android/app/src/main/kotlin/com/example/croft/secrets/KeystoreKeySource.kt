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
import java.security.GeneralSecurityException
import java.security.KeyStore
import java.security.UnrecoverableKeyException
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey

/**
 * The AES-256-GCM wrapping key in the Android Keystore: generated inside the keystore, never
 * exportable, in StrongBox when the phone has it and in the TEE otherwise. It needs no screen
 * unlock (mail syncs in the background), so it is not tied to the lock screen.
 */
class KeystoreKeySource(
    private val context: Context,
    private val alias: String = DEFAULT_ALIAS,
) : WrappingKeySource {
    @Synchronized
    override fun keyForSealing(): SecretKey = existing() ?: generate()

    @Synchronized
    override fun keyForOpening(): SecretKey = existing() ?: throw SecretBoxException(SecretBoxException.Kind.KEY_LOST)

    @Synchronized
    override fun discard() {
        try {
            keyStore().deleteEntry(alias)
        } catch (e: GeneralSecurityException) {
            throw SecretBoxException(SecretBoxException.Kind.UNAVAILABLE)
        }
    }

    private fun keyStore(): KeyStore = KeyStore.getInstance(PROVIDER).apply { load(null) }

    private fun existing(): SecretKey? = try {
        keyStore().getKey(alias, null) as? SecretKey
    } catch (e: UnrecoverableKeyException) {
        // The entry is there but its key cannot be used any more.
        throw SecretBoxException(SecretBoxException.Kind.KEY_LOST)
    } catch (e: GeneralSecurityException) {
        throw SecretBoxException(SecretBoxException.Kind.UNAVAILABLE)
    } catch (e: RuntimeException) {
        throw SecretBoxException(SecretBoxException.Kind.UNAVAILABLE)
    }

    private fun generate(): SecretKey {
        val strongBox = Build.VERSION.SDK_INT >= Build.VERSION_CODES.P &&
            context.packageManager.hasSystemFeature(PackageManager.FEATURE_STRONGBOX_KEYSTORE)
        try {
            return try {
                generate(strongBox)
            } catch (e: RuntimeException) {
                // The feature flag said yes but the secure element refused: use the TEE. The class
                // is tested by value, not in a catch clause, so API 26 and 27 never load it.
                if (strongBox && e is StrongBoxUnavailableException) generate(false) else throw e
            }
        } catch (e: GeneralSecurityException) {
            throw SecretBoxException(SecretBoxException.Kind.UNAVAILABLE)
        } catch (e: RuntimeException) {
            throw SecretBoxException(SecretBoxException.Kind.UNAVAILABLE)
        }
    }

    private fun generate(strongBox: Boolean): SecretKey {
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

    companion object {
        /** The alias of the wrapping key. One key wraps the database key and every account secret. */
        const val DEFAULT_ALIAS = "croft.wrapping.v1"
        private const val PROVIDER = "AndroidKeyStore"
    }
}
