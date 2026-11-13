// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.ui.swipe

import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.platform.LocalLayoutDirection
import androidx.compose.ui.test.center
import androidx.compose.ui.test.down
import androidx.compose.ui.test.hasAnyAncestor
import androidx.compose.ui.test.hasTestTag
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.moveBy
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performTouchInput
import androidx.compose.ui.test.up
import androidx.compose.ui.unit.LayoutDirection
import androidx.test.espresso.Espresso
import com.example.croft.haptics.HapticEvent
import com.example.croft.haptics.RecordingHaptics
import com.example.croft.swipe.SwipeAction
import com.example.croft.swipe.SwipeDirection
import com.example.croft.swipe.SwipeSensitivity
import com.example.croft.swipe.SwipeSettings
import com.example.croft.theme.AnimationLevel
import com.example.croft.ui.components.isInteractive
import com.example.croft.ui.components.probe
import com.example.croft.ui.components.show
import kotlin.math.abs
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

/** S47 Swipe actions and S48 Choose swipe action: the preview, the pickers, sensitivity, presets, Reset, Back and right-to-left. */
@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [35], qualifiers = "w360dp-h800dp-mdpi")
class SwipeSettingsScreenTest {
    @get:Rule
    val rule = createComposeRule()

    private var settings by mutableStateOf(SwipeSettings.Default)
    private var backs = 0
    private val haptics = RecordingHaptics()

    private fun show(level: AnimationLevel = AnimationLevel.OFF, demo: Boolean = false, rtl: Boolean = false, startAt: SwipeRoute = SwipeRoute.S47) {
        rule.show(level, haptics) {
            CompositionLocalProvider(LocalLayoutDirection provides if (rtl) LayoutDirection.Rtl else LayoutDirection.Ltr) {
                SwipeSettingsFlow(settings, { settings = it }, onBack = { backs++ }, startAt = startAt, demo = demo)
            }
        }
        if (rule.mainClock.autoAdvance) rule.waitForIdle()
    }

    private fun exists(text: String) = rule.onAllNodes(hasText(text)).fetchSemanticsNodes().isNotEmpty()

    @Test
    fun s47_showsThePreviewThePickersTheSensitivityThePresetsAndReset() {
        show()
        rule.onNodeWithTag(SwipeTags.S47).assertExists()
        rule.onNodeWithTag(SwipeTags.PREVIEW).assertExists()
        rule.onNodeWithText("Plays on a loop. Try your own swipe on the row.").assertExists()
        rule.onNodeWithTag(SwipeTags.picker(SwipeDirection.START_TO_END)).assertExists()
        rule.onNodeWithText("Swipe right").assertExists()
        rule.onNodeWithText("Mark read or unread").assertExists()
        rule.onNodeWithText("Swipe left").assertExists()
        rule.onNodeWithText("Delete").assertExists()
        for (s in listOf("Low", "Normal", "High")) rule.onNodeWithText(s).assertExists()
        rule.onNodeWithText("Delete forever asks first").assertExists()
        rule.onNodeWithText("This one cannot be turned off").assertExists()
        for (p in listOf("Triage", "Gmail style", "Tidy", "Inbox zero")) rule.onNodeWithText(p).assertExists()
        rule.onNodeWithTag(SwipeTags.RESET).assertExists()
    }

    @Test
    fun s47_presetsSetBothDirections_andTheMatchingOneIsSelected() {
        show()
        rule.onNodeWithTag(SwipeTags.preset(com.example.croft.swipe.SwipePreset.GMAIL_STYLE)).performClick()
        rule.waitForIdle()
        assertEquals(SwipeSettings(SwipeAction.ARCHIVE, SwipeAction.ARCHIVE), settings)
        rule.onNodeWithTag(SwipeTags.preset(com.example.croft.swipe.SwipePreset.TIDY)).performClick()
        rule.waitForIdle()
        assertEquals(SwipeSettings(SwipeAction.TOGGLE_READ, SwipeAction.ARCHIVE), settings)
        rule.onNodeWithTag(SwipeTags.preset(com.example.croft.swipe.SwipePreset.TRIAGE)).performClick()
        rule.waitForIdle()
        assertEquals(SwipeSettings.Default, settings)
    }

    @Test
    fun s47_sensitivityChangesTheArmPointSetting() {
        show()
        rule.onNodeWithText("High").performClick()
        rule.waitForIdle()
        assertEquals(SwipeSensitivity.HIGH, settings.sensitivity)
        rule.onNodeWithText("Low").performClick()
        rule.waitForIdle()
        assertEquals(SwipeSensitivity.LOW, settings.sensitivity)
    }

    @Test
    fun s47_resetGoesBackToTriageAtNormal() {
        settings = SwipeSettings(SwipeAction.SPAM, SwipeAction.ARCHIVE, SwipeSensitivity.HIGH)
        show()
        rule.onNodeWithTag(SwipeTags.RESET).performClick()
        rule.waitForIdle()
        assertEquals(SwipeSettings.Default, settings)
    }

