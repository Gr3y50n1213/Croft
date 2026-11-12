// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.mail.accounts

import com.example.croft.mail.AccountInfo
import com.example.croft.mail.AccountState

/**
 * The text "Copy diagnostics" (S60) puts on the clipboard. It is built from numbers and fixed words only: versions, the number of
 * accounts and each one's state in list order. **Never** an address, a server name, a folder name, a subject, an account's name
 * or any secret: nothing of an account is read here except its state and whether it is the sample mailbox. A test checks that.
 */
object Diagnostics {
    /** The report for these versions and [accounts]. */
    fun build(appVersion: String, androidVersion: String, ffiApiVersion: String, accounts: List<AccountInfo>): String = buildString {
        appendLine("Croft diagnostics")
        appendLine("App version: $appVersion")
        appendLine("Android: $androidVersion")
        appendLine("FFI API version: $ffiApiVersion")
        appendLine("Accounts: ${accounts.size}")
        accounts.forEachIndexed { index, account ->
            append("Account ${index + 1}: ").append(word(account.status))
            if (account.isDemo) append(" (sample mail)")
            appendLine()
        }
    }

    private fun word(state: AccountState) = when (state) {
        AccountState.OK -> "ok"
        AccountState.SYNCING -> "syncing"
        AccountState.OFFLINE -> "offline"
        AccountState.NEEDS_SIGN_IN -> "needs sign-in"
        AccountState.ERROR -> "error"
    }
}
