// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.mail.reading

import com.example.croft.mail.AccountId
import com.example.croft.mail.FolderRole
import com.example.croft.mail.MessageId
import java.io.File
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineDispatcher
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import uniffi.croft_ffi.AttachmentInfo as CoreAttachment
import uniffi.croft_ffi.BodyState as CoreBodyState
import uniffi.croft_ffi.Conversation as CoreConversation
import uniffi.croft_ffi.ConversationMessage as CoreMessage
import uniffi.croft_ffi.Core
import uniffi.croft_ffi.MailAddress as CoreAddress
import uniffi.croft_ffi.MessageBody as CoreBody
import uniffi.croft_ffi.ReadException
import uniffi.croft_ffi.RemoteImagesSetting as CoreSetting

/**
 * [ReadingRepository] over the Rust core (`uniffi.croft_ffi.Core`, FFI API version 9). The core's records are mapped to the app's own;
 * its fixed errors become [ReadFailure]. Nothing here keeps or prints message text, an address or a file name.
 */
class CoreReadingRepository(
    private val core: Core,
    private val io: CoroutineDispatcher = Dispatchers.IO,
) : ReadingRepository {
    override suspend fun openConversation(messageId: MessageId): ConversationData = guarded {
        withContext(io) {
            val conversation = core.openConversation(messageId)
            conversation.toApp(isSample(conversation.account))
        }
    }

    override suspend fun loadBody(messageId: MessageId, setting: RemoteImages): BodyData = guarded {
        // The core's own async call: it runs on the core's runtime, not on the caller's thread.
        core.loadBody(messageId, setting.toCore()).toApp()
    }

    override suspend fun saveAttachment(messageId: MessageId, attachmentId: Long, destDir: File): File = guarded {
        File(core.saveAttachment(messageId, attachmentId, destDir.absolutePath))
    }

    override suspend fun allowRemoteContent(messageId: MessageId) = guarded { withContext(io) { core.allowRemoteContent(messageId) } }

    override suspend fun trustSender(account: AccountId, address: String) = guarded { withContext(io) { core.trustSender(account, address) } }

    override suspend fun untrustSender(account: AccountId, address: String): Boolean = guarded { withContext(io) { core.untrustSender(account, address) } }

    override suspend fun trustedSenders(account: AccountId): List<String> = guarded { withContext(io) { core.trustedSenders(account) } }

    /** Whether [account] is the sample mailbox (`is_demo`). If that cannot be read the conversation is simply not marked: the mark is a hint. */
    private fun isSample(account: AccountId): Boolean = try {
        core.listAccounts().firstOrNull { it.id == account }?.isDemo ?: false
    } catch (e: Exception) {
        false
    }

    private suspend fun <T> guarded(block: suspend () -> T): T = try {
        block()
    } catch (e: CancellationException) {
        throw e
    } catch (e: ReadFailure) {
        throw e
    } catch (e: ReadException) {
        throw ReadFailure(e.toProblem())
    } catch (e: IllegalStateException) {
        // The core was closed (an erase) while a call was on its way: not a crash. Any other IllegalStateException is a bug and passes.
        if (e.message?.contains("destroyed", ignoreCase = true) != true) throw e
        throw ReadFailure(ReadProblem.INTERNAL)
    }

    private fun CoreConversation.toApp(localOnly: Boolean) = ConversationData(account, openedId, messages.map { it.toApp() }, localOnly)

    private fun CoreMessage.toApp() = ConversationMessageData(
        id = id, account = account, folder = folder, folderRole = FolderRole.fromSpecialUse(folderSpecialUse),
        senderName = senderName, senderAddress = senderAddress, to = to.map { it.toApp() }, cc = cc.map { it.toApp() },
        dateMs = dateMs, subject = subject, snippet = snippet, seen = seen, flagged = flagged, answered = answered,
        hasAttachments = hasAttachments, pending = pending, bodyStored = bodyState == CoreBodyState.LOADED, remoteAllowed = remoteAllowed,
    )

    private fun CoreAddress.toApp() = MailAddressData(name, address)

    private fun CoreBody.toApp() = BodyData(
        messageId = messageId, text = text, html = htmlSanitised, remoteAllowed = remoteAllowed,
        blockedRemoteCount = blockedRemoteCount.toInt(), trackersRemoved = trackersRemoved.toInt(), senderKnown = senderKnown,
        truncated = truncated, attachments = attachments.map { it.toApp() },
    )

    private fun CoreAttachment.toApp() = AttachmentData(
        id = id, fileName = fileName, mimeType = mimeType, size = size.toLong().coerceAtLeast(0), shownInline = shownInline, blocked = blocked,
    )

    private fun RemoteImages.toCore() = when (this) {
        RemoteImages.ASK -> CoreSetting.ASK
        RemoteImages.KNOWN_SENDERS -> CoreSetting.KNOWN_SENDERS
        RemoteImages.NEVER -> CoreSetting.NEVER
    }
}

/** The core's [ReadException] as a [ReadProblem]. Every variant has one (a test walks the sealed class). */
fun ReadException.toProblem(): ReadProblem = when (this) {
    is ReadException.NoSuchMessage -> ReadProblem.NO_SUCH_MESSAGE
    is ReadException.NoSuchAttachment -> ReadProblem.NO_SUCH_ATTACHMENT
    is ReadException.NoSuchAccount -> ReadProblem.NO_SUCH_ACCOUNT
    is ReadException.AccountUnavailable -> ReadProblem.ACCOUNT_UNAVAILABLE
    is ReadException.Offline -> ReadProblem.OFFLINE
    is ReadException.SignInNeeded -> ReadProblem.SIGN_IN_NEEDED
    is ReadException.NotOnServer -> ReadProblem.NOT_ON_SERVER
    is ReadException.TooLarge -> ReadProblem.TOO_LARGE
    is ReadException.Unreadable -> ReadProblem.UNREADABLE
    is ReadException.ServerUnavailable -> ReadProblem.SERVER_UNAVAILABLE
    is ReadException.ServerRefused -> ReadProblem.SERVER_REFUSED
    is ReadException.AttachmentBlocked -> ReadProblem.ATTACHMENT_BLOCKED
    is ReadException.BadDestination -> ReadProblem.BAD_DESTINATION
    is ReadException.WriteFailed -> ReadProblem.WRITE_FAILED
    is ReadException.InvalidAddress -> ReadProblem.INVALID_ADDRESS
    is ReadException.Storage -> ReadProblem.STORAGE
    is ReadException.Internal -> ReadProblem.INTERNAL
}
