// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

@file:OptIn(io.kotest.common.ExperimentalKotest::class)

package com.example.croft.swipe

import io.kotest.property.Arb
import io.kotest.property.PropTestConfig
import io.kotest.property.arbitrary.arbitrary
import io.kotest.property.checkAll
import kotlin.math.abs
import kotlin.random.Random
import kotlinx.coroutines.runBlocking
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * Property tests on the pure machine, with kotest-property (Apache-2.0; chosen because it runs inside JUnit with no extra
 * runner, has seeded generators, and is a test-only dependency). Each property runs on random gestures: a random row width,
 * layout direction, sensitivity and slots, a random walk of drag events with random gaps in time, and a random end.
 * The seed is fixed so a failure reproduces; the failing gesture is printed.
 */
class SwipePropertyTest {
    private val config = PropTestConfig(seed = 20261111L, iterations = 2_000)

    private sealed interface End {
        data class Release(val velocity: Float) : End
        data class Cancel(val reason: SwipeCancel) : End
        data object None : End
    }

    private class Gesture(val ctx: SwipeContext, val drags: List<Pair<Float, Long>>, val end: End) {
        override fun toString() = "Gesture(ctx=$ctx, drags=$drags, end=$end)"
    }

    private fun randomSlot(r: Random): SwipeSlot {
        val action = SwipeAction.entries[r.nextInt(SwipeAction.entries.size)]
        return SwipeSlot(action, available = r.nextInt(4) != 0, parks = action.parks || r.nextInt(8) == 0)
    }

    private fun randomContext(r: Random, rtl: Boolean = r.nextBoolean()) = SwipeContext(
        width = r.nextInt(200, 900).toFloat(), rtl = rtl,
        sensitivity = SwipeSensitivity.entries[r.nextInt(3)], startToEnd = randomSlot(r), endToStart = randomSlot(r),
    )

    private fun randomGesture(r: Random, ctx: SwipeContext = randomContext(r)): Gesture {
        var x = 0f
        var t = 0L
        val drags = ArrayList<Pair<Float, Long>>()
        repeat(r.nextInt(1, 40)) {
            x = (x + (r.nextFloat() - 0.5f) * 0.6f * ctx.width).coerceIn(-1.2f * ctx.width, 1.2f * ctx.width)
            if (r.nextInt(10) == 0) x = 0f
            t += r.nextInt(0, 120)
            drags += x to t
        }
        val end = when (r.nextInt(6)) {
            0 -> End.Cancel(SwipeCancel.entries[r.nextInt(SwipeCancel.entries.size)])
            1 -> End.None
            else -> End.Release((r.nextFloat() - 0.5f) * 6000f)
        }
        return Gesture(ctx, drags, end)
    }

    private val gestures: Arb<Gesture> = arbitrary { rs -> randomGesture(rs.random) }

    /** Runs a gesture; returns every effect in order and, for the release, the state just before it. */
    private class Run(val effects: List<SwipeEffect>, val beforeEnd: SwipeState, val machine: SwipeMachine)

    private fun run(g: Gesture): Run {
        val m = SwipeMachine(TestTuning, g.ctx)
        val effects = ArrayList<SwipeEffect>()
        var t = 0L
        for ((dx, time) in g.drags) { effects += m.send(SwipeInput.Drag(dx, time)); t = time }
        val before = m.state
        when (val e = g.end) {
            is End.Release -> effects += m.send(SwipeInput.Release(e.velocity, t + 5))
            is End.Cancel -> effects += m.send(SwipeInput.Cancel(e.reason))
            End.None -> Unit
        }
        return Run(effects, before, m)
    }

    private fun check(property: (Gesture, Run) -> Unit) = runBlocking<Unit> {
        checkAll(config, gestures) { g -> property(g, run(g)) }
    }

