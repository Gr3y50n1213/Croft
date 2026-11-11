// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.ui.swipe

import androidx.activity.ComponentActivity
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.test.center
import androidx.compose.ui.test.down
import androidx.compose.ui.test.hasTestTag
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.moveBy
import androidx.compose.ui.test.performTouchInput
import androidx.compose.ui.test.up
import com.example.croft.haptics.HapticEvent
import com.example.croft.swipe.SwipePhase
import com.example.croft.theme.AnimationLevel
import com.example.croft.ui.mail.InboxFixture
import com.example.croft.ui.mail.InboxTags
import com.example.croft.ui.mail.rowCard
import com.example.croft.ui.mail.showInbox
import com.example.croft.ui.mail.waitForRow
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

/**
 * Reduced-motion variants and interrupt tests of the swipe (docs/ux/motion.md, "Swipe reveal", "Swipe commit", "Swipe park or
 * cancel"). The clock is paused after the screen is up: nothing moves unless the test advances it. At Full the card slides off with
 * the fast spatial spring; at Reduced the slide-out is a fade and the collapse a fade-and-close, and the threshold is still signalled by
 * colour, icon and haptic but not by growth; at Off everything is at once. The phone is mdpi, so 1 dp is 1 px; a row is 336 dp wide.
 */
@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [35], qualifiers = "w360dp-h800dp-mdpi")
class SwipeMotionTest {
    @get:Rule
    val rule = createAndroidComposeRule<ComponentActivity>()

    private fun frames(n: Int) = repeat(n) { rule.mainClock.advanceTimeByFrame() }

    /** The screen up at [level] with the clock running, then paused. */
    private fun open(level: AnimationLevel): InboxFixture {
        val f = InboxFixture(80)
        rule.showInbox(f, level)
        rule.waitForRow(f.topId)
        rule.waitForIdle()
        rule.mainClock.autoAdvance = false
        return f
    }

    /** Drag the row to the left past the arm point (140 dp from where the slop was passed), holding the finger down. */
    private fun dragArmed(id: Long) {
        rule.rowCard(id).performTouchInput {
            down(center)
            moveBy(Offset(-10f, 0f), delayMillis = 100)
            repeat(5) { moveBy(Offset(-26f, 0f), delayMillis = 100) }
        }
        frames(3)
    }

    private fun rowIsGone(id: Long) = rule.onAllNodes(hasTestTag(InboxTags.row(id))).fetchSemanticsNodes().isEmpty()

    // ---- The threshold is signalled without growth at Reduced and Off ----

    private fun iconScaleWhenArmed(level: AnimationLevel): Float {
        val f = open(level)
        val id = f.topId
        dragArmed(id)
        rule.mainClock.advanceTimeBy(1_000)
        assertEquals("armed at $level", SwipePhase.ARMED.ordinal.toFloat(), rule.swipeProbe(id, "swipe-phase"), 0f)
        assertEquals("the arm haptic still plays at $level", HapticEvent.ThresholdArmed, f.haptics.events.last())
        val scale = rule.swipeProbe(id, "swipe-icon-scale")
        rule.rowCard(id).performTouchInput { up() }
        rule.mainClock.advanceTimeBy(3_000)
        return scale
    }

    @Test
    fun theIconGrowsToOnePointTwoAtFull() = assertEquals(1.2f, iconScaleWhenArmed(AnimationLevel.FULL), 0.02f)

    @Test
    fun theIconDoesNotGrowAtReduced_colourAndHapticSignalTheThreshold() = assertEquals(1f, iconScaleWhenArmed(AnimationLevel.REDUCED), 0.001f)

    @Test
    fun theIconDoesNotGrowAtOff() = assertEquals(1f, iconScaleWhenArmed(AnimationLevel.OFF), 0.001f)

    // ---- The commit sequence ----

    @Test
    fun commit_atFull_theCardSlidesOffWhileTheSlotCollapses() {
        val f = open(AnimationLevel.FULL)
        val id = f.topId
        dragArmed(id)
        val held = rule.swipeProbe(id, "swipe-offset")
        assertTrue("dragged to the left: $held", held < -120f)
        rule.rowCard(id).performTouchInput { up() }
        frames(4)
        val early = rule.swipeProbe(id, "swipe-offset")
        assertTrue("sliding further out after release ($held to $early)", early < held - 10f)
        assertEquals("no fade at Full", 1f, rule.swipeProbe(id, "swipe-fade"), 0.001f)
        rule.mainClock.advanceTimeBy(3_000)
        assertTrue("the row is gone once the sequence is over", rowIsGone(id))
        assertEquals(listOf("trash $id"), f.actions.calls)
    }

