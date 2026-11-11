// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.ui.swipe

import com.example.croft.mail.FolderRole
import com.example.croft.mail.MessageRowData
import com.example.croft.swipe.SwipeAction
import com.example.croft.swipe.SwipeSensitivity
import com.example.croft.swipe.SwipeSettings
import com.example.croft.ui.mail.SelectionAction
import com.example.croft.ui.mail.SelectionRules
import java.io.File
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/** Which swipe actions a row can do where it is, the Delete forever rule, the stored settings, and the non-gesture route of every result. */
class SwipeRulesTest {
    private fun row() = MessageRowData(1, 1, 1, "Sam", "sam@example.test", "Hello", "Hi", 0, seen = false, flagged = false, hasAttachments = false)

    @Test
    fun archiveIsNotOfferedInArchiveSentDraftsOutboxSpamAndTrash_norWithoutAnArchiveFolder() {
        for (role in listOf(FolderRole.ARCHIVE, FolderRole.SENT, FolderRole.DRAFTS, FolderRole.OUTBOX, FolderRole.SPAM, FolderRole.TRASH)) {
            assertFalse("Archive in $role", SwipeRules.available(SwipeAction.ARCHIVE, role, hasArchive = true, hasSpam = true))
        }
        for (role in listOf(FolderRole.INBOX, FolderRole.OTHER, FolderRole.STARRED)) {
            assertTrue("Archive in $role", SwipeRules.available(SwipeAction.ARCHIVE, role, hasArchive = true, hasSpam = true))
        }
        assertFalse(SwipeRules.available(SwipeAction.ARCHIVE, FolderRole.INBOX, hasArchive = false, hasSpam = true))
    }

    @Test
    fun markAsSpamIsNotOfferedInSpamTrashSentDraftsOutbox_norWithoutASpamFolder() {
        for (role in listOf(FolderRole.SPAM, FolderRole.TRASH, FolderRole.SENT, FolderRole.DRAFTS, FolderRole.OUTBOX)) {
            assertFalse("Spam in $role", SwipeRules.available(SwipeAction.SPAM, role, hasArchive = true, hasSpam = true))
        }
        assertTrue(SwipeRules.available(SwipeAction.SPAM, FolderRole.INBOX, hasArchive = true, hasSpam = true))
        assertTrue(SwipeRules.available(SwipeAction.SPAM, FolderRole.ARCHIVE, hasArchive = true, hasSpam = true))
        assertFalse(SwipeRules.available(SwipeAction.SPAM, FolderRole.INBOX, hasArchive = true, hasSpam = false))
    }

    @Test
    fun readStarDeleteMoveAndSnoozeAreAlwaysAvailable() {
        for (role in FolderRole.entries) for (a in listOf(SwipeAction.TOGGLE_READ, SwipeAction.TOGGLE_STAR, SwipeAction.DELETE, SwipeAction.MOVE_TO, SwipeAction.SNOOZE)) {
            assertTrue("$a in $role", SwipeRules.available(a, role, hasArchive = false, hasSpam = false))
        }
    }

    @Test
    fun deleteInTrashAndSpamDeletesForeverAndParksTheCard() {
        for (role in listOf(FolderRole.TRASH, FolderRole.SPAM)) {
            assertTrue(SwipeRules.deletesForever(SwipeAction.DELETE, role))
            assertTrue("the slot parks", SwipeRules.slot(SwipeAction.DELETE, role, true, true).parks)
        }
        assertFalse(SwipeRules.deletesForever(SwipeAction.DELETE, FolderRole.INBOX))
        assertFalse(SwipeRules.slot(SwipeAction.DELETE, FolderRole.INBOX, true, true).parks)
        assertFalse(SwipeRules.deletesForever(SwipeAction.ARCHIVE, FolderRole.TRASH))
        assertTrue("Move to always parks", SwipeRules.slot(SwipeAction.MOVE_TO, FolderRole.INBOX, true, true).parks)
    }

