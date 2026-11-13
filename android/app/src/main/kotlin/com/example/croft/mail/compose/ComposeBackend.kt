// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.mail.compose

import com.example.croft.mail.AccountId
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineDispatcher
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import uniffi.croft_ffi.ComposeException
import uniffi.croft_ffi.ComposeLimits
import uniffi.croft_ffi.Core
import uniffi.croft_ffi.CoreException
import uniffi.croft_ffi.DraftAttachment
import uniffi.croft_ffi.DraftCheck
import uniffi.croft_ffi.DraftContent
import uniffi.croft_ffi.DraftSummary
import uniffi.croft_ffi.DraftSyncOutcome
import uniffi.croft_ffi.FfiAddress
import uniffi.croft_ffi.FfiDraft
import uniffi.croft_ffi.FfiIdentity
import uniffi.croft_ffi.OutboxItem
import uniffi.croft_ffi.ReplyDraft
import uniffi.croft_ffi.SavedDraft
import uniffi.croft_ffi.SendStarted

/** Why a compose call was refused, as fixed categories (the core's `ComposeError`; never a text, an address or a path). */
enum class ComposeProblem {
    NO_SUCH_ACCOUNT, NO_SUCH_DRAFT, NO_SUCH_OUTBOX_ITEM, NO_SUCH_MESSAGE, NO_SUCH_IDENTITY, NO_SUCH_ATTACHMENT, IN_OUTBOX,
    NO_RECIPIENT, INVALID_ADDRESS, CANNOT_SEND, TOO_LATE, WRONG_STATE, ATTACHMENT_TOO_LARGE, TOO_MANY_ATTACHMENTS,
    ATTACHMENT_UNREADABLE, NO_ATTACHMENT_DIRECTORY, BAD_ATTACHMENT_DIRECTORY, NEEDS_CONFIRMATION, DRAFT_TOO_LARGE, FAILED,
}

/** A compose call that was refused. Carries only the category. */
class ComposeFailure(val problem: ComposeProblem) : Exception(problem.name)

/**
 * Everything the compose screens ask of the core (`core/ffi`, "Composing and sending"): the draft, attachment, send and Outbox
 * calls and the recipient helpers, shaped as the core has them (the UniFFI records are used as they are). Two implementations:
 * [CoreComposeBackend] (the real core) and [FakeComposeBackend] (tests, previews, screenshots). Every function is `suspend`: the
 * core's calls are not for the main thread.
 *
 * Failures are [ComposeFailure] with a fixed category.
 */
interface ComposeBackend {
    /** The sender identities of [account], the default first. */
    suspend fun identities(account: AccountId): List<FfiIdentity>

    /** The numbers the compose screen checks against (attachment size, count, recipients). */
    fun limits(): ComposeLimits

    /** Saves the draft on the device. [draftId] null starts a new draft. */
    suspend fun saveDraft(account: AccountId, draftId: Long?, content: DraftContent): SavedDraft

    suspend fun loadDraft(account: AccountId, draftId: Long): FfiDraft

    suspend fun listDrafts(account: AccountId): List<DraftSummary>

    suspend fun discardDraft(account: AccountId, draftId: Long)

    suspend fun syncDraftToServer(account: AccountId, draftId: Long, force: Boolean): DraftSyncOutcome

    /** Adds the file at [path] (a copy under the registered attachment folder). The name shown and sent is [name]. */
    suspend fun addAttachmentFile(account: AccountId, draftId: Long, path: String, name: String, mimeType: String): DraftAttachment

    suspend fun removeAttachment(account: AccountId, draftId: Long, attachmentId: Long)

    /** The facts the Send button's condition table is built from. */
    suspend fun checkDraft(account: AccountId, draftId: Long): DraftCheck

    suspend fun send(account: AccountId, draftId: Long): SendStarted

    /** Takes a send back inside its window. Returns the draft's id. */
    suspend fun undoSend(account: AccountId, outboxId: Long): Long

    suspend fun listOutbox(account: AccountId): List<OutboxItem>

    suspend fun retryOutboxItem(account: AccountId, outboxId: Long, confirmed: Boolean)

    /** Takes a Queued or Failed message back to its draft. Returns the draft's id. */
    suspend fun editOutboxItem(account: AccountId, outboxId: Long): Long

    suspend fun discardOutboxItem(account: AccountId, outboxId: Long)

    suspend fun suggestRecipients(account: AccountId, prefix: String, limit: Int): List<FfiAddress>

    suspend fun replyDraft(account: AccountId, messageId: Long): ReplyDraft

    suspend fun replyAllDraft(account: AccountId, messageId: Long): ReplyDraft

    suspend fun forwardDraft(account: AccountId, messageId: Long): ReplyDraft
}

/**
 * [ComposeBackend] over the Rust core. Each call runs on [io]. The core's fixed errors become [ComposeFailure]; no text from the
 * core is kept.
 */
