// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.secrets

import uniffi.croft_ffi.KeyVault
import uniffi.croft_ffi.ProviderException

/**
 * The core's key vault (T42a): its OpenPGP secret keys, kept in [store] (sealed with the Android Keystore key, in the app's
 * `noBackupFilesDir`). The core calls it on its own threads, never the UI thread.
 *
 * It wipes the array the core hands over once it is sealed. A Keystore that cannot answer now (or whose loss is only suspected)
 * is `ProviderException.NotReady`: the core then makes, imports or uses no key, and nothing is deleted. A definite loss reads as
 * "no secret" (it is gone). An entry that does not open (damaged) is `ProviderException.Failed` and is kept. Anything else is
 * `ProviderException.Failed`. Nothing here logs, and no exception carries a secret.
 *
 * The array [secretKey] returns crosses into the core and cannot be wiped from here (UniFFI copies it; the Java copy is left to the
 * garbage collector): noted in OPEN-ITEMS.
 */
class SecretStoreKeyVault(private val store: SecretStore) : KeyVault {
    override fun storeSecretKey(accountId: Long, fingerprint: String, secret: ByteArray) {
        try {
            store.putKey(accountId, fingerprint, secret)
        } catch (e: SecretBoxException) {
            throw notReadyOrFailed(e)
        } catch (e: IllegalArgumentException) {
            throw ProviderException.Failed()
        } catch (e: java.io.IOException) {
            throw ProviderException.Failed()
        } finally {
            secret.fill(0)
        }
    }

    override fun secretKey(accountId: Long, fingerprint: String): ByteArray? = try {
        store.getKey(accountId, fingerprint)
    } catch (e: SecretBoxException) {
        if (e.kind == SecretBoxException.Kind.KEY_LOST && !e.weak) null else throw notReadyOrFailed(e)
    } catch (e: IllegalArgumentException) {
        throw ProviderException.Failed()
    } catch (e: java.io.IOException) {
        throw ProviderException.Failed()
    }

    override fun deleteSecretKey(accountId: Long, fingerprint: String) {
        try {
            store.removeKey(accountId, fingerprint)
        } catch (e: IllegalArgumentException) {
            throw ProviderException.Failed()
        } catch (e: java.io.IOException) {
            throw ProviderException.Failed()
        }
    }

    private fun notReadyOrFailed(e: SecretBoxException): ProviderException =
        if (e.kind == SecretBoxException.Kind.UNAVAILABLE || e.weak) ProviderException.NotReady() else ProviderException.Failed()

    override fun toString(): String = "[redacted]"
}