    /** Every swipe result has a route that is not a gesture: the selection toolbar (its buttons or its overflow menu) or the row's own actions. */
    @Test
    fun everySwipeResultHasANonGestureRoute() {
        val inInbox = SelectionRules.toolbar(listOf(row()), FolderRole.INBOX, archiveAvailable = true, moveAvailable = true)
        val all = inInbox.primary + inInbox.overflow
        val routeOf = mapOf(
            SwipeAction.ARCHIVE to SelectionAction.ARCHIVE,
            SwipeAction.DELETE to SelectionAction.DELETE,
            SwipeAction.TOGGLE_READ to SelectionAction.MARK_READ,
            SwipeAction.TOGGLE_STAR to SelectionAction.STAR,
            SwipeAction.SPAM to SelectionAction.SPAM,
            SwipeAction.MOVE_TO to SelectionAction.MOVE_TO,
        )
        for (a in SwipeAction.picker.filter { it != SwipeAction.NONE }) {
            assertTrue("$a has a toolbar route", routeOf.getValue(a) in all)
        }
        // And in Trash, where Delete means delete forever.
        val inTrash = SelectionRules.toolbar(listOf(row()), FolderRole.TRASH, archiveAvailable = true)
        assertTrue(SelectionAction.DELETE_FOREVER in inTrash.primary)
    }

    @Test
    fun theSettingsFileRoundTripsAndNeverFails() {
        val s = SwipeSettings(SwipeAction.ARCHIVE, SwipeAction.SPAM, SwipeSensitivity.HIGH)
        assertEquals(s, SwipeSettingsJson.decodeOrDefault(SwipeSettingsJson.encode(s)))
        assertEquals(SwipeSettingsJson.encode(s), SwipeSettingsJson.encode(SwipeSettingsJson.decodeOrDefault(SwipeSettingsJson.encode(s))))
        assertEquals(SwipeSettings.Default, SwipeSettingsJson.decodeOrDefault(""))
        assertEquals(SwipeSettings.Default, SwipeSettingsJson.decodeOrDefault("not json"))
        assertEquals(SwipeSettings.Default, SwipeSettingsJson.decodeOrDefault("[1,2]"))
        assertEquals("a newer file is not read", SwipeSettings.Default, SwipeSettingsJson.decodeOrDefault("""{"version":99,"startToEnd":"archive","endToStart":"archive"}"""))
        assertEquals("an unknown action keeps that side's default", SwipeSettings(SwipeAction.TOGGLE_READ, SwipeAction.ARCHIVE), SwipeSettingsJson.decodeOrDefault("""{"startToEnd":"fly","endToStart":"archive"}"""))
        assertEquals(
            "Snooze has no sheet yet and is not read; Move to has one (S30) and is",
            SwipeSettings(SwipeAction.TOGGLE_READ, SwipeAction.MOVE_TO),
            SwipeSettingsJson.decodeOrDefault("""{"startToEnd":"snooze","endToStart":"move","sensitivity":"nope"}"""),
        )
    }

    @Test
    fun theFileIsStoredStartToEndSoRightToLeftMirrors() {
        val text = SwipeSettingsJson.encode(SwipeSettings.Default)
        assertTrue(text.contains("\"startToEnd\": \"read\""))
        assertTrue(text.contains("\"endToStart\": \"delete\""))
        assertFalse("no physical directions in the file", text.contains("left") || text.contains("right"))
    }

    @Test
    fun theSwipeModuleIsFreeOfUiTypes() {
        val dir = File("../swipe/src/main/kotlin")
        val sources = dir.walkTopDown().filter { it.isFile && it.extension == "kt" }.toList()
        assertTrue("found ${sources.size} files; is the working directory the app module?", sources.size >= 3)
        for (f in sources) for (line in f.readLines()) {
            if (line.startsWith("import ")) {
                assertFalse("${f.name}: $line", line.startsWith("import android.") || line.startsWith("import androidx.") || line.startsWith("import com.example.croft.ui") || line.startsWith("import com.example.croft.theme"))
            }
        }
    }
}
