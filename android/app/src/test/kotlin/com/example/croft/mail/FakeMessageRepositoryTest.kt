// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.mail

import java.io.File
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Test

/** The fake repository behaves as the T10 surface is documented to: it is what the list screens are written against. */
class FakeMessageRepositoryTest {
    private val now = 1_800_000_000_000L
    private var clockMs = now
    private fun repo(count: Int = 300) = FakeMessageRepository(FakeData.rows(count, now), { clockMs }, FakeData.accounts(now))
    private val inbox = MessageScope.UnifiedInbox
    private fun <T> run(block: suspend () -> T): T = runBlocking { withTimeout(10_000) { block() } }

    @Test
    fun itCanHoldFiveThousandRowsInSeveralAccountsAndEveryRowState() {
        val r = repo(5_000)
        assertEquals(5_000, r.size)
        val everything = run { r.listAccounts() }.flatMap { a -> run { r.listFolders(a.id) } }
        assertEquals(3 * FakeData.folderSpecs.size, everything.size)
        assertEquals("every row is in some folder", 5_000, everything.filter { it.role != FolderRole.STARRED && it.role != FolderRole.SNOOZED }.sumOf { it.totalCount })
        var cursor: RowCursor? = null
        var pages = 0
        var collected = 0
        do {
            val page = run { r.listMessages(inbox, cursor, 500) }
            collected += page.rows.size
            cursor = page.next
            pages++
        } while (cursor != null)
        assertEquals(r.snapshot(inbox).size, collected)
        assertTrue("paged through in several pages", pages > 1)
        val rows = FakeData.rows(5_000, now)
        assertTrue("unread rows", rows.any { !it.seen })
        assertTrue("read rows", rows.any { it.seen })
        assertTrue("starred rows", rows.any { it.flagged })
        assertTrue("rows with an attachment", rows.any { it.hasAttachments })
        assertTrue("encrypted rows", rows.any { it.encrypted })
        assertTrue("conversations", rows.any { it.threadCount > 1 })
        assertTrue("sent rows with recipients", rows.any { it.recipients != null })
        assertEquals("ids are unique", 5_000, rows.map { it.id }.toSet().size)
    }

    @Test
    fun theSameSeedGivesTheSameRows() {
        assertEquals(FakeData.rows(200, now, seed = 7), FakeData.rows(200, now, seed = 7))
        assertTrue(FakeData.rows(200, now, seed = 7) != FakeData.rows(200, now, seed = 8))
    }

    @Test
    fun listMessagesPagesByKeysetNewestFirstAndTheLastPageHasNoNext() {
        val r = repo(400)
        val total = r.snapshot(inbox)
        assertTrue("an inbox of more than three pages", total.size > 150)
        val first = run { r.listMessages(inbox, null, 50) }
        assertEquals(total.take(50), first.rows)
        assertNotNull(first.next)
        assertEquals(first.rows.last().cursor, first.next)
        val second = run { r.listMessages(inbox, first.next, 100) }
        assertEquals(total.drop(50).take(100), second.rows)
        val collected = ArrayList(first.rows + second.rows)
        var cursor = second.next
        while (cursor != null) {
            val page = run { r.listMessages(inbox, cursor, 100) }
            collected += page.rows
            cursor = page.next
        }
        assertEquals("every row once, in order", total, collected)
        assertEquals("newest first", collected.sortedWith(compareByDescending<MessageRowData> { it.dateMs }.thenByDescending { it.id }), collected)
        val exact = run { r.listMessages(inbox, null, total.size) }
        assertNull("a page that ends exactly at the last row has no next", exact.next)
    }

    @Test
    fun theUnifiedInboxIsTheInboxesOfEveryAccountAndAFolderIsOneFolder() {
        val r = repo(600)
        val unified = r.snapshot(inbox)
        assertEquals(setOf(1L, 2L, 3L), unified.map { it.account }.toSet())
        assertTrue(unified.all { it.folder % 100 == FakeData.INBOX })
        val sent = r.snapshot(MessageScope.Folder(1, FakeData.folderId(1, FakeData.SENT)))
        assertTrue(sent.isNotEmpty() && sent.all { it.account == 1L && it.folder == FakeData.folderId(1, FakeData.SENT) })
        val starred = r.snapshot(MessageScope.Folder(1, FakeData.folderId(1, FakeData.STARRED)))
        assertTrue(starred.isNotEmpty() && starred.all { it.flagged && it.account == 1L })
    }