class CoreComposeBackend(
    private val core: Core,
    private val io: CoroutineDispatcher = Dispatchers.IO,
) : ComposeBackend {
    override suspend fun identities(account: AccountId) = onIo { core.identities(account) }

    override fun limits(): ComposeLimits = uniffi.croft_ffi.composeLimits()

    override suspend fun saveDraft(account: AccountId, draftId: Long?, content: DraftContent) = onIo { core.saveDraft(account, draftId, content) }

    override suspend fun loadDraft(account: AccountId, draftId: Long) = onIo { core.loadDraft(account, draftId) }

    override suspend fun listDrafts(account: AccountId) = onIo { core.listDrafts(account) }

    override suspend fun discardDraft(account: AccountId, draftId: Long) = onIo { core.discardDraft(account, draftId) }

    override suspend fun syncDraftToServer(account: AccountId, draftId: Long, force: Boolean) = call { core.syncDraftToServer(account, draftId, force) }

    override suspend fun addAttachmentFile(account: AccountId, draftId: Long, path: String, name: String, mimeType: String) =
        call { core.addDraftAttachmentFile(account, draftId, path, name, mimeType) }

    override suspend fun removeAttachment(account: AccountId, draftId: Long, attachmentId: Long) = onIo { core.removeDraftAttachment(account, draftId, attachmentId) }

    override suspend fun checkDraft(account: AccountId, draftId: Long) = onIo { core.checkDraft(account, draftId) }

    override suspend fun send(account: AccountId, draftId: Long) = onIo { core.send(account, draftId) }

    override suspend fun undoSend(account: AccountId, outboxId: Long) = onIo { core.undoSend(account, outboxId.toULong()) }

    override suspend fun listOutbox(account: AccountId) = onIo { core.listOutbox(account) }

    override suspend fun retryOutboxItem(account: AccountId, outboxId: Long, confirmed: Boolean) = onIo { core.retryOutboxItem(account, outboxId.toULong(), confirmed) }

    override suspend fun editOutboxItem(account: AccountId, outboxId: Long) = onIo { core.editOutboxItem(account, outboxId.toULong()) }

    override suspend fun discardOutboxItem(account: AccountId, outboxId: Long) = onIo { core.discardOutboxItem(account, outboxId.toULong()) }

    /** T38b: the account's contacts first (`autocomplete`), then the addresses mail was sent to (`suggestRecipients`), each address once. */
    override suspend fun suggestRecipients(account: AccountId, prefix: String, limit: Int) = onIo {
        val n = limit.coerceIn(1, MAX_SUGGESTIONS)
        val contacts = try {
            core.autocomplete(account, prefix, n.toUInt()).filter { it.isContact }.map { FfiAddress(it.name, it.address) }
        } catch (e: CoreException) {
            emptyList()
        }
        val recent = core.suggestRecipients(account, prefix, n.toUInt())
        (contacts + recent).distinctBy { it.address.lowercase() }.take(n)
    }

    override suspend fun replyDraft(account: AccountId, messageId: Long) = onIo { core.replyDraft(account, messageId) }

    override suspend fun replyAllDraft(account: AccountId, messageId: Long) = onIo { core.replyAllDraft(account, messageId) }

    override suspend fun forwardDraft(account: AccountId, messageId: Long) = onIo { core.forwardDraft(account, messageId) }

    private suspend fun <T> onIo(block: () -> T): T = call { withContext(io) { block() } }

    /** The core's fixed errors as [ComposeFailure]; cancellation passes through. */
    private suspend fun <T> call(block: suspend () -> T): T = try {
        block()
    } catch (e: CancellationException) {
        throw e
    } catch (e: ComposeFailure) {
        throw e
    } catch (e: ComposeException) {
        throw ComposeFailure(e.toProblem())
    } catch (e: IllegalStateException) {
        // The core was closed (an erase) while a call was on its way. Any other IllegalStateException is a bug and passes.
        if (e.message?.contains("destroyed", ignoreCase = true) != true) throw e
        throw ComposeFailure(ComposeProblem.FAILED)
    }

    private fun ComposeException.toProblem(): ComposeProblem = when (this) {
        is ComposeException.NoSuchAccount -> ComposeProblem.NO_SUCH_ACCOUNT
        is ComposeException.NoSuchDraft -> ComposeProblem.NO_SUCH_DRAFT
        is ComposeException.NoSuchOutboxItem -> ComposeProblem.NO_SUCH_OUTBOX_ITEM
        is ComposeException.NoSuchMessage -> ComposeProblem.NO_SUCH_MESSAGE
        is ComposeException.NoSuchIdentity -> ComposeProblem.NO_SUCH_IDENTITY
        is ComposeException.NoSuchAttachment -> ComposeProblem.NO_SUCH_ATTACHMENT
        is ComposeException.InOutbox -> ComposeProblem.IN_OUTBOX
        is ComposeException.NoRecipient -> ComposeProblem.NO_RECIPIENT
        is ComposeException.InvalidAddress -> ComposeProblem.INVALID_ADDRESS
        is ComposeException.CannotSend -> ComposeProblem.CANNOT_SEND
        is ComposeException.TooLate -> ComposeProblem.TOO_LATE
        is ComposeException.WrongState -> ComposeProblem.WRONG_STATE
        is ComposeException.AttachmentTooLarge -> ComposeProblem.ATTACHMENT_TOO_LARGE
        is ComposeException.TooManyAttachments -> ComposeProblem.TOO_MANY_ATTACHMENTS
        is ComposeException.AttachmentUnreadable -> ComposeProblem.ATTACHMENT_UNREADABLE
        is ComposeException.NoAttachmentDirectory -> ComposeProblem.NO_ATTACHMENT_DIRECTORY
        is ComposeException.BadAttachmentDirectory -> ComposeProblem.BAD_ATTACHMENT_DIRECTORY
        is ComposeException.NeedsConfirmation -> ComposeProblem.NEEDS_CONFIRMATION
        is ComposeException.DraftTooLarge -> ComposeProblem.DRAFT_TOO_LARGE
        else -> ComposeProblem.FAILED
    }

    private companion object {
        const val MAX_SUGGESTIONS = 20
    }
}
