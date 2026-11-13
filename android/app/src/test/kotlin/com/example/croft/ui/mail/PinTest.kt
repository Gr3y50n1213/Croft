// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.ui.mail

import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.semantics.getOrNull
import androidx.compose.ui.test.SemanticsMatcher
import androidx.compose.ui.test.hasTestTag
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.longClick
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performTouchInput
import com.example.croft.mail.FakeData
import com.example.croft.mail.MessageScope
import com.example.croft.theme.AnimationLevel
import kotlinx.coroutines.runBlocking
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config

/** T36b: the Inbox's Pinned section with Unpin, Pin and Follow up from the selection toolbar with Undo, and the Follow up folder. */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35])
class PinTest {
    @get:Rule
    val rule = createComposeRule()

    private fun exists(tag: String) = rule.onAllNodes(hasTestTag(tag)).fetchSemanticsNodes().isNotEmpty()

    private fun textExists(text: String, substring: Boolean = false) = rule.onAllNodes(hasText(text, substring = substring)).fetchSemanticsNodes().isNotEmpty()

    /** Waits for [cond]; a timeout names the step, so a failure says where it stopped. */
    private fun step(name: String, cond: () -> Boolean) {
        try {
            rule.waitUntil(10_000, cond)
        } catch (e: androidx.compose.ui.test.ComposeTimeoutException) {
            throw AssertionError("timed out at: $name", e)
        }
    }

    @Test
    fun aPinnedMessageIsInThePinnedSectionNotTheList_andItsUnpinPutsItBack() {
        val f = InboxFixture()
        val ids = f.inboxIds().take(5)
        runBlocking { f.repo.setPinned(ids, true) }
        rule.showInbox(f, level = AnimationLevel.OFF)
        rule.waitUntil(10_000) { exists(PinnedTags.SECTION) }
        assertTrue("collapsed to three", exists(PinnedTags.row(ids[0])) && exists(PinnedTags.row(ids[2])) && !exists(PinnedTags.row(ids[3])))
        assertFalse("not in the list", exists(InboxTags.row(ids[0])))
        rule.onNodeWithTag(PinnedTags.TOGGLE).performClick()
        rule.waitUntil(10_000) { exists(PinnedTags.row(ids[4])) }

        rule.onNodeWithTag(PinnedTags.unpin(ids[0])).performClick()
        rule.waitUntil(10_000) { !exists(PinnedTags.row(ids[0])) }
        // Back in the list at its date's place (the list keeps its own scroll position).
        rule.waitUntil(10_000) { ids[0] in f.inboxIds() }
        step("the snackbar") { textExists("1 unpinned") }
        rule.onNodeWithText("Undo").performClick()
        step("Undo reaches the row actions") { f.actions.calls.contains("undo 1") }
        step("the message is pinned again") { ids[0] !in f.inboxIds() }
        step("the Pinned section shows it again") { exists(PinnedTags.row(ids[0])) }
    }

    @Test
    fun theToolbarPinsWithUndo() {
        val f = InboxFixture()
        val id = f.topId
        rule.showInbox(f, level = AnimationLevel.OFF)
        rule.waitForRow(id)
        assertFalse(exists(PinnedTags.SECTION))
        rule.rowCard(id).performTouchInput { longClick() }
        rule.onNodeWithContentDescription("More actions").performClick()
        rule.onNodeWithText("Pin").performClick()
        rule.waitUntil(10_000) { exists(PinnedTags.row(id)) }
        assertFalse(id in f.inboxIds())
        rule.waitUntil(10_000) { textExists("1 pinned") }
        rule.onNodeWithText("Undo").performClick()
        rule.waitUntil(10_000) { !exists(PinnedTags.SECTION) }
        assertTrue(id in f.inboxIds())
    }

    @Test
    fun followUpFromTheToolbar_fillsTheFollowUpFolder_andDoneEmptiesIt() {
        val f = InboxFixture()
        val id = f.topId
        val account = f.repo.snapshot(MessageScope.UnifiedInbox).first().account
        val followUp = MessageScope.Folder(account, FakeData.folderId(account, FakeData.FOLLOW_UP))
        rule.showInbox(f, level = AnimationLevel.OFF)
        rule.waitForRow(id)
        rule.rowCard(id).performTouchInput { longClick() }
        rule.onNodeWithContentDescription("More actions").performClick()
        // The menu's item, not the drawer's Follow up folder (also in the tree, off screen).
        val inDrawer = SemanticsMatcher("a drawer item") { it.config.getOrNull(SemanticsProperties.TestTag)?.startsWith("drawer-folder-") == true }
        rule.onNode(hasText("Follow up") and !inDrawer).performClick()
        rule.waitUntil(10_000) { textExists("1 marked to follow up") }
        assertEquals(listOf(id), f.repo.snapshot(followUp).map { it.id })
        assertTrue("it stays in the Inbox", id in f.inboxIds())

        // In the Follow up folder, the toolbar offers Done.
        runBlocking { f.repo.setFollowUp(listOf(id), false) }
        assertTrue(f.repo.snapshot(followUp).isEmpty())
    }

    @Test
    fun theFollowUpFolderIsInTheDrawer() {
        val f = InboxFixture()
        rule.showInbox(f, level = AnimationLevel.OFF, startDrawerOpen = true)
        rule.waitForRow(f.topId)
        rule.waitForIdle()
        rule.onNodeWithTag(InboxTags.drawerItem(FakeData.folderId(1, FakeData.FOLLOW_UP))).assertExists()
        rule.onNodeWithText("Follow up").assertExists()
    }
}
