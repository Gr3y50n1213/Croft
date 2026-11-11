// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.ui.swipe

import androidx.activity.ComponentActivity
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.SemanticsActions
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.test.SemanticsMatcher
import androidx.compose.ui.test.hasAnyDescendant
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.longClick
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performTouchInput
import com.example.croft.mail.FakeData
import com.example.croft.mail.MessageScope
import com.example.croft.swipe.SwipeAction
import com.example.croft.swipe.SwipeDirection
import com.example.croft.swipe.SwipeSettings
import com.example.croft.theme.AnimationLevel
import com.example.croft.ui.mail.InboxFixture
import com.example.croft.ui.mail.rowCard
import com.example.croft.ui.mail.showInbox
import com.example.croft.ui.mail.waitForRow
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

/**
 * The swipe actions for people who do not swipe (docs/ux/accessibility.md, "Screen readers"): custom actions on every row for each
 * configured swipe action plus Select, Reply and Star, that follow a settings change; a toolbar or menu route for every swipe result;
 * and the announcements.
 */
@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [35], qualifiers = "w360dp-h800dp-mdpi")
class SwipeSemanticsTest {
    @get:Rule
    val rule = createAndroidComposeRule<ComponentActivity>()

    private fun labels(id: Long) = rule.rowCard(id).fetchSemanticsNode().config[SemanticsActions.CustomActions].map { it.label }

    private fun invoke(id: Long, label: String) {
        val action = rule.rowCard(id).fetchSemanticsNode().config[SemanticsActions.CustomActions].first { it.label == label }
        rule.runOnUiThread { action.action() }
    }

    private fun textExists(text: String) = rule.onAllNodes(hasText(text)).fetchSemanticsNodes().isNotEmpty()
    private fun waitForText(text: String) = rule.waitUntil(10_000) { textExists(text) }

    @Test
    fun a11y_theCustomActionsFollowASettingsChange() {
        var settings by mutableStateOf(SwipeSettings.Default)
        val f = InboxFixture(80)
        rule.showInbox(f, AnimationLevel.OFF, swipe = { settings })
        rule.waitForRow(f.topId)
        val id = f.topId
        assertEquals("Triage: Select and Star, then right and left, then Reply", listOf("Select", "Star", "Mark as read", "Delete", "Reply"), labels(id))

        rule.runOnUiThread { settings = SwipeSettings(SwipeAction.ARCHIVE, SwipeAction.SPAM) }
        rule.waitForIdle()
        assertEquals(listOf("Select", "Star", "Archive", "Mark as spam", "Reply"), labels(id))

        rule.runOnUiThread { settings = SwipeSettings(SwipeAction.TOGGLE_STAR, SwipeAction.NONE) }
        rule.waitForIdle()
        assertEquals("Star is the row's own action already; None adds nothing", listOf("Select", "Star", "Reply"), labels(id))

        rule.runOnUiThread { settings = SwipeSettings(SwipeAction.DELETE, SwipeAction.DELETE) }
        rule.waitForIdle()
        assertEquals("the same action twice is listed once", listOf("Select", "Star", "Delete", "Reply"), labels(id))
    }

    @Test
    fun a11y_theActionNamesFollowTheRow() {
        val f = InboxFixture(80)
        rule.showInbox(f, AnimationLevel.OFF)
        rule.waitForRow(f.topId)
        val ids = f.inboxIds()
        val unread = ids.first { id -> f.repo.snapshot(MessageScope.UnifiedInbox).first { it.id == id }.let { !it.seen } }
        val read = ids.first { id -> f.repo.snapshot(MessageScope.UnifiedInbox).first { it.id == id }.let { it.seen } }
        rule.waitForRow(unread)
        rule.waitForRow(read)
        assertTrue("Mark as read on an unread row", "Mark as read" in labels(unread))
        assertTrue("Mark as unread on a read row", "Mark as unread" in labels(read) && "Mark as read" !in labels(read))
    }

    @Test
    fun a11y_anActionThatCannotApplyIsNotListed() {
        val sent = FakeData.folderId(1, FakeData.SENT)
        val f = InboxFixture(rows = 2, data = listOf(message(2, FakeData.SENT, seen = true), message(1, FakeData.SENT, seen = true)))
        rule.showInbox(f, AnimationLevel.OFF, startScope = MessageScope.Folder(1, sent), swipe = { SwipeSettings(SwipeAction.ARCHIVE, SwipeAction.SPAM) })
        rule.waitForRow(1)
        assertEquals("no Archive or Mark as spam in Sent", listOf("Select", "Star", "Reply"), labels(1))
    }

