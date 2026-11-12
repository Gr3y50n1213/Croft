// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.mail.reading

import com.example.croft.mail.AccountId
import com.example.croft.mail.MessageId
import java.io.File

/**
 * Everything the reading screens (S20, S21) ask of the core, shaped like its reading calls (`core/ffi/README.md`, "Reading a message").
 * Two implementations: [CoreReadingRepository] (the real core) and [FakeReadingRepository] (tests and screenshots). Every function is
 * `suspend` because the core's calls are not for the main thread. A refusal is a [ReadFailure] with a fixed category.
 */
interface ReadingRepository {
    /** The conversation of [messageId], oldest first (a local read; works offline). */
    suspend fun openConversation(messageId: MessageId): ConversationData

    /**
     * The body of [messageId] for display. Fetches the message once if it is not stored (needs the network then). It never marks the
     * message read. The core decides about remote images from [setting], the person's choice on this message and the known senders.
     */
    suspend fun loadBody(messageId: MessageId, setting: RemoteImages): BodyData

    /**
     * Saves attachment [attachmentId] of [messageId] into [destDir] (absolute, existing: the app's own cache folder for shared
     * attachments) and returns the file. A blocked attachment (a program or script) is refused with [ReadProblem.ATTACHMENT_BLOCKED].
     */
    suspend fun saveAttachment(messageId: MessageId, attachmentId: Long, destDir: File): File

    /** "Show images" on one message: call [loadBody] again afterwards. */
    suspend fun allowRemoteContent(messageId: MessageId)

    /** "Always for this sender": [address] joins the known senders of [account]. */
    suspend fun trustSender(account: AccountId, address: String)

    /** Removes [address] from the known senders of [account]; false when it was not there. */
    suspend fun untrustSender(account: AccountId, address: String): Boolean

    /** The known senders of [account] (lower case, sorted), for the S49 list. */
    suspend fun trustedSenders(account: AccountId): List<String>
}
