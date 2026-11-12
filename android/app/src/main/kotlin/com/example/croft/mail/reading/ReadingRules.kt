// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.mail.reading

import com.example.croft.mail.MessageRowData

/** Rules of the conversation screen as pure functions, so they can be tested without a screen. */
object ReadingRules {
    /**
     * Reply all is offered only when more than one other person is on the message (docs/UX.md, toolbar enablement rules): the sender and
     * the people in To and Cc, without [myAddress] and without repeats. The recipients of a message are known once its body was loaded.
     */
    fun showReplyAll(message: ConversationMessageData, myAddress: String?): Boolean {
        val me = myAddress?.trim()?.lowercase()
        val others = (listOf(message.senderAddress) + message.to.map { it.address } + message.cc.map { it.address })
            .mapNotNull { it?.trim()?.lowercase()?.takeIf { a -> a.isNotEmpty() } }
            .filter { it != me }
            .toSet()
        return others.size > 1
    }

    /**
     * The messages an action of the action bar changes: those of the conversation that are in the same folder as the message that was
     * opened (a reply in Sent is not archived along with the Inbox messages), the opened one first.
     */
    fun targets(conversation: ConversationData): List<ConversationMessageData> {
        val opened = conversation.opened ?: return emptyList()
        return conversation.messages.filter { it.folder == opened.folder }.sortedByDescending { it.id == opened.id }
    }

    /** [message] as the list row the row actions take. */
    fun rowOf(message: ConversationMessageData, localOnly: Boolean): MessageRowData = MessageRowData(
        id = message.id, account = message.account, folder = message.folder, senderName = message.senderName, senderAddress = message.senderAddress,
        subject = message.subject, snippet = message.snippet, dateMs = message.dateMs, seen = message.seen, flagged = message.flagged,
        hasAttachments = message.hasAttachments, pending = message.pending, localOnly = localOnly,
    )
}
