// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.ui.compose

import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.test.SemanticsMatcher
import androidx.compose.ui.test.hasAnyDescendant
import androidx.compose.ui.test.hasTestTag
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onAllNodesWithTag
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import com.example.croft.CroftRobolectricRunner
import com.example.croft.mail.compose.ComposeStart
import com.example.croft.mail.compose.OutboxController
import com.example.croft.theme.AnimationLevel
import com.example.croft.ui.ScreenshotKit
import com.example.croft.ui.components.show
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.annotation.Config
import uniffi.croft_ffi.OutboxItemState
import uniffi.croft_ffi.OutboxReason
import uniffi.croft_ffi.outboxReasonKey

/** S26 (Robolectric, in-memory backend): the three states with their reasons, and Retry, Edit and Discard with their questions. */
@RunWith(CroftRobolectricRunner::class)
@Config(sdk = [35])
class OutboxScreenTest {
    @get:Rule
    val rule = createComposeRule()

    private fun open(f: ComposeFixture, onEdit: (ComposeStart.Draft) -> Unit = {}) {
        // A tall window: a lazy list only composes the rows that fit, and the tests count the buttons of all four.
        ScreenshotKit.configure(360, 1600, false, 1.0f)
        rule.show(AnimationLevel.OFF) { OutboxHost(f.session, onBack = {}, onEdit = onEdit) }
    }

    private fun fixture(): ComposeFixture {
        val f = ComposeFixture()
        f.backend.addOutboxItem(1, outboxItem(1u, OutboxItemState.QUEUED, OutboxReason.TRYING_AGAIN, subject = "Meeting notes"))
        f.backend.addOutboxItem(1, outboxItem(2u, OutboxItemState.SENDING, subject = "Photos"))
        f.backend.addOutboxItem(1, outboxItem(3u, OutboxItemState.FAILED, OutboxReason.RECIPIENT_REFUSED, subject = "Invoice reply"))
        f.backend.addOutboxItem(1, outboxItem(4u, OutboxItemState.FAILED, OutboxReason.MAY_NOT_HAVE_BEEN_SENT, maybe = true, subject = "Maybe gone"))
        return f
    }

    @Test
    fun everyReasonOfTheCoreHasAStringOfTheApp() {
        for (reason in OutboxReason.entries) {
            assertNotNull("no string for ${outboxReasonKey(reason)}", reasonStringId(outboxReasonKey(reason)))
        }
        assertNull(reasonStringId("outbox.reason.from_a_newer_core"))
    }

    @Test
    fun queuedSendingAndFailedRowsShowTheirStateAndReason() {
        val f = fixture()
        open(f)
        rule.waitUntil(10_000) { rule.onAllNodes(hasText("Meeting notes")).fetchSemanticsNodes().isNotEmpty() }
        rule.onNodeWithText("Could not reach the server. Trying again.").assertExists()
        rule.onNodeWithText("Sending now").assertExists()
        rule.onNodeWithText("The server refused a recipient address.").assertExists()
        rule.onNodeWithText("May not have been sent. Check Sent before trying again.").assertExists()
        // A Failed row announces itself; a Sending row has neither Retry, Edit nor Discard.
        assertEquals("both Failed rows announce themselves", 2, rule.onAllNodes(SemanticsMatcher.expectValue(SemanticsProperties.LiveRegion, LiveRegionMode.Polite) and hasText("Failed")).fetchSemanticsNodes().size)
        assertEquals("Retry only on the two Failed rows", 2, rule.onAllNodesWithTag(OutboxTags.RETRY).fetchSemanticsNodes().size)
        assertEquals("Edit on Queued and Failed, not Sending", 3, rule.onAllNodesWithTag(OutboxTags.EDIT).fetchSemanticsNodes().size)
    }

    @Test
    fun retryOnAMessageThatMayHaveBeenSentAsksFirst_andOnlyRetryAnywayCallsTheCore() {
        val f = fixture()
        open(f)
        rule.waitUntil(10_000) { rule.onAllNodes(hasText("Maybe gone")).fetchSemanticsNodes().isNotEmpty() }
        // The last Retry button belongs to the "may have been sent" row (rows are ordered by time).
        rule.onAllNodesWithTag(OutboxTags.RETRY)[1].performClick()
        rule.waitForText("This message may already have been sent")
        assertFalse(f.backend.calls.any { it.startsWith("retry") })

        rule.onNodeWithText("Cancel").performClick()
        rule.waitUntil(10_000) { rule.onAllNodes(hasText("This message may already have been sent")).fetchSemanticsNodes().isEmpty() }
        assertFalse(f.backend.calls.any { it.startsWith("retry") })

        rule.onAllNodesWithTag(OutboxTags.RETRY)[1].performClick()
        rule.waitForText("This message may already have been sent")
        rule.onNodeWithTag("outbox-retry-anyway").performClick()
        rule.waitUntil(10_000) { f.backend.calls.contains("retryConfirmed") }
        assertEquals(listOf("retryConfirmed"), f.backend.calls.filter { it.startsWith("retry") })
    }

    @Test
    fun retryOnAnOrdinaryFailedMessageGoesAtOnce() {
        val f = fixture()
        open(f)
        rule.waitUntil(10_000) { rule.onAllNodes(hasText("Invoice reply")).fetchSemanticsNodes().isNotEmpty() }
        rule.onAllNodesWithTag(OutboxTags.RETRY)[0].performClick()
        rule.waitUntil(10_000) { f.backend.calls.contains("retry") }
        assertFalse(f.backend.calls.contains("retryConfirmed"))
    }

    @Test
    fun discardAsksFirst() {
        val f = fixture()
        open(f)
        rule.waitUntil(10_000) { rule.onAllNodes(hasText("Invoice reply")).fetchSemanticsNodes().isNotEmpty() }
        rule.onAllNodesWithTag(OutboxTags.DISCARD)[0].performClick()
        rule.waitForText("Discard this message?")
        assertFalse(f.backend.calls.contains("discardOutbox"))
        rule.onNodeWithText("Cancel").performClick()
        assertFalse(f.backend.calls.contains("discardOutbox"))
        rule.onAllNodesWithTag(OutboxTags.DISCARD)[0].performClick()
        rule.onNodeWithTag("outbox-discard-confirm").performClick()
        rule.waitUntil(10_000) { f.backend.calls.contains("discardOutbox") }
    }

    @Test
    fun editGoesBackToTheDraft() {
        val f = fixture()
        var edited: ComposeStart.Draft? = null
        open(f) { edited = it }
        rule.waitUntil(10_000) { rule.onAllNodes(hasText("Invoice reply")).fetchSemanticsNodes().isNotEmpty() }
        rule.onAllNodesWithTag(OutboxTags.EDIT)[1].performClick()
        rule.waitUntil(10_000) { edited != null }
        assertEquals(ComposeStart.Draft(1, 3), edited)
    }

    @Test
    fun anEmptyOutboxSaysSo() {
        val f = ComposeFixture()
        open(f)
        rule.waitForText("Nothing is waiting to be sent")
    }
}