    @Test
    fun a11y_everyConfigurableActionIsAnActionOfTheRowAndWorks() {
        val f = InboxFixture(80)
        rule.showInbox(f, AnimationLevel.OFF, swipe = { SwipeSettings(SwipeAction.ARCHIVE, SwipeAction.SPAM) })
        rule.waitForRow(f.topId)
        val ids = f.inboxIds()
        invoke(ids[0], "Archive")
        waitForText("1 conversation archived")
        invoke(ids[1], "Mark as spam")
        waitForText("1 conversation marked as spam")
        assertEquals(listOf("archive ${ids[0]}", "markSpam ${ids[1]}"), f.actions.calls)
        invoke(ids[2], "Reply")
        waitForText("Reply comes with the compose screen, which is next.")
    }

    @Test
    fun a11y_markReadAndStarAreQuietActions_noSnackbar() {
        val f = InboxFixture(80)
        rule.showInbox(f, AnimationLevel.OFF)
        rule.waitForRow(f.topId)
        val id = f.topId
        invoke(id, "Mark as read")
        rule.waitUntil(10_000) { f.actions.calls == listOf("markSeen $id true") }
        invoke(id, "Star")
        rule.waitUntil(10_000) { f.repo.snapshot(MessageScope.UnifiedInbox).first { it.id == id }.flagged }
        assertFalse(textExists("Undo"))
    }

    @Test
    fun a11y_deleteForeverFromTheRowsActionsAsksFirst() {
        val f = InboxFixture(rows = 2, data = listOf(message(2, FakeData.TRASH, seen = true), message(1, FakeData.TRASH, seen = true)))
        rule.showInbox(f, AnimationLevel.OFF, startScope = MessageScope.Folder(1, FakeData.folderId(1, FakeData.TRASH)))
        rule.waitForRow(1)
        assertTrue("Delete forever, not Delete, in Trash", "Delete forever" in labels(1) && "Delete" !in labels(1))
        invoke(1, "Delete forever")
        waitForText("Delete 1 conversation for good?")
        assertTrue(f.actions.calls.isEmpty())
        rule.onNodeWithText("Delete forever").performClick()
        waitForText("1 conversation deleted for good")
        assertEquals(listOf("deleteForever 1"), f.actions.calls)
    }

    /** Every swipe result (the picker's actions) is an action of the row for each direction that holds it. */
    @Test
    fun a11y_everySwipeResultHasANonGestureRoute_theRowsOwnActions() {
        var settings by mutableStateOf(SwipeSettings.Default)
        val f = InboxFixture(80)
        rule.showInbox(f, AnimationLevel.OFF, swipe = { settings })
        rule.waitForRow(f.topId)
        val id = f.topId
        val expected = mapOf(
            SwipeAction.TOGGLE_READ to "Mark as read", SwipeAction.ARCHIVE to "Archive", SwipeAction.DELETE to "Delete",
            SwipeAction.TOGGLE_STAR to "Star", SwipeAction.SPAM to "Mark as spam",
        )
        for (action in SwipeAction.picker.filter { it != SwipeAction.NONE }) for (direction in SwipeDirection.entries) {
            rule.runOnUiThread { settings = settings.with(direction, action) }
            rule.waitForIdle()
            assertTrue("$action ($direction) is a custom action: ${labels(id)}", expected.getValue(action) in labels(id))
        }
    }

    /** ... and a button of the selection toolbar or an item of its menu. */
    @Test
    fun a11y_everySwipeResultHasANonGestureRoute_theSelectionToolbarAndItsMenu() {
        val f = InboxFixture(80)
        rule.showInbox(f, AnimationLevel.OFF)
        rule.waitForRow(f.topId)
        rule.rowCard(f.topId).performTouchInput { longClick() }
        rule.waitForIdle()
        for (button in listOf("Archive", "Delete", "Mark as read")) rule.onNodeWithContentDescription(button).assertExists()
        rule.onNodeWithContentDescription("More actions").performClick()
        rule.waitForIdle()
        rule.onNodeWithText("Star").assertExists()
        rule.onNodeWithText("Mark as spam").assertExists()
    }
}
