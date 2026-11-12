// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.notify

import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import uniffi.croft_ffi.AccountStatus
import uniffi.croft_ffi.Core

/** [BackgroundChecker] over the process's core (the same instance the screens use: [com.example.croft.mail.MailRuntime]). */
class CoreBackgroundChecker(private val core: Core) : BackgroundChecker {
    override suspend fun accounts(): List<CheckAccount> = withContext(Dispatchers.IO) {
        core.listAccounts().map {
            CheckAccount(
                id = it.id,
                label = it.label.ifBlank { it.email },
                needsSignIn = it.status == AccountStatus.NEEDS_SIGN_IN,
                runnable = it.status != AccountStatus.ERROR,
                isDemo = it.isDemo,
            )
        }
    }

    override suspend fun syncOnce(account: Long, deadlineMs: Long): CheckOutcome {
        val r = core.syncOnce(account, deadlineMs.coerceAtLeast(0).toULong())
        return CheckOutcome(
            newMail = r.newMail.map { NewMail(account, it.messageId, it.uid, it.senderName, it.senderAddress, it.subject, it.dateMs) },
            timedOut = r.timedOut,
            transientFailure = r.transientFailure,
            needsSignIn = r.needsSignIn,
        )
    }

    override suspend fun markNotified(account: Long, uids: List<UInt>) = withContext(Dispatchers.IO) { core.markNotified(account, uids) }
}
