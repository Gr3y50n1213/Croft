// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.ui.swipe

import androidx.activity.ComponentActivity
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.semantics.SemanticsActions
import androidx.compose.ui.semantics.getOrNull
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.center
import androidx.compose.ui.test.down
import androidx.compose.ui.test.hasTestTag
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.longClick
import androidx.compose.ui.test.moveBy
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performTouchInput
import androidx.compose.ui.test.up
import com.example.croft.haptics.HapticEvent
import com.example.croft.mail.FakeData
import com.example.croft.mail.MessageScope
import com.example.croft.swipe.SwipeAction
import com.example.croft.swipe.SwipeSettings
import com.example.croft.theme.AnimationLevel
import com.example.croft.ui.components.SwipeEnabled
import com.example.croft.ui.mail.InboxFixture
import com.example.croft.ui.mail.InboxTags
import com.example.croft.ui.mail.rowCard
import com.example.croft.ui.mail.showInbox
import com.example.croft.ui.mail.waitForRow
import kotlinx.coroutines.runBlocking
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
 * The gesture layer of the swipe engine (S14, S15, S16) on the Inbox: slop, angle, the back-gesture strip, a second finger, a fling,
 * selection mode, a row removed mid-drag, an unavailable action and Delete forever. Real touch injection (`performTouchInput`) on the
 * real screen. The phone is 360 by 800 dp at 160 dpi, so 1 dp is 1 px; a row is 336 dp wide (12 dp of padding at each side), so the
 * arm point at Normal sensitivity is 117.6 dp and a fling counts from 67.2 dp. Every gesture ends with `up()`: a finger left on
 * the screen when the window goes away throws in the pull-to-refresh node (see the notes of T14).
 */
@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [35], qualifiers = "w360dp-h800dp-mdpi")
class SwipeGestureTest {
    @get:Rule
    val rule = createAndroidComposeRule<ComponentActivity>()

    private val inbox = MessageScope.UnifiedInbox

    private fun open(f: InboxFixture = InboxFixture(80), level: AnimationLevel = AnimationLevel.FULL, swipe: SwipeSettings = SwipeSettings.Default, scope: MessageScope = inbox): InboxFixture {
        rule.showInbox(f, level, startScope = scope, swipe = { swipe })
        rule.waitForRow(topIdOf(f, scope))
        return f
    }

    private fun topIdOf(f: InboxFixture, scope: MessageScope) = f.repo.snapshot(scope).first().id

    private fun textExists(text: String) = rule.onAllNodes(hasText(text)).fetchSemanticsNodes().isNotEmpty()
    private fun waitForText(text: String) = rule.waitUntil(10_000) { textExists(text) }

    /** A drag of [steps] moves of [dx] dp each, [stepMs] apart, from the row's centre, without lifting the finger. */
    private fun dragFrom(id: Long, dx: Float, steps: Int, stepMs: Long, startOffset: Float = 0f) {
        rule.rowCard(id).performTouchInput {
            down(center + Offset(startOffset, 0f))
            moveBy(Offset(if (dx > 0) 10f else -10f, 0f), delayMillis = stepMs) // past the 8 dp slop
            repeat(steps) { moveBy(Offset(dx, 0f), delayMillis = stepMs) }
        }
    }

    private fun lift(id: Long) { rule.rowCard(id).performTouchInput { up() }; rule.waitForIdle() }

    /** Drag the row of [id] and (unless [release] is false) let go. */
    private fun swipeGesture(id: Long, dx: Float, steps: Int, stepMs: Long, release: Boolean = true) {
        dragFrom(id, dx, steps, stepMs)
        if (release) lift(id)
    }

    // ================================================ Start of a swipe ================================================