    @Test
    fun nextChangeWaitsForAChangeAndThenNamesTheFolder() {
        val r = repo()
        val seen = run { r.injectNewMail(2, account = 2) }
        assertEquals(2, seen.size)
        val batch = run { r.nextChange(0) }
        assertTrue(batch.seq > 0)
        assertTrue(FolderRef(2, FakeData.folderId(2, FakeData.INBOX)) in batch.folders)
        val rows = r.snapshot(inbox)
        assertEquals("injected mail is newer than every row", seen, rows.take(2).map { it.id })
        assertTrue(rows.take(2).all { !it.seen })
        // Asking again from the new position has to wait.
        try {
            runBlocking { withTimeout(200) { r.nextChange(batch.seq) } }
            fail("nothing changed, so nextChange must not return")
        } catch (_: kotlinx.coroutines.TimeoutCancellationException) {
        }
    }

    @Test
    fun offlineMarksAccountsAndPendingRowsAndComingBackConfirmsThem() {
        val r = repo()
        val row = r.snapshot(inbox).first()
        r.setOffline(true)
        assertTrue(run { r.listAccounts() }.all { it.status == AccountState.OFFLINE })
        run { r.setFlagged(listOf(row.id), !row.flagged) }
        val pending = r.snapshot(inbox).first { it.id == row.id }
        assertTrue("an action while offline is pending (the clock)", pending.pending)
        assertEquals("applied locally at once", !row.flagged, pending.flagged)
        r.setOffline(false)
        assertTrue(run { r.listAccounts() }.all { it.status == AccountState.OK })
        assertFalse(r.snapshot(inbox).first { it.id == row.id }.pending)
    }

    @Test
    fun anAccountCanFlipToNeedsSignInAndSignedInBringsItBack() {
        val r = repo()
        r.setAccountStatus(2, AccountState.NEEDS_SIGN_IN)
        assertEquals(AccountState.NEEDS_SIGN_IN, run { r.listAccounts() }.first { it.id == 2L }.status)
        run { r.signedIn(2) }
        assertEquals(AccountState.OK, run { r.listAccounts() }.first { it.id == 2L }.status)
    }

    @Test
    fun archiveAndTrashMoveAndUndoWorksInsideTheWindowOnly() {
        val r = repo()
        val row = r.snapshot(inbox).first { it.account == 1L }
        val ops = run { r.archive(listOf(row.id)) }
        assertEquals(FakeData.folderId(1, FakeData.ARCHIVE), r.snapshot(MessageScope.Folder(1, FakeData.folderId(1, FakeData.ARCHIVE))).first { it.id == row.id }.folder)
        assertTrue(r.snapshot(inbox).none { it.id == row.id })
        assertTrue("undo inside the window", run { r.undo(ops.single()) })
        assertTrue(r.snapshot(inbox).any { it.id == row.id })
        val trashed = run { r.trash(listOf(row.id)) }
        clockMs += 6_000
        assertFalse("too late: the window is 5 s", run { r.undo(trashed.single()) })
        assertTrue(r.snapshot(inbox).none { it.id == row.id })
    }

    @Test
    fun aMissingArchiveFolderIsATypedErrorBeforeAnyChange() {
        val r = repo()
        r.removeFolder(1, FakeData.ARCHIVE)
        val ids = r.snapshot(inbox).filter { it.account == 1L }.take(2).map { it.id }
        try {
            run { r.archive(ids) }
            fail("expected NO_ARCHIVE_FOLDER")
        } catch (e: MailException) {
            assertEquals(MailError.NO_ARCHIVE_FOLDER, e.error)
        }
        assertTrue("nothing moved", ids.all { id -> r.snapshot(inbox).any { it.id == id } })
    }

    @Test
    fun deleteForeverRemovesAndCannotBeUndone() {
        val r = repo()
        val row = r.snapshot(inbox).first()
        val ops = run { r.deleteForever(listOf(row.id)) }
        assertTrue(r.snapshot(inbox).none { it.id == row.id })
        assertFalse(run { r.undo(ops.single()) })
    }

    @Test
    fun heldQueriesWaitUntilReleasedAndEveryRequestIsLogged() {
        val r = repo()
        r.holdQueries()
        val result = java.util.concurrent.atomic.AtomicReference<MessagePage?>()
        val t = Thread { result.set(runBlocking { r.listMessages(inbox, null, 50) }) }
        t.start()
        Thread.sleep(150)
        assertNull("still held", result.get())
        r.release()
        t.join(5_000)
        assertNotNull(result.get())
        assertEquals(listOf(ListRequest(inbox, null, 50)), r.requests)
    }

