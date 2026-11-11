// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.ui.mail

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.LazyListState
import androidx.compose.foundation.lazy.items
import androidx.compose.material3.Text
import androidx.compose.material3.pulltorefresh.PullToRefreshState
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.ProgressBarRangeInfo
import androidx.compose.ui.test.getUnclippedBoundsInRoot
import androidx.compose.ui.test.hasProgressBarRangeInfo
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.performClick
import androidx.compose.ui.unit.dp
import com.example.croft.theme.AnimationLevel
import com.example.croft.ui.components.probe
import com.example.croft.ui.components.show
import com.example.croft.ui.components.ui
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
 * Reduced-motion tests and interrupt tests of the message list (docs/ux/motion.md): row collapse, new rows sliding in, the
 * skeleton shimmer, the "N new" pill and the pull indicator. The clock is paused: nothing moves unless the test advances it.
 */
@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [35], qualifiers = "w360dp-h800dp-mdpi")
class ListMotionTest {
    @get:Rule
    val rule = createComposeRule()

    private fun frames(n: Int) = repeat(n) { rule.mainClock.advanceTimeByFrame() }
    private val full = AnimationLevel.FULL
    private val reduced = AnimationLevel.REDUCED
    private val off = AnimationLevel.OFF

    /** The list the screen uses, with 60 dp rows, built on the same `listItemMotion` the Inbox applies. */
    private fun showList(level: AnimationLevel, data: () -> List<Long>, tick: () -> Int = { 0 }) {
        val state = LazyListState()
        rule.show(level) {
            LaunchedEffect(tick()) { if (tick() > 0) state.scrollToItem(0) }
            LazyColumn(Modifier.fillMaxSize(), state = state) {
                items(data(), key = { it }) { id -> Box(Modifier.testTag("i$id").fillMaxWidth().height(60.dp).then(listItemMotion())) { Text("$id") } }
            }
        }
    }

    private fun top(id: Long) = rule.onNodeWithTag("i$id").getUnclippedBoundsInRoot().top.value

    // ---- Row collapse: the rows below slide up and close the gap ----

    private fun topOfFourthAfterRemovingThird(level: AnimationLevel): Triple<Float, Float, Float> {
        var data by mutableStateOf((1L..10L).toList())
        rule.mainClock.autoAdvance = false
        showList(level, { data })
        frames(3)
        val before = top(4)
        rule.ui { data = data - 3L }
        frames(2)
        val early = top(4)
        rule.mainClock.advanceTimeBy(2_000)
        return Triple(before, early, top(4))
    }

    @Test
    fun rowCollapse_atFull_theRowsBelowSlideUpOverSeveralFrames() {
        val (before, early, settled) = topOfFourthAfterRemovingThird(full)
        assertEquals(180f, before, 0.5f)
        assertEquals(120f, settled, 0.5f)
        assertTrue("still on its way two frames in ($early)", early > settled + 1f)
    }

    @Test
    fun rowCollapse_atReduced_theGapClosesAtOnce() {
        val (_, early, settled) = topOfFourthAfterRemovingThird(reduced)
        assertEquals(120f, settled, 0.5f)
        assertEquals("no travel at Reduced", settled, early, 0.5f)
    }

    @Test
    fun rowCollapse_atOff_theGapClosesAtOnce() {
        val (_, early, settled) = topOfFourthAfterRemovingThird(off)
        assertEquals("no travel at Off", settled, early, 0.5f)
    }

    // ---- New rows at the very top slide in and push the rest down ----

    private fun topOfFirstAfterInsertingAtTheTop(level: AnimationLevel, framesAfter: Int): Pair<Float, Float> {
        var data by mutableStateOf((1L..10L).toList())
        var tick by mutableIntStateOf(0)
        rule.mainClock.autoAdvance = false
        showList(level, { data }, { tick })
        frames(3)
        rule.ui { data = listOf(0L) + data; tick++ }
        frames(framesAfter)
        val early = top(1)
        rule.mainClock.advanceTimeBy(2_000)
        return early to top(1)
    }

    @Test
    fun newRowSlideIn_atFull_theOldRowsAreStillMovingDownAfterAFewFrames() {
        val (early, settled) = topOfFirstAfterInsertingAtTheTop(full, 3)
        assertEquals(60f, settled, 0.5f)
        assertTrue("not yet at its place ($early)", early < settled - 1f)
    }

    @Test
    fun newRowSlideIn_atReduced_theRowsAppearWithoutMovement() {
        val (early, settled) = topOfFirstAfterInsertingAtTheTop(reduced, 6)
        assertEquals(60f, settled, 0.5f)
        assertEquals(settled, early, 0.5f)
    }

    @Test
    fun newRowSlideIn_atOff_theRowsAppearWithoutMovement() {
        val (early, settled) = topOfFirstAfterInsertingAtTheTop(off, 6)
        assertEquals(settled, early, 0.5f)
    }

    // ---- Skeleton shimmer ----

