// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.ui.reading

import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.assertTextContains
import androidx.compose.ui.test.hasTestTag
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollTo
import com.example.croft.mail.FakeData
import com.example.croft.mail.MailSession
import com.example.croft.mail.compose.ComposeStart
import com.example.croft.mail.reading.MailAddressData
import com.example.croft.mail.reading.ReadProblem
import com.example.croft.ui.mail.MoveTags
import kotlinx.coroutines.runBlocking
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config

/** S20 Conversation: what it shows, the action bar, the overflow menu, the reply bar, the sample marker, offline and errors. */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35], qualifiers = "w360dp-h800dp-mdpi")
class ConversationScreenTest {
    @get:Rule
    val rule = createComposeRule()

    private fun top(tag: String) = rule.onNodeWithTag(tag).fetchSemanticsNode().boundsInRoot.top

    @Test
    fun theMessagesAreOldestFirst_theOlderOnesCollapsed_andTheNewestExpanded() {
        val f = ReadingFixture()
        f.show(rule)
        rule.waitForTag(BodyTags.body(NEWEST))
        for (id in listOf(OLD_ONE, OLD_TWO, NEWEST)) rule.onNodeWithTag(ReadingTags.message(id)).assertExists()
        assertTrue(top(ReadingTags.message(OLD_ONE)) < top(ReadingTags.message(OLD_TWO)))
        assertTrue(top(ReadingTags.message(OLD_TWO)) < top(ReadingTags.message(NEWEST)))
        // Collapsed: no body, only sender, date and snippet.
        rule.onNodeWithTag(BodyTags.body(OLD_ONE)).assertDoesNotExist()
        rule.onNodeWithTag(BodyTags.body(OLD_TWO)).assertDoesNotExist()
        rule.onNodeWithText("Snippet of message $OLD_TWO").assertExists()
        // Only the expanded ones were loaded.
        assertEquals(listOf("loadBody:$NEWEST:ask"), f.reading.calls.filter { it.startsWith("loadBody") })
        rule.onNodeWithTag(ReadingTags.SUBJECT).assertTextContains("Re: Server migration notes")
        rule.onNodeWithTag(ReadingTags.FOLDER).assertTextContains("Inbox")
    }

    @Test
    fun theOpenedMessageIsExpandedToo_evenWhenItIsAnOlderOne() {
        val f = ReadingFixture()
        f.show(rule, id = OLD_TWO)
        rule.waitForTag(BodyTags.body(OLD_TWO))
        rule.waitForTag(BodyTags.body(NEWEST))
        rule.onNodeWithTag(BodyTags.body(OLD_ONE)).assertDoesNotExist()
    }

    @Test
    fun aTapOnACollapsedMessageExpandsItAndLoadsItsBody_andATapOnItsHeaderCollapsesIt() {
        val f = ReadingFixture()
        f.show(rule)
        rule.waitForTag(BodyTags.body(NEWEST))
        rule.onNodeWithTag(ReadingTags.message(OLD_ONE)).performClick()
        rule.waitForTag(BodyTags.body(OLD_ONE))
        rule.waitForTag(BodyTags.text(OLD_ONE))
        rule.onNodeWithTag(BodyTags.text(OLD_ONE)).assertTextContains("The plan is attached.")
        assertTrue(f.reading.calls.contains("loadBody:$OLD_ONE:ask"))
        rule.onNodeWithTag(ReadingTags.header(OLD_ONE)).performClick()
        rule.waitForIdle()
        rule.onNodeWithTag(BodyTags.body(OLD_ONE)).assertDoesNotExist()
    }

    // ---- The action bar ----

    @Test
    fun archive_runsTheInboxsOwnRowActionOnTheMessagesOfTheFolder_andClosesWithAnUndoNotice() {
        val f = ReadingFixture()
        f.show(rule)
        rule.waitForTag(BodyTags.body(NEWEST))
        rule.onNodeWithTag(ReadingTags.ARCHIVE).performClick()
        rule.waitUntil(10_000) { f.closed.isNotEmpty() }
        assertEquals(listOf("archive $NEWEST", "archive $OLD_ONE", "archive $OLD_TWO").toSet(), f.actions.calls.filter { it.startsWith("archive") }.toSet())
        assertTrue("the opened message first", f.actions.calls.first { it.startsWith("archive") } == "archive $NEWEST")
        val notice = f.closed.single()!!
        assertTrue(notice.text.isNotBlank())
        assertTrue("an Undo that cancels the queued moves", !notice.ops.isNullOrEmpty())
        assertEquals("the mail left the Inbox", 0, f.repo.snapshot(com.example.croft.mail.MessageScope.Folder(1, f.inbox)).size)
        // Undo is the Inbox's: the same call.
        runBlocking { f.actions.undo(notice.ops!!) }
        assertEquals(3, f.repo.snapshot(com.example.croft.mail.MessageScope.Folder(1, f.inbox)).size)
    }

