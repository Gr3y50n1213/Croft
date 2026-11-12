// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.notify

import android.content.Context
import android.content.Intent

/** What a tap on a notification asks the app to open (S32). */
sealed interface NotificationOpen {
    /**
     * `croft://message/<account>/<id>/<uid>`: the message, with the Inbox under it. The app checks the row is still that Inbox message
     * of that account ([uid]) before it opens it: a deleted message's row id can be given to a new one.
     */
    data class Message(val account: Long, val messageId: Long, val uid: UInt) : NotificationOpen

    /** `croft://reply/<account>/<id>/<uid>`: compose, filled in as a reply (checked the same way). */
    data class Reply(val account: Long, val messageId: Long, val uid: UInt) : NotificationOpen

    /** `croft://inbox`: the Inbox (the summary, the test notification, "Sign in again"). */
    data object Inbox : NotificationOpen
}

/**
 * Reads a notification's tap. Only an intent that came through the **non-exported** `OpenFromNotification` alias counts: another app can
 * start `MainActivity` (it is exported for `mailto:` and shares) but cannot reach the alias, so it cannot send Croft to a message.
 */
object NotificationIntents {
    fun parse(intent: Intent?, context: Context): NotificationOpen? {
        if (intent == null) return null
        val component = intent.component ?: return null
        if (component.packageName != context.packageName || component.className != AndroidNotifier.OPEN_ALIAS) return null
        val uri = intent.data ?: return null
        if (uri.scheme != "croft") return null
        val parts = uri.pathSegments
        val account = parts.getOrNull(0)?.toLongOrNull()
        val message = parts.getOrNull(1)?.toLongOrNull()
        val uid = parts.getOrNull(2)?.toUIntOrNull()
        val ok = account != null && message != null && message > 0 && uid != null
        return when (uri.host) {
            "message" -> if (ok) NotificationOpen.Message(account!!, message!!, uid!!) else NotificationOpen.Inbox
            "reply" -> if (ok) NotificationOpen.Reply(account!!, message!!, uid!!) else NotificationOpen.Inbox
            "inbox" -> NotificationOpen.Inbox
            else -> null
        }
    }
}
