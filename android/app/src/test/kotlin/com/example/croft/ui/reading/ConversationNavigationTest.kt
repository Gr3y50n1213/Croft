// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.ui.reading

import androidx.activity.ComponentActivity
import androidx.compose.ui.test.hasTestTag
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.performClick
import com.example.croft.CroftTheme
import com.example.croft.RustInfo
import com.example.croft.theme.AnimationLevel
import com.example.croft.theme.ThemeSettings
import com.example.croft.ui.nav.CroftNavHost
import com.example.croft.ui.mail.InboxTags
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config

/** A tap on a row of the Inbox opens S20 for that message; Back returns to the Inbox; an action closes S20 and the Inbox shows the Undo. */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35], qualifiers = "w360dp-h800dp-mdpi")
class ConversationNavigationTest {
    @get:Rule
    val rule = createAndroidComposeRule<ComponentActivity>()

    private val f = ReadingFixture()

    private fun showApp() {
        rule.setContent {
            CroftTheme(ThemeSettings.Default.copy(animation = AnimationLevel.OFF), systemDark = false) {
                CroftNavHost(RustInfo("9.8.7", 1u), f.session, ThemeSettings.Default.copy(animation = AnimationLevel.OFF), {})
            }
        }
        rule.waitUntil(10_000) { rule.onAllNodes(hasTestTag(InboxTags.row(NEWEST))).fetchSemanticsNodes().isNotEmpty() }
    }

    private fun back() {
        rule.runOnUiThread { rule.activity.onBackPressedDispatcher.onBackPressed() }
        rule.waitForIdle()
    }

    @Test
    fun aTapOnAnInboxRowOpensTheConversationOfThatMessage_andBackReturnsToTheInbox() {
        showApp()
        rule.onNodeWithTag(InboxTags.row(NEWEST)).performClick()
        rule.waitForTag(ReadingTags.SCREEN)
        rule.waitForTag(BodyTags.body(NEWEST))
        assertTrue(f.reading.calls.contains("openConversation:$NEWEST"))
        back()
        rule.waitForTag(InboxTags.SCREEN)
        rule.onNodeWithTag(ReadingTags.SCREEN).assertDoesNotExist()
        assertTrue("the Inbox's row is still there", rule.onAllNodes(hasTestTag(InboxTags.row(NEWEST))).fetchSemanticsNodes().isNotEmpty())
    }

    @Test
    fun archivingInTheConversationGoesBackToTheInboxWhichShowsUndo() {
        showApp()
        rule.onNodeWithTag(InboxTags.row(NEWEST)).performClick()
        rule.waitForTag(BodyTags.body(NEWEST))
        rule.onNodeWithTag(ReadingTags.ARCHIVE).performClick()
        rule.waitForTag(InboxTags.SCREEN)
        rule.waitUntil(10_000) { rule.onAllNodes(hasText("Undo")).fetchSemanticsNodes().isNotEmpty() }
        assertTrue(f.actions.calls.contains("archive $NEWEST"))
        rule.onNodeWithTag(ReadingTags.SCREEN).assertDoesNotExist()
    }

    @Test
    fun replyOpensComposeAndItsBackComesBackToTheConversation() {
        showApp()
        rule.onNodeWithTag(InboxTags.row(NEWEST)).performClick()
        rule.waitForTag(BodyTags.body(NEWEST))
        rule.onNodeWithTag(ReadingTags.REPLY).performClick()
        // Compose has no backend in this fixture and closes at once, back to where it was opened: the conversation, opened again.
        rule.waitUntil(10_000) { f.reading.calls.count { it == "openConversation:$NEWEST" } >= 2 }
        rule.waitForTag(ReadingTags.SCREEN)
        rule.onNodeWithTag(InboxTags.SCREEN).assertDoesNotExist()
    }
}
