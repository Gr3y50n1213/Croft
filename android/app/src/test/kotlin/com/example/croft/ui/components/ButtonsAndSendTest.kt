// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.ui.components

import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.semantics.getOrNull
import androidx.compose.ui.test.assertIsEnabled
import androidx.compose.ui.test.assertIsNotEnabled
import androidx.compose.ui.test.assertTextEquals
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.longClick
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performTouchInput
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.assertCountEquals
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.assertTextContains
import com.example.croft.haptics.HapticEvent
import com.example.croft.haptics.RecordingHaptics
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import kotlinx.coroutines.delay
import kotlinx.coroutines.test.advanceTimeBy
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.launch
import androidx.compose.runtime.getValue
import androidx.compose.runtime.setValue
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config

/** Buttons and the Send button: docs/ux/controls.md, "Buttons" and "Send button". */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35])
class ButtonsAndSendTest {
    @get:Rule
    val rule = createComposeRule()

    private fun widthOf(tag: String) = rule.onNodeWithTag(tag).fetchSemanticsNode().size.width

    // ---- Buttons ----

    @Test
    fun everyKindRendersEnabledAndTapsOnce() {
        val taps = ButtonKind.entries.associateWith { 0 }.toMutableMap()
        rule.show {
            androidx.compose.foundation.layout.Column {
                for (kind in ButtonKind.entries) CroftButton("Label", { taps[kind] = taps.getValue(kind) + 1 }, Modifier.testTag("button-${kind.name}"), kind = kind)
            }
        }
        for (kind in ButtonKind.entries) {
            rule.onNodeWithTag("button-${kind.name}").assertIsEnabled().performClick()
            assertEquals("$kind", 1, taps[kind])
        }
    }

    @Test
    fun aLoadingButtonKeepsItsWidth_isAnnouncedAsBusy_andIsNotTappable() {
        var progress by androidx.compose.runtime.mutableStateOf(ButtonProgress.IDLE)
        var taps = 0
        rule.show { CroftButton("Archive conversation", { taps++ }, Modifier.testTag("b"), progress = progress) }
        val idleWidth = widthOf("b")
        rule.ui { progress = ButtonProgress.LOADING }
        rule.waitForIdle()
        assertEquals("width does not change while loading", idleWidth, widthOf("b"))
        assertEquals("Busy", rule.onNodeWithTag("b").fetchSemanticsNode().config[SemanticsProperties.StateDescription])
        rule.onNodeWithTag("b").performClick()
        assertEquals("not tappable while loading", 0, taps)
        rule.ui { progress = ButtonProgress.SUCCESS }
        rule.waitForIdle()
        assertEquals("width does not change for the check either", idleWidth, widthOf("b"))
        rule.onNodeWithTag("b").performClick()
        assertEquals(0, taps)
    }

    @Test
    fun aLoadingButtonOffersCancelAfterTenSeconds() {
        var cancelled = 0
        rule.mainClock.autoAdvance = false
        rule.show { CroftButton("Sync", {}, Modifier.testTag("b"), progress = ButtonProgress.LOADING, onCancel = { cancelled++ }) }
        rule.mainClock.advanceTimeBy(9_900)
        rule.onAllNodesWithText("Cancel").assertCountEquals(0)
        rule.mainClock.advanceTimeBy(300)
        rule.onNodeWithText("Cancel").assertExists()
        rule.onNodeWithTag("b").performClick()
        assertEquals(1, cancelled)
    }

    @Test
    fun theControllerShowsLoading_thenTheCheckFor600ms_thenTheLabel() = kotlinx.coroutines.test.runTest {
        val c = ButtonProgressController()
        val seen = mutableListOf(c.progress)
        val job = launch {
            c.run { delay(1_000); true }
        }
        runCurrent(); seen += c.progress                 // loading
        advanceTimeBy(1_001); runCurrent(); seen += c.progress // the check
        advanceTimeBy(598); runCurrent(); seen += c.progress   // 1599: still the check
        advanceTimeBy(2); runCurrent(); seen += c.progress     // 1601: the label again
        job.join()
        assertEquals(
            listOf(ButtonProgress.IDLE, ButtonProgress.LOADING, ButtonProgress.SUCCESS, ButtonProgress.SUCCESS, ButtonProgress.IDLE),
            seen,
        )
    }

