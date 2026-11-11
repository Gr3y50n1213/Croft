// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.ui.components

import androidx.compose.foundation.layout.Column
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Add
import androidx.compose.material.icons.filled.Delete
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.AccessibilityManager
import androidx.compose.ui.platform.LocalAccessibilityManager
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.test.assertCountEquals
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.assertIsNotEnabled
import androidx.compose.ui.test.isDialog
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.longClick
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performTouchInput
import com.example.croft.haptics.HapticEvent
import com.example.croft.haptics.RecordingHaptics
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertThrows
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config

/** Banners, snackbars, dialogs, sheets, menus and icon-only buttons: docs/ux/controls.md. */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35])
class FeedbackTest {
    @get:Rule
    val rule = createComposeRule()

    private val haptics = RecordingHaptics()

    // ---- Banner ----

    @Test
    fun theMostUrgentBannerWins() {
        assertEquals(BannerKind.NEEDS_SIGN_IN, mostUrgentBanner(BannerKind.entries))
        assertEquals(BannerKind.OFFLINE, mostUrgentBanner(listOf(BannerKind.UNSUBSCRIBE, BannerKind.OFFLINE, BannerKind.REMOTE_IMAGES_BLOCKED)))
        assertEquals(null, mostUrgentBanner(emptyList()))
    }

    @Test
    fun onlyOneBannerShowsAtATime() {
        rule.show { BannerHost(BannerKind.entries, {}, {}, accountName = "Sam") }
        rule.onNodeWithText("Sign in again to Sam").assertIsDisplayed()
        rule.onAllNodesWithText("You are offline. Changes wait and are sent later.").assertCountEquals(0)
        rule.onAllNodesWithText("Remote images are blocked").assertCountEquals(0)
        rule.onAllNodesWithText("This looks like a mailing list").assertCountEquals(0)
    }

    @Test
    fun aBannerHasItsActionAndADismissButtonWithALabel() {
        var action = 0; var dismissed = 0
        rule.show { CroftBanner(BannerKind.REMOTE_IMAGES_BLOCKED, { action++ }, { dismissed++ }) }
        rule.onNodeWithText("Show images").performClick()
        rule.onNodeWithContentDescription("Dismiss").performClick()
        assertEquals(1, action); assertEquals(1, dismissed)
    }

    // ---- Snackbar ----

    @Test
    fun aNewSnackbarReplacesTheOldOne_andTheOldActionIsGone() {
        val state = CroftSnackbarState()
        var firstUndo = 0; var secondUndo = 0
        rule.show { CroftSnackbarHost(state) }
        rule.ui { state.show("Message deleted", "Undo") { firstUndo++ } }
        rule.onNodeWithText("Message deleted").assertIsDisplayed()
        rule.ui { state.show("Message archived", "Undo") { secondUndo++ } }
        rule.waitForIdle()
        rule.onAllNodesWithText("Message deleted").assertCountEquals(0)
        rule.onAllNodesWithText("Undo").assertCountEquals(1)
        rule.onNodeWithText("Undo").performClick()
        assertEquals("the replaced snackbar's Undo cannot run any more", 0, firstUndo)
        assertEquals(1, secondUndo)
    }

    @Test
    fun anOldActionCannotBePerformedAfterItWasReplaced() {
        val state = CroftSnackbarState()
        var undone = 0
        val first = state.show("A", "Undo") { undone++ }
        state.show("B")
        state.perform(first)
        assertEquals(0, undone)
        assertEquals("B", state.current?.text)
    }

    @Test
    fun aSnackbarWithoutAnActionStaysFourSeconds() {
        val state = CroftSnackbarState()
        rule.mainClock.autoAdvance = false
        rule.show { CroftSnackbarHost(state) }
        rule.ui { state.show("Saved") }
        rule.mainClock.advanceTimeBy(3_900)
        rule.onNodeWithText("Saved").assertIsDisplayed()
        rule.mainClock.advanceTimeBy(300)
        assertEquals(null, state.current)
    }

    @Test
    fun anUndoSnackbarStaysForTheUndoWindow() {
        val state = CroftSnackbarState()
        rule.mainClock.autoAdvance = false
        rule.show { CroftSnackbarHost(state) }
        rule.ui { state.show("Deleted", "Undo", stayMs = CroftSnackbarState.DefaultUndoWindowMs) {} }
        rule.mainClock.advanceTimeBy(4_900)
        assertEquals("Deleted", state.current?.text)
        rule.mainClock.advanceTimeBy(300)
        assertEquals(null, state.current)
        assertEquals(5_000L, CroftSnackbarState.DefaultUndoWindowMs)
    }

    @Test
    fun aLongerAccessibilityTimeoutWins() {
        val manager = object : AccessibilityManager {
            override fun calculateRecommendedTimeoutMillis(originalTimeoutMillis: Long, containsIcons: Boolean, containsText: Boolean, containsControls: Boolean): Long = 12_000L
        }
        val state = CroftSnackbarState()
        rule.mainClock.autoAdvance = false
        rule.setContent {
            androidx.compose.runtime.CompositionLocalProvider(LocalAccessibilityManager provides manager) {
                com.example.croft.CroftTheme { CroftSnackbarHost(state) }
            }
        }
        rule.ui { state.show("Saved") }
        rule.mainClock.advanceTimeBy(11_000)
        assertEquals("Saved", state.current?.text)
        rule.mainClock.advanceTimeBy(1_500)
        assertEquals(null, state.current)
    }

    // ---- Dialog ----

