// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.mail.accounts

import com.example.croft.mail.AccountId
import com.example.croft.mail.AccountInfo
import com.example.croft.mail.FolderInfo
import kotlinx.coroutines.CancellationException
import uniffi.croft_ffi.Core
import uniffi.croft_ffi.CoreException

/**
 * What S44, S45 and S60 change in the core and the secret store (listing goes through the session's repository). The screens talk
 * only to this, so they are tested over a recording version and the real thing over the real core on the JVM.
 */
interface AccountOps {
    /** "Try with sample mail": the core's own sample account (`add_demo_account`: no network, no credential; there is only ever one). Returns its id. */
    suspend fun addSample(): AccountId

    /**
     * Removes [id] from this device: its saved password first, then its rows in the core (see [CoreAccountOps]). The server is not
     * touched. Throws when the core refuses; the secret is gone by then, and the account stays and needs a sign-in (so the removal can be repeated).
     */
    suspend fun remove(id: AccountId)

    /** "Erase all Croft data on this device": every account as [remove] does, then every leftover secret and the app's own files of mail state. The app clears its settings stores itself. */
    suspend fun eraseEverything()
}

/**
 * [AccountOps] over the real [Core].
 *
 * **Removal order: the saved password, then the core's rows.** [removeAccountAndSecrets] is `AccountSecrets.removeAccount` of T11
 * (secrets first, then `Core.removeAccount`). If the app is killed between the two, the account is still there with no password: it
 * shows "Needs sign-in" and can be removed again; no password is ever left behind for an account that is gone. The other order could
 * leave a password for a deleted id. Ids are never reused (the core keeps a high-water mark, tested in the core), so even such a
 * leftover could never be given to another account; and `SecretStore.putForNewAccount` clears anything under a new id first.
 */
class CoreAccountOps(
    private val core: Core,
    private val removeAccountAndSecrets: suspend (AccountId) -> Unit,
    /** Deletes every account secret that is left (after the accounts are gone). */
    private val clearSecrets: () -> Unit,
    /** Deletes the app's own mail state files (folder choices, list positions). */
    private val clearLocalFiles: () -> Unit,
) : AccountOps {
    override suspend fun addSample(): AccountId = core.addDemoAccount().id

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
        clearSecrets()
        clearLocalFiles()
    }

    override fun toString(): String = "[redacted]"
}

/** What the removal dialog lists as deleted on this device, for one account. */
data class RemovalPlan(
    /** Messages stored on this device (the folders' totals). */
    val messages: Int,
    val folders: Int,
    /** A saved password exists for this account (the sample mailbox has none). */
    val savedPassword: Boolean,
    val sample: Boolean,
) {
    companion object {
        /** The plan for [account] with these [folders] (virtual mailboxes such as Starred are not stored folders and are left out). */
        fun of(account: AccountInfo, folders: List<FolderInfo>): RemovalPlan {
            val stored = folders.filter { !it.isVirtual }
            return RemovalPlan(stored.sumOf { it.totalCount }, stored.size, savedPassword = !account.isDemo, sample = account.isDemo)
        }
    }
}
