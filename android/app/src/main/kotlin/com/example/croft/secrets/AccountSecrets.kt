// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.secrets

import com.example.croft.mail.onboarding.SignInNotSavedException
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
 * - **Add again.** The core answers an address that is already added, with the same servers, with
 *   that account's id and `updated` set (it keeps the account and starts it with the new password;
 *   the core, not this class, decides which it was). The new password then **replaces** the old one
 *   under that id (the old entry stays if the replacement cannot be sealed), and the account's
 *   sign-in-needed mark is cleared. If storing fails the core's update is undone
 *   (`Core.rollbackAccountUpdate`): the account stays as it was, with its synced mail, its old
 *   settings and its old secret, and [SignInNotSavedException] is thrown for the screen to say so.
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
     * Returns the account's id: a new one, or the existing account's when the address was already
     * added with the same servers (see the class text; the core says which, `AddedAccount.updated`).
     * [acceptChangedCertificate] is passed to the core as it is: true only after the person confirmed
     * the changed-certificate warning. Throws the core's exception when it refuses
     * (`CoreException.AccountExists` for the same address on other servers,
     * `CoreException.CertificateChanged` for a pin it would replace unconfirmed), and
     * [SecretBoxException] or [IOException] when the secret of a **new** account cannot be stored (the
     * account is then removed again). When the new password of an **existing** account cannot be stored,
     * the core's update is undone and [SignInNotSavedException] is thrown: the account is as it was.
     * Runs on the IO dispatcher: storing waits on the Keystore.
     */
    suspend fun addPasswordAccount(
        core: Core,
        discovered: FfiDiscovered,
        displayName: String,
        pinnedFingerprint: String?,
        acceptChangedCertificate: Boolean,
        password: ByteArray,
    ): Long = withContext(dispatcher) {
        // The copy the credential is built from is wiped whatever happens.
        val copy = password.copyOf()
        val added = try {
            val credential = FfiCredential.password(copy)
            try {
                core.addAccount(discovered, displayName, pinnedFingerprint, acceptChangedCertificate, credential)
            } finally {
                credential.close()
            }
        } finally {
            copy.fill(0)
        }
        val id = added.id
        try {
            if (added.updated) {
                store.put(id, SecretKind.PASSWORD, password)
                // A password sign-in: no token left from another kind of sign-in may answer instead of it.
                store.remove(id, SecretKind.OAUTH_ACCESS)
                store.remove(id, SecretKind.OAUTH_REFRESH)
                signInNeeded.clear(id)
            } else {
                store.putForNewAccount(id, SecretKind.PASSWORD, password)
            }
        } catch (e: Exception) {
            if (e !is SecretBoxException && e !is IOException) throw e
            if (added.updated) {
                // The account exists, with its mail and its old secret (a failed `put` leaves the old entry): the core's update (new
                // settings, new credential in memory) is undone so that the account and its stored password agree again.
                try {
                    core.rollbackAccountUpdate(id)
                } catch (ignored: Exception) {
                    // Best effort; the account then shows a sign-in problem at worst, with its old settings or the new ones.
                }
                throw SignInNotSavedException()
            }
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
