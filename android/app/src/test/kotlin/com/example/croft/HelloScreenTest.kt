// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft

import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onAllNodesWithTag
import androidx.compose.ui.test.hasTestTag
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollTo
import com.example.croft.mail.MailRuntime
import com.example.croft.ui.mail.InboxTags
import com.example.croft.ui.nav.SettingsTags
import com.example.croft.ui.onboarding.OnboardingTags
import org.junit.Assert.assertTrue
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
    @get:Rule(order = 0)
    val secrets = InMemorySecretsRule()

    @get:Rule(order = 1)
    val rule = createAndroidComposeRule<MainActivity>()

    /**
     * The stored theme is read before anything is drawn, so the first screen appears a moment after the activity starts. With no
     * account (as in a fresh install) that screen is S01 Welcome (T17b); "Try with sample mail" adds the core's own sample account
     * (T17c: the real core, with an in-memory wrapping key, see [InMemorySecretsRule]) and opens the Inbox that these tests go on from.
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

    @Test
    fun trySampleMailOnTheRealCoreWithoutNetworkReachesTheInboxThenSettingsAndAccounts() {
        rule.waitUntil(10_000) { rule.onAllNodes(hasTestTag(OnboardingTags.WELCOME)).fetchSemanticsNodes().isNotEmpty() }
        // Airplane mode, the app's own flag: the sample mailbox needs no network.
        kotlinx.coroutines.runBlocking { MailRuntime.openedCore(rule.activity.applicationContext).core.setNetworkAvailable(false) }
        rule.onNodeWithTag(OnboardingTags.SAMPLE_BUTTON).performClick()
        rule.waitUntil(10_000) { rule.onAllNodes(hasTestTag(InboxTags.SCREEN)).fetchSemanticsNodes().isNotEmpty() }
        // The sample messages come from the core and each row says it is not saved to a server.
        rule.waitUntil(20_000) { rule.onAllNodes(hasTestTag(com.example.croft.ui.components.ROW_NOT_SAVED_TAG), useUnmergedTree = true).fetchSemanticsNodes().isNotEmpty() }
        rule.onNodeWithTag(InboxTags.MENU_BUTTON).performClick()
        rule.waitForIdle()
        rule.onNodeWithTag("drawer-settings").performScrollTo().performClick()
        rule.waitUntil(10_000) { rule.onAllNodes(hasTestTag(SettingsTags.SCREEN)).fetchSemanticsNodes().isNotEmpty() }
        rule.onNodeWithTag(SettingsTags.ACCOUNTS).performClick()
        rule.waitUntil(10_000) { rule.onAllNodes(hasText("Sample mail")).fetchSemanticsNodes().isNotEmpty() }
    }
}

/**
 * A restart with the sample mailbox already added: the app opens the Inbox, not S01 (an account, the sample one, exists).
 */
@RunWith(CroftRobolectricRunner::class)
@Config(sdk = [35])
class SampleMailRestartTest {
    @get:Rule(order = 0)
    val secrets = InMemorySecretsRule()

    @get:Rule(order = 1)
    val rule = androidx.compose.ui.test.junit4.createEmptyComposeRule()

    @Test
    fun afterARestartTheAppOpensTheInboxBecauseTheSampleAccountExists() {
        val app = androidx.test.core.app.ApplicationProvider.getApplicationContext<android.content.Context>()
        kotlinx.coroutines.runBlocking { MailRuntime.openedCore(app).core.addDemoAccount() }
        // The process ends: the core is closed and forgotten. The activity then opens it again from the file, with the same wrapping key,
        // as a restart does, and finds the account that is stored there.
        MailRuntime.forgetCore()
        androidx.test.core.app.ActivityScenario.launch(MainActivity::class.java).use {
            rule.waitUntil(20_000) { rule.onAllNodes(hasTestTag(InboxTags.SCREEN)).fetchSemanticsNodes().isNotEmpty() }
            assertTrue(rule.onAllNodes(hasTestTag(OnboardingTags.WELCOME)).fetchSemanticsNodes().isEmpty())
        }
    }
}
