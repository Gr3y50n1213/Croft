// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.mail

import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineDispatcher
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.withContext
import uniffi.croft_ffi.AccountInfo as CoreAccountInfo
import uniffi.croft_ffi.AccountStatus as CoreAccountStatus
import uniffi.croft_ffi.ChangeBatch as CoreChangeBatch
import uniffi.croft_ffi.Core
import uniffi.croft_ffi.CoreException
import uniffi.croft_ffi.FailureReason as CoreFailureReason
import uniffi.croft_ffi.FolderInfo as CoreFolderInfo
import uniffi.croft_ffi.MessageRow as CoreMessageRow
import uniffi.croft_ffi.MessageScope as CoreMessageScope
import uniffi.croft_ffi.RowCursor as CoreRowCursor

/**
 * [MessageRepository] over the Rust core (`uniffi.croft_ffi.Core`, FFI API version 7): the T14 interface the screens use, mapped
 * onto the T10 calls. Nothing here waits for the network: every call is a local database read or a queued change, run on [io]
 * because even those are not for the main thread.
 *
 * - **Lists.** `listMessages` maps [MessageScope] to the core's (a folder, the unified Inbox, or Starred for the virtual
 *   Starred mailbox of [VirtualFolders]) and the core's row to [MessageRowData]. `listFolders` adds that Starred mailbox.
 * - **Changes.** `nextChange` is the core's change log; a failure notice carries the messages of the call that made the
 *   operation (the repository remembers the last [MAX_REMEMBERED_OPS] calls), so the list can mark them and Retry can repeat.
 * - **Folder rules.** Archive and Trash use the account's special-use folder; an account without one uses the folder the
 *   person chose once ([FolderMapping]); with neither the action throws [MailException] with the account, and the screen asks.
 *   Every account of a call is checked before the first change, as in the core.
 * - **Errors.** The core's fixed categories become [MailError]; no text from the core is kept.
 */