    @Test
    fun s14_slop_aDragUnderEightDpMovesNothing_thenTheCardFollowsFromWhereItTookHold() {
        val f = open()
        val id = f.topId
        rule.rowCard(id).performTouchInput { down(center); moveBy(Offset(7f, 0f)) }
        assertEquals("under the slop", 0f, rule.cardOffset(id), 0.5f)
        rule.rowCard(id).performTouchInput { moveBy(Offset(3f, 0f)) }
        assertEquals("no jump when the slop is passed", 0f, rule.cardOffset(id), 0.5f)
        rule.rowCard(id).performTouchInput { moveBy(Offset(25f, 0f)) }
        rule.waitForIdle()
        assertEquals("one to one after that", 25f, rule.cardOffset(id), 1f)
        rule.rowCard(id).performTouchInput { up() }
        rule.waitForIdle()
        assertEquals("not armed: it springs back", 0f, rule.cardOffset(id), 1f)
        assertTrue(f.actions.calls.isEmpty())
    }

    @Test
    fun s14_angle_aDragWithinThirtyDegreesStartsASwipe_andOneSteeperThanThatNeverBecomesOne() {
        val f = open()
        val id = f.topId
        // 50 degrees: the list's scroll. Going on sideways afterwards does not turn it into a swipe.
        rule.rowCard(id).performTouchInput { down(center); moveBy(Offset(10f, 12f)); moveBy(Offset(120f, 0f)) }
        rule.waitForIdle()
        assertEquals("a scroll that turns sideways stays a scroll", 0f, rule.cardOffset(id), 0.5f)
        rule.rowCard(id).performTouchInput { up() }
        rule.waitForIdle()

        // 14 degrees: a swipe.
        val again = f.topId
        rule.rowCard(again).performTouchInput { down(center); moveBy(Offset(40f, 10f)); moveBy(Offset(30f, 0f)) }
        rule.waitForIdle()
        assertEquals(30f, rule.cardOffset(again), 1f)
        rule.rowCard(again).performTouchInput { up() }
        rule.waitForIdle()
    }

    @Test
    fun s14_aDragThatStartsInTheSystemBackGestureStripIsLeftToTheSystem() {
        val f = open()
        val id = f.topId
        // The strip is 24 dp from the window's edge; the row starts 12 dp in, so 6 dp into the row is 18 dp from the edge.
        rule.rowCard(id).performTouchInput { down(Offset(6f, center.y)); moveBy(Offset(10f, 0f)); moveBy(Offset(100f, 0f)) }
        rule.waitForIdle()
        assertEquals(0f, rule.cardOffset(id), 0.5f)
        rule.rowCard(id).performTouchInput { up() }
        rule.waitForIdle()
        assertTrue(f.haptics.events.isEmpty())

        // 60 dp into the row is well clear of it.
        rule.rowCard(id).performTouchInput { down(Offset(60f, center.y)); moveBy(Offset(10f, 0f)); moveBy(Offset(40f, 0f)) }
        rule.waitForIdle()
        assertEquals(40f, rule.cardOffset(id), 1f)
        rule.rowCard(id).performTouchInput { up() }
        rule.waitForIdle()
    }

    @Test
    fun s14_aSecondFingerCancelsTheSwipe_withNoActionAndTheCardSpringsBack() {
        val f = open()
        val id = f.topId
        dragFrom(id, dx = 30f, steps = 5, stepMs = 100) // 160 dp: armed
        rule.waitForIdle()
        assertEquals(HapticEvent.ThresholdArmed, f.haptics.events.last())
        rule.rowCard(id).performTouchInput { down(1, center + Offset(0f, 20f)) }
        rule.waitForIdle()
        assertEquals("the card goes back", 0f, rule.cardOffset(id), 1f)
        rule.rowCard(id).performTouchInput { up(1); up(0) }
        rule.waitForIdle()
        assertTrue("no action", f.actions.calls.isEmpty())
        assertFalse(HapticEvent.Committed in f.haptics.events)
    }

    // ================================================ Release: arm, fling, haptics ================================================

