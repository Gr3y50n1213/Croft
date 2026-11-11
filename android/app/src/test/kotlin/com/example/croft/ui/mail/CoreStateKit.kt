// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.ui.mail

import com.example.croft.haptics.RecordingHaptics
import com.example.croft.mail.CoreMessageRepository
import com.example.croft.mail.CoreRowActions
import com.example.croft.mail.InMemoryAnchorStore
import com.example.croft.mail.InMemoryFolderMapping
import com.example.croft.mail.MailSession
import java.time.ZoneId
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.first
import uniffi.croft_ffi.AccountInfo as CoreAccount
import uniffi.croft_ffi.AccountStatus
import uniffi.croft_ffi.ChangeBatch
import uniffi.croft_ffi.Core
import uniffi.croft_ffi.FailureReason
import uniffi.croft_ffi.FolderInfo as CoreFolder
import uniffi.croft_ffi.FolderRef
import uniffi.croft_ffi.MessagePage
import uniffi.croft_ffi.MessageRow as CoreRow
import uniffi.croft_ffi.MessageScope as CoreScope
import uniffi.croft_ffi.NoHandle
import uniffi.croft_ffi.OpFailure
import uniffi.croft_ffi.RowCursor as CoreCursor

/**
 * The real core's side of a screen test, scripted: a `uniffi.croft_ffi.Core` with no Rust behind it that answers the calls the Inbox
 * makes from a table of accounts, folders and rows in the shape the real core returns (thread counts, recipients, pending rows,
 * account statuses). The repository in front of it is the app's own [CoreMessageRepository], so the screens run through the real
 * mapping, the real folder rules and the real failure handling; what is scripted is only what the Rust core would answer.
 * (The live core, with a real server, is tested in `:mailtests`.) Trash and archive hide the rows and bump the change log, like the
 * core's queue; [reject] puts them back with a failure notice, like a permanent refusal.
 */
class ScriptedCore(
    private val accountList: MutableList<CoreAccount>,
    private val folderList: List<CoreFolder>,
    rows: List<CoreRow>,
) : Core(NoHandle) {
    private val all = rows.toMutableList()
    private val hidden = HashMap<Long, List<CoreRow>>()
    private val seq = MutableStateFlow(0L)
    private val pendingFailures = ArrayList<OpFailure>()
    private var nextOp = 1uL

    /** Set an account's status (Offline, Needs sign-in, ...) and tell the screen. */
    fun setStatus(account: Long, status: AccountStatus) {
        synchronized(this) {
            val i = accountList.indexOfFirst { it.id == account }
            accountList[i] = accountList[i].copy(status = status)
        }
        seq.value = seq.value + 1
    }

    /** The server refused operation [op] for good: its rows come back and a failure notice follows. */
    fun reject(op: ULong, account: Long) {
        synchronized(this) {
            hidden.remove(op.toLong())?.let { all.addAll(it) }
            pendingFailures += OpFailure(account, op, FailureReason.REJECTED)
        }
        seq.value = seq.value + 1
    }

    override fun listAccounts(): List<CoreAccount> = synchronized(this) { accountList.toList() }

    override fun listFolders(accountId: Long): List<CoreFolder> = folderList.filter { f -> f.id / 100 == accountId }

    override fun listMessages(scope: CoreScope, before: CoreCursor?, limit: UInt): MessagePage = synchronized(this) {
        val visible = all.filter { r ->
            when (scope) {
                CoreScope.UnifiedInbox -> folderList.any { it.id == r.folder && it.specialUse == "inbox" }
                is CoreScope.Folder -> r.account == scope.account && r.folder == scope.folderId
                is CoreScope.Starred -> r.flagged && (scope.account == null || r.account == scope.account)
            }
        }.sortedWith(compareByDescending<CoreRow> { it.dateMs }.thenByDescending { it.id })
            .filter { before == null || it.dateMs < before.dateMs || (it.dateMs == before.dateMs && it.id < before.id) }
        val page = visible.take(limit.toInt())
        MessagePage(page, if (visible.size > page.size) CoreCursor(page.last().dateMs, page.last().id) else null)
    }

    override suspend fun nextChange(after: ULong): ChangeBatch {
        val now = seq.first { it.toULong() > after }
        return synchronized(this) {
            val failures = pendingFailures.toList().also { pendingFailures.clear() }
            ChangeBatch(
                seq = now.toULong(), everything = true,
                folders = folderList.map { FolderRef(it.id / 100, it.id) },
                accounts = accountList.map { it.id },
                failures = failures,
            )
        }
    }

    private fun remove(ids: List<Long>): List<ULong> = synchronized(this) {
        val op = nextOp++
        val gone = all.filter { it.id in ids }
        all.removeAll(gone.toSet())
        hidden[op.toLong()] = gone
        seq.value = seq.value + 1
        listOf(op)
    }

    override fun archive(ids: List<Long>): List<ULong> = remove(ids)
    override fun trash(ids: List<Long>): List<ULong> = remove(ids)
    override fun moveTo(ids: List<Long>, folderId: Long): List<ULong> = remove(ids)
    override fun deleteForever(ids: List<Long>): List<ULong> = remove(ids)

    override fun undo(opId: ULong): Boolean = synchronized(this) {
        val back = hidden.remove(opId.toLong()) ?: return false
        all.addAll(back)
        seq.value = seq.value + 1
        true
    }

    override fun markSeen(ids: List<Long>, value: Boolean) = edit(ids) { it.copy(seen = value, pending = true) }
    override fun setFlagged(ids: List<Long>, value: Boolean) = edit(ids) { it.copy(flagged = value, pending = true) }

    private fun edit(ids: List<Long>, f: (CoreRow) -> CoreRow) {
        synchronized(this) { all.replaceAll { if (it.id in ids) f(it) else it } }
        seq.value = seq.value + 1
    }
}