class CoreMessageRepository(
    private val core: Core,
    private val mapping: FolderMapping = InMemoryFolderMapping(),
    private val io: CoroutineDispatcher = Dispatchers.IO,
) : MessageRepository {
    /** Which account a message is in, learned from the rows served, so that an action given only ids can find its account's folders. */
    private val accountOf = object : LinkedHashMap<MessageId, AccountId>(1024, 0.75f, false) {
        override fun removeEldestEntry(eldest: MutableMap.MutableEntry<MessageId, AccountId>): Boolean = size > MAX_KNOWN_MESSAGES
    }

    /** The accounts seen so far and which of them is the sample mailbox (`is_demo`): its rows say "Not saved to server". */
    private val knownAccounts = HashSet<AccountId>()
    private val sampleAccounts = HashSet<AccountId>()

    private fun learnAccounts(accounts: List<CoreAccountInfo>) = synchronized(knownAccounts) {
        knownAccounts.clear(); sampleAccounts.clear()
        accounts.forEach { knownAccounts += it.id; if (it.isDemo) sampleAccounts += it.id }
    }

    /**
     * Whether [account] is the sample mailbox. An account not seen yet is looked up once (a local read); if that read fails the rows
     * are simply not marked (the mark is a hint on a row, never a reason to fail a list).
     */
    private fun isSample(account: AccountId): Boolean {
        synchronized(knownAccounts) { if (account in knownAccounts) return account in sampleAccounts }
        try {
            learnAccounts(core.listAccounts())
        } catch (e: Exception) {
            return false
        }
        return synchronized(knownAccounts) { account in sampleAccounts }
    }

    /** What an operation was, for Retry and for marking failed rows. */
    private class Call(val kind: Kind, val ids: List<MessageId>, val folder: FolderId?)

    private enum class Kind { ARCHIVE, TRASH, MOVE, DELETE_FOREVER }

    private val calls = object : LinkedHashMap<OpId, Call>(64, 0.75f, false) {
        override fun removeEldestEntry(eldest: MutableMap.MutableEntry<OpId, Call>): Boolean = size > MAX_REMEMBERED_OPS
    }

    // ---- Reading ----

    override suspend fun listAccounts(): List<AccountInfo> = onIo { core.listAccounts().also { learnAccounts(it) }.map { it.toApp() } }

    override suspend fun listFolders(account: AccountId): List<FolderInfo> = onIo {
        core.listFolders(account).map { it.toApp(account) } +
            FolderInfo(VirtualFolders.starred(account), account, "Starred", FolderRole.STARRED.specialUse, 0, 0)
    }

    override suspend fun listMessages(scope: MessageScope, before: RowCursor?, limit: Int): MessagePage = onIo {
        val page = core.listMessages(scope.toCore(), before?.let { CoreRowCursor(it.dateMs, it.id) }, limit.coerceIn(0, MAX_PAGE).toUInt())
        synchronized(accountOf) { page.rows.forEach { accountOf[it.id] = it.account } }
        MessagePage(page.rows.map { it.toApp(isSample(it.account)) }, page.next?.let { RowCursor(it.dateMs, it.id) })
    }

    override suspend fun nextChange(after: Long): ChangeBatch {
        val batch = try {
            core.nextChange(after.coerceAtLeast(0).toULong())
        } catch (e: CoreException) {
            // The log could not be read: tell the screen to re-run everything, and not in a hot loop.
            delay(RETRY_AFTER_ERROR_MS)
            return ChangeBatch(seq = after, everything = true)
        }
        return batch.toApp()
    }

    // ---- Actions ----

    override suspend fun markSeen(ids: List<MessageId>, value: Boolean) = onIo { core.markSeen(ids, value) }

    override suspend fun setFlagged(ids: List<MessageId>, value: Boolean) = onIo { core.setFlagged(ids, value) }

    override suspend fun archive(ids: List<MessageId>): List<OpId> = onIo { moveByRole(ids, FolderRole.ARCHIVE, Kind.ARCHIVE) }

    override suspend fun trash(ids: List<MessageId>): List<OpId> = onIo { moveByRole(ids, FolderRole.TRASH, Kind.TRASH) }

    override suspend fun moveTo(ids: List<MessageId>, folderId: FolderId): List<OpId> = onIo { doMoveTo(ids, folderId) }

    override suspend fun deleteForever(ids: List<MessageId>): List<OpId> = onIo { doDeleteForever(ids) }

    private fun doMoveTo(ids: List<MessageId>, folderId: FolderId): List<OpId> {
        val ops = core.moveTo(ids, folderId).map { it.toLong() }
        remember(ops, Call(Kind.MOVE, ids, folderId))
        return ops
    }

    private fun doDeleteForever(ids: List<MessageId>): List<OpId> {
        val ops = core.deleteForever(ids).map { it.toLong() }
        remember(ops, Call(Kind.DELETE_FOREVER, ids, null))
        return ops
    }

    override suspend fun undo(opId: OpId): Boolean = onIo { core.undo(opId.toULong()) }

    override suspend fun retryFailed(opId: OpId): List<OpId> = onIo {
        val call = synchronized(calls) { calls[opId] } ?: throw MailException(MailError.NOT_FOUND)
        // One message at a time: a message that is gone (a delete that did go through) or already in place must not stop the rest.
        val ops = ArrayList<OpId>()
        for (id in call.ids) {
            try {
                ops += when (call.kind) {
                    Kind.ARCHIVE -> moveByRole(listOf(id), FolderRole.ARCHIVE, Kind.ARCHIVE)
                    Kind.TRASH -> moveByRole(listOf(id), FolderRole.TRASH, Kind.TRASH)
                    Kind.MOVE -> doMoveTo(listOf(id), call.folder ?: throw MailException(MailError.NOT_FOUND))
                    Kind.DELETE_FOREVER -> doDeleteForever(listOf(id))
                }
            } catch (e: MailException) {
                if (e.error != MailError.NOT_FOUND) throw e
            }
        }
        ops
    }

    override suspend fun setUndoWindowMs(ms: Long) = onIo { core.setUndoWindowMs(ms) }

    override suspend fun syncNow(account: AccountId) = onIo { core.syncNow(account) }

    override suspend fun setNetworkAvailable(available: Boolean) = onIo { core.setNetworkAvailable(available) }

    override suspend fun signedIn(account: AccountId) { map { core.signedIn(account) } }

    // ---- Folder rules ----

    /**
     * Moves [ids] to each account's folder for [role]. The destination of every account is found first (its special-use folder,
     * else the one the person chose and still exists), and the call fails with the first account that has none before anything
     * is changed.
     */
    private fun moveByRole(ids: List<MessageId>, role: FolderRole, kind: Kind): List<OpId> {
        val error = if (kind == Kind.TRASH) MailError.NO_TRASH_FOLDER else MailError.NO_ARCHIVE_FOLDER
        val groups = ids.distinct().groupBy { synchronized(accountOf) { accountOf[it] } }
        class Step(val ids: List<MessageId>, val viaMapping: FolderId?)
        val steps = ArrayList<Step>()
        for ((account, group) in groups) {
            if (account == null) { steps += Step(group, null); continue }
            val folders = core.listFolders(account)
            val hasSpecial = folders.any { it.specialUse == role.specialUse }
            val chosen = mapping.mapped(account, role)?.takeIf { id -> folders.any { it.id == id } }
            when {
                hasSpecial -> steps += Step(group, null)
                chosen != null -> steps += Step(group, chosen)
                else -> {
                    mapping.forget(account, role)
                    throw MailException(error, account)
                }
            }
        }
        val ops = ArrayList<OpId>()
        for (step in steps) {
            val made = when {
                step.viaMapping != null -> core.moveTo(step.ids, step.viaMapping)
                kind == Kind.TRASH -> core.trash(step.ids)
                else -> core.archive(step.ids)
            }.map { it.toLong() }
            remember(made, Call(kind, step.ids, step.viaMapping))
            ops += made
        }
        return ops
    }

    private fun remember(ops: List<OpId>, call: Call) {
        synchronized(calls) { ops.forEach { calls[it] = call } }
    }

    // ---- Mapping ----

    private suspend fun <T> onIo(block: () -> T): T = map { withContext(io) { block() } }

    /** The core's fixed errors as [MailException]; cancellation passes through. */
    private suspend fun <T> map(block: suspend () -> T): T = try {
        block()
    } catch (e: CancellationException) {
        throw e
    } catch (e: MailException) {
        throw e
    } catch (e: IllegalStateException) {
        // The core was closed (an erase) while a call was on its way: not a crash. Any other IllegalStateException is a bug and passes.
        if (e.message?.contains("destroyed", ignoreCase = true) != true) throw e
        throw MailException(MailError.FAILED)
    } catch (e: CoreException) {
        throw MailException(
            when (e) {
                is CoreException.NoArchiveFolder -> MailError.NO_ARCHIVE_FOLDER
                is CoreException.NoTrashFolder -> MailError.NO_TRASH_FOLDER
                is CoreException.NoSuchMessage, is CoreException.NoSuchFolder, is CoreException.NoSuchAccount -> MailError.NOT_FOUND
                else -> MailError.FAILED
            },
        )
    }

    private fun CoreAccountInfo.toApp() = AccountInfo(id, label, email, status.toApp(), lastSyncedMs, isDemo)

    private fun CoreAccountStatus.toApp() = when (this) {
        CoreAccountStatus.OK -> AccountState.OK
        CoreAccountStatus.SYNCING -> AccountState.SYNCING
        CoreAccountStatus.OFFLINE -> AccountState.OFFLINE
        CoreAccountStatus.NEEDS_SIGN_IN -> AccountState.NEEDS_SIGN_IN
        CoreAccountStatus.ERROR -> AccountState.ERROR
    }

    private fun CoreFolderInfo.toApp(account: AccountId) = FolderInfo(id, account, path, specialUse, unreadCount.toInt(), totalCount.toInt())

    private fun MessageScope.toCore(): CoreMessageScope = when (this) {
        MessageScope.UnifiedInbox -> CoreMessageScope.UnifiedInbox
        is MessageScope.Folder ->
            if (VirtualFolders.isVirtual(folderId)) CoreMessageScope.Starred(account) else CoreMessageScope.Folder(account, folderId)
    }

    private fun CoreMessageRow.toApp(localOnly: Boolean) = MessageRowData(
        id = id, account = account, folder = folder, senderName = senderName, senderAddress = senderAddress,
        subject = subject, snippet = snippet, dateMs = dateMs, seen = seen, flagged = flagged,
        hasAttachments = hasAttachments, pending = pending, threadCount = threadCount.toInt().coerceAtLeast(1),
        recipients = recipients.joinToString(", ").ifBlank { null }, localOnly = localOnly,
    )

    private fun CoreChangeBatch.toApp() = ChangeBatch(
        seq = seq.toLong(),
        everything = everything,
        folders = folders.map { FolderRef(it.account, it.folderId) },
        accounts = accounts,
        failures = failures.map { f ->
            val op = f.opId.toLong()
            OpFailure(f.account, op, f.reason.toApp(), synchronized(calls) { calls[op]?.ids }.orEmpty())
        },
    )

    private fun CoreFailureReason.toApp() = when (this) {
        CoreFailureReason.LOST_AFTER_MOVE -> FailureReason.LOST_AFTER_MOVE
        CoreFailureReason.LOST_AFTER_RESET -> FailureReason.LOST_AFTER_RESET
        CoreFailureReason.AMBIGUOUS_COPY -> FailureReason.AMBIGUOUS_COPY
        CoreFailureReason.REJECTED -> FailureReason.REJECTED
    }

    private companion object {
        const val MAX_PAGE = 500
        const val MAX_KNOWN_MESSAGES = 20_000
        const val MAX_REMEMBERED_OPS = 200
        const val RETRY_AFTER_ERROR_MS = 1_000L
    }
}