    @Test
    fun s15_aFlingFastEnoughCommitsFromTwentyPercentEvenWhenNotArmed() {
        val f = open()
        val id = f.topId
        // 105 dp (the first move only passes the slop) in 70 ms is 1,500 dp/s, at 31 percent of the width: not armed (35 percent), but a fling.
        rule.rowCard(id).performTouchInput {
            down(center)
            repeat(8) { moveBy(Offset(-15f, 0f), delayMillis = 10) }
            up()
        }
        rule.waitUntil(10_000) { f.actions.calls == listOf("trash $id") }
        waitForText("1 conversation deleted")
        assertEquals(listOf(HapticEvent.Committed), f.haptics.events)
    }

    @Test
    fun s15_theSameDragSlowlyDoesNotCommit() {
        val f = open()
        val id = f.topId
        swipeGesture(id, dx = -15f, steps = 6, stepMs = 100)
        rule.waitForIdle()
        assertTrue(f.actions.calls.isEmpty())
        assertEquals(0f, rule.cardOffset(id), 1f)
        assertTrue("no haptic below the arm point", f.haptics.events.isEmpty())
        assertFalse(textExists("1 conversation deleted"))
    }

    @Test
    fun s15_aSlowDragPastTheArmPointCommitsOnRelease_withUndoInTheSnackbar() {
        val f = open()
        val id = f.topId
        swipeGesture(id, dx = -26f, steps = 5, stepMs = 100) // 140 dp: armed
        waitForText("1 conversation deleted")
        assertEquals(listOf("trash $id"), f.actions.calls)
        assertTrue(f.repo.snapshot(inbox).none { it.id == id })
        rule.onNodeWithText("Undo").assertExists()
        assertEquals(listOf(HapticEvent.ThresholdArmed, HapticEvent.Committed), f.haptics.events)
    }

    @Test
    fun s14_dragAcrossTheThresholdBackAcrossAgainAndReleaseGivesArmedDisarmedArmedCommitted() {
        val f = open()
        val id = f.topId
        rule.rowCard(id).performTouchInput {
            down(center)
            moveBy(Offset(10f, 0f), delayMillis = 100)  // past the slop
            moveBy(Offset(100f, 0f), delayMillis = 100) // 100 dp: below the 117.6 dp arm point
            moveBy(Offset(40f, 0f), delayMillis = 100)  // 140 dp: armed
            moveBy(Offset(-60f, 0f), delayMillis = 100) // 80 dp: disarmed
            moveBy(Offset(60f, 0f), delayMillis = 100)  // 140 dp: armed again
            up()
        }
        rule.waitForIdle()
        assertEquals(
            listOf(HapticEvent.ThresholdArmed, HapticEvent.ThresholdDisarmed, HapticEvent.ThresholdArmed, HapticEvent.Committed),
            f.haptics.events,
        )
        // Right is Mark read in the default settings: it keeps the row, changes it in place and shows no snackbar.
        rule.waitUntil(10_000) { f.actions.calls == listOf("markSeen $id true") }
        assertTrue(f.repo.snapshot(inbox).any { it.id == id && it.seen })
        assertFalse(textExists("Undo"))
    }

    // ================================================ Where a swipe is not allowed ================================================

    @Test
    fun s11_selectionModeSwitchesSwipesOff() {
        val f = open()
        val ids = f.inboxIds()
        rule.rowCard(ids[0]).performTouchInput { longClick() }
        rule.waitForIdle()
        f.haptics.clear()
        swipeGesture(ids[1], dx = -30f, steps = 6, stepMs = 50)
        rule.waitForIdle()
        assertEquals(0f, rule.cardOffset(ids[1]), 0.5f)
        assertTrue(f.actions.calls.isEmpty())
        assertTrue("no haptic from a swipe in selection mode: ${f.haptics.events}", f.haptics.events.none { it == HapticEvent.ThresholdArmed || it == HapticEvent.Committed })
        assertEquals(false, rule.rowCard(ids[1]).fetchSemanticsNode().config.getOrNull(SwipeEnabled))
    }

