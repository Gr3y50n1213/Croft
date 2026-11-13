// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.mail.reading

import com.example.croft.mail.OpId

/** T37: how S31 unsubscribes (the core decides, from the message's `List-Unsubscribe`). */
enum class UnsubscribeWay { ONE_CLICK, EMAIL, WEBSITE }

/**
 * T37: what S21's banner and S31 show, and what the confirmed call hands back (the core does nothing if the message no longer gives
 * the same). [host] for one-click and website; [address], [subject] and [body] for email; [senderAddress] is what "Also delete" goes by.
 */
data class UnsubscribeInfoData(
    val way: UnsubscribeWay,
    val host: String?,
    val address: String?,
    val sender: String,
    val olderCount: Int,
    val already: Boolean,
    val senderAddress: String? = null,
    val subject: String? = null,
    val body: String? = null,
) {
    // Hosts and addresses are shown on screen but never logged.
    override fun toString(): String = "UnsubscribeInfoData(way=$way, older=$olderCount, already=$already)"
}

/** T37: why unsubscribing did not happen. */
enum class UnsubscribeProblem { UNREACHABLE, REFUSED, CANNOT_SEND, CHANGED, NO_TRASH_FOLDER }

/**
 * T37: what unsubscribing did: [done]; or [openUrl], a page for the app to open in the browser (the person already confirmed in S31);
 * or [problem]. [deletedOps] are the older messages moved to Trash, for Undo.
 */
data class UnsubscribeResultData(
    val done: Boolean,
    val openUrl: String?,
    val problem: UnsubscribeProblem?,
    val deletedOps: List<OpId>,
    /** Unsubscribed, but moving the older mail to Trash failed. */
    val deleteFailed: Boolean = false,
) {
    override fun toString(): String = "UnsubscribeResultData(done=$done, problem=$problem, deleted=${deletedOps.size})"
}