    @Test
    fun scopeKeysRoundTrip() {
        for (s in listOf(MessageScope.UnifiedInbox, MessageScope.Folder(3, 301))) assertEquals(s, MessageScope.fromKey(s.key))
        assertNull(MessageScope.fromKey("nonsense"))
        assertNull(MessageScope.fromKey(null))
    }

    @Test
    fun folderRolesComeFromTheSpecialUse() {
        assertEquals(FolderRole.SPAM, FolderRole.fromSpecialUse("junk"))
        assertEquals(FolderRole.TRASH, FolderRole.fromSpecialUse("TRASH"))
        assertEquals(FolderRole.OTHER, FolderRole.fromSpecialUse(""))
        assertEquals("Receipts", FolderInfo(1, 1, "Mail/Receipts", "", 0, 0).name)
    }

    @Test
    fun theTextOfARowNeverShowsInItsDebugString() {
        val row = FakeData.rows(10, now).first()
        assertFalse(row.toString().contains(row.subject))
        assertFalse(row.toString().contains(row.senderAddress.orEmpty()))
        assertFalse(FakeData.accounts(now).first().toString().contains("@"))
    }

    @Test
    fun theAnchorFileKeepsAPlaceAcrossAStore() {
        val file = File.createTempFile("croft-anchors", ".json")
        try {
            val a = FileAnchorStore(file)
            val anchor = ListAnchor(42, 1_700_000_000_000L, -12.5f)
            a.save("unified", anchor)
            a.save("folder:1:101", ListAnchor(7, 5L, 3f))
            a.flush()
            val b = FileAnchorStore(file)
            assertEquals(anchor, b.load("unified"))
            assertEquals(ListAnchor(7, 5L, 3f), b.load("folder:1:101"))
            assertNull(b.load("folder:2:201"))
            b.save("unified", null)
            b.flush()
            assertNull(FileAnchorStore(file).load("unified"))
        } finally { file.delete() }
    }

    @Test
    fun anUnreadableAnchorFileIsEmptyNotACrash() {
        val file = File.createTempFile("croft-anchors", ".json")
        try {
            file.writeText("{ not json")
            assertNull(FileAnchorStore(file).load("unified"))
        } finally { file.delete() }
    }

    @Test
    fun categoriesFilterTheInboxCountUnreadAndAlwaysMovesTheSender() = run {
        val r = repo(0)
        val ids = r.injectNewMail(3)
        val sender = r.snapshot(MessageScope.UnifiedInbox).first { it.id == ids[0] }.senderAddress
        assertEquals(3, r.listMessagesIn(MessageScope.UnifiedInbox, MailCategory.PRIMARY, null, 50).rows.size)
        assertEquals(0, r.listMessagesIn(MessageScope.UnifiedInbox, MailCategory.SOCIAL, null, 50).rows.size)
        assertEquals(3L, r.categoryCounts(null)[MailCategory.PRIMARY])
        r.setCategory(ids[0], MailCategory.SOCIAL, always = false)
        assertEquals(listOf(ids[0]), r.listMessagesIn(MessageScope.UnifiedInbox, MailCategory.SOCIAL, null, 50).rows.map { it.id })
        assertEquals(1L, r.categoryCounts(null)[MailCategory.SOCIAL])
        // "Always do this" moves every message of that sender.
        r.setCategory(ids[0], MailCategory.UPDATES, always = true)
        val sameSender = r.snapshot(MessageScope.UnifiedInbox).filter { it.senderAddress == sender }.map { it.id }.toSet()
        assertEquals(sameSender, r.listMessagesIn(MessageScope.UnifiedInbox, MailCategory.UPDATES, null, 50).rows.map { it.id }.toSet())
        // No category: everything.
        assertEquals(3, r.listMessagesIn(MessageScope.UnifiedInbox, null, null, 50).rows.size)
    }

    @Test
    fun theSampleGuessUsesTheSenderOnly() {
        assertEquals(MailCategory.UPDATES, guessCategory("no-reply@example.org"))
        assertEquals(MailCategory.PROMOTIONS, guessCategory("deals@shop.example"))
        assertEquals(MailCategory.SOCIAL, guessCategory("x@facebookmail.com"))
        assertEquals(MailCategory.PRIMARY, guessCategory("ann@example.org"))
    }
}
