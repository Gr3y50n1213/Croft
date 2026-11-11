// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.swipe

import kotlin.math.abs

/** The numbers the machine needs; the app fills them from `tokens.json` (`swipe`), tests give their own. */
data class SwipeTuning(
    /** Arm at this fraction of the row width at sensitivity Low, Normal and High. */
    val armLow: Float,
    val armNormal: Float,
    val armHigh: Float,
    /** A release faster than this (dp per second, in the swipe direction) commits from [flingMinTravelFraction]. */
    val flingMinVelocity: Float,
    val flingMinTravelFraction: Float,
    /** Snooze, Move to and Delete forever hold the card at this fraction of the row width. */
    val parkFraction: Float,
    /** Beyond this fraction of the width the card follows the finger at [resistanceFactor] speed. */
    val resistanceStartFraction: Float,
    val resistanceFactor: Float,
    /** A second arm or disarm within this many milliseconds of the last one is held back (jitter). */
    val crossingGuardMs: Long,
) {
    /** The arming fraction of [s]. */
    fun armFraction(s: SwipeSensitivity): Float = when (s) {
        SwipeSensitivity.LOW -> armLow
        SwipeSensitivity.NORMAL -> armNormal
        SwipeSensitivity.HIGH -> armHigh
    }
}

/** What one direction does for one row right now. [available] is false when the row cannot do it (Archive in Sent). */
data class SwipeSlot(val action: SwipeAction, val available: Boolean = true, val parks: Boolean = action.parks)

/**
 * Everything about the row that the machine needs besides the drag: its width (the unit of [width] is the unit of every
 * distance and velocity, dp in the app), the layout direction, the sensitivity and the two slots.
 */
data class SwipeContext(
    val width: Float,
    val rtl: Boolean,
    val sensitivity: SwipeSensitivity,
    val startToEnd: SwipeSlot,
    val endToStart: SwipeSlot,
) {
    /** The slot of [d]. */
    fun slot(d: SwipeDirection): SwipeSlot = if (d == SwipeDirection.START_TO_END) startToEnd else endToStart
}

/** The states of docs/ux/swipe.md, "Swipe state machine". */
enum class SwipePhase {
    /** Nothing is happening, or a drag is going on toward a direction that has no action (the row does not move). */
    IDLE,

    /** The card follows the finger below the threshold. */
    DRAGGING,

    /** Past the threshold: full colour, icon scaled, arm haptic fired. */
    ARMED,

    /** Released short of the threshold, or cancelled: the card springs back to zero. */
    SETTLING_BACK,

    /** Released armed (or flung): the card slides off, or springs back for read and star. */
    COMMITTING,

    /** Armed Snooze, Move to or Delete forever: the card is held at the park fraction while a sheet or dialog is open. */
    PARKED,

    /** The commit finished. The row leaves, or the caller resets the machine for rows that stay. */
    DONE,
}

/**
 * What the machine knows. [distance] is the finger's distance in [direction] (never negative, never resisted);
 * [offset] is where the card is, or is heading: signed, positive to the right on screen, with resistance applied while
 * dragging, [SwipeTuning.parkFraction] of the width when parked and the full width (slid off) when committing.
 */
data class SwipeState(
    val phase: SwipePhase = SwipePhase.IDLE,
    val direction: SwipeDirection? = null,
    val action: SwipeAction = SwipeAction.NONE,
    val distance: Float = 0f,
    val offset: Float = 0f,
    /** Time of the last arm or disarm, for the guard. */
    val lastCrossingMs: Long? = null,
    /** An unavailable action was dragged past its arm point (Rejected has played). */
    val rejectedSignalled: Boolean = false,
)

/** Why a gesture was cancelled without an action. */
enum class SwipeCancel {
    /** Another pointer touched the row. */
    SECOND_POINTER,

    /** Selection mode started. */
    SELECTION,

    /** The row left the screen or the list (a sync removed it). */
    ROW_REMOVED,

    /** Swipes were switched off (pull to refresh, fast scrolling). */
    DISABLED,
}

