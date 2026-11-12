// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.mail.compose

import com.example.croft.mail.AccountId
import java.io.File
import uniffi.croft_ffi.ComposeLimits
import uniffi.croft_ffi.DraftAttachment
import uniffi.croft_ffi.DraftCheck
import uniffi.croft_ffi.DraftContent
import uniffi.croft_ffi.DraftProblem
import uniffi.croft_ffi.DraftSummary
import uniffi.croft_ffi.DraftSyncOutcome
import uniffi.croft_ffi.FfiAddress
import uniffi.croft_ffi.FfiDraft
import uniffi.croft_ffi.FfiIdentity
import uniffi.croft_ffi.OutboxItem
import uniffi.croft_ffi.OutboxItemState
import uniffi.croft_ffi.ReplyDraft
import uniffi.croft_ffi.SavedDraft
import uniffi.croft_ffi.SendStarted
import uniffi.croft_ffi.ServerCopy

/**
 * An in-memory [ComposeBackend] for tests, previews and screenshots: drafts, attachments, a send that fills an Outbox, Undo and the
 * Outbox actions, with the core's own rules for `check_draft` (no recipient, invalid recipient, attachments over 25 MB). It records what
 * was called (`calls`) so a test can see that something was never called. Nothing leaves the process.
 */
class FakeComposeBackend(
    private val tools: AddressTools = SimpleAddressTools,
    private val clock: () -> Long = System::currentTimeMillis,
    var undoWindowMs: Long = 5_000,
    val identitiesOf: (AccountId) -> List<FfiIdentity> = { a -> listOf(FfiIdentity(a * 10 + 1, "Sam Example", "sam$a@example.org", true)) },
) : ComposeBackend {
    /** Every call by name (no data), in order. */
    val calls = ArrayList<String>()

    private class Draft(val id: Long, val account: AccountId, var content: DraftContent, val attachments: MutableList<DraftAttachment> = ArrayList(), var outbox: Long? = null)

    private val drafts = LinkedHashMap<Long, Draft>()
    private var nextDraft = 1L
    private var nextAttachment = 1L
    private var nextOutbox = 1L

    /** The Outbox, as tests set it: items per account. `send` adds to it. */
    val outbox = ArrayList<Pair<AccountId, OutboxItem>>()

    /** What the next `syncDraftToServer` answers. */
    var syncOutcome: DraftSyncOutcome = DraftSyncOutcome.SAVED

    /** Replies the fake gives for reply, reply all and forward. */
    var reply: ReplyDraft = ReplyDraft(emptyContent(), true)

    /** Recipient suggestions (filtered by prefix). */
    var suggestions: List<FfiAddress> = emptyList()

    /** Files given to `addAttachmentFile`, by name; the bytes are read from the path while the call runs. */
    val attachedBytes = LinkedHashMap<String, ByteArray>()

    /** When set, the next `retryOutboxItem` answers like this. */
    var failRetryWith: ComposeProblem? = null

    private fun rec(name: String) = synchronized(calls) { calls += name }

    override suspend fun identities(account: AccountId): List<FfiIdentity> = identitiesOf(account).also { rec("identities") }

    override fun limits() = ComposeLimits(
        attachmentWarnBytes = 25UL * MB, maxAttachmentBytes = 40UL * MB, maxAttachmentsTotalBytes = 40UL * MB, maxAttachments = 50u,
        maxRecipients = 200u, maxSubjectChars = 998u, maxBodyBytes = 1_000_000u, maxSuggestions = 20u,
    )

    override suspend fun saveDraft(account: AccountId, draftId: Long?, content: DraftContent): SavedDraft {
        rec("saveDraft")
        val d = if (draftId == null) Draft(nextDraft++, account, content).also { drafts[it.id] = it } else drafts[draftId] ?: throw ComposeFailure(ComposeProblem.NO_SUCH_DRAFT)
        if (d.outbox != null) throw ComposeFailure(ComposeProblem.IN_OUTBOX)
        d.content = content
        return SavedDraft(d.id, "<fake-${d.id}@croft.invalid>", clock())
    }

    override suspend fun loadDraft(account: AccountId, draftId: Long): FfiDraft {
        rec("loadDraft")
        val d = drafts[draftId]?.takeIf { it.account == account } ?: throw ComposeFailure(ComposeProblem.NO_SUCH_DRAFT)
        return FfiDraft(d.id, account, "<fake-${d.id}@croft.invalid>", d.content, d.attachments.toList(), ServerCopy.NOT_COPIED, d.outbox?.toULong(), 0, 0)
    }

    override suspend fun listDrafts(account: AccountId): List<DraftSummary> = drafts.values.filter { it.account == account }.map {
        DraftSummary(it.id, account, it.content.subject, it.content.body.take(120), it.content.to.take(3).map { a -> a.name ?: a.address }, it.attachments.isNotEmpty(), it.outbox?.toULong(), 0)
    }

    override suspend fun discardDraft(account: AccountId, draftId: Long) { rec("discardDraft"); drafts.remove(draftId) }

    override suspend fun syncDraftToServer(account: AccountId, draftId: Long, force: Boolean): DraftSyncOutcome { rec(if (force) "syncForced" else "sync"); return syncOutcome }

    override suspend fun addAttachmentFile(account: AccountId, draftId: Long, path: String, name: String, mimeType: String): DraftAttachment {
        rec("addAttachmentFile")
        val d = drafts[draftId] ?: throw ComposeFailure(ComposeProblem.NO_SUCH_DRAFT)
        val bytes = try { File(path).readBytes() } catch (e: java.io.IOException) { throw ComposeFailure(ComposeProblem.ATTACHMENT_UNREADABLE) }
        attachedBytes[name] = bytes
        return DraftAttachment(nextAttachment++, name, mimeType, bytes.size.toULong()).also { d.attachments += it }
    }

    override suspend fun removeAttachment(account: AccountId, draftId: Long, attachmentId: Long) { rec("removeAttachment"); drafts[draftId]?.attachments?.removeAll { it.id == attachmentId } }

    override suspend fun checkDraft(account: AccountId, draftId: Long): DraftCheck {
        rec("checkDraft")
        val d = drafts[draftId] ?: throw ComposeFailure(ComposeProblem.NO_SUCH_DRAFT)
        val all = d.content.to + d.content.cc + d.content.bcc
        val bytes = d.attachments.sumOf { it.sizeBytes }
        val problems = buildList {
            if (all.isEmpty()) add(DraftProblem.NO_RECIPIENT)
            if (all.any { !tools.isValid(it.address) }) add(DraftProblem.INVALID_RECIPIENT)
            if (bytes > 25UL * MB) add(DraftProblem.ATTACHMENTS_TOO_LARGE)
        }
        return DraftCheck(problems, all.filter { !tools.isValid(it.address) }.map { it.address }, d.content.subject.isBlank(), d.content.body.isBlank() && d.attachments.isEmpty(), bytes, 25UL * MB)
    }

    override suspend fun send(account: AccountId, draftId: Long): SendStarted {
        rec("send")
        val d = drafts[draftId] ?: throw ComposeFailure(ComposeProblem.NO_SUCH_DRAFT)
        if (d.outbox != null) throw ComposeFailure(ComposeProblem.IN_OUTBOX)
        val id = nextOutbox++
        d.outbox = id
        val at = clock() + undoWindowMs
        val all = d.content.to + d.content.cc + d.content.bcc
        outbox += account to OutboxItem(id.toULong(), OutboxItemState.QUEUED, at, 0u, null, false, d.id, d.content.subject, all.size.toUInt(), all.take(2).map { it.address })
        return SendStarted(id.toULong(), at)
    }

    override suspend fun undoSend(account: AccountId, outboxId: Long): Long {
        rec("undoSend")
        val entry = outbox.firstOrNull { it.first == account && it.second.id.toLong() == outboxId } ?: throw ComposeFailure(ComposeProblem.NO_SUCH_OUTBOX_ITEM)
        if (clock() >= entry.second.sendAtMs) throw ComposeFailure(ComposeProblem.TOO_LATE)
        outbox.remove(entry)
        val draft = drafts.values.first { it.outbox == outboxId }
        draft.outbox = null
        return draft.id
    }

    override suspend fun listOutbox(account: AccountId): List<OutboxItem> = outbox.filter { it.first == account }.map { it.second }

    override suspend fun retryOutboxItem(account: AccountId, outboxId: Long, confirmed: Boolean) {
        rec(if (confirmed) "retryConfirmed" else "retry")
        failRetryWith?.let { throw ComposeFailure(it) }
        val i = outbox.indexOfFirst { it.first == account && it.second.id.toLong() == outboxId }
        if (i < 0) throw ComposeFailure(ComposeProblem.NO_SUCH_OUTBOX_ITEM)
        if (outbox[i].second.mayHaveBeenSent && !confirmed) throw ComposeFailure(ComposeProblem.NEEDS_CONFIRMATION)
        outbox[i] = account to outbox[i].second.copy(state = OutboxItemState.QUEUED, reason = null, mayHaveBeenSent = false)
    }

    override suspend fun editOutboxItem(account: AccountId, outboxId: Long): Long {
        rec("editOutbox")
        val entry = outbox.firstOrNull { it.first == account && it.second.id.toLong() == outboxId } ?: throw ComposeFailure(ComposeProblem.NO_SUCH_OUTBOX_ITEM)
        if (entry.second.state == OutboxItemState.SENDING) throw ComposeFailure(ComposeProblem.WRONG_STATE)
        outbox.remove(entry)
        val draft = drafts.values.firstOrNull { it.outbox == outboxId }
        draft?.outbox = null
        return entry.second.draftId ?: throw ComposeFailure(ComposeProblem.NO_SUCH_DRAFT)
    }

    override suspend fun discardOutboxItem(account: AccountId, outboxId: Long) {
        rec("discardOutbox")
        val entry = outbox.firstOrNull { it.first == account && it.second.id.toLong() == outboxId } ?: throw ComposeFailure(ComposeProblem.NO_SUCH_OUTBOX_ITEM)
        outbox.remove(entry)
        entry.second.draftId?.let { drafts.remove(it) }
    }

    override suspend fun suggestRecipients(account: AccountId, prefix: String, limit: Int): List<FfiAddress> =
        suggestions.filter { it.address.startsWith(prefix, true) || it.name?.startsWith(prefix, true) == true }.take(limit)

    override suspend fun replyDraft(account: AccountId, messageId: Long) = reply

    override suspend fun replyAllDraft(account: AccountId, messageId: Long) = reply

    override suspend fun forwardDraft(account: AccountId, messageId: Long) = reply

    /** Adds an item to the Outbox as the core would show it (Failed, Sending ...), for the Outbox screen. */
    fun addOutboxItem(account: AccountId, item: OutboxItem) { outbox += account to item }

    companion object {
        private val MB: ULong = 1024UL * 1024UL

        fun emptyContent() = DraftContent(null, emptyList(), emptyList(), emptyList(), "", "", null, emptyList())
    }
}

/** A plain stand-in for the core's address functions, for tests and screenshots that do not load the native library. */
object SimpleAddressTools : AddressTools {
    private val bare = Regex("[^\\s<>@,;\"]+@[^\\s<>@,;\"]+\\.[^\\s<>@,;\"]+")
    private val named = Regex("^\\s*\"?([^<\"]*?)\"?\\s*<([^>]+)>\\s*$")

    override fun parse(text: String): List<FfiAddress> = text.split(',', ';').mapNotNull { part ->
        val p = part.trim()
        if (p.isEmpty()) return@mapNotNull null
        named.matchEntire(p)?.let { m -> return@mapNotNull FfiAddress(m.groupValues[1].trim().ifEmpty { null }, m.groupValues[2].trim()) }
        bare.find(p)?.let { FfiAddress(null, it.value) }
    }

    override fun isValid(address: String): Boolean = bare.matches(address.trim())
}
