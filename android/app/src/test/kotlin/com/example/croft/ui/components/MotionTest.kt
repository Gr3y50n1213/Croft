// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.ui.components

import androidx.compose.animation.core.SnapSpec
import androidx.compose.animation.core.SpringSpec
import androidx.compose.foundation.layout.Column
import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.asAndroidBitmap
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.ProgressBarRangeInfo
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.test.captureToImage
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performTouchInput
import androidx.compose.ui.test.down
import androidx.compose.ui.test.up
import androidx.compose.ui.test.center
import androidx.compose.ui.test.hasProgressBarRangeInfo
import com.example.croft.CroftTheme
import com.example.croft.haptics.HapticEvent
import com.example.croft.haptics.RecordingHaptics
import com.example.croft.theme.AnimationLevel
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotEquals
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

/**
 * Every animated control has the substitute that docs/ux/motion.md names for Reduced and Off, and can be interrupted by touch.
 * The pausing clock is the compose test clock: nothing moves unless the test advances it. Controls expose what they show
 * through `MotionProbes` (a scale, a rotation, a progress), so a test can see "moved" and "did not move".
 */
@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [35])
class MotionTest {
    @get:Rule
    val rule = createComposeRule()

    private val full = AnimationLevel.FULL
    private val reduced = AnimationLevel.REDUCED
    private val off = AnimationLevel.OFF

    private fun frames(n: Int) = repeat(n) { rule.mainClock.advanceTimeByFrame() }
    private fun width(tag: String) = rule.onNodeWithTag(tag).fetchSemanticsNode().size.width

    // ---- The motion scheme behind Material's own components ----

    private fun schemeAt(level: AnimationLevel): Triple<Any, Any, Any> {
        var spatial: Any? = null; var effects: Any? = null; var slow: Any? = null
        rule.show(level) {
            val m = MaterialTheme.motionScheme
            spatial = m.fastSpatialSpec<Float>(); effects = m.defaultEffectsSpec<Float>(); slow = m.slowSpatialSpec<Float>()
        }
        rule.waitForIdle()
        return Triple(spatial!!, effects!!, slow!!)
    }

    @Test
    fun atFull_materialComponentsGetTheExpressiveSprings() {
        val (spatial, effects, _) = schemeAt(full)
        spatial as SpringSpec<*>; effects as SpringSpec<*>
        assertEquals(800f, spatial.stiffness, 0f); assertEquals(0.6f, spatial.dampingRatio, 0f)
        assertEquals(1600f, effects.stiffness, 0f)
    }

    @Test
    fun atReduced_materialComponentsGetTheStandardSpringsWithoutOvershoot() {
        val (spatial, _, slow) = schemeAt(reduced)
        spatial as SpringSpec<*>; slow as SpringSpec<*>
        assertEquals(1400f, spatial.stiffness, 0f); assertEquals(0.9f, spatial.dampingRatio, 0f)
        assertEquals(300f, slow.stiffness, 0f)
    }

    @Test
    fun atOff_materialComponentsGetNoSpringsAtAll() {
        val (spatial, effects, slow) = schemeAt(off)
        assertTrue(spatial is SnapSpec<*>); assertTrue(effects is SnapSpec<*>); assertTrue(slow is SnapSpec<*>)
    }

    // ---- Switch (a Material component): the thumb is where it ends up on the first frame at Off, and travelling at Full ----

    private fun switchPicturesAfterToggle(level: AnimationLevel): Pair<android.graphics.Bitmap, android.graphics.Bitmap> {
        var on by mutableStateOf(false)
        rule.mainClock.autoAdvance = false
        rule.show(level) { androidx.compose.foundation.layout.Box(Modifier.testTag("sw")) { CroftSwitchRow("Sync", on, { on = it }) } }
        frames(5)
        rule.ui { on = true }
        frames(2)
        val early = rule.onNodeWithTag("sw").captureToImage().asAndroidBitmap().copy(android.graphics.Bitmap.Config.ARGB_8888, false)
        rule.mainClock.advanceTimeBy(2_000)
        val settled = rule.onNodeWithTag("sw").captureToImage().asAndroidBitmap().copy(android.graphics.Bitmap.Config.ARGB_8888, false)
        return early to settled
    }