    @Test
    fun aFailedRunGoesBackToTheLabelWithoutTheCheck() = kotlinx.coroutines.test.runTest {
        val c = ButtonProgressController()
        c.run { false }
        assertEquals(ButtonProgress.IDLE, c.progress)
    }

    @Test
    fun aDisabledButtonStaysInTheTreeAsDisabled_andAnErrorShowsWordsNotJustColour() {
        rule.show {
            CroftButton("Archive", {}, Modifier.testTag("off"), enabled = false)
            CroftButton("Retry", {}, Modifier.testTag("err"), kind = ButtonKind.OUTLINED, errorText = "Could not reach the server")
        }
        rule.onNodeWithTag("off").assertExists().assertIsNotEnabled()
        rule.onNodeWithText("Could not reach the server").assertIsDisplayed()
    }

    @Test
    fun aDisabledButtonWithAReasonExplainsOnTap_andOnLongPress() {
        val reasons = mutableListOf<String>()
        val haptics = RecordingHaptics()
        rule.show(haptics = haptics) {
            CroftButton("Move", {}, Modifier.testTag("b"), enabled = false, disabledReason = "Pick a folder first", onExplain = { reasons += it })
        }
        rule.onNodeWithTag("b").assertIsNotEnabled().performClick()
        assertEquals(listOf("Pick a folder first"), reasons)
        assertEquals(listOf(HapticEvent.Rejected), haptics.events)
        rule.onNodeWithTag("b").performTouchInput { longClick() }
        assertEquals(listOf("Pick a folder first", "Pick a folder first"), reasons)
        assertEquals("the reason is also the state description for screen readers", "Pick a folder first",
            rule.onNodeWithTag("b").fetchSemanticsNode().config[SemanticsProperties.StateDescription])
    }

    // ---- Send ----

    private class Calls {
        var send = 0; var askEmpty = 0; var encryption = 0; var schedule = 0
        val explained = mutableListOf<String>()
        val actions = SendActions({ send++ }, { askEmpty++ }, { encryption++ }, { schedule++ }, { explained += it })
    }

    private val calls = SendCondition.entries.associateWith { Calls() }
    private val haptics = RecordingHaptics()

    /** Every condition at once (a test may set the content only once). */
    private fun showAll(limitMb: Int = DEFAULT_SIZE_LIMIT_MB) {
        rule.show(haptics = haptics) {
            androidx.compose.foundation.layout.Column {
                for (c in SendCondition.entries) SendButton(c, calls.getValue(c).actions, Modifier.testTag("send-${c.name}"), sizeLimitMb = limitMb)
            }
        }
    }

    private fun node(c: SendCondition) = rule.onNodeWithTag("send-${c.name}")

    @Test
    fun sendEnablementFollowsTheTable() {
        val expected = mapOf(
            SendCondition.NO_RECIPIENT to false, SendCondition.INVALID_RECIPIENT to false, SendCondition.EMPTY_CONFIRM to true,
            SendCondition.READY to true, SendCondition.OVER_SIZE_LIMIT to false, SendCondition.UPLOADING to true,
            SendCondition.OFFLINE_OR_SIGN_IN to true, SendCondition.SENDING to false, SendCondition.SCHEDULED to true,
            SendCondition.ENCRYPTION_NO_KEY to true,
        )
        assertEquals("every condition of the table is covered", SendCondition.entries.toSet(), expected.keys)
        for ((c, enabled) in expected) assertEquals("$c", enabled, c.enabled)
    }