/** What happens to the machine. Distances are the finger's total displacement from where the gesture started. */
sealed interface SwipeInput {
    /** The finger is [dx] from its start, positive to the right; [timeMs] is the event time. */
    data class Drag(val dx: Float, val timeMs: Long) : SwipeInput

    /** The finger lifted at [velocity] (dp per second, positive to the right). */
    data class Release(val velocity: Float, val timeMs: Long) : SwipeInput

    /** The gesture ended without an action. */
    data class Cancel(val reason: SwipeCancel) : SwipeInput

    /** The animation that the last state started (spring back, slide out) has finished. */
    data object AnimationEnd : SwipeInput

    /** The sheet or dialog of a parked card closed: [chosen] is true when something was chosen or confirmed. */
    data class SheetResult(val chosen: Boolean) : SwipeInput

    /** The row is back in its place (the action failed, or the row stays): start over without effects. */
    data object Reset : SwipeInput
}

/** The haptic moments the machine asks for; the app maps them to its `HapticEvent`s. */
enum class SwipeHaptic { ThresholdArmed, ThresholdDisarmed, Committed, Rejected }

/** What the UI is asked to do. */
sealed interface SwipeEffect {
    /** Play a haptic. */
    data class Haptic(val kind: SwipeHaptic) : SwipeEffect

    /** Release armed or flung: start the commit (slide out and collapse, or spring back for read and star). */
    data class Commit(val action: SwipeAction, val direction: SwipeDirection) : SwipeEffect

    /** Release armed on an action that needs a sheet or dialog: hold the card and open it. */
    data class Park(val action: SwipeAction, val direction: SwipeDirection) : SwipeEffect

    /** Spring the card to zero. */
    data object SpringBack : SwipeEffect

    /** An unavailable action was asked for: the snackbar "Not available here" and its announcement. */
    data class NotAvailable(val action: SwipeAction) : SwipeEffect

    /** The commit (or the parked choice) is done: run the action. */
    data class Completed(val action: SwipeAction, val direction: SwipeDirection) : SwipeEffect
}

/** The result of one input. */
data class SwipeStep(val state: SwipeState, val effects: List<SwipeEffect>)

/**
 * The swipe state machine as a pure function: [reduce] takes the context, the state and an input and returns the next state
 * and the effects. No UI types, no clock (times arrive with the inputs). [SwipeMachine] keeps the state for a caller that
 * wants a plain object.
 *
 * The arm point is [SwipeTuning.armFraction] of the width. Crossing it fires one event (arm or disarm) unless the last
 * one was less than [SwipeTuning.crossingGuardMs] ago, in which case the state stays as it was until a later drag event, or
 * the release, looks again. Because every event flips the state, two arm events never follow each other without a
 * disarm between within one gesture. An unavailable action never arms: it plays Rejected once when the arm point is
 * reached, and releases with [SwipeEffect.NotAvailable] and a spring back.
 */
object SwipeReducer {
    /** Apply [input]. */
    fun reduce(tuning: SwipeTuning, ctx: SwipeContext, state: SwipeState, input: SwipeInput): SwipeStep = when (input) {
        is SwipeInput.Drag -> drag(tuning, ctx, state, input)
        is SwipeInput.Release -> release(tuning, ctx, state, input)
        is SwipeInput.Cancel -> cancel(tuning, ctx, state)
        SwipeInput.AnimationEnd -> animationEnd(ctx, state)
        is SwipeInput.SheetResult -> sheetResult(ctx, state, input.chosen)
        SwipeInput.Reset -> SwipeStep(SwipeState(), emptyList())
    }

    /** The card's offset for a finger [distance]: one to one up to the resistance point, then at the resistance factor, never past the width. */
    fun resisted(tuning: SwipeTuning, width: Float, distance: Float): Float {
        val start = tuning.resistanceStartFraction * width
        val d = maxOf(0f, distance)
        val moved = if (d <= start) d else start + (d - start) * tuning.resistanceFactor
        return minOf(moved, width)
    }

    /** The distance at which the action arms. */
    fun threshold(tuning: SwipeTuning, ctx: SwipeContext): Float = tuning.armFraction(ctx.sensitivity) * ctx.width