    private fun shimmerValues(level: AnimationLevel): Pair<Float, Float> {
        rule.mainClock.autoAdvance = false
        rule.show(level) { SkeletonList() }
        frames(2)
        val a = rule.probe("shimmer")
        rule.mainClock.advanceTimeBy(600)
        return a to rule.probe("shimmer")
    }

    @Test
    fun skeletonShimmer_atFull_loopsOverTime() {
        val (a, b) = shimmerValues(full)
        assertNotEquals("the shimmer moved ($a to $b)", a, b, 0.0001f)
    }

    @Test
    fun skeletonShimmer_isStaticAtReduced() {
        val (a, b) = shimmerValues(reduced)
        assertEquals(0f, a, 0f)
        assertEquals(0f, b, 0f)
    }

    @Test
    fun skeletonShimmer_isStaticAtOff() {
        val (a, b) = shimmerValues(off)
        assertEquals(0f, a, 0f)
        assertEquals(0f, b, 0f)
    }

    // ---- The "N new" pill ----

    private fun showPill(level: AnimationLevel, count: () -> Int, onClick: () -> Unit = {}) {
        rule.mainClock.autoAdvance = false
        rule.show(level) { Box(Modifier.fillMaxSize()) { NewMailPill(count(), onClick, Modifier.align(Alignment.TopCenter)) } }
        frames(3)
    }

    @Test
    fun pill_atFull_risesWithTheSpring_andIsNotThereWithoutNewMail() {
        var count by mutableIntStateOf(0)
        showPill(full, { count })
        rule.onNodeWithTag(InboxTags.NEW_PILL).assertDoesNotExistCompat()
        rule.ui { count = 3 }
        frames(2)
        val p = rule.probe("pill")
        assertTrue("on its way ($p)", p > 0f && p < 1f)
        assertTrue("rising, not appearing in place", rule.probe("pill-rise") > 0f)
        rule.mainClock.advanceTimeBy(2_000)
        assertEquals(1f, rule.probe("pill"), 0.02f)
        assertEquals(0f, rule.probe("pill-rise"), 0.5f)
    }

    @Test
    fun pill_atReduced_fadesInWithoutTravel() {
        var count by mutableIntStateOf(0)
        showPill(reduced, { count })
        rule.ui { count = 2 }
        frames(1)
        assertEquals("no rise at Reduced", 0f, rule.probe("pill-rise"), 0f)
        rule.mainClock.advanceTimeBy(2_000)
        assertEquals(1f, rule.probe("pill"), 0.02f)
        assertEquals(0f, rule.probe("pill-rise"), 0f)
    }

    @Test
    fun pill_atOff_isThereAtOnce() {
        var count by mutableIntStateOf(0)
        showPill(off, { count })
        rule.ui { count = 2 }
        frames(1)
        assertEquals(1f, rule.probe("pill"), 0f)
    }

    @Test
    fun pill_aTapWhileItIsMovingIsHandledAtOnce() {
        var count by mutableIntStateOf(0)
        var taps = 0
        showPill(full, { count }, { taps++ })
        rule.ui { count = 3 }
        frames(2)
        rule.onNodeWithTag(InboxTags.NEW_PILL).performClick()
        assertEquals("input never waits for an animation", 1, taps)
    }

    @Test
    fun pill_hidingMidRiseCarriesOnFromWhereItIs_itDoesNotJump() {
        var count by mutableIntStateOf(0)
        showPill(full, { count })
        rule.ui { count = 3 }
        frames(3)
        val mid = rule.probe("pill")
        assertTrue(mid > 0f && mid < 1f)
        rule.ui { count = 0 }
        frames(1)
        val next = rule.probe("pill")
        assertTrue("from $mid to $next in one frame: no jump to either end", kotlin.math.abs(next - mid) < 0.5f)
        rule.mainClock.advanceTimeBy(3_000)
        rule.onNodeWithTag(InboxTags.NEW_PILL).assertDoesNotExistCompat()
    }

    @Test
    fun pill_aCountThatChangesWhileItIsThereDoesNotRestartIt() {
        var count by mutableIntStateOf(2)
        showPill(full, { count })
        rule.mainClock.advanceTimeBy(2_000)
        rule.ui { count = 3 }
        frames(1)
        assertEquals("already there: it stays put", 1f, rule.probe("pill"), 0.02f)
    }

    // ---- Pull indicator ----

    @Test
    fun pullIndicator_atOffIsAStaticArc_andAtReducedASpinner() {
        val state = PullToRefreshState()
        rule.show(off) { Box(Modifier.fillMaxSize()) { PullIndicator(state, refreshing = true) } }
        rule.waitForIdle()
        rule.onNodeWithTag(InboxTags.PULL_INDICATOR).assertExistsCompat()
        rule.onNode(hasProgressBarRangeInfo(ProgressBarRangeInfo(0.75f, 0f..1f, 0))).assertExistsCompat()
    }
}

private fun androidx.compose.ui.test.SemanticsNodeInteraction.assertDoesNotExistCompat() = assertDoesNotExist()
private fun androidx.compose.ui.test.SemanticsNodeInteraction.assertExistsCompat() = assertExists()