/** A mailbox through the real repository over a [ScriptedCore]: two accounts, the usual folders, rows in the core's shape. */
class CoreFixture(
    statusOf: (Long) -> AccountStatus = { AccountStatus.OK },
    /** The accounts that have an Archive folder (the others have none: the first Archive asks which folder to use). */
    archiveFor: Set<Long> = setOf(1L, 2L),
) {
    val accounts = mutableListOf(
        CoreAccount(1, "Sam", "sam@example.org", statusOf(1), NOW - 4 * 60_000, false),
        CoreAccount(2, "Work", "sam@acme.test", statusOf(2), NOW - 4 * 60_000, false),
    )

    // Folder ids are `account * 100 + position`: 1 Inbox, 2 Sent, 3 Archive, 4 Trash, 5 Junk.
    private val folders = (1L..2L).flatMap { a ->
        listOf("inbox" to "INBOX", "sent" to "Sent", "archive" to "Archive", "trash" to "Trash", "junk" to "Junk").mapIndexed { i, (use, path) ->
            // An account without an Archive folder still has a plain folder of that name: its special use is empty.
            CoreFolder(a * 100 + i + 1, path, if (use == "archive" && a !in archiveFor) "" else use, 0u, 0u)
        }
    }

    private fun row(id: Long, account: Long, from: String, subject: String, minutesAgo: Int, seen: Boolean = false, flagged: Boolean = false, pending: Boolean = false, threads: UInt = 1u) =
        CoreRow(id, account, account * 100 + 1, from, "${from.lowercase().replace(' ', '.')}@example.org", subject, "A preview of $subject.", NOW - minutesAgo * 60_000L, seen, flagged, false, pending, threads, emptyList())

    val rows = listOf(
        row(10, 1, "Mira Okafor", "Lunch on Friday?", 2, threads = 3u),
        row(9, 2, "Priya Shah", "Meeting moved to Thursday", 9, pending = true),
        row(8, 1, "Hollis Printing", "Your proof is ready", 40, seen = true, flagged = true),
        row(7, 1, "Nora Lindqvist", "Photos from the weekend", 95, seen = true),
        row(6, 2, "Ada Nwosu", "Agenda for Monday", 180),
    )

    val core = ScriptedCore(accounts, folders, rows)
    val mapping = InMemoryFolderMapping()
    // Unconfined, not Dispatchers.Default: the scripted core is in memory, and on a background thread the compose test's unconfined
    // effect dispatcher resumes the screen's coroutines on that thread, so the list's state is written off the main thread while the
    // test waits for the frame (the list then sometimes stayed on its skeleton). On the real main dispatcher the app resumes on main.
    val repo = CoreMessageRepository(core, mapping, Dispatchers.Unconfined)
    val haptics = RecordingHaptics()
    val session = MailSession(repo, InMemoryAnchorStore(), { NOW }, ZoneId.of("UTC"), CoreRowActions(repo, mapping), mapping)
}