    @Test
    fun delete_movesToTrash_withTheSameNotice() {
        val f = ReadingFixture()
        f.show(rule)
        rule.waitForTag(BodyTags.body(NEWEST))
        rule.onNodeWithTag(ReadingTags.DELETE).performClick()
        rule.waitUntil(10_000) { f.closed.isNotEmpty() }
        assertTrue(f.actions.calls.contains("trash $NEWEST"))
        assertTrue(!f.closed.single()!!.ops.isNullOrEmpty())
    }

    @Test
    fun markUnread_marksTheOpenedMessage_andGoesBackToTheList() {
        val f = ReadingFixture()
        f.show(rule)
        rule.waitForTag(BodyTags.body(NEWEST))
        rule.onNodeWithTag(ReadingTags.MARK_UNREAD).performClick()
        rule.waitUntil(10_000) { f.closed.isNotEmpty() }
        assertTrue(f.actions.calls.contains("markSeen $NEWEST false"))
        assertNull("no Undo notice for a flag", f.closed.single())
        assertTrue("it was not marked read in between", !f.actions.calls.contains("markSeen $NEWEST true"))
    }

    @Test
    fun theOverflowMenuHasMarkAsUnreadMoveToAndReportSpam_andNothingThatDoesNotExistYet() {
        val f = ReadingFixture()
        f.show(rule)
        rule.waitForTag(BodyTags.body(NEWEST))
        rule.onNodeWithTag(ReadingTags.MORE).performClick()
        rule.waitForIdle()
        for (item in listOf("Mark as unread", "Move to", "Report spam")) assertTrue("$item is offered", rule.onAllNodes(hasText(item)).fetchSemanticsNodes().isNotEmpty())
        for (item in listOf("Show original", "Snooze", "Mute thread", "Unsubscribe", "Print", "Block sender")) {
            assertTrue("$item is not offered", rule.onAllNodes(hasText(item)).fetchSemanticsNodes().isEmpty())
        }
        rule.onNodeWithText("Report spam").performClick()
        rule.waitUntil(10_000) { f.closed.isNotEmpty() }
        assertTrue(f.actions.calls.contains("markSpam $NEWEST"))
    }

    @Test
    fun moveTo_opensTheFolderSheet_andTheChosenFolderGetsTheMessages() {
        val f = ReadingFixture()
        f.show(rule)
        rule.waitForTag(BodyTags.body(NEWEST))
        rule.onNodeWithTag(ReadingTags.MORE).performClick()
        rule.onNodeWithText("Move to").performClick()
        rule.waitForTag(MoveTags.SHEET)
        // The folder the message is in is not offered.
        rule.onNodeWithTag(MoveTags.folder(f.inbox)).assertDoesNotExist()
        val work = FakeData.folderId(1, FakeData.WORK)
        rule.onNodeWithTag(MoveTags.folder(work)).performClick()
        rule.waitUntil(10_000) { f.closed.isNotEmpty() }
        assertTrue(f.actions.calls.contains("moveTo $NEWEST $work"))
    }

    // ---- The reply bar ----

    @Test
    fun theReplyBarOpensComposeThroughTheComposeEntryPoints() {
        val f = ReadingFixture()
        f.show(rule)
        rule.waitForTag(BodyTags.body(NEWEST))
        rule.onNodeWithTag(ReadingTags.REPLY).performClick()
        rule.onNodeWithTag(ReadingTags.REPLY_ALL).performClick()
        rule.onNodeWithTag(ReadingTags.FORWARD).performClick()
        assertEquals(listOf(ComposeStart.Reply(1, NEWEST), ComposeStart.ReplyAll(1, NEWEST), ComposeStart.Forward(1, NEWEST)), f.composed.toList())
        // A reply button on the message itself answers that message.
        rule.onNodeWithTag(ReadingTags.reply(NEWEST)).performScrollTo().performClick()
        assertEquals(ComposeStart.Reply(1, NEWEST), f.composed.last())
    }

    @Test
    fun replyAllIsHiddenWhenThereIsOnlyOneOtherPerson() {
        val f = ReadingFixture(newestTo = listOf(MailAddressData("Sam Rivera", ME)), newestCc = emptyList())
        f.show(rule)
        rule.waitForTag(BodyTags.body(NEWEST))
        rule.onNodeWithTag(ReadingTags.REPLY).assertExists()
        rule.onNodeWithTag(ReadingTags.FORWARD).assertExists()
        rule.onNodeWithTag(ReadingTags.REPLY_ALL).assertDoesNotExist()
    }

