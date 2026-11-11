// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.ui.swipe

import androidx.activity.ComponentActivity
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.longClick
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performTouchInput
import com.example.croft.haptics.HapticEvent
import com.example.croft.mail.FakeData
import com.example.croft.theme.AnimationLevel
import com.example.croft.ui.mail.InboxFixture
import com.example.croft.ui.mail.InboxTags
import com.example.croft.ui.mail.showInbox
import com.example.croft.ui.mail.waitForRow
import kotlinx.coroutines.runBlocking
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

/**
 * Long-press menus of the table in docs/ux/controls.md that exist so far: the message row's long-press is selection mode (S11,
 * tested with the Inbox), and a mailbox in the drawer (S12) opens its menu: Mark all as read (with Undo), Sync now, Mailbox settings.
 */
@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [35], qualifiers = "w360dp-h800dp-mdpi")
class SwipeMenuTest {
    @get:Rule
    val rule = createAndroidComposeRule<ComponentActivity>()

    private val inboxFolder = FakeData.folderId(1, FakeData.INBOX)

    private fun textExists(text: String) = rule.onAllNodes(hasText(text)).fetchSemanticsNodes().isNotEmpty()
    private fun waitForText(text: String) = rule.waitUntil(10_000) { textExists(text) }

    private fun openDrawer(): InboxFixture {
        val f = InboxFixture(80)
        rule.showInbox(f, AnimationLevel.OFF, startDrawerOpen = true)
        rule.waitForRow(f.topId)
        rule.waitForIdle()
        return f
    }

    @Test
    fun s12_aLongPressOnAMailboxOpensItsMenuWithTheLongPressHaptic_andDoesNotOpenTheMailbox() {
        val f = openDrawer()
        rule.onNodeWithTag(InboxTags.drawerItem(inboxFolder)).performTouchInput { longClick() }
        rule.waitForIdle()
        for (item in listOf("Mark all as read", "Sync now", "Mailbox settings")) rule.onNodeWithText(item).assertExists()
        assertEquals(listOf(HapticEvent.SelectionStart), f.haptics.events)
        // The release after the long-press is not a tap: the unified Inbox is still what is shown.
        assertTrue(f.repo.requests.all { it.scope == com.example.croft.mail.MessageScope.UnifiedInbox })
    }

    @Test
    fun s12_aShortTapStillOpensTheMailbox_andPlaysNoLongPressHaptic() {
        val f = openDrawer()
        rule.onNodeWithTag(InboxTags.drawerItem(FakeData.folderId(1, FakeData.SENT))).performClick()
        rule.waitUntil(10_000) { f.repo.requests.any { it.scope == com.example.croft.mail.MessageScope.Folder(1, FakeData.folderId(1, FakeData.SENT)) } }
        assertFalse(HapticEvent.SelectionStart in f.haptics.events)
    }

    @Test
    fun s12_markAllAsReadReadsEveryUnreadMessageOfTheMailbox_withAnUndoThatPutsThemBack() {
        val f = openDrawer()
        val unread = f.repo.snapshot(com.example.croft.mail.MessageScope.UnifiedInbox).count { !it.seen }
        assertTrue("the fixture has unread mail", unread > 0)
        rule.onNodeWithTag(InboxTags.drawerItem(inboxFolder)).performTouchInput { longClick() }
        rule.waitForIdle()
        rule.onNodeWithText("Mark all as read").performClick()
        waitForText("$unread conversations marked as read")
        assertEquals(0, f.repo.snapshot(com.example.croft.mail.MessageScope.UnifiedInbox).count { !it.seen })
        rule.onNodeWithText("Undo").performClick()
        rule.waitUntil(10_000) { f.repo.snapshot(com.example.croft.mail.MessageScope.UnifiedInbox).count { !it.seen } == unread }
    }

    @Test
    fun s12_markAllAsReadIsHiddenWhenNothingIsUnread() {
        val f = InboxFixture(80)
        runBlocking { f.repo.markSeen(f.repo.snapshot(com.example.croft.mail.MessageScope.UnifiedInbox).map { it.id }, true) }
        rule.showInbox(f, AnimationLevel.OFF, startDrawerOpen = true)
        rule.waitForRow(f.topId)
        rule.waitForIdle()
        rule.onNodeWithTag(InboxTags.drawerItem(inboxFolder)).performTouchInput { longClick() }
        rule.waitForIdle()
        rule.onNodeWithText("Sync now").assertExists()
        assertFalse("an item that cannot apply is hidden", textExists("Mark all as read"))
    }

    @Test
    fun s12_syncNowAsksTheMailboxsAccountToSync() {
        val f = openDrawer()
        rule.onNodeWithTag(InboxTags.drawerItem(inboxFolder)).performTouchInput { longClick() }
        rule.waitForIdle()
        rule.onNodeWithText("Sync now").performClick()
        waitForText("Syncing now.")
    }
}