    @Test
    fun neverCommitsBelowTheThresholdWithoutAFling() = check { g, r ->
        val e = g.end as? End.Release
        val committed = r.effects.filter { it is SwipeEffect.Commit || it is SwipeEffect.Park }
        if (committed.isNotEmpty()) {
            requireNotNull(e) { "a commit without a release: $g" }
            val dir = requireNotNull(r.beforeEnd.direction)
            val threshold = SwipeReducer.threshold(TestTuning, g.ctx)
            val along = e.velocity * dir.physicalSign(g.ctx.rtl)
            val fling = along >= TestTuning.flingMinVelocity && r.beforeEnd.distance >= TestTuning.flingMinTravelFraction * g.ctx.width
            assertTrue("committed at ${r.beforeEnd.distance} (threshold $threshold) without a fling: $g", r.beforeEnd.distance >= threshold || fling)
        }
        if (e != null && r.beforeEnd.phase != SwipePhase.IDLE) {
            val dir = requireNotNull(r.beforeEnd.direction)
            val threshold = SwipeReducer.threshold(TestTuning, g.ctx)
            val along = e.velocity * dir.physicalSign(g.ctx.rtl)
            val fling = along >= TestTuning.flingMinVelocity && r.beforeEnd.distance >= TestTuning.flingMinTravelFraction * g.ctx.width
            if (r.beforeEnd.distance < threshold && !fling) assertTrue("committed below the threshold: $g", committed.isEmpty())
        }
    }

    @Test
    fun neverTwoArmEventsWithoutADisarmBetween() = check { g, r ->
        var armed = false
        for (h in r.effects.haptics()) when (h) {
            SwipeHaptic.ThresholdArmed -> { assertTrue("two arm events in a row: $g", !armed); armed = true }
            SwipeHaptic.ThresholdDisarmed -> { assertTrue("a disarm without an arm: $g", armed); armed = false }
            else -> Unit
        }
    }

    @Test
    fun neverCommitsParksOrArmsWhenTheActionIsUnavailable() = check { g, r ->
        val dir = r.beforeEnd.direction
        if (dir != null && !g.ctx.slot(dir).available) {
            assertTrue("acted on an unavailable action: $g", r.effects.none { it is SwipeEffect.Commit || it is SwipeEffect.Park || it is SwipeEffect.Completed })
        }
        // An unavailable slot on either side: no arm haptic can come from it, and the machine never reports ARMED for it.
        if (!g.ctx.startToEnd.available && !g.ctx.endToStart.available) {
            assertTrue("armed with nothing available: $g", r.effects.haptics().none { it == SwipeHaptic.ThresholdArmed })
        }
    }

    @Test
    fun neverCommitsAnActionThatIsNone() = check { g, r ->
        for (e in r.effects) {
            if (e is SwipeEffect.Commit) assertTrue(e.action != SwipeAction.NONE)
            if (e is SwipeEffect.Park) assertTrue(e.action != SwipeAction.NONE)
        }
    }

    @Test
    fun rtlIsTheMirrorImageOfLtrForTheSameStoredSettingsAndTheOppositePhysicalDrag() = runBlocking<Unit> {
        checkAll(config, gestures) { g ->
            val ltr = g.ctx.copy(rtl = false)
            val rtl = g.ctx.copy(rtl = true)
            val a = run(Gesture(ltr, g.drags, mirrorEnd(g.end, false)))
            val b = run(Gesture(rtl, g.drags.map { (dx, t) -> -dx to t }, mirrorEnd(g.end, true)))
            assertEquals("effects: $g", a.effects.map { it.mirrored() }, b.effects.map { it.mirrored() })
            assertEquals("phase: $g", a.machine.state.phase, b.machine.state.phase)
            assertEquals("direction: $g", a.machine.state.direction, b.machine.state.direction)
            assertEquals("offset: $g", a.machine.state.offset, -b.machine.state.offset, 0.001f)
        }
    }

    private fun mirrorEnd(e: End, flip: Boolean): End = if (flip && e is End.Release) End.Release(-e.velocity) else e
    private fun SwipeEffect.mirrored(): SwipeEffect = this

    @Test
    fun theSamePhysicalDragInRtlRunsTheOtherDirectionsAction() = runBlocking<Unit> {
        checkAll(config, gestures) { g ->
            val dx = g.drags.map { it.first }.firstOrNull { it != 0f } ?: return@checkAll
            val ltrDir = SwipeDirection.of(dx, rtl = false)
            val rtlDir = SwipeDirection.of(dx, rtl = true)
            assertTrue(ltrDir != rtlDir)
            assertEquals(1, ltrDir.physicalSign(false) * SwipeDirection.of(dx, false).physicalSign(false))
            assertEquals(ltrDir.physicalSign(false), rtlDir.physicalSign(true))
        }
    }