    /** A read or star that was committed only springs back, so a new touch can take the card (swipe again to reverse it). */
    private fun isGesture(s: SwipeState) =
        s.phase == SwipePhase.IDLE || s.phase == SwipePhase.DRAGGING || s.phase == SwipePhase.ARMED || s.phase == SwipePhase.SETTLING_BACK ||
            (s.phase == SwipePhase.COMMITTING && !s.action.removesRow)

    private fun drag(t: SwipeTuning, ctx: SwipeContext, s: SwipeState, input: SwipeInput.Drag): SwipeStep {
        // Once released into a commit that removes the row, or parked, the card is no longer the finger's.
        if (!isGesture(s)) return SwipeStep(s, emptyList())
        // A touch during the spring back takes the card over: a new gesture from the card's place.
        val base = if (s.phase == SwipePhase.SETTLING_BACK || s.phase == SwipePhase.COMMITTING) SwipeState() else s
        val effects = ArrayList<SwipeEffect>()
        if (input.dx == 0f) {
            val dir = base.direction ?: return SwipeStep(base, emptyList())
            return crossing(t, ctx, base.copy(distance = 0f, offset = 0f), dir, 0f, input.timeMs, guarded = true, effects)
        }
        val dir = SwipeDirection.of(input.dx, ctx.rtl)
        val slot = ctx.slot(dir)
        var cur = base
        // Moving over to the other side while armed: the armed action is not the one under the card any more.
        if (cur.direction != null && cur.direction != dir && cur.phase == SwipePhase.ARMED) {
            effects += SwipeEffect.Haptic(SwipeHaptic.ThresholdDisarmed)
            cur = cur.copy(phase = SwipePhase.DRAGGING, lastCrossingMs = input.timeMs)
        }
        if (cur.direction != null && cur.direction != dir) cur = cur.copy(rejectedSignalled = false)
        if (slot.action == SwipeAction.NONE) {
            val idle = SwipeState(lastCrossingMs = cur.lastCrossingMs)
            return SwipeStep(idle, effects)
        }
        val distance = abs(input.dx)
        val moved = cur.copy(phase = if (cur.phase == SwipePhase.IDLE) SwipePhase.DRAGGING else cur.phase, direction = dir, action = slot.action, distance = distance)
        return crossing(t, ctx, moved, dir, distance, input.timeMs, guarded = true, effects)
    }

    /** Looks at [distance] against the arm point and flips arm or reject once if it should (and the guard allows). */
    private fun crossing(
        t: SwipeTuning,
        ctx: SwipeContext,
        s: SwipeState,
        dir: SwipeDirection,
        distance: Float,
        timeMs: Long,
        guarded: Boolean,
        effects: MutableList<SwipeEffect>,
    ): SwipeStep {
        val slot = ctx.slot(dir)
        val sign = dir.physicalSign(ctx.rtl)
        val beyond = distance >= threshold(t, ctx) && slot.action != SwipeAction.NONE
        val last = s.lastCrossingMs
        val allowed = !guarded || last == null || timeMs - last >= t.crossingGuardMs
        var phase = if (s.phase == SwipePhase.IDLE || s.phase == SwipePhase.SETTLING_BACK) SwipePhase.DRAGGING else s.phase
        var rejected = s.rejectedSignalled
        var lastCrossing = s.lastCrossingMs
        if (slot.available) {
            val armed = phase == SwipePhase.ARMED
            if (beyond != armed && allowed) {
                phase = if (beyond) SwipePhase.ARMED else SwipePhase.DRAGGING
                effects += SwipeEffect.Haptic(if (beyond) SwipeHaptic.ThresholdArmed else SwipeHaptic.ThresholdDisarmed)
                lastCrossing = timeMs
            }
        } else if (beyond != rejected && allowed) {
            rejected = beyond
            if (beyond) effects += SwipeEffect.Haptic(SwipeHaptic.Rejected)
            lastCrossing = timeMs
        }
        val offset = sign * resisted(t, ctx.width, distance)
        return SwipeStep(s.copy(phase = phase, direction = dir, action = slot.action, distance = distance, offset = offset, lastCrossingMs = lastCrossing, rejectedSignalled = rejected), effects)
    }