    @Test
    fun s14_aRowRemovedByASyncMidDragEndsTheDragWithoutAnAction() {
        val f = open()
        val id = f.topId
        dragFrom(id, dx = 20f, steps = 4, stepMs = 60)
        // Another device archived it: the row leaves the list under the finger.
        runBlocking { f.repo.archive(listOf(id)) }
        rule.waitUntil(10_000) { rule.onAllNodes(hasTestTag(InboxTags.row(id))).fetchSemanticsNodes().isEmpty() || f.repo.snapshot(inbox).none { it.id == id } }
        rule.waitForIdle()
        // The finger may still be down on whatever is there now; lift it wherever it is.
        rule.onNodeWithTag(InboxTags.LIST).performTouchInput { up() }
        rule.waitForIdle()
        assertTrue("no action ran through the row actions: ${f.actions.calls}", f.actions.calls.isEmpty())
        assertFalse(textExists("1 conversation deleted"))
        assertFalse(HapticEvent.Committed in f.haptics.events)
    }

    @Test
    fun s14_anUnavailableActionNeverArms_playsRejected_andSaysNotAvailableHere() {
        val sent = FakeData.folderId(1, FakeData.SENT)
        val f = InboxFixture(rows = 2, data = listOf(message(2, FakeData.SENT, seen = true), message(1, FakeData.SENT, seen = true)))
        val scope = MessageScope.Folder(1, sent)
        open(f, swipe = SwipeSettings(SwipeAction.ARCHIVE, SwipeAction.ARCHIVE), scope = scope)
        swipeGesture(2, dx = 30f, steps = 5, stepMs = 100) // 160 dp: past where Archive would arm
        waitForText("Not available here")
        rule.waitForIdle()
        // The notice is heard: the snackbar is a polite live region.
        rule.onNode(
            androidx.compose.ui.test.SemanticsMatcher.keyIsDefined(androidx.compose.ui.semantics.SemanticsProperties.LiveRegion) and
                androidx.compose.ui.test.hasAnyDescendant(hasText("Not available here")),
        ).assertExists()
        assertEquals(listOf(HapticEvent.Rejected), f.haptics.events)
        assertTrue(f.actions.calls.isEmpty())
        assertEquals(0f, rule.cardOffset(2), 1f)
    }

    @Test
    fun s15_deleteInTrashParksTheCardAtFortyPercentBehindAConfirmation_andCancelSpringsItBack() {
        val f = InboxFixture(rows = 2, data = listOf(message(2, FakeData.TRASH, seen = true), message(1, FakeData.TRASH, seen = true)))
        open(f, scope = MessageScope.Folder(1, FakeData.folderId(1, FakeData.TRASH)))
        swipeGesture(2, dx = -26f, steps = 5, stepMs = 100)
        waitForText("Delete 1 conversation for good?")
        rule.waitForIdle()
        assertEquals("parked at 40 percent of 336 dp", -134.4f, rule.cardOffset(2), 2f)
        assertTrue("nothing has been deleted yet", f.actions.calls.isEmpty())

        rule.onNodeWithText("Cancel").performClick()
        rule.waitForIdle()
        assertEquals("Cancel springs it back", 0f, rule.cardOffset(2), 1f)
        assertTrue(f.actions.calls.isEmpty())
        assertTrue(f.repo.snapshot(MessageScope.Folder(1, FakeData.folderId(1, FakeData.TRASH))).any { it.id == 2L })
    }

    @Test
    fun s15_confirmingDeleteForeverDeletesItForGood_withoutAnUndo() {
        val f = InboxFixture(rows = 2, data = listOf(message(2, FakeData.TRASH, seen = true), message(1, FakeData.TRASH, seen = true)))
        open(f, scope = MessageScope.Folder(1, FakeData.folderId(1, FakeData.TRASH)))
        swipeGesture(2, dx = -26f, steps = 5, stepMs = 100)
        waitForText("Delete 1 conversation for good?")
        rule.onNodeWithText("Delete forever").performClick()
        waitForText("1 conversation deleted for good")
        assertEquals(listOf("deleteForever 2"), f.actions.calls)
        assertFalse("no Undo for a delete that cannot be undone", textExists("Undo"))
    }