    @Test
    fun atOff_theSwitchThumbIsAtItsEndPositionOnTheFirstFrames() {
        val (early, settled) = switchPicturesAfterToggle(off)
        assertTrue("no travel at Off", early.sameAs(settled))
    }

    @Test
    fun atFull_theSwitchThumbIsStillTravellingOnTheFirstFrames() {
        val (early, settled) = switchPicturesAfterToggle(full)
        assertFalse("travel at Full", early.sameAs(settled))
    }

    private fun assertFalse(message: String, v: Boolean) = assertTrue(message, !v)

    // ---- Spinner ----

    private fun spinnerInfo(level: AnimationLevel): ProgressBarRangeInfo {
        rule.show(level) { CroftSpinner(Modifier.testTag("spin")) }
        return rule.onNodeWithTag("spin").fetchSemanticsNode().config[SemanticsProperties.ProgressBarRangeInfo]
    }

    @Test
    fun theSpinnerTurnsAtFullAndReduced_andStandsStillAtOff() {
        assertEquals(ProgressBarRangeInfo.Indeterminate, spinnerInfo(full))
    }

    @Test
    fun theSpinnerIsAStaticArcAtOff() {
        val info = spinnerInfo(off)
        assertEquals(0.75f, info.current, 0f)
        assertNotEquals(ProgressBarRangeInfo.Indeterminate, info)
    }

    @Test
    fun theSpinnerKeepsTurningAtReduced() {
        assertEquals(ProgressBarRangeInfo.Indeterminate, spinnerInfo(reduced))
    }

    // ---- Extended FAB: label slides and narrows (spatial fast); swap at once at Reduced and Off ----

    private fun fabWidths(level: AnimationLevel): Triple<Int, Int, Int> {
        var state by mutableStateOf(FabState.EXPANDED)
        rule.mainClock.autoAdvance = false
        rule.show(level) { CroftExtendedFab("Compose", {}, state, Modifier.testTag("fab")) }
        frames(10)
        val expanded = width("fab")
        rule.ui { state = FabState.COLLAPSED }
        frames(2)
        val early = width("fab")
        rule.mainClock.advanceTimeBy(2_000)
        return Triple(expanded, early, width("fab"))
    }

    @Test
    fun theFabNarrowsOverSeveralFramesAtFull() {
        val (expanded, early, collapsed) = fabWidths(full)
        assertTrue("collapsed is narrower", collapsed < expanded)
        assertTrue("still narrowing two frames in ($expanded, $early, $collapsed)", early > collapsed)
    }

    @Test
    fun theFabSwapsAtOnceAtReduced() {
        val (expanded, early, collapsed) = fabWidths(reduced)
        assertTrue(collapsed < expanded)
        assertEquals(collapsed, early)
    }

    @Test
    fun theFabSwapsAtOnceAtOff() {
        val (expanded, early, collapsed) = fabWidths(off)
        assertTrue(collapsed < expanded)
        assertEquals(collapsed, early)
    }

    @Test
    fun aTapOnTheFabDuringItsAnimationIsHandledAtOnce() {
        var state by mutableStateOf(FabState.EXPANDED)
        var taps = 0
        rule.mainClock.autoAdvance = false
        rule.show(full) { CroftExtendedFab("Compose", { taps++ }, state, Modifier.testTag("fab")) }
        frames(10)
        rule.ui { state = FabState.COLLAPSED }
        frames(2)
        rule.onNodeWithTag("fab").performClick()
        assertEquals("input never waits for an animation", 1, taps)
    }

