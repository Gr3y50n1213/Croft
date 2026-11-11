// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.secrets

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
 * - **Remove.** The secrets are deleted **before** `Core.removeAccount`. A crash in between leaves
 *   an account that needs a sign-in, which is safe; the other order could leave a secret behind.
 */
class AccountSecrets(private val store: SecretStore, private val signInNeeded: SignInNeeded) {
    /**
     * Adds a password account. [password] is the UTF-8 secret the person just verified; it is
     * copied into the core and into the store, and the caller wipes its own array afterwards.
     * Returns the new account's id. Throws the core's exception when it refuses, and
     * [SecretBoxException] when the secret cannot be stored (the account is then removed again).
     */
    suspend fun addPasswordAccount(
        core: Core,
        discovered: FfiDiscovered,
        displayName: String,
        pinnedFingerprint: String?,
        password: ByteArray,
    ): Long {
        val credential = FfiCredential.password(password.copyOf())
        val id = try {
            core.addAccount(discovered, displayName, pinnedFingerprint, credential)
        } finally {
            credential.close()
        }
        try {
            store.putForNewAccount(id, SecretKind.PASSWORD, password)
        } catch (e: SecretBoxException) {
            // Do not keep an account whose password could not be kept: it could never sign in again.
            try {
                store.removeAccount(id)
                core.removeAccount(id)
            } catch (ignored: Exception) {
                // Best effort; the account then needs a sign-in, which is what it would show anyway.
            }
            throw e
        }
        return id
    }

    /** Removes the account's secrets, then the account from the core (in that order, see the class text). */
    suspend fun removeAccount(core: Core, accountId: Long) {
        store.removeAccount(accountId)
        core.removeAccount(accountId)
        signInNeeded.clear(accountId)
    }

    override fun toString(): String = "[redacted]"
}
