// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.ui.reading

import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.performClick
import com.example.croft.mail.MailSettings
import com.example.croft.mail.reading.MarkReadDelay
import com.example.croft.ui.components.show
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config

/**
 * Mark read after the S49 delay (immediately, 2 s, never), counted from when the body is on the screen. Loading never marks it. The clock
 * is the test's own (`mainClock`), so the times are exact.
 */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35], qualifiers = "w360dp-h800dp-mdpi")
class MarkReadTimingTest {
    @get:Rule
    val rule = createComposeRule()

    private fun marks(f: ReadingFixture) = f.actions.calls.filter { it.startsWith("markSeen") }

    private fun start(f: ReadingFixture, delay: MarkReadDelay) {
        rule.mainClock.autoAdvance = false
        f.show(rule, settings = { MailSettings(markRead = delay) })
        // The body loads in the first frames; nothing waits for a network.
        rule.mainClock.advanceTimeBy(200)
        rule.waitForIdle()
        rule.onNodeWithTag(BodyTags.body(NEWEST)).assertExists()
    }

    @Test
    fun after2Seconds_notBefore() {
        val f = ReadingFixture()
        start(f, MarkReadDelay.AFTER_2_SECONDS)
        assertTrue("loading the body did not mark it", marks(f).isEmpty())
        rule.mainClock.advanceTimeBy(1_500)
        assertTrue("not at 1.7 s", marks(f).isEmpty())
        rule.mainClock.advanceTimeBy(500)
        assertEquals("at 2.2 s", listOf("markSeen $NEWEST true"), marks(f))
        // Only once, and only the unread message (the older ones were read already).
        rule.mainClock.advanceTimeBy(10_000)
        assertEquals(listOf("markSeen $NEWEST true"), marks(f))
        assertTrue("the list's own row is read", f.repo.snapshot(com.example.croft.mail.MessageScope.UnifiedInbox).first { it.id == NEWEST }.seen)
    }

    @Test
    fun immediately_rightAfterTheBodyIsShown() {
        val f = ReadingFixture()
        start(f, MarkReadDelay.IMMEDIATELY)
        assertEquals(listOf("markSeen $NEWEST true"), marks(f))
    }

    @Test
    fun never_neverMarks() {
        val f = ReadingFixture()
        start(f, MarkReadDelay.NEVER)
        rule.mainClock.advanceTimeBy(60_000)
        assertTrue(marks(f).isEmpty())
        assertEquals(false, f.repo.snapshot(com.example.croft.mail.MessageScope.UnifiedInbox).first { it.id == NEWEST }.seen)
    }

    @Test
    fun aMessageLeftBeforeTheDelayIsNotMarked() {
        val f = ReadingFixture()
        var visible by mutableStateOf(true)
        rule.mainClock.autoAdvance = false
        rule.show { if (visible) f.Screen(settings = MailSettings(markRead = MarkReadDelay.AFTER_2_SECONDS)) }
        rule.mainClock.advanceTimeBy(1_000)
        rule.runOnUiThread { visible = false }
        rule.mainClock.advanceTimeBy(5_000)
        assertTrue(marks(f).isEmpty())
    }

    @Test
    fun markAsUnreadWhileWaitingCancelsTheMarkAndKeepsItUnread() {
        val f = ReadingFixture()
        start(f, MarkReadDelay.AFTER_2_SECONDS)
        rule.mainClock.advanceTimeBy(500)
        rule.onNodeWithTag(ReadingTags.MARK_UNREAD).performClick()
        rule.mainClock.advanceTimeBy(5_000)
        assertEquals(listOf("markSeen $NEWEST false"), marks(f))
    }

    @Test
    fun anOlderMessageThatIsUnreadIsMarkedWhenItIsExpanded() {
        val f = ReadingFixture(olderOneSeen = false)
        start(f, MarkReadDelay.AFTER_2_SECONDS)
        rule.mainClock.advanceTimeBy(2_500)
        assertEquals("the newest, after 2 s", listOf("markSeen $NEWEST true"), marks(f))
        rule.onNodeWithTag(ReadingTags.message(OLD_ONE)).performClick()
        rule.mainClock.advanceTimeBy(500)
        assertEquals(1, marks(f).size)
        rule.mainClock.advanceTimeBy(2_000)
        assertEquals(listOf("markSeen $NEWEST true", "markSeen $OLD_ONE true"), marks(f))
    }
}
