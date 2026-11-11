// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.ui.accounts

import android.content.Context
import com.example.croft.mail.AccountId
import com.example.croft.mail.MailRuntime
import com.example.croft.mail.accounts.AccountOps
import com.example.croft.mail.accounts.CoreAccountOps
import com.example.croft.mail.accounts.SavedSignIn
import com.example.croft.mail.onboarding.StorageUnavailableException
import com.example.croft.secrets.SecretKind
import kotlinx.coroutines.CancellationException

/**
 * [AccountOps] over the app's real parts: the core and the secret store of [MailRuntime], opened when the first call needs them (off the
 * main thread). Removal is `AccountSecrets.removeAccount` (the core's rows first, then the saved secret; see `CoreAccountOps`). If the core cannot be opened
 * every call fails with [StorageUnavailableException] and nothing is changed.
 */
fun appAccountOps(context: Context): AccountOps = LazyAccountOps(context.applicationContext)

private class LazyAccountOps(private val context: Context) : AccountOps {
    private suspend fun real(): AccountOps {
        val open = try {
            MailRuntime.openedCore(context)
        } catch (e: CancellationException) {
            throw e
        } catch (e: Exception) {
            throw StorageUnavailableException()
        } catch (e: UnsatisfiedLinkError) {
            throw StorageUnavailableException()
        }
        return CoreAccountOps(
            core = open.core,
            removeAccountAndSecrets = { id -> open.accountSecrets.removeAccount(open.core, id) },
            savedSignIn = { id ->
                val store = open.secrets.secrets
                when {
                    store.has(id, SecretKind.PASSWORD) -> SavedSignIn.PASSWORD
                    store.has(id, SecretKind.OAUTH_REFRESH) || store.has(id, SecretKind.OAUTH_ACCESS) -> SavedSignIn.TOKEN
                    else -> SavedSignIn.NONE
                }
            },
            closeCore = { open.core.close() },
            eraseStorage = { open.secrets.eraseEverything() },
            clearLocalFiles = { if (!MailRuntime.afterErase(context)) throw java.io.IOException("a file could not be deleted") },
        )
    }

    override suspend fun addSample(): AccountId = real().addSample()

    override suspend fun savedSignIn(id: AccountId): SavedSignIn = real().savedSignIn(id)

    override suspend fun remove(id: AccountId) = real().remove(id)

    override suspend fun eraseEverything() = real().eraseEverything()

    override fun toString(): String = "[redacted]"
}
