// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.ui.mail

import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.test.down
import androidx.compose.ui.test.getUnclippedBoundsInRoot
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.longClick
import androidx.compose.ui.test.moveBy
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performTextInput
import androidx.compose.ui.test.performTouchInput
import androidx.compose.ui.test.up
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.width
import com.example.croft.mail.FolderRole
import com.example.croft.swipe.SwipeAction
import com.example.croft.swipe.SwipeSettings
import com.example.croft.theme.AnimationLevel
import com.example.croft.ui.components.show
import com.example.croft.ui.swipe.LocalBackGestureInset
import com.example.croft.ui.swipe.LocalSwipeSettings
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config

/**
 * S30 Move to and the folder rules, on the real repository over a scripted core. Account 1's folders are 101 Inbox, 102 Sent,
 * 103 Archive, 104 Trash, 105 Junk (see [CoreFixture]).
 */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35])
class MoveToTest {
    @get:Rule
    val rule = createComposeRule()

    private fun show(f: CoreFixture, swipe: SwipeSettings = SwipeSettings.Default) {
        rule.show(AnimationLevel.OFF, f.haptics) {
            CompositionLocalProvider(LocalSwipeSettings provides swipe, LocalBackGestureInset provides 24.dp) {
                InboxScreen(f.session, onOpenSettings = {})
            }
        }
        rule.waitForRow(10, 20_000)
    }

    private fun exists(text: String, substring: Boolean = false) = rule.onAllNodes(hasText(text, substring = substring)).fetchSemanticsNodes().isNotEmpty()

    private fun waitForText(text: String, substring: Boolean = false) = rule.waitUntil(20_000) { exists(text, substring) }

    private fun openMoveToFromTheToolbar() {
        rule.rowCard(10).performTouchInput { longClick() }
        rule.onNodeWithContentDescription("More actions").performClick()
        rule.onNodeWithText("Move to").performClick()
        rule.waitUntil(20_000) { rule.onAllNodes(androidx.compose.ui.test.hasTestTag(MoveTags.SHEET)).fetchSemanticsNodes().isNotEmpty() }
    }

    @Test
    fun s30_theToolbarsMoveToListsTheAccountsOtherFolders_andAPickedFolderMovesTheMailWithUndo() {
        val f = CoreFixture()
        show(f)
        openMoveToFromTheToolbar()
        // The folders of the message's own account, except the one it is in; never the other account's or a virtual mailbox.
        for (id in listOf(102L, 103L, 104L, 105L)) rule.onNodeWithTag(MoveTags.folder(id)).assertExists()
        rule.onNodeWithTag(MoveTags.folder(101)).assertDoesNotExist()
        rule.onNodeWithTag(MoveTags.folder(201)).assertDoesNotExist()
        rule.onNodeWithTag(MoveTags.folder(103)).performClick()
        waitForText("1 conversation moved")
        assertTrue(exists("Undo"))
        rule.waitUntil(20_000) { !exists("Lunch on Friday?") }
        assertEquals("the destination is a recent one now", listOf(103L), f.mapping.recents(1))
        rule.onNodeWithText("Undo").performClick()
        rule.waitForRow(10, 20_000)
    }

    @Test
    fun s30_theSearchNarrowsTheTree() {
        val f = CoreFixture()
        show(f)
        openMoveToFromTheToolbar()
        rule.onNodeWithTag(MoveTags.SEARCH).performTextInput("arch")
        rule.waitUntil(20_000) { rule.onAllNodes(androidx.compose.ui.test.hasTestTag(MoveTags.folder(104))).fetchSemanticsNodes().isEmpty() }
        rule.onNodeWithTag(MoveTags.folder(103)).assertExists()
        rule.onNodeWithTag(MoveTags.folder(102)).assertDoesNotExist()
    }

    @Test
    fun s30_aSwipeOnMoveToHoldsTheCardBehindTheSheet_aFolderSendsItAway_dismissingBringsItBack() {
        val f = CoreFixture()
        val swipe = SwipeSettings(startToEnd = SwipeAction.MOVE_TO, endToStart = SwipeAction.DELETE)
        show(f, swipe)
        val width = rule.onNodeWithTag(InboxTags.row(10)).getUnclippedBoundsInRoot().width.value
        fun swipeRight() = rule.rowCard(10).performTouchInput { down(center); moveBy(Offset(10f, 0f)); moveBy(Offset(width * 0.5f, 0f)); up() }

        swipeRight()
        rule.waitUntil(20_000) { rule.onAllNodes(androidx.compose.ui.test.hasTestTag(MoveTags.SHEET)).fetchSemanticsNodes().isNotEmpty() }
        // Back out of the sheet: nothing moves and the row is where it was.
        androidx.test.espresso.Espresso.pressBack()
        rule.waitUntil(20_000) { rule.onAllNodes(androidx.compose.ui.test.hasTestTag(MoveTags.SHEET)).fetchSemanticsNodes().isEmpty() }
        rule.onNodeWithTag(InboxTags.row(10)).assertExists()
        assertTrue(f.mapping.recents(1).isEmpty())

        // Again, and this time a folder: the card leaves, the mail moves, Undo is offered.
        swipeRight()
        rule.waitUntil(20_000) { rule.onAllNodes(androidx.compose.ui.test.hasTestTag(MoveTags.SHEET)).fetchSemanticsNodes().isNotEmpty() }
        rule.onNodeWithTag(MoveTags.folder(102)).performClick()
        waitForText("1 conversation moved")
        rule.waitUntil(20_000) { !exists("Lunch on Friday?") }
        assertEquals(listOf(102L), f.mapping.recents(1))
    }

    @Test
    fun anAccountWithoutAnArchiveFolderAsksOnce_theChoiceMovesTheMail_andIsNotAskedAgain() {
        val f = CoreFixture(archiveFor = setOf(2L))
        show(f)
        // Account 1 has no Archive folder: archiving asks which folder to use.
        rule.rowCard(10).performTouchInput { longClick() }
        rule.onNodeWithContentDescription("Archive").performClick()
        waitForText("Choose a folder to use as Archive")
        assertTrue("Archive can be declined for the account", exists("Don't offer Archive for this account"))
        rule.onNodeWithTag(MoveTags.folder(103)).performClick()
        waitForText("1 conversation archived")
        assertEquals("remembered for the account", 103L, f.mapping.mapped(1, FolderRole.ARCHIVE))
        rule.waitUntil(20_000) { !exists("Lunch on Friday?") }

        // The next Archive of that account does not ask.
        rule.onNodeWithText("Undo").performClick()
        rule.waitForRow(10, 20_000)
        rule.rowCard(10).performTouchInput { longClick() }
        rule.onNodeWithContentDescription("Archive").performClick()
        waitForText("1 conversation archived")
        assertTrue(!exists("Choose a folder to use as Archive"))
    }

    @Test
    fun decliningArchiveHidesItForThatAccount() {
        val f = CoreFixture(archiveFor = setOf(2L))
        show(f)
        rule.rowCard(10).performTouchInput { longClick() }
        rule.onNodeWithContentDescription("Archive").performClick()
        waitForText("Choose a folder to use as Archive")
        rule.onNodeWithTag(MoveTags.DECLINE).performClick()
        waitForText("Couldn't archive", substring = true)
        assertTrue(f.mapping.isDeclined(1, FolderRole.ARCHIVE))
        rule.onNodeWithContentDescription("Archive").assertDoesNotExist()
    }
}