    @Test
    fun hiddenRemovesTheFabAtOnceAtOff() {
        var state by mutableStateOf(FabState.EXPANDED)
        rule.mainClock.autoAdvance = false
        rule.show(off) { CroftExtendedFab("Compose", {}, state, Modifier.testTag("fab")) }
        frames(5)
        rule.onNodeWithTag("fab").assertExists()
        rule.ui { state = FabState.HIDDEN }
        frames(3)
        rule.onNodeWithTag("fab").assertDoesNotExist()
    }

    @Test
    fun hiddenFadesAtFullBeforeItIsGone() {
        var state by mutableStateOf(FabState.EXPANDED)
        rule.mainClock.autoAdvance = false
        rule.show(full) { CroftExtendedFab("Compose", {}, state, Modifier.testTag("fab")) }
        frames(5)
        rule.ui { state = FabState.HIDDEN }
        frames(2)
        rule.onNodeWithTag("fab").assertExists()
        rule.mainClock.advanceTimeBy(2_000)
        rule.onNodeWithTag("fab").assertDoesNotExist()
    }

    // ---- Filter chip: the check slides in (spatial fast); instant at Reduced and Off ----

    private fun chipWidths(level: AnimationLevel): Triple<Int, Int, Int> {
        var selected by mutableStateOf(false)
        rule.mainClock.autoAdvance = false
        rule.show(level) { androidx.compose.foundation.layout.Row { CroftFilterChip("Unread", selected, { selected = !selected }, Modifier.testTag("chip")) } }
        frames(10)
        val before = width("chip")
        rule.ui { selected = true }
        frames(2)
        val early = width("chip")
        rule.mainClock.advanceTimeBy(2_000)
        return Triple(before, early, width("chip"))
    }

    @Test
    fun theChipsCheckSlidesInAtFull() {
        val (before, early, after) = chipWidths(full)
        assertTrue("wider once selected ($before, $after)", after > before)
        assertTrue("still sliding two frames in ($early, $after)", early < after)
    }

    @Test
    fun theChipsCheckAppearsAtOnceAtReducedAndOff() {
        for (level in listOf(reduced)) {
            val (before, early, after) = chipWidths(level)
            assertTrue(after > before)
            assertEquals("$level", after, early)
        }
    }

    @Test
    fun theChipsCheckAppearsAtOnceAtOff() {
        val (before, early, after) = chipWidths(off)
        assertTrue(after > before)
        assertEquals(after, early)
    }

    // ---- Avatar flip: a flip at Full, a cross-fade at Reduced, at once at Off ----

    private var selected by mutableStateOf(false)

    private fun showSelectableRow(level: AnimationLevel) {
        rule.mainClock.autoAdvance = false
        rule.show(level) {
            MessageRow("Sam Rivera", "Lunch", "Are you free?", "9:12", MessageRowState(selected = selected), {}, Modifier.testTag("row"))
        }
        frames(5)
    }

    @Test
    fun theAvatarFlipsAtFull() {
        showSelectableRow(full)
        rule.ui { selected = true }
        frames(3)
        val rotation = rule.probe("avatar-rotation")
        assertTrue("mid-flip: $rotation", rotation > 0f && rotation < 180f * 1.2f)
        assertTrue(rule.probe("avatar-flip") in 0.01f..1.2f)
        rule.mainClock.advanceTimeBy(2_000)
        assertEquals(180f, rule.probe("avatar-rotation"), 0.5f)
    }

    @Test
    fun theAvatarCrossFadesAtReducedWithoutTurning() {
        showSelectableRow(reduced)
        rule.ui { selected = true }
        frames(4)
        assertTrue("fading", rule.probe("avatar-flip") in 0.001f..0.999f)
        assertEquals("no rotation at Reduced", 0f, rule.probe("avatar-rotation"), 0f)
        rule.mainClock.advanceTimeBy(2_000)
        assertEquals(1f, rule.probe("avatar-flip"), 0.01f)
        assertEquals(0f, rule.probe("avatar-rotation"), 0f)
    }

    @Test
    fun theAvatarChangesAtOnceAtOff() {
        showSelectableRow(off)
        rule.ui { selected = true }
        frames(2)
        assertEquals(1f, rule.probe("avatar-flip"), 0f)
        assertEquals(0f, rule.probe("avatar-rotation"), 0f)
    }

