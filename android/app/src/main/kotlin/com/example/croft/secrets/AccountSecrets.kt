// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.secrets

import java.io.IOException
import kotlinx.coroutines.CoroutineDispatcher
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import uniffi.croft_ffi.Core
import uniffi.croft_ffi.FfiCredential
import uniffi.croft_ffi.FfiDiscovered

/**
 * Adding and removing an account together with its stored secrets, in the order that is safe if
 * the app is killed half way.
 *
 * - **Add.** The new account's password goes into `Core.addAccount`; the core uses it for the
 *   account's first connection and never asks the credential provider about a new account. Only
 *   when the call has returned is the password stored under the returned id (after clearing any
 *   entry under that id; the core never reuses an id, so there should be none). When
 *   `addAccount` fails nothing is stored. A crash between the two steps leaves an account that
 *   needs a sign-in at the next start, which is safe. If storing fails, the account is removed
 *   again so that no account exists without its secret.
 * Both calls are suspend functions that run on the IO dispatcher (the Keystore can make them wait).
 * - **Remove.** The secrets are deleted **before** `Core.removeAccount`. A crash in between leaves
 *   an account that needs a sign-in, which is safe; the other order could leave a secret behind.
 */
class AccountSecrets(
    private val store: SecretStore,
    private val signInNeeded: SignInNeeded,
    private val dispatcher: CoroutineDispatcher = Dispatchers.IO,
) {
    /**
     * Adds a password account. [password] is the UTF-8 secret the person just verified; it is
     * copied into the core and into the store, and the caller wipes its own array afterwards.
     * Returns the new account's id. Throws the core's exception when it refuses, and
     * [SecretBoxException] or [IOException] when the secret cannot be stored (the account is then
     * removed again). Runs on the IO dispatcher: storing waits on the Keystore.
     */
    suspend fun addPasswordAccount(
        core: Core,
        discovered: FfiDiscovered,
        displayName: String,
        pinnedFingerprint: String?,
        password: ByteArray,
    ): Long = withContext(dispatcher) {
        // The copy the credential is built from is wiped whatever happens.
        val copy = password.copyOf()
        val id = try {
            val credential = FfiCredential.password(copy)
            try {
                core.addAccount(discovered, displayName, pinnedFingerprint, credential)
            } finally {
                credential.close()
            }
        } finally {
            copy.fill(0)
        }
        try {
            store.putForNewAccount(id, SecretKind.PASSWORD, password)
        } catch (e: Exception) {
            if (e !is SecretBoxException && e !is IOException) throw e
            // Do not keep an account whose password could not be kept: it could never sign in again.
            try {
                store.removeAccount(id)
                core.removeAccount(id)
            } catch (ignored: Exception) {
                // Best effort; the account then needs a sign-in, which is what it would show anyway.
            }
            throw e
        }
        id
    }

    /** Removes the account's secrets, then the account from the core (in that order, see the class text). */
    suspend fun removeAccount(core: Core, accountId: Long) = withContext(dispatcher) {
        store.removeAccount(accountId)
        core.removeAccount(accountId)
        signInNeeded.clear(accountId)
    }

    override fun toString(): String = "[redacted]"
}
