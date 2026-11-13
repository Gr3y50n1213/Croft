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
 * message that is being handed to the server, so nothing is sent after it returns), then the saved secret, then the app's own entries for
 * the id. [removeAccountAndSecrets] is `AccountSecrets.removeAccount`. A crash anywhere leaves an id the core no longer lists, and the
 * start-up sweep ([AccountSweep]) deletes what is left. An id is not given to another account while the database exists (the core keeps
 * a high-water mark, tested), so a leftover could never reach another account.
 *
 * **Erase** ("Erase all Croft data on this device"), in an order that leaves nothing restorable after a kill at any step:
 * 1. [eraseQuarantine] first: the earlier databases, wrapped secrets and wrapped key of a wrong key-loss call. With the quarantine gone, a
 *    start that finds no account secret has nothing to restore, whatever step the app was killed at afterwards;
 * 2. every account as [remove] does;
 * 3. the core is closed ([closeCore]);
 * 4. the database's side files and file, every secret, the wrapped key and the Keystore key ([eraseStorage], checked: it throws and
 *    keeps the Keystore key if anything survives);
 * 5. Croft's other files ([clearLocalFiles]).
 * After step 4 the next start is a fresh install, and nothing that was keyed by an account id survives (ids start again at 1 with the
 * new database). A step that fails stops the erase with its error; the call can be repeated.
 */
class CoreAccountOps(
    private val core: Core,
    private val removeAccountAndSecrets: suspend (AccountId) -> Unit,
    private val savedSignIn: (AccountId) -> SavedSignIn,
    /** Deletes the quarantine (`AppSecrets.eraseQuarantine`). */
    private val eraseQuarantine: () -> Unit,
    /** Closes the core and forgets it (nothing may use it afterwards; the next use opens a new one). */
    private val closeCore: () -> Unit,
    /** Deletes the database file and its side files, every secret and the Keystore key (`AppSecrets.eraseEverything`). */
    private val eraseStorage: () -> Unit,
    /** Deletes Croft's other files. */
    private val clearLocalFiles: () -> Unit,
    /** Ends this device's Companion session (best effort; its secret is erased with the rest either way). */
    private val endCompanion: suspend () -> Unit = {},
) : AccountOps {
    override suspend fun addSample(): AccountId = core.addDemoAccount().id

    override suspend fun savedSignIn(id: AccountId): SavedSignIn = savedSignIn.invoke(id)

    override suspend fun remove(id: AccountId) = removeAccountAndSecrets(id)

    override suspend fun eraseEverything() {
        // Deleting files waits on the disk and the Keystore: not on the main thread.
        withContext(Dispatchers.IO) { eraseQuarantine() }
        try {
            endCompanion()
        } catch (e: CancellationException) {
            throw e
        } catch (e: Exception) {
            // The secret goes with the rest; the device stays in the Companion's list until signed out there.
        }
        for (account in core.listAccounts()) {
            try {
                removeAccountAndSecrets(account.id)
            } catch (e: CancellationException) {
                throw e
            } catch (e: CoreException.NoSuchAccount) {
                // Already gone: that is what we wanted.
            }
        }
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
