// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.ui.reading

import androidx.compose.ui.semantics.SemanticsActions
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.semantics.getOrNull
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithContentDescription
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config

/** Accessibility of S20 and S21: the blocked banner announces itself, chips are labelled, collapsed messages can be expanded with TalkBack's actions. */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35], qualifiers = "w360dp-h800dp-mdpi")
class ReadingSemanticsTest {
    @get:Rule
    val rule = createComposeRule()

    @Test
    fun theBlockedBannerIsAPoliteLiveRegionWithItsTextAndNamedButtons() {
        val f = ReadingFixture()
        f.show(rule)
        rule.waitForTag(BodyTags.remoteBar(NEWEST))
        val bar = rule.onNodeWithTag(BodyTags.remoteBar(NEWEST)).fetchSemanticsNode()
        assertEquals(LiveRegionMode.Polite, bar.config.getOrNull(SemanticsProperties.LiveRegion))
        assertTrue(rule.onAllNodes(hasText("Remote images blocked: 2 pictures")).fetchSemanticsNodes().isNotEmpty())
        rule.onNodeWithContentDescription("Dismiss").assertExists()
    }

    @Test
    fun chipsAreLabelledWithTheirNameAndTheirState() {
        val body = htmlBody(NEWEST, attachments = listOf(PDF, NOTES, PROGRAM))
        val f = ReadingFixture(newestBody = body)
        f.show(rule)
        rule.waitForTag(BodyTags.chip(NEWEST, PDF.id))
        for ((attachment, state) in listOf(PDF to "Not downloaded", NOTES to "Not downloaded", PROGRAM to "Blocked file type")) {
            val node = rule.onNodeWithTag(BodyTags.chip(NEWEST, attachment.id)).fetchSemanticsNode()
            val description = node.config.getOrNull(SemanticsProperties.ContentDescription)!!.joinToString()
            assertTrue(description, description.startsWith(attachment.fileName) && description.contains(state))
            assertEquals(state, node.config.getOrNull(SemanticsProperties.StateDescription))
            assertNotNull("a chip can be activated", node.config.getOrNull(SemanticsActions.OnClick))
        }
    }

    @Test
    fun aCollapsedMessageSaysItIsCollapsedAndCanBeExpandedWithAnAction() {
        val f = ReadingFixture()
        f.show(rule)
        rule.waitForTag(BodyTags.body(NEWEST))
        val node = rule.onNodeWithTag(ReadingTags.message(OLD_TWO)).fetchSemanticsNode()
        // The row is one element: sender, date and snippet are read together, with its state.
        assertEquals("Collapsed", node.config.getOrNull(SemanticsProperties.StateDescription))
        val texts = node.config.getOrNull(SemanticsProperties.Text).orEmpty().joinToString { it.text }
        assertTrue(texts, texts.contains("Mira Okafor") && texts.contains("Snippet of message $OLD_TWO"))
        val actions = node.config.getOrNull(SemanticsActions.CustomActions).orEmpty()
        assertEquals(listOf("Expand message"), actions.map { it.label })
        assertEquals("Expand message", node.config.getOrNull(SemanticsActions.OnClick)?.label)
    }

    @Test
    fun theCustomActionExpandsTheMessage() {
        val f = ReadingFixture()
        f.show(rule)
        rule.waitForTag(BodyTags.body(NEWEST))
        val action = rule.onNodeWithTag(ReadingTags.message(OLD_ONE)).fetchSemanticsNode().config.getOrNull(SemanticsActions.CustomActions)!!.single()
        rule.runOnUiThread { assertTrue(action.action()) }
        rule.waitForTag(BodyTags.body(OLD_ONE))
        // Expanded, it says so, and a tap on its header collapses it again.
        val header = rule.onNodeWithTag(ReadingTags.header(OLD_ONE)).fetchSemanticsNode()
        assertEquals("Expanded", header.config.getOrNull(SemanticsProperties.StateDescription))
        assertEquals("Collapse message", header.config.getOrNull(SemanticsActions.OnClick)?.label)
    }

    @Test
    fun theIconButtonsOfTheActionBarAreAllNamed() {
        val f = ReadingFixture()
        f.show(rule)
        rule.waitForTag(BodyTags.body(NEWEST))
        for (label in listOf("Back", "Archive", "Delete", "Mark as unread", "More options", "Reply", "Reply all", "Forward", "Reply to Mira Okafor", "Star")) {
            assertTrue("$label is named", rule.onAllNodes(androidx.compose.ui.test.hasContentDescription(label)).fetchSemanticsNodes().isNotEmpty() || rule.onAllNodes(hasText(label)).fetchSemanticsNodes().isNotEmpty())
        }
    }

    @Test
    fun theSubjectIsAHeading() {
        val f = ReadingFixture()
        f.show(rule)
        rule.waitForTag(ReadingTags.SUBJECT)
        assertTrue(rule.onNodeWithTag(ReadingTags.SUBJECT).fetchSemanticsNode().config.contains(SemanticsProperties.Heading))
    }
}
