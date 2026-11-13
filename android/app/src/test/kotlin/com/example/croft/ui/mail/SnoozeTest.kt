// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.ui.mail

import androidx.compose.ui.test.hasAnyAncestor
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
import com.example.croft.ui.components.ROW_NOTE_TAG
import java.time.ZoneId
import kotlinx.coroutines.runBlocking
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config

/** T36: snoozing from the selection toolbar with Undo, the Snoozed folder with its wake times and Unsnooze, and the chip of a woken message. */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35])
class SnoozeTest {
    @get:Rule
    val rule = createComposeRule()

    private fun exists(text: String, substring: Boolean = false) = rule.onAllNodes(hasText(text, substring = substring)).fetchSemanticsNodes().isNotEmpty()

    private fun rowGone(id: Long) = rule.onAllNodes(hasTestTag(InboxTags.row(id))).fetchSemanticsNodes().isEmpty()

    private fun note(id: Long) = rule.onNode(hasTestTag(ROW_NOTE_TAG) and hasAnyAncestor(hasTestTag(InboxTags.row(id))), useUnmergedTree = true)

    private val tomorrow = SnoozePresets.presets(NOW, ZoneId.of("UTC")).first { it.kind == SnoozePresets.Kind.TOMORROW }.atMs

    @Test
    fun theToolbarsSnoozeOpensS29_aPresetHidesTheMessage_andUndoBringsItBack() {
        val f = InboxFixture()
        rule.showInbox(f, level = AnimationLevel.OFF)
        val id = f.topId
        rule.waitForRow(id)
        rule.rowCard(id).performTouchInput { longClick() }
        rule.onNodeWithContentDescription("More actions").performClick()
        rule.onNodeWithText("Snooze").performClick()
        rule.waitUntil(10_000) { rule.onAllNodes(hasTestTag(SnoozeTags.SHEET)).fetchSemanticsNodes().isNotEmpty() }
        for (kind in SnoozePresets.Kind.entries) rule.onNodeWithTag(SnoozeTags.preset(kind)).assertExists()
        rule.onNodeWithTag(SnoozeTags.PICK).assertExists()
        rule.onNodeWithTag(SnoozeTags.preset(SnoozePresets.Kind.TOMORROW)).performClick()
        rule.waitUntil(10_000) { rowGone(id) }
        assertFalse("not in the Inbox", id in f.inboxIds())
        assertEquals(tomorrow, runBlocking { f.repo.nextSnoozeWake() })
        rule.waitUntil(10_000) { exists("Snoozed on this phone until", substring = true) }
        rule.onNodeWithText("Undo").performClick()
        rule.waitForRow(id)
        assertTrue("back in the Inbox", id in f.inboxIds())
        assertNull("nothing is snoozed", runBlocking { f.repo.nextSnoozeWake() })
    }

    @Test
    fun theSnoozedFolderListsItsWakeTimes_andUnsnoozeBringsTheMessageBack() {
        val f = InboxFixture()
        val id = f.topId
        val account = f.repo.snapshot(MessageScope.UnifiedInbox).first().account
        runBlocking { f.repo.snooze(listOf(id), tomorrow) }
        rule.showInbox(f, level = AnimationLevel.OFF, startScope = MessageScope.Folder(account, FakeData.folderId(account, FakeData.SNOOZED)))
        rule.waitForRow(id)
        note(id).assertExists()
        val label = SnoozePresets.label(tomorrow, NOW, ZoneId.of("UTC"), java.util.Locale.getDefault())
        assertTrue("the row says until when", exists("Until $label"))
        rule.rowCard(id).performTouchInput { longClick() }
        rule.onNodeWithContentDescription("Unsnooze").performClick()
        rule.waitUntil(10_000) { rowGone(id) }
        assertTrue("back in the Inbox", id in f.inboxIds())
        assertNull(runBlocking { f.repo.nextSnoozeWake() })
    }

    @Test
    fun aWokenMessageIsUnreadAndShowsTheSnoozedChip() {
        val f = InboxFixture()
        val id = f.topId
        runBlocking { f.repo.markSeen(listOf(id), true) }
        runBlocking { f.repo.snooze(listOf(id), NOW + 60_000) }
        assertFalse(id in f.inboxIds())
        f.clockMs = NOW + 61_000
        assertEquals(listOf(id), runBlocking { f.repo.wakeDueSnoozes() })
        assertTrue("nothing wakes twice", runBlocking { f.repo.wakeDueSnoozes() }.isEmpty())
        rule.showInbox(f, level = AnimationLevel.OFF)
        rule.waitForRow(id)
        val row = f.repo.snapshot(MessageScope.UnifiedInbox).first { it.id == id }
        assertFalse("unread again", row.seen)
        assertTrue(row.backFromSnooze)
        note(id).assertExists()
        assertTrue(exists("Snoozed"))
    }
}
