// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.ui.mail

import com.example.croft.mail.FolderRole
import com.example.croft.mail.MessageRowData
import java.time.ZoneId
import java.time.ZonedDateTime
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/** The pure rules of the message list: the selection toolbar (docs/ux/controls.md, message-list.md) and the date headers. */
class ListRulesTest {
    private fun row(id: Long, seen: Boolean = true, flagged: Boolean = false, date: Long = 1_000L * id) =
        MessageRowData(id, 1, 101, "Sender $id", "s$id@example.org", "Subject", "Snippet", date, seen, flagged, hasAttachments = false)

    private fun toolbar(rows: List<MessageRowData>, role: FolderRole = FolderRole.INBOX, archive: Boolean = true) = SelectionRules.toolbar(rows, role, archive)

    // ---- S11 toolbar ----

    @Test
    fun markAsReadAppearsIfAtLeastOneSelectedMessageIsUnread_otherwiseMarkAsUnread() {
        assertTrue(SelectionAction.MARK_READ in toolbar(listOf(row(1, seen = false))).primary)
        assertTrue("mixed: at least one unread", SelectionAction.MARK_READ in toolbar(listOf(row(1, seen = true), row(2, seen = false))).primary)
        val allRead = toolbar(listOf(row(1), row(2))).primary
        assertTrue(SelectionAction.MARK_UNREAD in allRead && SelectionAction.MARK_READ !in allRead)
    }

    @Test
    fun theInboxOffersArchiveDeleteAndTheReadToggle_inThatOrder() {
        assertEquals(listOf(SelectionAction.ARCHIVE, SelectionAction.DELETE, SelectionAction.MARK_READ), toolbar(listOf(row(1, seen = false))).primary)
    }

    @Test
    fun archiveIsHiddenWhereTheFolderRulesSayAndWhenAnAccountHasNoArchive() {
        for (role in listOf(FolderRole.ARCHIVE, FolderRole.SENT, FolderRole.DRAFTS, FolderRole.SPAM, FolderRole.TRASH)) {
            assertFalse("no Archive in $role", SelectionAction.ARCHIVE in toolbar(listOf(row(1)), role).primary)
        }
        assertFalse(SelectionAction.ARCHIVE in toolbar(listOf(row(1)), FolderRole.INBOX, archive = false).primary)
        assertTrue("a custom folder can archive", SelectionAction.ARCHIVE in toolbar(listOf(row(1)), FolderRole.OTHER).primary)
    }

    @Test
    fun deleteBecomesDeleteForeverInTrashAndSpam_andRestoreTakesTheArchivesPlace() {
        for (role in listOf(FolderRole.TRASH, FolderRole.SPAM)) {
            val p = toolbar(listOf(row(1)), role).primary
            assertTrue(SelectionAction.DELETE_FOREVER in p && SelectionAction.DELETE !in p)
            assertEquals(SelectionAction.RESTORE, p.first())
        }
        assertEquals(SelectionAction.RESTORE, toolbar(listOf(row(1)), FolderRole.ARCHIVE).primary.first())
        assertTrue(SelectionAction.DELETE in toolbar(listOf(row(1)), FolderRole.ARCHIVE).primary)
    }

    @Test
    fun starFollowsTheSelectionInTheOverflowMenu() {
        assertEquals(listOf(SelectionAction.STAR), toolbar(listOf(row(1), row(2, flagged = true))).overflow)
        assertEquals(listOf(SelectionAction.UNSTAR), toolbar(listOf(row(1, flagged = true), row(2, flagged = true))).overflow)
    }

    @Test
    fun bulkArchiveOrDeleteOfMoreThanFiftyAsksFirst_andDeleteForeverAlwaysDoes() {
        assertFalse(SelectionRules.needsConfirmation(SelectionAction.ARCHIVE, 50))
        assertTrue(SelectionRules.needsConfirmation(SelectionAction.ARCHIVE, 51))
        assertFalse(SelectionRules.needsConfirmation(SelectionAction.DELETE, 1))
        assertTrue(SelectionRules.needsConfirmation(SelectionAction.DELETE, 5_000))
        assertTrue(SelectionRules.needsConfirmation(SelectionAction.DELETE_FOREVER, 1))
        assertFalse(SelectionRules.needsConfirmation(SelectionAction.MARK_READ, 5_000))
        assertFalse(SelectionRules.needsConfirmation(SelectionAction.STAR, 5_000))
    }

    // ---- Date headers ----

    private val zone = ZoneId.of("UTC")
    private fun at(y: Int, m: Int, d: Int, h: Int = 12) = ZonedDateTime.of(y, m, d, h, 0, 0, 0, zone).toInstant().toEpochMilli()
    private val now = at(2026, 11, 11, 15)

    @Test
    fun dateBucketsAreTodayYesterdayThisWeekThenMonths() {
        assertEquals(DateBucket.Today, DateBucket.of(at(2026, 11, 11, 0), now, zone))
        assertEquals(DateBucket.Yesterday, DateBucket.of(at(2026, 11, 10, 23), now, zone))
        assertEquals(DateBucket.ThisWeek, DateBucket.of(at(2026, 11, 9), now, zone))
        assertEquals(DateBucket.ThisWeek, DateBucket.of(at(2026, 11, 5), now, zone))
        assertEquals(DateBucket.Month(2026, 11), DateBucket.of(at(2026, 11, 4), now, zone))
        assertEquals(DateBucket.Month(2026, 9), DateBucket.of(at(2026, 9, 30), now, zone))
        assertEquals(DateBucket.Month(2025, 12), DateBucket.of(at(2025, 12, 1), now, zone))
        assertEquals("the future counts as today", DateBucket.Today, DateBucket.of(now + 86_400_000L * 3, now, zone))
    }

    @Test
    fun aHeaderComesBeforeEachNewBucket_andRowsKeepTheirKeys() {
        val rows = listOf(
            row(9, date = at(2026, 11, 11, 14)), row(8, date = at(2026, 11, 11, 9)), row(7, date = at(2026, 11, 10)),
            row(6, date = at(2026, 10, 2)), row(5, date = at(2026, 10, 1)),
        )
        val items = buildListItems(rows, now, zone)
        assertEquals(listOf("h:today", 9L, 8L, "h:yesterday", 7L, "h:m2026-10", 6L, 5L), items.map { it.key })
        assertEquals(items.size, items.map { it.key }.toSet().size)
    }

    @Test
    fun anEmptyListHasNoHeaders() {
        assertTrue(buildListItems(emptyList(), now, zone).isEmpty())
    }
}