    @Test
    fun commit_atReduced_theSlideOutIsAFadeAndTheCollapseAFadeAndClose() {
        val f = open(AnimationLevel.REDUCED)
        val id = f.topId
        dragArmed(id)
        val held = rule.swipeProbe(id, "swipe-offset")
        rule.rowCard(id).performTouchInput { up() }
        frames(4)
        assertEquals("the card does not travel at Reduced", held, rule.swipeProbe(id, "swipe-offset"), 1f)
        assertTrue("the row fades instead", rule.swipeProbe(id, "swipe-fade") < 1f)
        rule.mainClock.advanceTimeBy(3_000)
        assertTrue(rowIsGone(id))
        assertEquals(listOf("trash $id"), f.actions.calls)
    }

    @Test
    fun commit_atOff_isAtOnce() {
        val f = open(AnimationLevel.OFF)
        val id = f.topId
        dragArmed(id)
        rule.rowCard(id).performTouchInput { up() }
        frames(4)
        rule.mainClock.advanceTimeBy(500)
        assertTrue("gone almost at once at Off", rowIsGone(id) || rule.swipeProbe(id, "swipe-collapse") == 0f)
        assertEquals(listOf("trash $id"), f.actions.calls)
    }

    @Test
    fun cancel_atReduced_theCardStillSpringsBackAndRunsNothing() {
        for (level in listOf(AnimationLevel.REDUCED)) {
            val f = open(level)
            val id = f.topId
            rule.rowCard(id).performTouchInput {
                down(center)
                moveBy(Offset(-10f, 0f), delayMillis = 100)
                moveBy(Offset(-50f, 0f), delayMillis = 100)
                up()
            }
            frames(2)
            val early = rule.swipeProbe(id, "swipe-offset")
            assertTrue("on its way back at $level ($early)", early in -50f..0f)
            rule.mainClock.advanceTimeBy(3_000)
            assertEquals(0f, rule.swipeProbe(id, "swipe-offset"), 0.5f)
            assertTrue(f.actions.calls.isEmpty())
        }
    }

    // ---- Interrupt: a touch hands the card to the finger, from where it is ----

    @Test
    fun interrupt_aTouchDuringTheSpringBackTakesTheCardFromWhereItIs() {
        val f = open(AnimationLevel.FULL)
        val id = f.topId
        rule.rowCard(id).performTouchInput {
            down(center)
            moveBy(Offset(10f, 0f), delayMillis = 100)
            moveBy(Offset(60f, 0f), delayMillis = 100)
            up()
        }
        frames(2)
        val atTouch = rule.cardOffset(id)
        assertTrue("on its way back, neither at 60 nor at 0 yet: $atTouch", atTouch in 5f..55f)
        // The spring keeps running while the finger comes down and passes the slop (the injected events take test time), so the card
        // is taken from where it is at that moment, which is a little nearer to 0 than where it was seen above.
        rule.rowCard(id).performTouchInput { down(center); moveBy(Offset(10f, 0f), delayMillis = 16) }
        val held = rule.swipeProbe(id, "swipe-offset")
        assertTrue("the finger took the card where the spring had it, not at 0 or at 60: $held (seen at $atTouch)", held in 1f..atTouch + 1f)
        rule.rowCard(id).performTouchInput { moveBy(Offset(20f, 0f), delayMillis = 16) }
        val now = rule.swipeProbe(id, "swipe-offset")
        assertEquals("it follows the finger from where it was taken", held + 20f, now, 1f)
        rule.rowCard(id).performTouchInput { up() }
        rule.mainClock.advanceTimeBy(3_000)
    }

    @Test
    fun interrupt_theSlideOutOfACommittedCardIsNotGivenBackToAFinger_butTheRestOfTheListStaysUsable() {
        val f = open(AnimationLevel.FULL)
        val ids = f.inboxIds()
        dragArmed(ids[0])
        rule.rowCard(ids[0]).performTouchInput { up() }
        frames(2)
        // The row below can be swiped while the first one is still leaving.
        rule.rowCard(ids[1]).performTouchInput { down(center); moveBy(Offset(10f, 0f), delayMillis = 16); moveBy(Offset(30f, 0f), delayMillis = 16) }
        frames(1)
        assertTrue("the other row follows the finger at once: ${rule.cardOffset(ids[1])}", rule.cardOffset(ids[1]) > 5f)
        rule.rowCard(ids[1]).performTouchInput { up() }
        rule.mainClock.advanceTimeBy(3_000)
    }
}
