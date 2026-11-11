// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.mail

import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Test
import uniffi.croft_ffi.Core
import uniffi.croft_ffi.NoHandle

/** [CoreMessageRepository] against the real core and the in-process server: lists, folders, accounts, the folder rules and the errors. */
class CoreMessageRepositoryTest {
    private val rigs = mutableListOf<Rig>()

    private fun rig(messages: Int = 3, window: Long = 600): Rig = Rig(messages, window).also { rigs += it }

    @After
    fun tearDown() = rigs.forEach { it.close() }

    @Test(timeout = 60_000)
    fun theListsAreShapedForTheScreens() {
        val r = rig()
        val rows = r.awaitInbox(3)
        val row = rows.first()
        assertEquals(r.account, row.account)
        assertEquals(r.folder("INBOX").id, row.folder)
        assertEquals("Ann Example", row.senderName)
        assertTrue(row.subject.startsWith("Message "))
        assertFalse(row.seen || row.flagged || row.pending)
        assertEquals("alone in its conversation", 1, row.threadCount)
        assertNull("no recipients stored by the sync yet", row.recipients)
        assertFalse(row.encrypted)

        // The unified Inbox is the same three rows, and paging by cursor walks without a repeat.
        val unified = r.run { r.repo.listMessages(MessageScope.UnifiedInbox, null, 50) }
        assertEquals(rows.map { it.id }, unified.rows.map { it.id })
        val first = r.run { r.repo.listMessages(r.scope("INBOX"), null, 2) }
        assertEquals(2, first.rows.size)
        val second = r.run { r.repo.listMessages(r.scope("INBOX"), first.next, 2) }
        assertEquals(1, second.rows.size)
        assertNull(second.next)
        assertEquals(rows.map { it.id }, (first.rows + second.rows).map { it.id })
    }

    @Test(timeout = 60_000)
    fun accountsReportTheirStatusAndTheLastSyncedTime() {
        val r = rig()
        r.awaitInbox(3)
        val synced = await("the last synced time") { r.run { r.repo.listAccounts() }.single().lastSyncedMs }
        assertTrue(synced > 1_700_000_000_000L)
        val account = r.run { r.repo.listAccounts() }.single()
        assertEquals("Alice", account.label)
        assertEquals(AccountState.OK, account.status)

        r.run { r.repo.setNetworkAvailable(false) }
        assertEquals(AccountState.OFFLINE, r.status())
        // The change notice for the status chip arrives.
        await("a notice that lists the account") { r.changes.firstOrNull { account.id in it.accounts } }
        r.run { r.repo.setNetworkAvailable(true) }
        await("online again") { r.status().takeIf { it == AccountState.OK } }
    }

    @Test(timeout = 60_000)
    fun starredIsAMailboxOfTheFlaggedMessagesOfEveryFolderExceptTrashAndSpam() {
        val r = rig()
        r.awaitInbox(3)
        val folders = r.run { r.repo.listFolders(r.account) }
        val starred = folders.single { it.role == FolderRole.STARRED }
        assertTrue(starred.isVirtual)
        assertEquals(VirtualFolders.starred(r.account), starred.id)
        assertTrue("real folders are not virtual", folders.filter { it.role != FolderRole.STARRED }.none { it.isVirtual })
        val scope = MessageScope.Folder(r.account, starred.id)
        assertEquals(emptyList<MessageRowData>(), r.run { r.repo.listMessages(scope, null, 50) }.rows)

        // Starred on the server in the Inbox, and in Archive; one in Trash does not count.
        val inboxUid = r.server.uids("INBOX").first()
        r.server.changeFlags("INBOX", inboxUid, listOf("\\Flagged"), emptyList())
        val archived = r.server.injectMessage("Archive", "Starred in Archive")
        r.server.changeFlags("Archive", archived, listOf("\\Flagged"), emptyList())
        val trashed = r.server.injectMessage("Trash", "Starred in Trash")
        r.server.changeFlags("Trash", trashed, listOf("\\Flagged"), emptyList())
        r.run { r.repo.syncNow(r.account) }

        val rows = await("two starred messages") { r.run { r.repo.listMessages(scope, null, 50) }.rows.takeIf { it.size == 2 } }
        assertEquals(setOf("Message 1", "Starred in Archive"), rows.map { it.subject }.toSet())
        assertTrue(rows.all { it.flagged })

        // Starring from the app shows in the mailbox at once.
        val other = r.inbox().first { !it.flagged }
        r.run { r.repo.setFlagged(listOf(other.id), true) }
        assertEquals(3, r.run { r.repo.listMessages(scope, null, 50) }.rows.size)
        r.run { r.repo.setFlagged(listOf(other.id), false) }
        assertEquals(2, r.run { r.repo.listMessages(scope, null, 50) }.rows.size)
    }