    private fun release(t: SwipeTuning, ctx: SwipeContext, s: SwipeState, input: SwipeInput.Release): SwipeStep {
        if (s.phase != SwipePhase.DRAGGING && s.phase != SwipePhase.ARMED) return SwipeStep(s, emptyList())
        val dir = s.direction ?: return SwipeStep(SwipeState(), emptyList())
        val effects = ArrayList<SwipeEffect>()
        // Look once more at where the finger ended, whatever the guard says, so the decision matches what was drawn last.
        val settled = crossing(t, ctx, s, dir, s.distance, input.timeMs, guarded = false, effects).state
        val slot = ctx.slot(dir)
        val sign = dir.physicalSign(ctx.rtl)
        val along = input.velocity * sign
        val fling = along >= t.flingMinVelocity && settled.distance >= t.flingMinTravelFraction * ctx.width
        val armed = settled.phase == SwipePhase.ARMED
        val back = SwipeState(phase = SwipePhase.SETTLING_BACK, direction = dir, action = slot.action, distance = 0f, offset = 0f)
        if (!slot.available) {
            if (settled.rejectedSignalled || fling) effects += SwipeEffect.NotAvailable(slot.action)
            effects += SwipeEffect.SpringBack
            return SwipeStep(back, effects)
        }
        if (!armed && !fling) {
            effects += SwipeEffect.SpringBack
            return SwipeStep(back, effects)
        }
        effects += SwipeEffect.Haptic(SwipeHaptic.Committed)
        return if (slot.parks) {
            effects += SwipeEffect.Park(slot.action, dir)
            SwipeStep(settled.copy(phase = SwipePhase.PARKED, offset = sign * t.parkFraction * ctx.width), effects)
        } else {
            effects += SwipeEffect.Commit(slot.action, dir)
            SwipeStep(settled.copy(phase = SwipePhase.COMMITTING, offset = sign * ctx.width), effects)
        }
    }

    private fun cancel(t: SwipeTuning, ctx: SwipeContext, s: SwipeState): SwipeStep = when (s.phase) {
        SwipePhase.DRAGGING, SwipePhase.ARMED, SwipePhase.PARKED ->
            SwipeStep(SwipeState(phase = SwipePhase.SETTLING_BACK, direction = s.direction, action = s.action), listOf(SwipeEffect.SpringBack))
        // Nothing to undo, already going back, or past the point of no return (Undo covers that).
        else -> SwipeStep(s, emptyList())
    }

    private fun animationEnd(ctx: SwipeContext, s: SwipeState): SwipeStep = when (s.phase) {
        SwipePhase.SETTLING_BACK -> SwipeStep(SwipeState(), emptyList())
        SwipePhase.COMMITTING -> {
            val dir = s.direction
            if (dir == null) SwipeStep(SwipeState(), emptyList())
            else SwipeStep(s.copy(phase = SwipePhase.DONE), listOf(SwipeEffect.Completed(s.action, dir)))
        }
        else -> SwipeStep(s, emptyList())
    }

    private fun sheetResult(ctx: SwipeContext, s: SwipeState, chosen: Boolean): SwipeStep {
        if (s.phase != SwipePhase.PARKED) return SwipeStep(s, emptyList())
        val dir = s.direction ?: return SwipeStep(SwipeState(), emptyList())
        return if (chosen) {
            val full = dir.physicalSign(ctx.rtl) * ctx.width
            SwipeStep(s.copy(phase = SwipePhase.DONE, offset = full), listOf(SwipeEffect.Completed(s.action, dir)))
        } else {
            SwipeStep(SwipeState(phase = SwipePhase.SETTLING_BACK, direction = dir, action = s.action), listOf(SwipeEffect.SpringBack))
        }
    }
}

/** [SwipeReducer] with the state kept: one per row gesture host. */
class SwipeMachine(private val tuning: SwipeTuning, var context: SwipeContext) {
    /** The current state. */
    var state: SwipeState = SwipeState()
        private set

    /** Feed [input]; returns the effects it caused. */
    fun send(input: SwipeInput): List<SwipeEffect> {
        val step = SwipeReducer.reduce(tuning, context, state, input)
        state = step.state
        return step.effects
    }
}
