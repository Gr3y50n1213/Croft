// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.ui.components

import androidx.compose.foundation.layout.Column
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.SemanticsActions
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.test.assertIsNotSelected
import androidx.compose.ui.test.assertIsSelected
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.longClick
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.hasAnyAncestor
import androidx.compose.ui.test.hasTestTag
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performTouchInput
import androidx.compose.ui.test.down
import androidx.compose.ui.test.moveBy
import androidx.compose.ui.test.up
import androidx.compose.ui.test.center
import androidx.compose.ui.unit.dp
import androidx.compose.ui.platform.LocalViewConfiguration
import com.example.croft.haptics.HapticEvent
import com.example.croft.haptics.RecordingHaptics
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config

/** The message row and the attachment chip: docs/ux/controls.md and accessibility.md. */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35])
class RowAndAttachmentTest {
    @get:Rule
    val rule = createComposeRule()

    private val haptics = RecordingHaptics()
    /** The interactive node of the row (the card), not the layout box that carries the tag. */
    private fun row() = rule.onNode(isInteractive and hasAnyAncestor(hasTestTag("row")))

    private fun showRow(state: MessageRowState, onClick: () -> Unit = {}, onLongClick: (() -> Unit)? = {}, onStar: (() -> Unit)? = {}, onAvatar: (() -> Unit)? = {}) {
        rule.show(haptics = haptics) {
            MessageRow("Sam Rivera", "Lunch on Friday?", "Are you free around noon?", "9:12", state, onClick, Modifier.testTag("row"), onLongClick = onLongClick, onAvatarClick = onAvatar, onToggleStar = onStar)
        }
    }

    private fun description() = row().fetchSemanticsNode().config[SemanticsProperties.ContentDescription].single()

    @Test
    fun screenReadersHearTheStateFirst_thenTheContent() {
        showRow(MessageRowState(unread = true, starred = true, hasAttachment = true))
        assertEquals("Unread. From Sam Rivera. Lunch on Friday?. 9:12. Has attachment. Starred. Are you free around noon?", description())
    }

    @Test
    fun aReadRowSaysRead_andEncryptionAndPendingAndFailedAreInWords() {
        showRow(MessageRowState(encryption = EncryptionMark.SIGNED_INVALID, pendingOffline = true, failed = true))
        val d = description()
        assertTrue(d, d.startsWith("Read. From Sam Rivera."))
        assertTrue(d, "Signature is not valid" in d && "Waiting for the network" in d && "The server refused the change" in d)
        assertEquals("Failed. Retry is available", row().fetchSemanticsNode().config[SemanticsProperties.StateDescription])
    }

    @Test
    fun theRowOffersSelectAndStarAsAccessibilityActions_soNoOneNeedsASwipeOrALongPress() {
        var selected = 0; var starred = 0
        showRow(MessageRowState(), onLongClick = { selected++ }, onStar = { starred++ }, onAvatar = { selected++ })
        val actions = row().fetchSemanticsNode().config[SemanticsActions.CustomActions]
        assertEquals(listOf("Select", "Star"), actions.map { it.label })
        actions[0].action(); actions[1].action()
        assertEquals(1, selected)
        assertEquals(1, starred)
    }

    @Test
    fun theActionLabelsFollowTheState() {
        showRow(MessageRowState(selected = true, starred = true))
        assertEquals(listOf("Deselect", "Remove star"), row().fetchSemanticsNode().config[SemanticsActions.CustomActions].map { it.label })
        row().assertIsSelected()
    }

    @Test
    fun aTapOpensTheRow_andANotSelectedRowIsNotSelected() {
        var opened = 0
        showRow(MessageRowState(), onClick = { opened++ })
        row().assertIsNotSelected().performClick()
        assertEquals(1, opened)
        assertTrue(haptics.events.isEmpty())
    }

    @Test
    fun aLongPressRunsAtTheLongPressTime_beforeTheFingerLeaves_withTheHaptic() {
        var longPresses = 0
        var taps = 0
        rule.mainClock.autoAdvance = false
        showRow(MessageRowState(), onClick = { taps++ }, onLongClick = { longPresses++ })
        rule.runOnUiThread { }
        row().performTouchInput { down(center) }
        rule.mainClock.advanceTimeBy(200)
        assertEquals("not yet: the system's long-press time has not passed", 0, longPresses)
        rule.mainClock.advanceTimeBy(500)
        assertEquals("the result opens while the finger is still down", 1, longPresses)
        assertEquals(listOf(HapticEvent.SelectionStart), haptics.events)
        row().performTouchInput { up() }
        rule.mainClock.advanceTimeBy(100)
        assertEquals("a long-press is not also a tap", 0, taps)
    }

    @Test
    fun movingPastTheTouchSlopBeforeTheLongPressTimeCancelsIt() {
        var longPresses = 0
        rule.mainClock.autoAdvance = false
        showRow(MessageRowState(), onLongClick = { longPresses++ })
        row().performTouchInput { down(center); moveBy(Offset(0f, 80f)) }
        rule.mainClock.advanceTimeBy(1_000)
        row().performTouchInput { up() }
        assertEquals(0, longPresses)
        assertTrue(haptics.events.isEmpty())
    }

    @Test
    fun theLongPressLifts_andALiftReleasesWhenTheFingerLeaves() {
        rule.mainClock.autoAdvance = false
        showRow(MessageRowState())
        row().performTouchInput { down(center) }
        rule.mainClock.advanceTimeBy(1_000)
        assertTrue("lifted while held", rule.probe("lift") > 0.9f)
        row().performTouchInput { up() }
        rule.mainClock.advanceTimeBy(1_000)
        assertEquals("settled back", 0f, rule.probe("lift"), 0.01f)
    }

    // ---- Attachment chip ----

    @Test
    fun everyAttachmentStateHasItsWordsAndItsTapMeaning() {
        val taps = AttachmentState.entries.associateWith { 0 }.toMutableMap()
        var longPresses = 0
        rule.show(haptics = haptics) {
            Column {
                for (s in AttachmentState.entries) {
                    AttachmentChip("report.pdf", "2.4 MB", s, { taps[s] = taps.getValue(s) + 1 }, Modifier.testTag("att-${s.name}"), progress = 0.5f, onLongClick = { longPresses++ })
                }
            }
        }
        val words = mapOf(
            AttachmentState.NOT_DOWNLOADED to "Not downloaded", AttachmentState.DOWNLOADING to "Downloading",
            AttachmentState.DOWNLOADED to "Downloaded", AttachmentState.FAILED to "Download failed",
            AttachmentState.BLOCKED to "Blocked file type", AttachmentState.OVER_LIMIT to "Over the size limit",
        )
        assertEquals(AttachmentState.entries.toSet(), words.keys)
        for ((s, w) in words) {
            val node = rule.onNodeWithTag("att-${s.name}")
            assertEquals(w, node.fetchSemanticsNode().config[SemanticsProperties.StateDescription])
            node.performClick()
            assertEquals("$s", 1, taps[s])
        }
        rule.onNodeWithTag("att-DOWNLOADED").performTouchInput { longClick() }
        assertEquals(1, longPresses)
        assertEquals(listOf(HapticEvent.SelectionStart), haptics.events)
    }
}
