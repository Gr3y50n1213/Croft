// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.ui.mail

import com.example.croft.mail.FolderInfo
import com.example.croft.mail.VirtualFolders
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/** S30 Move to, the rules apart from the screen: which folders can be chosen, the recent ones, the tree and the search. */
class MoveRulesTest {
    private fun folder(id: Long, path: String, use: String = "") = FolderInfo(id, 1, path, use, 0, 0)

    private val folders = listOf(
        folder(1, "INBOX", "inbox"),
        folder(2, "Archive", "archive"),
        folder(3, "Projects"),
        folder(4, "Projects/Croft"),
        folder(5, "projects/Ideas"),
        folder(6, "Receipts"),
        folder(VirtualFolders.starred(1), "Starred", "starred"),
    )

    @Test
    fun aMessageCanBeMovedToAnyRealFolderButItsOwnAndNeverToAVirtualMailbox() {
        val targets = MoveRules.targets(folders, excluding = setOf(1))
        assertFalse(targets.any { it.id == 1L })
        assertFalse("Starred is a query, not a folder", targets.any { it.isVirtual })
        assertEquals(setOf(2L, 3L, 4L, 5L, 6L), targets.map { it.id }.toSet())
    }

    @Test
    fun theTreeKeepsAParentBeforeItsChildrenAndIndentsByDepth() {
        val tree = MoveRules.tree(MoveRules.targets(folders, excluding = emptySet()))
        val paths = tree.map { it.folder.path }
        assertTrue(paths.indexOf("Projects") < paths.indexOf("Projects/Croft"))
        assertEquals(0, tree.first { it.folder.id == 3L }.depth)
        assertEquals(1, tree.first { it.folder.id == 4L }.depth)
    }

    @Test
    fun recentDestinationsAreNewestFirst_atMostThree_andOnlyFoldersThatCanStillBeChosen() {
        val targets = MoveRules.targets(folders, excluding = setOf(2))
        val recents = MoveRules.recents(targets, listOf(2, 6, 99, 3, 4, 5))
        assertEquals("2 is excluded, 99 does not exist", listOf(6L, 3L, 4L), recents.map { it.id })
    }

    @Test
    fun theSearchMatchesAnyPartOfThePathWithoutCaseAndShowsEverythingForABlankQuery() {
        val targets = MoveRules.targets(folders, excluding = setOf(1))
        assertEquals(setOf(3L, 4L, 5L), MoveRules.search(targets, "PROJ").map { it.id }.toSet())
        assertEquals(listOf(4L), MoveRules.search(targets, "croft").map { it.id })
        assertEquals(targets, MoveRules.search(targets, "  "))
        assertTrue(MoveRules.search(targets, "zzz").isEmpty())
    }
}
