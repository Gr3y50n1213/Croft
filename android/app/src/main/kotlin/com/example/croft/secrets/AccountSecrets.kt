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
import uniffi.croft_ffi.CoreException
import uniffi.croft_ffi.FfiCredential
import uniffi.croft_ffi.FfiDiscovered

/**
 * Adding and removing an account together with its stored secrets, in the order that is safe if
 * the app is killed half way.
 *
 * - **Add.** The new account's password goes into `Core.addAccount`; the core uses it for the
 *   account's first connection and never asks the credential provider about a new account. Only
 *   when the call has returned is the password stored under the returned id (after clearing any
 *   entry under that id; the core does not reuse an id while its database exists, and an erase deletes every secret with the
 *   database, so there should be none). When
 *   `addAccount` fails nothing is stored. A crash between the two steps leaves an account that
 *   needs a sign-in at the next start, which is safe. If storing fails, the account is removed
 *   again so that no account exists without its secret.
 * - **Add again.** The core answers an address that is already added, with the same servers, with
 *   that account's id and `updated` set (it keeps the account and starts it with the new password;
 *   the core, not this class, decides which it was). The new password then **replaces** the old one
 *   under that id (the old entry stays if the replacement cannot be sealed), and the account's
 *   sign-in-needed mark is cleared. If storing fails the core's update is undone
 *   (`Core.rollbackAccountUpdate`): the account stays as it was, with its synced mail, its old
 *   settings and its old secret, and [SignInNotSavedException] is thrown for the screen to say so. Each update has a token and the
 *   rollback only undoes that update, never a later sign-in's; if it could not be undone the plain storage failure is thrown instead.
 * Both calls are suspend functions that run on the IO dispatcher (the Keystore can make them wait).
 * - **Remove.** `Core.removeAccount` is called first (it stops the account's task and waits for a message that is being handed to the
 *   server, so nothing new is sent after it returns), then the account's secrets are deleted. A crash in between leaves a secret for an
 *   id the core no longer lists; the sweep at the next start (`AccountSweep`) deletes it. The order is the owner's decision of 11 November
 *   (docs/DECISIONS.md). An id is not given to another account while the database exists (the core keeps a high-water mark, tested);
 *   after "Erase all Croft data" the database and every secret are gone together. The quarantine (T11) is not touched by a single
 *   removal: its files belong to an earlier database whose account numbers may repeat these, and it expires by itself.
 */
class AccountSecrets(
    private val store: SecretStore,
    private val signInNeeded: SignInNeeded,
    private val dispatcher: CoroutineDispatcher = Dispatchers.IO,
    /** Told "core" after the core let the account go and "secrets" after its secrets were deleted; for tests of the order. */
    private val onStep: (String) -> Unit = {},
) {
    /**
     * Adds a password account. [password] is the UTF-8 secret the person just verified; it is
     * copied into the core and into the store, and the caller wipes its own array afterwards.
     * Returns the account's id: a new one, or the existing account's when the address was already
     * added with the same servers (see the class text; the core says which, `AddedAccount.updated`).
     * [confirmedFingerprint] is passed to the core as it is: the certificate the person confirmed on the
     * changed-certificate warning, or null. Throws the core's exception when it refuses
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
        confirmedFingerprint: String?,
        password: ByteArray,
    ): Long = add(core, discovered, displayName, pinnedFingerprint, confirmedFingerprint, password, SecretKind.PASSWORD)

    /**
     * Adds a bring-your-own OAuth account (T21) with the sign-in secret `Core.oauthFinish` returned: exactly like
     * [addPasswordAccount], with the secret stored as [SecretKind.OAUTH_REFRESH] (a password left from an earlier sign-in of the same
     * account is removed, so it cannot answer instead). The caller wipes [secret] afterwards.
     */
    suspend fun addOAuthAccount(
        core: Core,
        discovered: FfiDiscovered,
        displayName: String,
        secret: ByteArray,
    ): Long = add(core, discovered, displayName, null, null, secret, SecretKind.OAUTH_REFRESH)

    private suspend fun add(
        core: Core,
        discovered: FfiDiscovered,
        displayName: String,
        pinnedFingerprint: String?,
        confirmedFingerprint: String?,
        password: ByteArray,
        kind: SecretKind,
    ): Long = withContext(dispatcher) {
        // The copy the credential is built from is wiped whatever happens.
        val copy = password.copyOf()
        val added = try {
            val credential = if (kind == SecretKind.OAUTH_REFRESH) FfiCredential.oauthStored(copy) else FfiCredential.password(copy)
            try {
                core.addAccount(discovered, displayName, pinnedFingerprint, confirmedFingerprint, credential)
            } finally {
                credential.close()
            }
        } finally {
            copy.fill(0)
        }
        val id = added.id
        try {
            if (added.updated) {
                store.put(id, kind, password)
                // No secret left from another kind of sign-in may answer instead of this one.
                SecretKind.entries.filter { it != kind }.forEach { store.remove(id, it) }
                signInNeeded.clear(id)
                // Stored: the core no longer needs to keep what it would undo.
                try {
                    core.commitAccountUpdate(id, added.updateToken)
                } catch (ignored: Exception) {
                    // It only drops a note kept in memory.
                }
            } else {
                store.putForNewAccount(id, kind, password)
            }
        } catch (e: Exception) {
            if (e !is SecretBoxException && e !is IOException) throw e
            if (added.updated) {
                // The account exists, with its mail and its old secret (a failed `put` leaves the old entry): the core's update (new
                // settings, new credential in memory) is undone, only if it is still the account's latest update, so that the account
                // and its stored password agree again. Then "nothing was changed" is true and the screen says so. If the undo did not
                // happen (a later sign-in of the account stands, or the undo failed), that is not true: the ordinary storage failure.
                val undone = try {
                    core.rollbackAccountUpdate(id, added.updateToken)
                } catch (ignored: Exception) {
                    false
                }
                if (undone) throw SignInNotSavedException()
                throw e
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

    /**
     * Removes the account: the account in the core first, then its secrets (see the class text). An account the core no longer has (a
     * removal that was cut short) is not an error: what is left of it is deleted. Any other failure of the core leaves the account and its
     * secrets in place, so the removal can be repeated.
     */
    suspend fun removeAccount(core: Core, accountId: Long) = withContext(dispatcher) {
        try {
            core.removeAccount(accountId)
        } catch (e: CoreException.NoSuchAccount) {
            // Already gone: finish the rest.
        }
        onStep("core")
        store.removeAccount(accountId)
        signInNeeded.clear(accountId)
        onStep("secrets")
    }

    override fun toString(): String = "[redacted]"
}
