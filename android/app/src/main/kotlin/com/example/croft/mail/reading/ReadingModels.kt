// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.mail.reading

import com.example.croft.mail.AccountId
import com.example.croft.mail.FolderId
import com.example.croft.mail.FolderRole
import com.example.croft.mail.MessageId

/**
 * The S49 "Remote images" setting (docs/ux/customisation.md: Ask, Always for known senders, Never; default Ask). It is passed to the core
 * on every body load, and the core decides ([ReadingRepository.loadBody]).
 */
enum class RemoteImages(val key: String) {
    /** Blocked until the person taps "Show images" on the message. */
    ASK("ask"),

    /** As [ASK], and shown without asking for the senders on the known-senders list. */
    KNOWN_SENDERS("known"),

    /** Never loaded, not even where the person allowed them before. */
    NEVER("never"),
    ;

    companion object {
        /** The choice stored as [key], or the default for anything else. */
        fun fromKey(key: String?): RemoteImages = entries.firstOrNull { it.key == key } ?: ASK
    }
}

/**
 * The S49 "Mark as read when opened" setting (docs/ux/customisation.md: Immediately, After 2 s, Never; default After 2 s). The time is
 * counted from the moment the message's body is on the screen. Loading a body never marks it read; the app does, with `markSeen`.
 */
enum class MarkReadDelay(val key: String, val delayMs: Long?) {
    IMMEDIATELY("immediately", 0L),
    AFTER_2_SECONDS("2s", 2_000L),
    NEVER("never", null),
    ;

    companion object {
        /** The choice stored as [key], or the default for anything else. */
        fun fromKey(key: String?): MarkReadDelay = entries.firstOrNull { it.key == key } ?: AFTER_2_SECONDS
    }
}

/** One address of a message header. The address is shown on screen and never logged. */
data class MailAddressData(val name: String?, val address: String?) {
    /** What to show: the name, else the address. */
    val display: String get() = name?.takeIf { it.isNotBlank() } ?: address.orEmpty()

    override fun toString(): String = "MailAddressData(..)"
}

/** One message of a conversation (S20), as the core's `ConversationMessage`. Content and addresses are not printed. */
data class ConversationMessageData(
    val id: MessageId,
    val account: AccountId,
    val folder: FolderId,
    val folderRole: FolderRole,
    val senderName: String?,
    val senderAddress: String?,
    val to: List<MailAddressData>,
    val cc: List<MailAddressData>,
    val dateMs: Long,
    val subject: String,
    val snippet: String,
    val seen: Boolean,
    val flagged: Boolean,
    val answered: Boolean,
    val hasAttachments: Boolean,
    val pending: Boolean,
    /** The body is on the device: loading it works offline. */
    val bodyStored: Boolean,
    /** The person tapped "Show images" on this message. */
    val remoteAllowed: Boolean,
) {
    /** What the header shows as the sender. */
    val displayName: String get() = senderName?.takeIf { it.isNotBlank() } ?: senderAddress.orEmpty()

    /** The person's own reply (the Sent folder): the header says "You". */
    val isOwn: Boolean get() = folderRole == FolderRole.SENT

    override fun toString(): String = "ConversationMessageData(id=$id, account=$account, folder=$folder)"
}

/** A conversation: its messages, oldest first, and the one it was opened from. */
data class ConversationData(
    val account: AccountId,
    val openedId: MessageId,
    val messages: List<ConversationMessageData>,
    /** The account is the sample mailbox: every change stays on this device and the screen says "Not saved to server". */
    val localOnly: Boolean = false,
) {
    /** The message the person opened. */
    val opened: ConversationMessageData? get() = messages.firstOrNull { it.id == openedId }

    /** Ids of the messages that start expanded: the opened one and the newest (docs/ux S20). */
    val initiallyExpanded: Set<MessageId> get() = buildSet { add(openedId); messages.lastOrNull()?.let { add(it.id) } }

    override fun toString(): String = "ConversationData(account=$account, messages=${messages.size})"
}

/** One attachment chip (S21), as the core's `AttachmentInfo`. The file name is shown and never logged. */
data class AttachmentData(
    val id: Long,
    val fileName: String,
    val mimeType: String,
    val size: Long,
    /** A picture drawn inside the HTML: no chip. */
    val shownInline: Boolean,
    /** A program or script (`.exe`, `.apk`, `.bat`, ...): flagged, never opened, never saved. */
    val blocked: Boolean,
) {
    override fun toString(): String = "AttachmentData(id=$id, size=$size, blocked=$blocked)"
}

/** A body ready to show (S21), as the core's `MessageBody`. Text and HTML are not printed. */
data class BodyData(
    val messageId: MessageId,
    val text: String?,
    /** The sanitised HTML: the only HTML a renderer may get. Null for a plain-text message, and for one too complex or too large. */
    val html: String?,
    /** Remote images were allowed for this load: the renderer may use the network (https only) for images. */
    val remoteAllowed: Boolean,
    val blockedRemoteCount: Int,
    val trackersRemoved: Int,
    val senderKnown: Boolean,
    /** Cut at a size limit, or too complex to show as HTML: with [html] null the text version is shown, with a quiet note. */
    val truncated: Boolean,
    val attachments: List<AttachmentData>,
) {
    /** HTML was expected but is not there (too complex, or past a limit): "This message is shown as plain text". */
    val shownAsPlainText: Boolean get() = html == null && truncated

    /** The attachments that get a chip (not the pictures drawn in the HTML). */
    val chips: List<AttachmentData> get() = attachments.filter { !it.shownInline }

    override fun toString(): String = "BodyData(messageId=$messageId, html=${html != null}, blocked=$blockedRemoteCount, attachments=${attachments.size})"
}

/** Why a reading call failed: the core's `ReadError` as fixed categories. Each one has a plain-language text ([ReadCopy]). */
enum class ReadProblem {
    NO_SUCH_MESSAGE, NO_SUCH_ATTACHMENT, NO_SUCH_ACCOUNT, ACCOUNT_UNAVAILABLE, OFFLINE, SIGN_IN_NEEDED, NOT_ON_SERVER, TOO_LARGE,
    UNREADABLE, SERVER_UNAVAILABLE, SERVER_REFUSED, ATTACHMENT_BLOCKED, BAD_DESTINATION, WRITE_FAILED, INVALID_ADDRESS, STORAGE, INTERNAL,
    ;

    /** Trying again can work (a connection or a server that was not there). */
    val canRetry: Boolean get() = this == OFFLINE || this == SERVER_UNAVAILABLE || this == WRITE_FAILED || this == INTERNAL
}

/** A reading call that was refused. Carries only the category (never message text, an address, a file name or a path). */
class ReadFailure(val problem: ReadProblem) : Exception(problem.name)