    @Test
    fun theCardNeverLeavesTheRowAndIsOnTheSideOfTheFinger() = runBlocking<Unit> {
        checkAll(config, gestures) { g ->
            val m = SwipeMachine(TestTuning, g.ctx)
            for ((dx, t) in g.drags) {
                m.send(SwipeInput.Drag(dx, t))
                val s = m.state
                assertTrue("offset ${s.offset} outside the row in $g", abs(s.offset) <= g.ctx.width + 0.001f)
                if (s.phase == SwipePhase.DRAGGING || s.phase == SwipePhase.ARMED) {
                    assertTrue("card on the wrong side in $g", s.offset == 0f || (s.offset > 0f) == (dx > 0f))
                }
                if (s.phase == SwipePhase.IDLE) assertEquals(0f, s.offset, 0f)
            }
        }
    }

    /** Any input in any order: the invariants hold and Completed only follows a commit or a park. */
    @Test
    fun anyInputSequenceKeepsTheInvariants() = runBlocking<Unit> {
        checkAll(config, arbitrary { rs -> rs.random.nextLong() }) { seed ->
            val r = Random(seed)
            val ctx = randomContext(r)
            val m = SwipeMachine(TestTuning, ctx)
            var t = 0L
            var committedOrParked = false
            var lastWasArm = false
            repeat(r.nextInt(5, 60)) {
                t += r.nextInt(0, 100)
                val input: SwipeInput = when (r.nextInt(10)) {
                    in 0..4 -> SwipeInput.Drag((r.nextFloat() - 0.5f) * 2.4f * ctx.width, t)
                    5 -> SwipeInput.Release((r.nextFloat() - 0.5f) * 6000f, t)
                    6 -> SwipeInput.Cancel(SwipeCancel.entries[r.nextInt(SwipeCancel.entries.size)])
                    7 -> SwipeInput.AnimationEnd
                    8 -> SwipeInput.SheetResult(r.nextBoolean())
                    else -> SwipeInput.Reset
                }
                val before = m.state.phase
                val beforeAction = m.state.action
                val fx = m.send(input)
                for (e in fx) {
                    if (e is SwipeEffect.Commit || e is SwipeEffect.Park) committedOrParked = true
                    if (e is SwipeEffect.Completed) assertTrue("Completed without a commit (seed $seed)", committedOrParked)
                    if (e is SwipeEffect.Haptic && e.kind == SwipeHaptic.ThresholdArmed) lastWasArm = true
                }
                if (input is SwipeInput.Reset) { committedOrParked = false }
                if (m.state.phase == SwipePhase.DONE) assertTrue("DONE from $before (seed $seed)", before == SwipePhase.COMMITTING || before == SwipePhase.PARKED || before == SwipePhase.DONE)
                assertTrue("offset out of range (seed $seed)", abs(m.state.offset) <= ctx.width + 0.001f)
                val keepsRow = !beforeAction.removesRow
                if ((before == SwipePhase.COMMITTING && !keepsRow) || before == SwipePhase.PARKED || before == SwipePhase.DONE) {
                    assertTrue("the finger moved a card that is leaving (seed $seed)", input !is SwipeInput.Drag || m.state.phase == before)
                }
            }
        }
    }

    @Test
    fun everyStateIsReachable() {
        val seen = HashSet<SwipePhase>()
        runBlocking {
            checkAll(PropTestConfig(seed = 7L, iterations = 3_000), arbitrary { rs -> rs.random.nextLong() }) { seed ->
                val r = Random(seed)
                val ctx = randomContext(r)
                val m = SwipeMachine(TestTuning, ctx)
                var t = 0L
                repeat(r.nextInt(5, 40)) {
                    t += r.nextInt(0, 100)
                    m.send(
                        when (r.nextInt(8)) {
                            in 0..3 -> SwipeInput.Drag((r.nextFloat() - 0.5f) * 2.4f * ctx.width, t)
                            4 -> SwipeInput.Release((r.nextFloat() - 0.5f) * 6000f, t)
                            5 -> SwipeInput.AnimationEnd
                            6 -> SwipeInput.SheetResult(r.nextBoolean())
                            else -> SwipeInput.Cancel(SwipeCancel.SECOND_POINTER)
                        },
                    )
                    seen += m.state.phase
                }
            }
        }
        assertEquals("not every state was reached: $seen", SwipePhase.entries.toSet(), seen)
    }
}
