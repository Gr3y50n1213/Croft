// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.mail.compose

import com.example.croft.mail.AccountId

/** What the encryption policy says about a message about to be sent. */
sealed interface EncryptionDecision {
    /** Nothing to check: send. */
    data object NotRequired : EncryptionDecision

    /** Encryption is required and [missing] (addresses) have no key. The person must choose: Send unencrypted, or Cancel (S24). */
    data class RequiredKeysMissing(val missing: List<String>) : EncryptionDecision {
        override fun toString(): String = "RequiredKeysMissing(${missing.size})"
    }
}

/**
 * S24, the seam only: compose asks this before it sends ([ComposeController.send]). No keys exist before phase 3, so the app's
 * policy is [NoEncryptionPolicy] ("not required"). When a policy says [EncryptionDecision.RequiredKeysMissing] the screen opens the
 * encryption check, naming the recipients, and the message goes only after "Send unencrypted"; Cancel sends nothing. Encryption
 * never downgrades silently (CLAUDE.md).
 */
fun interface EncryptionPolicy {
    /** Whether a message from [account] to [recipients] (bare addresses of To, Cc and Bcc) may go as it is. */
    suspend fun check(account: AccountId, recipients: List<String>): EncryptionDecision
}

/** The default: encryption is not required. */
object NoEncryptionPolicy : EncryptionPolicy {
    override suspend fun check(account: AccountId, recipients: List<String>): EncryptionDecision = EncryptionDecision.NotRequired
}

/** For tests and screenshots: encryption is required and the given recipients have no key (the first recipient when none are named). */
class RequiredKeyMissingPolicy(private val missing: List<String>? = null) : EncryptionPolicy {
    override suspend fun check(account: AccountId, recipients: List<String>): EncryptionDecision {
        val names = missing ?: recipients.take(1)
        return if (names.isEmpty()) EncryptionDecision.NotRequired else EncryptionDecision.RequiredKeysMissing(names)
    }
}
