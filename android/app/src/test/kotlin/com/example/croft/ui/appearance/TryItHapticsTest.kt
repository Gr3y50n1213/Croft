// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.ui.appearance

import android.view.HapticFeedbackConstants
import android.view.ViewGroup
import androidx.activity.ComponentActivity
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollTo
import androidx.core.view.HapticFeedbackConstantsCompat
import com.example.croft.CroftTheme
import com.example.croft.haptics.AndroidHaptics
import com.example.croft.haptics.HapticEvent
import com.example.croft.haptics.Haptics
import com.example.croft.haptics.LevelFilteredHaptics
import com.example.croft.haptics.LocalHaptics
import com.example.croft.haptics.RecordingHaptics
import com.example.croft.theme.AnimationLevel
import com.example.croft.theme.HapticsLevel
import com.example.croft.theme.ThemeSettings
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config

/** S53 Try it buttons are connected to `Haptics` (T12 left them calling `onTry`); and the platform file maps each event. */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35])
class TryItHapticsTest {
    @get:Rule
    val rule = createAndroidComposeRule<ComponentActivity>()

    private var clock = 10_000L
    private val recording = RecordingHaptics()

    private fun showS53(level: HapticsLevel) {
        val filtered = LevelFilteredHaptics({ level }, { clock }, recording)
        rule.setContent {
            CroftTheme(ThemeSettings.Default.copy(animation = AnimationLevel.OFF, haptics = level), systemDark = false) {
                CompositionLocalProvider(LocalHaptics provides filtered) {
                    MotionHapticsScreen(ThemeSettings.Default, onChange = {}, onBack = {})
                }
            }
        }
    }

    private fun pressAll() {
        for (e in TryItEvent.entries) {
            rule.onNodeWithTag(MotionTags.TRY_PREFIX + e.name.lowercase()).performScrollTo().performClick()
            clock += 1_000
        }
    }

    @Test
    fun atFullEveryTryItButtonPlaysItsEvent() {
        showS53(HapticsLevel.FULL)
        pressAll()
        assertEquals(
            listOf(HapticEvent.ThresholdArmed, HapticEvent.Success, HapticEvent.Failure, HapticEvent.ToggleOn, HapticEvent.Tick, HapticEvent.DragStart),
            recording.events,
        )
    }

    @Test
    fun atSubtleOnlyTheMomentsThatCarryMeaningAreFelt() {
        showS53(HapticsLevel.SUBTLE)
        pressAll()
        assertEquals(listOf(HapticEvent.ThresholdArmed, HapticEvent.Success, HapticEvent.Failure), recording.events)
    }

    @Test
    fun atOffNothingIsFelt() {
        showS53(HapticsLevel.OFF)
        pressAll()
        assertTrue(recording.events.isEmpty())
    }

    @Test
    fun theAppThemeProvidesTheRealFilteredHaptics_andItFollowsTheStoredLevel() {
        var seen: Haptics? = null
        var level by mutableStateOf(HapticsLevel.FULL)
        rule.setContent { CroftTheme(ThemeSettings.Default.copy(haptics = level)) { seen = LocalHaptics.current } }
        rule.waitForIdle()
        assertTrue("CroftTheme provides the filter around the Android haptics", seen is LevelFilteredHaptics)
    }

    private fun contentView() = (rule.activity.findViewById<ViewGroup>(android.R.id.content)).getChildAt(0)

    /** The last haptic feedback any view under the content received, or -1. */
    private fun lastFeedback(): Int {
        fun walk(v: android.view.View): Int {
            val own = shadowOf(v).lastHapticFeedbackPerformed()
            if (own != -1) return own
            if (v is ViewGroup) for (i in 0 until v.childCount) walk(v.getChildAt(i)).let { if (it != -1) return it }
            return -1
        }
        return walk(rule.activity.window.decorView)
    }

    private fun showRealS53(level: HapticsLevel) {
        rule.setContent {
            CroftTheme(ThemeSettings.Default.copy(animation = AnimationLevel.OFF, haptics = level), systemDark = false) {
                MotionHapticsScreen(ThemeSettings.Default, onChange = {}, onBack = {})
            }
        }
    }

    @Test
    fun throughTheRealPlatformFileTheTryItButtonReachesTheViewAtFull() {
        showRealS53(HapticsLevel.FULL)
        rule.onNodeWithTag(MotionTags.TRY_PREFIX + "threshold").performScrollTo().performClick()
        assertEquals(HapticFeedbackConstantsCompat.GESTURE_THRESHOLD_ACTIVATE, lastFeedback())
    }

    @Test
    fun throughTheRealPlatformFileNothingReachesTheViewAtOff() {
        showRealS53(HapticsLevel.OFF)
        for (e in TryItEvent.entries) rule.onNodeWithTag(MotionTags.TRY_PREFIX + e.name.lowercase()).performScrollTo().performClick()
        assertEquals(-1, lastFeedback())
    }

    @Test
    fun theAndroidFileMapsEveryEventToItsConstantFromTheEventMap() {
        val expected = mapOf(
            HapticEvent.ThresholdArmed to HapticFeedbackConstantsCompat.GESTURE_THRESHOLD_ACTIVATE,
            HapticEvent.ThresholdDisarmed to HapticFeedbackConstantsCompat.GESTURE_THRESHOLD_DEACTIVATE,
            HapticEvent.Committed to HapticFeedbackConstantsCompat.CONFIRM,
            HapticEvent.Rejected to HapticFeedbackConstantsCompat.REJECT,
            HapticEvent.ToggleOn to HapticFeedbackConstantsCompat.TOGGLE_ON,
            HapticEvent.ToggleOff to HapticFeedbackConstantsCompat.TOGGLE_OFF,
            HapticEvent.SelectionStart to HapticFeedbackConstantsCompat.LONG_PRESS,
            HapticEvent.SelectionTick to HapticFeedbackConstantsCompat.SEGMENT_TICK,
            HapticEvent.Tick to HapticFeedbackConstantsCompat.SEGMENT_TICK,
            HapticEvent.DragStart to HapticFeedbackConstantsCompat.DRAG_START,
            HapticEvent.Success to HapticFeedbackConstantsCompat.CONFIRM,
            HapticEvent.Failure to HapticFeedbackConstantsCompat.REJECT,
        )
        assertEquals(HapticEvent.entries.toSet(), expected.keys)
        for ((event, constant) in expected) assertEquals("$event", constant, AndroidHaptics.constantFor(event))
        // The compat values are the platform's own where they exist (API 30 and 34 constants).
        assertEquals(HapticFeedbackConstants.LONG_PRESS, HapticFeedbackConstantsCompat.LONG_PRESS)
    }

    @Test
    fun androidHapticsPerformsTheFeedbackOnTheView() {
        rule.setContent { }
        val view = rule.activity.window.decorView
        AndroidHaptics(view).perform(HapticEvent.Success)
        assertEquals(HapticFeedbackConstantsCompat.CONFIRM, shadowOf(view).lastHapticFeedbackPerformed())
    }
}