    /** A core that answers from a script (the test server always has an Archive folder): the folder rules need an account without one. */
    private class ScriptedCore(private val folders: List<uniffi.croft_ffi.FolderInfo>) : Core(NoHandle) {
        val moves = mutableListOf<Pair<List<Long>, Long>>()
        val calls = mutableListOf<String>()

        override fun listMessages(scope: uniffi.croft_ffi.MessageScope, before: uniffi.croft_ffi.RowCursor?, limit: UInt) = uniffi.croft_ffi.MessagePage(
            rows = listOf(100L, 101L).map { row(it) },
            next = null,
        )

        private fun row(id: Long) = uniffi.croft_ffi.MessageRow(
            id = id, account = 7, folder = 1, senderName = null, senderAddress = null, subject = "S$id", snippet = "", dateMs = id,
            seen = false, flagged = false, hasAttachments = false, pending = false, threadCount = 1u, recipients = emptyList(),
        )

        override fun listFolders(accountId: Long) = folders
        override fun archive(ids: List<Long>): List<ULong> { calls += "archive"; throw uniffi.croft_ffi.CoreException.NoArchiveFolder() }
        override fun trash(ids: List<Long>): List<ULong> { calls += "trash"; return ids.map { 1uL } }
        override fun moveTo(ids: List<Long>, folderId: Long): List<ULong> { moves += ids to folderId; return ids.map { 5uL } }
    }

    private fun folder(id: Long, path: String, use: String) = uniffi.croft_ffi.FolderInfo(id, path, use, 0u, 0u)

    @Test
    fun anAccountWithoutAnArchiveFolderRefusesWithItsId_andTheChosenFolderIsRemembered() {
        val core = ScriptedCore(listOf(folder(1, "INBOX", "inbox"), folder(3, "Trash", "trash"), folder(5, "Old mail", "")))
        val mapping = InMemoryFolderMapping()
        val repo = CoreMessageRepository(core, mapping, kotlinx.coroutines.Dispatchers.Unconfined)
        fun archive(vararg ids: Long) = kotlinx.coroutines.runBlocking { repo.archive(ids.toList()) }
        kotlinx.coroutines.runBlocking { repo.listMessages(MessageScope.UnifiedInbox, null, 50) } // the repository learns the account of the rows

        // Refused before anything changes, naming the account, so that the screen can ask which folder to use.
        val e = try { archive(100); null } catch (e: MailException) { e }
        assertNotNull(e)
        assertEquals(MailError.NO_ARCHIVE_FOLDER, e!!.error)
        assertEquals(7L, e.account)
        assertTrue("the core was not asked to change anything", core.calls.isEmpty() && core.moves.isEmpty())

        // Trash has its folder: the core's own Trash is used.
        kotlinx.coroutines.runBlocking { repo.trash(listOf(100)) }
        assertEquals(listOf("trash"), core.calls)

        // The person chose "Old mail" once: archiving is a move there, for a whole selection, and the choice is kept.
        mapping.remember(7, FolderRole.ARCHIVE, 5)
        assertEquals(listOf(5L, 5L), archive(100, 101))
        assertEquals(listOf(listOf(100L, 101L) to 5L), core.moves)
        assertEquals(5L, mapping.mapped(7, FolderRole.ARCHIVE))

        // A chosen folder that no longer exists is forgotten, and the person is asked again.
        mapping.remember(7, FolderRole.ARCHIVE, 99)
        val gone = try { archive(100); null } catch (e: MailException) { e }
        assertEquals(MailError.NO_ARCHIVE_FOLDER, gone!!.error)
        assertNull(mapping.mapped(7, FolderRole.ARCHIVE))
    }