    @Test
    fun aDisabledSendIsAnnouncedAsDisabled_withItsReason_andATapShowsTheReasonOnce() {
        showAll()
        val cases = mapOf(
            SendCondition.NO_RECIPIENT to "Add a recipient",
            SendCondition.INVALID_RECIPIENT to "Fix the address",
            SendCondition.OVER_SIZE_LIMIT to "Attachments are over the 25 MB limit",
        )
        for ((c, reason) in cases) {
            node(c).assertExists().assertIsNotEnabled()
            assertEquals(c.name, reason, node(c).fetchSemanticsNode().config[SemanticsProperties.StateDescription])
            haptics.clear()
            node(c).performClick()
            assertEquals(c.name, listOf(reason), calls.getValue(c).explained)
            assertEquals(c.name, 0, calls.getValue(c).send)
            assertEquals("a refused action buzzes", listOf(HapticEvent.Rejected), haptics.events)
        }
    }

    @Test
    fun theSizeLimitInTheReasonIsTheServers() {
        showAll(limitMb = 10)
        node(SendCondition.OVER_SIZE_LIMIT).performClick()
        assertEquals(listOf("Attachments are over the 10 MB limit"), calls.getValue(SendCondition.OVER_SIZE_LIMIT).explained)
    }

    @Test
    fun tapsDoWhatTheTableSays() {
        showAll()
        val cases = mapOf<SendCondition, (Calls) -> Int>(
            SendCondition.EMPTY_CONFIRM to { c -> c.askEmpty },
            SendCondition.READY to { c -> c.send },
            SendCondition.UPLOADING to { c -> c.send },
            SendCondition.OFFLINE_OR_SIGN_IN to { c -> c.send },
            SendCondition.SCHEDULED to { c -> c.send },
            SendCondition.ENCRYPTION_NO_KEY to { c -> c.encryption },
        )
        for ((c, counter) in cases) {
            node(c).assertIsEnabled().performClick()
            assertEquals(c.name, 1, counter(calls.getValue(c)))
            assertEquals(c.name, 0, calls.getValue(c).explained.size)
        }
    }

    @Test
    fun aSendingButtonIsBusyAndNotTappable() {
        showAll()
        node(SendCondition.SENDING).performClick()
        assertEquals(0, calls.getValue(SendCondition.SENDING).send)
        assertTrue(node(SendCondition.SENDING).fetchSemanticsNode().config[SemanticsProperties.StateDescription].contains("Busy"))
    }

    @Test
    fun scheduledSendSaysSchedule_andTheOthersSaySend() {
        showAll()
        node(SendCondition.SCHEDULED).assertTextContains("Schedule")
        node(SendCondition.READY).assertTextContains("Send")
    }

    @Test
    fun longPressOnAnEnabledSendOpensScheduleSend_withTheLongPressHaptic() {
        showAll()
        node(SendCondition.READY).performTouchInput { longClick() }
        assertEquals(1, calls.getValue(SendCondition.READY).schedule)
        assertEquals(0, calls.getValue(SendCondition.READY).send)
        assertEquals(listOf(HapticEvent.SelectionStart), haptics.events)
    }

    @Test
    fun longPressOnADisabledSendExplainsInsteadOfScheduling() {
        showAll()
        node(SendCondition.NO_RECIPIENT).performTouchInput { longClick() }
        assertEquals(0, calls.getValue(SendCondition.NO_RECIPIENT).schedule)
        assertEquals(listOf("Add a recipient"), calls.getValue(SendCondition.NO_RECIPIENT).explained)
    }

    @Test
    fun theStatesThatNeedAnExplanationCarryOne() {
        showAll()
        for (c in listOf(SendCondition.UPLOADING, SendCondition.OFFLINE_OR_SIGN_IN, SendCondition.ENCRYPTION_NO_KEY)) {
            assertNotNull(c.name, node(c).fetchSemanticsNode().config.getOrNull(SemanticsProperties.StateDescription))
        }
    }
}
