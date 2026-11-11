// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft

import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onAllNodesWithTag
import androidx.compose.ui.test.hasTestTag
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollTo
import com.example.croft.ui.mail.InboxTags
import com.example.croft.ui.nav.SettingsTags
import com.example.croft.ui.onboarding.OnboardingTags
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.annotation.Config

/** The screen on a simulated device (Robolectric): no emulator needed. */
@RunWith(CroftRobolectricRunner::class)
@Config(sdk = [35])
class HelloScreenTest {
    @get:Rule
    val rule = createComposeRule()

    @Test
    fun showsTheNameAndBothVersions() {
        rule.setContent { CroftTheme { HelloScreen(RustInfo("9.8.7", 1u)) } }
        rule.onNodeWithText("Croft").assertIsDisplayed()
        rule.onNodeWithText("Core version 9.8.7").assertIsDisplayed()
        rule.onNodeWithText("FFI API version 1").assertIsDisplayed()
    }

    @Test
    fun showsAnErrorLineWhenTheCoreIsMissing() {
        rule.setContent { CroftTheme { HelloScreen(null) } }
        rule.onNodeWithText("The Croft core could not be loaded.").assertIsDisplayed()
    }
}

/**
 * The real activity, calling the real Rust library (host build) through the generated bindings. Since T14 the start screen is
 * the Inbox; the core version line is in Settings, About, which the drawer opens.
 */
@RunWith(CroftRobolectricRunner::class)
@Config(sdk = [35])
class MainActivityTest {
    @get:Rule
    val rule = createAndroidComposeRule<MainActivity>()

    /**
     * The stored theme is read before anything is drawn, so the first screen appears a moment after the activity starts. With no
     * account (as in a fresh install) that screen is S01 Welcome (T17b); "Try with sample mail" opens the sample Inbox that
     * these tests go on from.
     */
    private fun waitForStart() {
        rule.waitUntil(10_000) { rule.onAllNodes(hasTestTag(OnboardingTags.WELCOME)).fetchSemanticsNodes().isNotEmpty() }
        rule.onNodeWithTag(OnboardingTags.SAMPLE_BUTTON).performClick()
        rule.waitUntil(10_000) { rule.onAllNodes(hasTestTag(InboxTags.SCREEN)).fetchSemanticsNodes().isNotEmpty() }
    }

    /** Settings are the last entry of the Inbox's drawer. */
    private fun openSettings() {
        waitForStart()
        rule.onNodeWithTag(InboxTags.MENU_BUTTON).performClick()
        rule.waitForIdle()
        rule.onNodeWithTag("drawer-settings").performScrollTo().performClick()
        rule.waitUntil(10_000) { rule.onAllNodes(hasTestTag(SettingsTags.SCREEN)).fetchSemanticsNodes().isNotEmpty() }
    }

    @Test
    fun showsTheVersionFromRust() {
        openSettings()
        val expected = System.getProperty("croft.cargoVersion")
        // The app refuses a library with another version, so the screen shows the one it expects.
        rule.onNodeWithText("Core version $expected. FFI API version $EXPECTED_FFI_API_VERSION").assertIsDisplayed()
    }

    @Test
    fun theSettingsEntryOpensTheListAndTheScreenSurvivesTheActivityBeingRecreated() {
        openSettings()
        rule.onNodeWithTag(SettingsTags.SCREEN).assertIsDisplayed()
        rule.activityRule.scenario.recreate()
        rule.waitUntil(10_000) { rule.onAllNodesWithTag(SettingsTags.SCREEN).fetchSemanticsNodes().isNotEmpty() }
        rule.onNodeWithTag(SettingsTags.SCREEN).assertIsDisplayed()
    }
}
