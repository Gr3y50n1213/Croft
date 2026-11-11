// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.haptics

import android.view.View
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.compositionLocalOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.ui.platform.LocalView
import androidx.core.view.HapticFeedbackConstantsCompat
import androidx.core.view.ViewCompat
import com.example.croft.design.CroftTokens
import com.example.croft.theme.HapticsLevel

/**
 * The moments of a gesture that carry a haptic (docs/ux/haptics.md, event map). Screens call these; only
 * [AndroidHaptics] knows which platform constant each one is. Every haptic in the app goes through [Haptics].
 */
enum class HapticEvent(
    /** The lowest level at which the event is felt: Subtle keeps the moments that carry meaning, Full adds the rest. */
    val minLevel: HapticsLevel,
) {
    /** Swipe or pull reaches its threshold. */
    ThresholdArmed(HapticsLevel.SUBTLE),

    /** Swipe goes back past its threshold. */
    ThresholdDisarmed(HapticsLevel.FULL),

    /** A swipe commits (release while armed). */
    Committed(HapticsLevel.FULL),

    /** An action is not available, or a change was refused. */
    Rejected(HapticsLevel.SUBTLE),

    /** A switch or star turns on. */
    ToggleOn(HapticsLevel.FULL),

    /** A switch or star turns off. */
    ToggleOff(HapticsLevel.FULL),

    /** A long-press that opens selection mode, a context menu or Schedule send. */
    SelectionStart(HapticsLevel.SUBTLE),

    /** A row is added to or removed from the selection. */
    SelectionTick(HapticsLevel.FULL),

    /** A slider step, the fast scroller reaching the next date. */
    Tick(HapticsLevel.FULL),

    /** An item starts a drag to reorder. */
    DragStart(HapticsLevel.FULL),

    /** Send accepted, destructive dialog confirmed. */
    Success(HapticsLevel.SUBTLE),

    /** Send failed, sync error, a change the server refused. */
    Failure(HapticsLevel.SUBTLE),
}

/** The one haptics interface of the UI layer. */
interface Haptics {
    /** Fire [event] once. Whether it is felt depends on the level and the 40 ms guard (see [LevelFilteredHaptics]). */
    fun perform(event: HapticEvent)
}

/** Does nothing: the default where no provider is above (previews, tests that do not care). */
object NoHaptics : Haptics {
    override fun perform(event: HapticEvent) = Unit
}

/** The haptics for the composition below. [CroftTheme][com.example.croft.CroftTheme] provides the real one. */
val LocalHaptics = compositionLocalOf<Haptics> { NoHaptics }

/** The 40 ms guard from `tokens.json` (`haptics.minGapMs`). */
val SameEventGuardMs: Long = CroftTokens.Haptics.minGapMs.toLong()

/**
 * The central filter: drops the events the current [level] does not include (Off sends none, Subtle keeps the events
 * whose [HapticEvent.minLevel] is Subtle, Full sends all), and drops the same event when it fires again within
 * [SameEventGuardMs] of the last one that went through. The clock is injected so tests can move time.
 */
class LevelFilteredHaptics(
    private val level: () -> HapticsLevel,
    private val clockMs: () -> Long,
    private val next: Haptics,
) : Haptics {
    private val lastSent = HashMap<HapticEvent, Long>()

    override fun perform(event: HapticEvent) {
        if (!allows(level(), event)) return
        val now = clockMs()
        val last = lastSent[event]
        if (last != null && now - last < SameEventGuardMs) return
        lastSent[event] = now
        next.perform(event)
    }

    companion object {
        /** Whether [event] is felt at [level]. */
        fun allows(level: HapticsLevel, event: HapticEvent): Boolean = when (level) {
            HapticsLevel.OFF -> false
            HapticsLevel.SUBTLE -> event.minLevel == HapticsLevel.SUBTLE
            HapticsLevel.FULL -> true
        }
    }
}

/**
 * The Android platform file: the only place that knows the constants. `ViewCompat.performHapticFeedback` takes the
 * [HapticFeedbackConstantsCompat] value and falls back on phones that lack the newer ones (the map in
 * docs/ux/haptics.md). It respects the person's system touch feedback setting.
 */
class AndroidHaptics(private val view: View) : Haptics {
    override fun perform(event: HapticEvent) {
        ViewCompat.performHapticFeedback(view, constantFor(event))
    }

    companion object {
        /** The platform constant behind each event. */
        fun constantFor(event: HapticEvent): Int = when (event) {
            HapticEvent.ThresholdArmed -> HapticFeedbackConstantsCompat.GESTURE_THRESHOLD_ACTIVATE
            HapticEvent.ThresholdDisarmed -> HapticFeedbackConstantsCompat.GESTURE_THRESHOLD_DEACTIVATE
            HapticEvent.Committed -> HapticFeedbackConstantsCompat.CONFIRM
            HapticEvent.Rejected -> HapticFeedbackConstantsCompat.REJECT
            HapticEvent.ToggleOn -> HapticFeedbackConstantsCompat.TOGGLE_ON
            HapticEvent.ToggleOff -> HapticFeedbackConstantsCompat.TOGGLE_OFF
            HapticEvent.SelectionStart -> HapticFeedbackConstantsCompat.LONG_PRESS
            HapticEvent.SelectionTick -> HapticFeedbackConstantsCompat.SEGMENT_TICK
            HapticEvent.Tick -> HapticFeedbackConstantsCompat.SEGMENT_TICK
            HapticEvent.DragStart -> HapticFeedbackConstantsCompat.DRAG_START
            HapticEvent.Success -> HapticFeedbackConstantsCompat.CONFIRM
            HapticEvent.Failure -> HapticFeedbackConstantsCompat.REJECT
        }
    }
}

/** Provides the real haptics (this view, filtered by [level] and guarded) to everything below. */
@Composable
fun ProvideHaptics(level: HapticsLevel, content: @Composable () -> Unit) {
    val view = LocalView.current
    val current = rememberUpdatedState(level)
    val haptics = remember(view) {
        LevelFilteredHaptics(level = { current.value }, clockMs = { android.os.SystemClock.elapsedRealtime() }, next = AndroidHaptics(view))
    }
    CompositionLocalProvider(LocalHaptics provides haptics, content = content)
}