    // ================================================ Archive, spam and the row actions ================================================

    @Test
    fun s16_archiveAndSpamSwipesLeaveTheRowWithUndo() {
        val f = open(swipe = SwipeSettings(SwipeAction.ARCHIVE, SwipeAction.SPAM))
        val ids = f.inboxIds()
        swipeGesture(ids[0], dx = 30f, steps = 5, stepMs = 100)
        waitForText("1 conversation archived")
        swipeGesture(ids[1], dx = -30f, steps = 5, stepMs = 100)
        waitForText("1 conversation marked as spam")
        assertEquals(listOf("archive ${ids[0]}", "markSpam ${ids[1]}"), f.actions.calls)
    }

    @Test
    fun s16_undoReturnsTheRowThroughTheSameActions() {
        val f = open()
        val id = f.topId
        swipeGesture(id, dx = -26f, steps = 5, stepMs = 100)
        waitForText("1 conversation deleted")
        rule.onNodeWithText("Undo").performClick()
        rule.waitUntil(10_000) { f.repo.snapshot(inbox).any { it.id == id } }
        assertEquals(listOf("trash $id", "undo 1"), f.actions.calls)
    }

    @Test
    fun s14_aSwipeDoesNotOpenTheMessage_butATapStillDoes() {
        val opened = ArrayList<Long>()
        val f = InboxFixture(80)
        rule.showInbox(f, AnimationLevel.OFF, onOpenMessage = { opened += it })
        rule.waitForRow(f.topId)
        val id = f.topId
        swipeGesture(id, dx = 10f, steps = 4, stepMs = 50) // a short drag that springs back
        assertTrue("a drag is not a tap: $opened", opened.isEmpty())
        rule.rowCard(id).performClick()
        rule.waitForIdle()
        assertEquals(listOf(id), opened)
    }

    // ================================================ Settings change the gesture ================================================

    @Test
    fun s47_aSettingsChangeChangesWhatTheNextSwipeDoes() {
        var settings by mutableStateOf(SwipeSettings.Default)
        val f = InboxFixture(80)
        rule.showInbox(f, AnimationLevel.OFF, swipe = { settings })
        rule.waitForRow(f.topId)
        val id = f.topId
        rule.runOnUiThread { settings = SwipeSettings(SwipeAction.TOGGLE_STAR, SwipeAction.ARCHIVE) }
        rule.waitForIdle()
        swipeGesture(id, dx = -26f, steps = 5, stepMs = 100)
        waitForText("1 conversation archived")
        assertEquals(listOf("archive $id"), f.actions.calls)
    }

    @Test
    fun s14_noneInADirectionMeansTheRowDoesNotMoveAtAll() {
        val f = open(swipe = SwipeSettings(SwipeAction.NONE, SwipeAction.DELETE))
        val id = f.topId
        rule.rowCard(id).performTouchInput { down(center); moveBy(Offset(10f, 0f)); moveBy(Offset(120f, 0f)) }
        rule.waitForIdle()
        assertEquals(0f, rule.cardOffset(id), 0.5f)
        rule.rowCard(id).performTouchInput { up() }
        rule.waitForIdle()
        assertTrue(f.haptics.events.isEmpty())
        assertTrue(f.actions.calls.isEmpty())
    }

    @Test
    fun s14_aRowsActionsAreUsableWithoutAnyGesture() {
        val f = open()
        val id = f.topId
        val actions = rule.rowCard(id).fetchSemanticsNode().config[SemanticsActions.CustomActions]
        assertTrue(actions.any { it.label == "Delete" })
        rule.runOnUiThread { actions.first { it.label == "Mark as read" }.action() }
        rule.waitUntil(10_000) { f.actions.calls == listOf("markSeen $id true") }
    }
}