    @Test
    fun markAsSpamUsesTheJunkFolderOrTheChosenOne_andRefusesWithTheAccountOtherwise() {
        val core = ScriptedCore(listOf(folder(1, "INBOX", "inbox"), folder(5, "Old mail", "")))
        val mapping = InMemoryFolderMapping()
        val repo = CoreMessageRepository(core, mapping, kotlinx.coroutines.Dispatchers.Unconfined)
        val actions = CoreRowActions(repo, mapping)
        val row = MessageRowData(100, 7, 1, null, null, "S", "", 1, seen = false, flagged = false, hasAttachments = false)
        val e = try { kotlinx.coroutines.runBlocking { actions.markSpam(row) }; null } catch (e: MailException) { e }
        assertEquals(MailError.NO_SPAM_FOLDER, e!!.error)
        assertEquals(7L, e.account)
        mapping.remember(7, FolderRole.SPAM, 5)
        kotlinx.coroutines.runBlocking { actions.markSpam(row) }
        assertEquals(listOf(listOf(100L) to 5L), core.moves)
    }

    @Test(timeout = 60_000)
    fun moveToAFolderAndSpamUseTheRowActions() {
        val r = rig(window = 300)
        val row = r.awaitInbox(3).first()
        r.settle()
        val actions = CoreRowActions(r.repo, r.mapping)
        val archive = r.folder("Archive")

        r.run { actions.moveTo(row, archive.id) }
        assertEquals("a destination is remembered for the Move to sheet", listOf(archive.id), r.mapping.recents(r.account))
        await("moved on the server") { r.server.subjects("Archive").takeIf { it == listOf(row.subject) } }

        // Mark as spam goes to the Junk folder.
        val next = r.inbox().first()
        r.run { actions.markSpam(next) }
        await("in Junk on the server") { r.server.subjects("Junk").takeIf { it == listOf(next.subject) } }
    }

    @Test(timeout = 60_000)
    fun theCoresFixedErrorsBecomeTypedMailErrors() {
        val r = rig()
        r.awaitInbox(3)
        fun error(block: suspend () -> Unit): MailError = try { r.run(block); fail("expected an error"); MailError.FAILED } catch (e: MailException) { e.error }
        assertEquals(MailError.NOT_FOUND, error { r.repo.markSeen(listOf(424_242), true) })
        assertEquals(MailError.NOT_FOUND, error { r.repo.moveTo(listOf(r.inbox().first().id), 424_242) })
        assertEquals(MailError.NOT_FOUND, error { r.repo.syncNow(424_242) })
        assertEquals(MailError.NOT_FOUND, error { r.repo.retryFailed(424_242) })
        assertFalse("too late or unknown: false, not an error", r.run { r.repo.undo(424_242) })
    }

    @Test
    fun virtualAndRealScopesRoundTripThroughTheirKeys() {
        val starred = MessageScope.Folder(3, VirtualFolders.starred(3))
        assertEquals(starred, MessageScope.fromKey(starred.key))
        val real = MessageScope.Folder(3, 17)
        assertEquals(real, MessageScope.fromKey(real.key))
        assertTrue(VirtualFolders.isVirtual(VirtualFolders.starred(1)))
        assertFalse(VirtualFolders.isVirtual(1))
    }
}