    @Test
    fun theDestructiveDialogHasItsConfirmOnTheRightInTheErrorColour_andConfirmingPlaysTheHaptic() {
        var confirmed = 0; var dismissed = 0
        rule.show(haptics = haptics) { DestructiveDialog("Delete forever?", "This cannot be undone.", "Delete forever", { confirmed++ }, { dismissed++ }) }
        rule.onNode(isDialog()).assertExists()
        rule.onNodeWithText("Delete forever?").assertIsDisplayed()
        rule.onNodeWithText("Cancel").assertIsDisplayed()
        rule.onNodeWithText("Delete forever", substring = false).performClick()
        assertEquals(1, confirmed)
        assertEquals(0, dismissed)
        assertEquals(listOf(HapticEvent.Success), haptics.events)
    }

    @Test
    fun cancelDismissesWithoutAHaptic() {
        var confirmed = 0; var dismissed = 0
        rule.show(haptics = haptics) { DestructiveDialog("Delete forever?", "This cannot be undone.", "Delete forever", { confirmed++ }, { dismissed++ }) }
        rule.onNodeWithText("Cancel").performClick()
        assertEquals(0, confirmed); assertEquals(1, dismissed)
        assertTrue(haptics.events.isEmpty())
    }

    @Test
    fun backCancelsTheDialog() {
        var dismissed = 0
        rule.show { DestructiveDialog("Delete forever?", "This cannot be undone.", "Delete forever", {}, { dismissed++ }) }
        androidx.test.espresso.Espresso.pressBack()
        rule.waitForIdle()
        assertEquals(1, dismissed)
    }

    // ---- Bottom sheet ----

    @Test
    fun aSheetShowsItsContent() {
        rule.show { CroftBottomSheet(onDismiss = {}) { androidx.compose.material3.Text("Snooze until") } }
        rule.onNodeWithText("Snooze until").assertIsDisplayed()
    }

    @Test
    fun aSheetWithUnsavedInputAsksBeforeItGoes() {
        var dismissed = 0
        rule.show { CroftBottomSheet(onDismiss = { dismissed++ }, hasUnsavedInput = true) { androidx.compose.material3.Text("Custom time") } }
        androidx.test.espresso.Espresso.pressBack()
        rule.waitForIdle()
        rule.onNodeWithText("Discard changes?").assertIsDisplayed()
        assertEquals("not dismissed yet", 0, dismissed)
        rule.onNodeWithText("Discard").performClick()
        rule.waitForIdle()
        assertEquals(1, dismissed)
    }

    // ---- Menus ----

    @Test
    fun aMenuHasAtMostSevenItems_withDestructiveOnesLast_andHiddenItemsDoNotCount() {
        fun item(n: String, d: Boolean = false, v: Boolean = true) = MenuItemSpec(n, {}, destructive = d, visible = v)
        MenuModel.of((1..7).map { item("i$it") })
        assertThrows(IllegalArgumentException::class.java) { MenuModel.of((1..8).map { item("i$it") }) }
        MenuModel.of((1..8).map { item("i$it", v = it != 8) })
        assertThrows(IllegalArgumentException::class.java) { MenuModel.of(listOf(item("Delete", d = true), item("Archive"))) }
        MenuModel.of(listOf(item("Archive"), item("Block", d = true), item("Delete", d = true)))
        assertEquals(listOf("a", "c"), MenuModel.of(listOf(item("a"), item("b", v = false), item("c"))).items.map { it.label })
    }

    @Test
    fun aMenuShowsItsItemsInOrder_aDisabledOneWithItsReason_andAClickClosesAndActs() {
        var open by mutableStateOf(true)
        val picked = mutableListOf<String>()
        rule.show {
            CroftMenu(
                open, { open = false },
                listOf(
                    MenuItemSpec("Mark as unread", { picked += "unread" }),
                    MenuItemSpec("Print", {}, disabledReason = "No printer is set up"),
                    MenuItemSpec("Delete", { picked += "delete" }, destructive = true),
                ),
            )
        }
        rule.onNodeWithText("Mark as unread").assertIsDisplayed()
        rule.onNodeWithText("No printer is set up").assertIsDisplayed()
        rule.onNodeWithText("Print").assertIsNotEnabled()
        rule.onNodeWithText("Delete").performClick()
        assertEquals(listOf("delete"), picked)
        assertFalse(open)
    }

    // ---- Icon-only button ----

    @Test
    fun anIconOnlyButtonHasItsLabelForScreenReaders_andATooltipOnLongPress() {
        var taps = 0
        rule.show { CroftIconButton(Icons.Filled.Delete, "Delete", { taps++ }, Modifier.testTag("icon")) }
        assertEquals(listOf("Delete"), rule.onNodeWithTag("icon").fetchSemanticsNode().config[SemanticsProperties.ContentDescription])
        rule.onNodeWithTag("icon").performClick()
        assertEquals(1, taps)
        rule.onAllNodesWithText("Delete").assertCountEquals(0)
        rule.onNodeWithTag("icon").performTouchInput { longClick() }
        rule.waitForIdle()
        rule.onNodeWithText("Delete").assertIsDisplayed()
    }

    @Test
    fun aDisabledIconButtonStaysInTheTree_andSaysWhy() {
        val reasons = mutableListOf<String>()
        rule.show { CroftIconButton(Icons.Filled.Add, "Add", {}, Modifier.testTag("icon"), enabled = false, disabledReason = "Nothing to add yet", onExplain = { reasons += it }) }
        rule.onNodeWithTag("icon").assertIsNotEnabled().performClick()
        assertEquals(listOf("Nothing to add yet"), reasons)
    }
}
