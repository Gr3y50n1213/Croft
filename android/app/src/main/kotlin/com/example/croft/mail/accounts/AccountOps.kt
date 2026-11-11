// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.mail.accounts

import com.example.croft.mail.AccountId
import com.example.croft.mail.AccountInfo
import com.example.croft.mail.FolderInfo
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import uniffi.croft_ffi.Core
import uniffi.croft_ffi.CoreException

/** What the removal path returns as "accounts left" when the account is gone but the count could not be read afterwards (not 0: the screen stays where it is). */
const val REMOVED_COUNT_UNKNOWN = -1

/** What sign-in a device keeps for an account: nothing (the sample mailbox), a password, or OAuth tokens (T21). */
enum class SavedSignIn { NONE, PASSWORD, TOKEN }

/**
 * What S44, S45 and S60 change in the core and the secret store (listing goes through the session's repository). The screens talk
 * only to this, so they are tested over a recording version and the real thing over the real core on the JVM.
 */
interface AccountOps {
    /** "Try with sample mail": the core's own sample account (`add_demo_account`: no network, no credential; there is only ever one). Returns its id. */
    suspend fun addSample(): AccountId

    /** What sign-in this device has saved for [id], read from the secret store (the removal list names it). */
    suspend fun savedSignIn(id: AccountId): SavedSignIn

    /**
     * Removes [id] from this device: the account in the core first (its task stops, nothing new is sent), then its saved secret and its
     * quarantined copies (see [CoreAccountOps]). The server is not touched. Throws when the core refuses; nothing is deleted then and the
     * removal can be repeated.
     */
    suspend fun remove(id: AccountId)

    /**
     * "Erase all Croft data on this device": every account as [remove] does, then the core is closed and its database deleted, then every
     * secret, the quarantine, the Keystore key and Croft's files (see [CoreAccountOps]). The app clears its settings stores itself. The
     * next start is a fresh install.
     */
    suspend fun eraseEverything()
}

/**
 * [AccountOps] over the real [Core].
 *
 * **Removal order** (the owner's decision, docs/DECISIONS.md): `Core.removeAccount` first (it stops the account's task and waits for a
 * message that is being handed to the server, so nothing is sent after it returns), then the saved secret and its quarantined copies,
 * then the app's own entries for the id. [removeAccountAndSecrets] is `AccountSecrets.removeAccount`, which writes a removal mark first;
 * a crash anywhere leaves the mark and the start-up sweep ([AccountSweep]) finishes the job. Ids are never reused (the core keeps a
 * high-water mark, tested), so a leftover could never reach another account.
 *
 * **Erase:** every account is removed, the core is closed ([closeCore]), the database, the secrets, the quarantine and the Keystore key
 * are deleted ([eraseStorage], `AppSecrets.eraseEverything`) and Croft's other files ([clearLocalFiles]). The next start is a fresh install.
 */
class CoreAccountOps(
    private val core: Core,
    private val removeAccountAndSecrets: suspend (AccountId) -> Unit,
    private val savedSignIn: (AccountId) -> SavedSignIn,
    /** Closes the core (nothing may use it afterwards). */
    private val closeCore: () -> Unit,
    /** Deletes the database file and its side files, every secret, the quarantine and the Keystore key. */
    private val eraseStorage: () -> Unit,
    /** Deletes Croft's other files, and forgets the closed core (the next use opens a new one). */
    private val clearLocalFiles: () -> Unit,
) : AccountOps {
    override suspend fun addSample(): AccountId = core.addDemoAccount().id

    override suspend fun savedSignIn(id: AccountId): SavedSignIn = savedSignIn.invoke(id)

    override suspend fun remove(id: AccountId) = removeAccountAndSecrets(id)

    override suspend fun eraseEverything() {
        for (account in core.listAccounts()) {
            try {
                removeAccountAndSecrets(account.id)
            } catch (e: CancellationException) {
                throw e
            } catch (e: CoreException.NoSuchAccount) {
                // Already gone: that is what we wanted.
            }
        }
        // Closing the core and deleting files wait on the disk and the Keystore: not on the main thread.
        withContext(Dispatchers.IO) {
            closeCore()
            eraseStorage()
            clearLocalFiles()
        }
    }

    override fun toString(): String = "[redacted]"
}

/** What the removal dialog lists as deleted on this device, for one account. */
data class RemovalPlan(
    /** Messages stored on this device (the folders' totals). */
    val messages: Int,
    val folders: Int,
    /** What sign-in this device has saved for the account (the sample mailbox has none). */
    val savedSignIn: SavedSignIn,
    val sample: Boolean,
) {
    companion object {
        /** The plan for [account] with these [folders] and this [savedSignIn] (virtual mailboxes such as Starred are not stored folders and are left out). */
        fun of(account: AccountInfo, folders: List<FolderInfo>, savedSignIn: SavedSignIn): RemovalPlan {
            val stored = folders.filter { !it.isVirtual }
            return RemovalPlan(stored.sumOf { it.totalCount }, stored.size, savedSignIn, sample = account.isDemo)
        }
    }
}