    @Test
    fun aTouchDuringTheFlipTakesItOver_theAvatarTurnsBackFromWhereItIs() {
        showSelectableRow(full)
        rule.ui { selected = true }
        frames(3)
        val midway = rule.probe("avatar-flip")
        assertTrue(midway > 0f)
        rule.ui { selected = false } // the person taps again before it finished
        var highest = midway
        repeat(60) { rule.mainClock.advanceTimeByFrame(); highest = maxOf(highest, rule.probe("avatar-flip")) }
        assertTrue("it went back from where it was; it never ran on to the end ($highest)", highest < 0.99f)
        assertEquals(0f, rule.probe("avatar-flip"), 0.01f)
    }

    // ---- Long-press lift: 1 dp and 102 percent at Full; the highlight only at Reduced and Off ----

    private fun liftWhileHeld(level: AnimationLevel): Pair<Float, List<HapticEvent>> {
        val haptics = RecordingHaptics()
        rule.mainClock.autoAdvance = false
        rule.show(level, haptics) { MessageRow("Sam", "Lunch", "Are you free?", "9:12", MessageRowState(), {}, Modifier.testTag("row"), onLongClick = {}) }
        frames(3)
        rule.onNode(isInteractive).performTouchInput { down(center) }
        rule.mainClock.advanceTimeBy(1_500)
        val lift = rule.probe("lift")
        rule.onNode(isInteractive).performTouchInput { up() }
        return lift to haptics.events
    }

    @Test
    fun theLongPressLiftsAtFull() {
        val (lift, haptics) = liftWhileHeld(full)
        assertEquals(1f, lift, 0.02f)
        assertEquals(listOf(HapticEvent.SelectionStart), haptics)
    }

    @Test
    fun theLongPressOnlyHighlightsAtReduced_butTheResultAndTheHapticStillCome() {
        val (lift, haptics) = liftWhileHeld(reduced)
        assertEquals(0f, lift, 0f)
        assertEquals(listOf(HapticEvent.SelectionStart), haptics)
    }

    @Test
    fun theLongPressDoesNotMoveAnythingAtOff() {
        val (lift, haptics) = liftWhileHeld(off)
        assertEquals(0f, lift, 0f)
        assertEquals(listOf(HapticEvent.SelectionStart), haptics)
    }

    // ---- New row tint and the failed edge ----

    @Test
    fun theNewRowTintFadesOver1200msAtFull() {
        rule.mainClock.autoAdvance = false
        rule.show(full) { MessageRow("Sam", "Lunch", "Are you free?", "9:12", MessageRowState(isNew = true), {}, Modifier.testTag("row")) }
        frames(2)
        assertTrue("starts strong", rule.probe("new-tint") > 0.9f)
        rule.mainClock.advanceTimeBy(600)
        assertEquals(0.5f, rule.probe("new-tint"), 0.1f)
        rule.mainClock.advanceTimeBy(700)
        assertEquals(0f, rule.probe("new-tint"), 0.01f)
    }

    @Test
    fun theNewRowTintFadesAtReducedToo_butIsNotShownAtOff() {
        rule.mainClock.autoAdvance = false
        rule.show(reduced) { MessageRow("Sam", "Lunch", "Are you free?", "9:12", MessageRowState(isNew = true), {}, Modifier.testTag("row")) }
        frames(2)
        assertTrue(rule.probe("new-tint") > 0.9f)
    }

    @Test
    fun theNewRowTintIsNotShownAtOff() {
        rule.mainClock.autoAdvance = false
        rule.show(off) { MessageRow("Sam", "Lunch", "Are you free?", "9:12", MessageRowState(isNew = true), {}, Modifier.testTag("row")) }
        frames(2)
        assertEquals(0f, rule.probe("new-tint"), 0f)
    }

