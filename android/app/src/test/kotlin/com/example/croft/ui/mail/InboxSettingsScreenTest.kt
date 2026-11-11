// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.ui.mail

import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import com.example.croft.mail.MailSettings
import com.example.croft.theme.AnimationLevel
import com.example.croft.ui.components.show
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config

/** S49 Inbox and categories, as far as it exists: the Undo time. */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35])
class InboxSettingsScreenTest {
    @get:Rule
    val rule = createComposeRule()

    @Test
    fun s49_showsTheThreeUndoTimes_theCurrentOneChosen_andAChoiceIsReported() {
        var settings by mutableStateOf(MailSettings())
        rule.show(AnimationLevel.OFF) { InboxSettingsScreen(settings, onChange = { settings = it }, onBack = {}) }
        rule.onNodeWithTag(InboxSettingsTags.SCREEN).assertExists()
        for (s in listOf("3 s", "5 s", "10 s")) rule.onNodeWithText(s).assertExists()
        rule.onNodeWithText("10 s").performClick()
        rule.waitForIdle()
        assertEquals(10, settings.undoSeconds)
        assertEquals(10_000L, settings.undoWindowMs)
        rule.onNodeWithText("3 s").performClick()
        rule.waitForIdle()
        assertEquals(3_000L, settings.undoWindowMs)
        assertTrue(rule.onAllNodes(hasText("How long Undo stays", substring = true)).fetchSemanticsNodes().isNotEmpty())
    }
}