    // ---- The sample mailbox ----

    @Test
    fun theSampleMailboxSaysNotSavedToServer_aRealAccountDoesNot() {
        val sample = ReadingFixture(localOnly = true)
        sample.show(rule)
        rule.waitForTag(BodyTags.body(NEWEST))
        rule.onNodeWithTag(ReadingTags.NOT_SAVED).assertIsDisplayed().assertTextContains("Not saved to server")
    }

    @Test
    fun aRealAccountHasNoSampleMarker() {
        val f = ReadingFixture()
        f.show(rule)
        rule.waitForTag(BodyTags.body(NEWEST))
        rule.onNodeWithTag(ReadingTags.NOT_SAVED).assertDoesNotExist()
    }

    // ---- Back, offline, errors ----

    @Test
    fun back_returnsToTheList() {
        val f = ReadingFixture()
        f.show(rule)
        rule.waitForTag(BodyTags.body(NEWEST))
        rule.onNodeWithTag(ReadingTags.BACK).performClick()
        assertEquals(1, f.backs.size)
    }

    @Test
    fun aBodyThatIsNotStoredWhileOfflineSaysSo_withRetry_andLoadsByItselfWhenTheAccountChanges() {
        val f = ReadingFixture()
        f.reading.loadFails = ReadProblem.OFFLINE
        f.show(rule)
        rule.waitForTag(BodyTags.notDownloaded(NEWEST))
        rule.onNodeWithTag(BodyTags.notDownloaded(NEWEST)).assertTextContains("Not downloaded yet. It will load when you're online.")
        rule.onNodeWithTag(BodyTags.retry(NEWEST)).assertExists()
        // The connection is back: the next change of the account's mail loads it without a tap.
        f.reading.loadFails = null
        runBlocking { f.repo.markSeen(listOf(OLD_ONE), true) }
        rule.waitForTag(BodyTags.html(NEWEST))
        rule.onNodeWithTag(BodyTags.notDownloaded(NEWEST)).assertDoesNotExist()
    }

    @Test
    fun retry_loadsTheBodyAgain() {
        val f = ReadingFixture()
        f.reading.loadFails = ReadProblem.SERVER_UNAVAILABLE
        f.show(rule)
        rule.waitForTag(BodyTags.retry(NEWEST))
        rule.onNodeWithTag(BodyTags.failure(NEWEST)).assertTextContains("The mail server isn't answering", substring = true)
        f.reading.loadFails = null
        rule.onNodeWithTag(BodyTags.retry(NEWEST)).performClick()
        rule.waitForTag(BodyTags.html(NEWEST))
    }

    @Test
    fun aConversationThatIsGoneSaysSoInPlainWords() {
        val f = ReadingFixture()
        f.show(rule, id = 999)
        rule.waitForTag(ReadingTags.FAILED)
        rule.onNodeWithTag(ReadingTags.FAILED).assertTextContains("no longer on this phone", substring = true)
        rule.onNodeWithTag(ReadingTags.ARCHIVE).assertDoesNotExist()
        rule.onNodeWithTag(ReadingTags.BACK).assertExists()
    }

    @Test
    fun aSessionWithoutTheReadingCallsSaysSomethingWentWrongInsteadOfCrashing() {
        val f = ReadingFixture()
        val without = MailSession(f.repo, com.example.croft.mail.InMemoryAnchorStore(), { com.example.croft.ui.mail.NOW }, java.time.ZoneId.of("UTC"), f.actions)
        rule.setContent {
            com.example.croft.CroftTheme { ConversationHost(without, NEWEST, com.example.croft.mail.MailSettings(), {}, {}, {}) }
        }
        rule.waitForTag(ReadingTags.FAILED)
        rule.onNodeWithTag(ReadingTags.FAILED).assertTextContains("Something went wrong", substring = true)
    }

    @Test
    fun theNewestMessageShowsItsRecipients_andMeIsNamedMe() {
        val f = ReadingFixture()
        f.show(rule)
        rule.waitForTag(BodyTags.body(NEWEST))
        assertNotNull(rule.onAllNodes(hasText("to me, Dan", substring = true)).fetchSemanticsNodes().firstOrNull())
        assertTrue(rule.onAllNodes(hasTestTag(ReadingTags.header(NEWEST))).fetchSemanticsNodes().isNotEmpty())
    }
}