    @Test
    fun s48_theRadioListShowsEveryActionWithItsIcon_includingMoveToAndSnooze() {
        show(startAt = SwipeRoute.S48_START_TO_END)
        rule.onNodeWithTag(SwipeTags.S48).assertExists()
        for (a in SwipeAction.picker) rule.onNodeWithTag(SwipeTags.option(a)).assertExists()
        for (name in listOf("Mark read or unread", "Archive", "Delete", "Snooze", "Move to", "Star", "Mark as spam", "None")) rule.onNodeWithText(name).assertExists()
        assertFalse(exists("Coming later"))
        rule.onNodeWithText("The pill and icon use the colour shown here while you swipe.", substring = true).assertExists()
    }

    @Test
    fun s47_pickingAnActionInS48SetsThatDirectionOnly_andReturnsToS47() {
        show()
        rule.onNodeWithTag(SwipeTags.picker(SwipeDirection.START_TO_END)).performClick()
        rule.waitForIdle()
        rule.onNodeWithTag(SwipeTags.S48).assertExists()
        rule.onNodeWithTag(SwipeTags.option(SwipeAction.ARCHIVE)).performClick()
        rule.waitForIdle()
        assertEquals(SwipeSettings(SwipeAction.ARCHIVE, SwipeAction.DELETE), settings)
        rule.onNodeWithTag(SwipeTags.S47).assertExists()
    }

    @Test
    fun s48_theSameActionForBothDirectionsIsAllowed() {
        show()
        rule.onNodeWithTag(SwipeTags.picker(SwipeDirection.END_TO_START)).performClick()
        rule.waitForIdle()
        rule.onNodeWithTag(SwipeTags.option(SwipeAction.TOGGLE_READ)).performClick()
        rule.waitForIdle()
        assertEquals(SwipeSettings(SwipeAction.TOGGLE_READ, SwipeAction.TOGGLE_READ), settings)
    }

    @Test
    fun s48_backReturnsToS47First() {
        show()
        rule.onNodeWithTag(SwipeTags.picker(SwipeDirection.START_TO_END)).performClick()
        rule.waitForIdle()
        Espresso.pressBack()
        rule.waitForIdle()
        rule.onNodeWithTag(SwipeTags.S47).assertExists()
        assertEquals("Back from S48 does not leave S47", 0, backs)
        assertEquals("nothing was changed", SwipeSettings.Default, settings)
    }

    @Test
    fun s47_inRightToLeftTheFirstPickerIsTheLeftSwipe() {
        show(rtl = true)
        // START_TO_END (Mark read) is the stored first direction; in RTL it is a swipe to the left.
        val first = rule.onNodeWithTag(SwipeTags.picker(SwipeDirection.START_TO_END)).fetchSemanticsNode().config
        val description = first[androidx.compose.ui.semantics.SemanticsProperties.ContentDescription].joinToString()
        assertTrue(description, description.startsWith("Swipe left") && description.contains("Mark read or unread"))
    }

    @Test
    fun s47_thePreviewRowIsARealRowYouCanSwipe_andAnActionThatRemovesItIsShownThenUndone() {
        show()
        val row = rule.onNode(isInteractive and hasAnyAncestor(hasTestTag(SwipeTags.PREVIEW)))
        row.performTouchInput { down(center); moveBy(Offset(-10f, 0f)); moveBy(Offset(-130f, 0f)) }
        rule.waitForIdle()
        assertEquals(HapticEvent.ThresholdArmed, haptics.events.last())
        row.performTouchInput { up() }
        rule.waitForIdle()
        assertEquals(listOf(HapticEvent.ThresholdArmed, HapticEvent.Committed), haptics.events)
        // The row comes back (nothing in the preview is a message), at rest, ready for the next swipe.
        rule.onNode(isInteractive and hasAnyAncestor(hasTestTag(SwipeTags.PREVIEW))).assertExists()
        assertEquals(SwipeSettings.Default, settings)
    }

    @Test
    fun s47_theLoopedDemonstrationMovesTheRowInEachDirection_silently() {
        rule.mainClock.autoAdvance = false
        show(level = AnimationLevel.FULL, demo = true)
        var farRight = 0f
        var farLeft = 0f
        repeat(400) {
            rule.mainClock.advanceTimeBy(50)
            val offset = rule.probe("swipe-offset")
            if (offset > farRight) farRight = offset
            if (offset < farLeft) farLeft = offset
        }
        assertTrue("it dragged to the right (Mark read): $farRight", farRight > 100f)
        assertTrue("it dragged to the left (Delete): $farLeft", farLeft < -100f)
        assertTrue("the demonstration makes no vibration: ${haptics.events}", haptics.events.isEmpty())
    }

    @Test
    fun s47_theDemonstrationDoesNotRunAtOff() {
        rule.mainClock.autoAdvance = false
        show(level = AnimationLevel.OFF, demo = true)
        var far = 0f
        repeat(200) {
            rule.mainClock.advanceTimeBy(50)
            far = maxOf(far, abs(rule.probe("swipe-offset")))
        }
        assertEquals(0f, far, 0.5f)
    }
}
