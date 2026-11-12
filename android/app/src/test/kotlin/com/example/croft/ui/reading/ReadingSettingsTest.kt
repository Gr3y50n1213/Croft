// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.ui.reading

import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.semantics.getOrNull
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollTo
import com.example.croft.mail.MailSettings
import com.example.croft.mail.reading.MarkReadDelay
import com.example.croft.mail.reading.RemoteImages
import com.example.croft.theme.AnimationLevel
import com.example.croft.ui.components.show
import com.example.croft.ui.mail.InboxSettingsScreen
import com.example.croft.ui.mail.InboxSettingsTags
import kotlinx.coroutines.runBlocking
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config

/** S49 rows of reading: Remote images, Mark as read when opened, and the known senders list. */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35], qualifiers = "w360dp-h900dp-mdpi")
class ReadingSettingsTest {
    @get:Rule
    val rule = createComposeRule()

    private fun selected(text: String) = rule.onNodeWithText(text).fetchSemanticsNode().config.getOrNull(SemanticsProperties.Selected) == true

    @Test
    fun remoteImagesAndMarkReadHaveTheirChoices_theDefaultsChosen_andAChoiceIsReported() {
        var settings by mutableStateOf(MailSettings())
        rule.show(AnimationLevel.OFF) { InboxSettingsScreen(settings, onChange = { settings = it }, onBack = {}) }
        rule.onNodeWithTag(InboxSettingsTags.REMOTE_IMAGES).performScrollTo().assertExists()
        rule.onNodeWithTag(InboxSettingsTags.MARK_READ).performScrollTo().assertExists()
        for (s in listOf("Ask", "Known senders", "Immediately", "After 2 s")) rule.onNodeWithText(s).performScrollTo().assertExists()
        assertTrue("Ask is the default", selected("Ask"))
        assertTrue("After 2 s is the default", selected("After 2 s"))

        rule.onNodeWithText("Known senders").performScrollTo().performClick()
        rule.waitForIdle()
        assertEquals(RemoteImages.KNOWN_SENDERS, settings.remoteImages)
        rule.onNodeWithText("Immediately").performScrollTo().performClick()
        rule.waitForIdle()
        assertEquals(MarkReadDelay.IMMEDIATELY, settings.markRead)
        assertEquals("the other settings are kept", 5, settings.undoSeconds)
    }

    @Test
    fun theKnownSendersAreListedWithARemoveButton_thatUntrustsThem() {
        val f = ReadingFixture()
        runBlocking { f.reading.trustSender(1, "ann@example.org"); f.reading.trustSender(1, "bo@example.net") }
        rule.show(AnimationLevel.OFF) { InboxSettingsScreen(MailSettings(remoteImages = RemoteImages.KNOWN_SENDERS), onChange = {}, onBack = {}, session = f.session) }
        rule.onNodeWithTag(InboxSettingsTags.knownSender("ann@example.org")).performScrollTo().assertExists()
        rule.onNodeWithTag(InboxSettingsTags.knownSender("bo@example.net")).performScrollTo().assertExists()
        rule.onNodeWithContentDescription("Stop showing images from ann@example.org").performScrollTo().performClick()
        rule.waitUntil(10_000) { f.reading.calls.contains("untrustSender:1") }
        rule.waitForIdle()
        rule.onNodeWithTag(InboxSettingsTags.knownSender("ann@example.org")).assertDoesNotExist()
        assertEquals(listOf("bo@example.net"), runBlocking { f.reading.trustedSenders(1) })
    }

    @Test
    fun withNoKnownSendersTheListSaysHowToAddOne() {
        val f = ReadingFixture()
        rule.show(AnimationLevel.OFF) { InboxSettingsScreen(MailSettings(), onChange = {}, onBack = {}, session = f.session) }
        rule.onNodeWithText("No senders yet.", substring = true).performScrollTo().assertExists()
    }
}