    @Test
    fun theFailedEdgeFadesAfterTwoSecondsAtFull() {
        rule.mainClock.autoAdvance = false
        rule.show(full) { MessageRow("Sam", "Lunch", "Are you free?", "9:12", MessageRowState(failed = true), {}, Modifier.testTag("row")) }
        frames(2)
        assertEquals(1f, rule.probe("failed-edge"), 0f)
        rule.mainClock.advanceTimeBy(1_900)
        assertEquals("still solid before 2 s", 1f, rule.probe("failed-edge"), 0f)
        rule.mainClock.advanceTimeBy(100)
        val fading = rule.probe("failed-edge")
        assertTrue("fading: $fading", fading < 1f && fading > 0f)
        rule.mainClock.advanceTimeBy(1_000)
        assertEquals(0f, rule.probe("failed-edge"), 0.01f)
    }

    @Test
    fun theFailedEdgeGoesWithoutFadingAtReduced() {
        rule.mainClock.autoAdvance = false
        rule.show(reduced) { MessageRow("Sam", "Lunch", "Are you free?", "9:12", MessageRowState(failed = true), {}, Modifier.testTag("row")) }
        frames(2)
        assertEquals(1f, rule.probe("failed-edge"), 0f)
        rule.mainClock.advanceTimeBy(2_100)
        frames(1)
        assertEquals("gone in one step", 0f, rule.probe("failed-edge"), 0f)
    }

    // ---- Dialog: scale from 90 percent with a fade at Full; the fade only at Reduced; at once at Off ----

    private fun showDialog(level: AnimationLevel) {
        rule.mainClock.autoAdvance = false
        rule.show(level) { DestructiveDialog("Delete forever?", "This cannot be undone.", "Delete forever", {}, {}) }
        frames(2)
    }

    @Test
    fun theDialogScalesAndFadesInAtFull() {
        showDialog(full)
        val scale = rule.probe("enter-scale"); val enter = rule.probe("enter")
        assertTrue("fading in: $enter", enter < 1f)
        assertTrue("scaling up from 90 percent: $scale", scale in 0.9f..1f)
        rule.mainClock.advanceTimeBy(1_000)
        assertEquals(1f, rule.probe("enter-scale"), 0.001f)
    }

    @Test
    fun theDialogOnlyFadesAtReduced() {
        showDialog(reduced)
        assertTrue(rule.probe("enter") < 1f)
        assertEquals(1f, rule.probe("enter-scale"), 0f)
    }

    @Test
    fun theDialogIsThereAtOnceAtOff() {
        showDialog(off)
        assertEquals(1f, rule.probe("enter"), 0f)
        assertEquals(1f, rule.probe("enter-scale"), 0f)
    }

    // ---- Banner: slides in and pushes the list at Full; just there at Reduced and Off ----

    private fun bannerHeights(level: AnimationLevel): Triple<Int, Int, Int> {
        var active by mutableStateOf(emptyList<BannerKind>())
        rule.mainClock.autoAdvance = false
        rule.show(level) { Column { BannerHost(active, {}, {}, Modifier.testTag("banner"), accountName = "Sam") } }
        frames(3)
        rule.ui { active = listOf(BannerKind.OFFLINE) }
        frames(2)
        val early = rule.onNodeWithTag("banner").fetchSemanticsNode().size.height
        rule.mainClock.advanceTimeBy(2_000)
        val settled = rule.onNodeWithTag("banner").fetchSemanticsNode().size.height
        return Triple(0, early, settled)
    }

    @Test
    fun theBannerExpandsOverFramesAtFull() {
        val (_, early, settled) = bannerHeights(full)
        assertTrue("still growing ($early of $settled)", early < settled)
    }

    @Test
    fun theBannerIsThereAtOnceAtReducedAndOff() {
        for (level in listOf(reduced)) {
            val (_, early, settled) = bannerHeights(level)
            assertEquals("$level", settled, early)
        }
    }

    @Test
    fun theBannerIsThereAtOnceAtOff() {
        val (_, early, settled) = bannerHeights(off)
        assertEquals(settled, early)
    }
}
