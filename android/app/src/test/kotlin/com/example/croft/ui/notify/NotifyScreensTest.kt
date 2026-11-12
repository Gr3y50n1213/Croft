// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.ui.notify

import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.test.SemanticsMatcher
import androidx.compose.ui.test.assert
import androidx.compose.ui.test.assertHasClickAction
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.assertIsEnabled
import androidx.compose.ui.test.assertIsNotEnabled
import androidx.compose.ui.test.assertIsSelected
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollTo
import com.example.croft.CroftRobolectricRunner
import com.example.croft.CroftTheme
import com.example.croft.haptics.LocalHaptics
import com.example.croft.haptics.NoHaptics
import com.example.croft.notify.CheckAccount
import com.example.croft.notify.CheckFrequency
import com.example.croft.notify.NotifyAction
import com.example.croft.notify.NotifySettings
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.annotation.Config

@RunWith(CroftRobolectricRunner::class)
@Config(sdk = [35])
class NotifyScreensTest {
    @get:Rule val rule = createComposeRule()

    private val accounts = listOf(CheckAccount(1, "Work"), CheckAccount(2, "Home"), CheckAccount(3, "Sample", isDemo = true))

    @Composable
    private fun Themed(content: @Composable () -> Unit) {
        CroftTheme { CompositionLocalProvider(LocalHaptics provides NoHaptics) { content() } }
    }

    private class Intro {
        var settings = NotifySettings()
        var asked = 0
        var answer: ((Boolean) -> Unit)? = null
        var finished = false
    }

    private fun showIntro(intro: Intro) {
        rule.setContent {
            Themed {
                NotifyIntroHost(
                    settings = intro.settings,
                    onChange = { intro.settings = it },
                    accounts = { accounts },
                    onFinished = { intro.finished = true },
                    requestPermission = { onResult -> intro.asked++; intro.answer = onResult },
                )
            }
        }
    }

    @Test
    fun theRationaleExplainsWhyAndOffersOnlyPeriodicSync() {
        showIntro(Intro())
        rule.onNodeWithText("Croft can check your mail in the background", substring = true).assertIsDisplayed()
        rule.onNodeWithText("never the message itself", substring = true).assertExists()
        rule.onNodeWithTag(NotifyTags.PERIODIC).assertIsSelected()
        rule.onNodeWithText("Battery saver can delay checks", substring = true).assertExists()
        rule.onNodeWithText("UnifiedPush", substring = true).assertDoesNotExist()
        rule.onNodeWithTag(NotifyTags.INTRO_TURN_ON).assertHasClickAction()
            .assert(SemanticsMatcher.expectValue(SemanticsProperties.Role, Role.Button))
        rule.onNodeWithTag(NotifyTags.INTRO_SKIP).assertHasClickAction()
    }

    @Test
    fun onAndroid13AndLaterTurnOnAsksOnceAndARefusalShowsTheCalmBannerNotASecondPrompt() {
        val intro = Intro()
        showIntro(intro)
        rule.onNodeWithTag(NotifyTags.INTRO_TURN_ON).performClick()
        rule.waitUntil(5_000) { rule.waitForIdle(); intro.asked == 1 }
        intro.answer!!(false)
        rule.waitForIdle()
        rule.onNodeWithTag(NotifyTags.DENIED).assertExists()
        rule.onNodeWithTag(NotifyTags.DENIED_SETTINGS).assertHasClickAction()
        rule.onNodeWithTag(NotifyTags.INTRO_TURN_ON).assertDoesNotExist()
        assertEquals(1, intro.asked)
        rule.onNodeWithTag(NotifyTags.INTRO_DONE).performScrollTo().performClick()
        rule.waitUntil(5_000) { rule.waitForIdle(); intro.finished }
        assertTrue(intro.settings.introDone)
    }

    @Test
    fun aGrantOffersSendTestAndDone() {
        val intro = Intro()
        showIntro(intro)
        rule.onNodeWithTag(NotifyTags.INTRO_TURN_ON).performClick()
        rule.waitUntil(5_000) { rule.waitForIdle(); intro.asked == 1 }
        intro.answer!!(true)
        rule.waitForIdle()
        rule.onNodeWithTag(NotifyTags.INTRO_TEST).assertExists()
        rule.onNodeWithTag(NotifyTags.INTRO_DONE).assertExists()
        rule.onNodeWithTag(NotifyTags.DENIED).assertDoesNotExist()
    }

    @Test
    @Config(sdk = [30])
    fun belowAndroid13ThereIsNoPrompt() {
        val intro = Intro()
        showIntro(intro)
        rule.onNodeWithTag(NotifyTags.INTRO_TURN_ON).performClick()
        rule.waitUntil(5_000) { rule.waitForIdle(); rule.onAllNodesWithTagExists(NotifyTags.INTRO_DONE) }
        assertEquals(0, intro.asked)
        rule.onNodeWithTag(NotifyTags.INTRO_TEST).assertExists()
    }

    @Test
    fun skippingIsAllowedAndRemembered() {
        val intro = Intro()
        showIntro(intro)
        rule.onNodeWithTag(NotifyTags.INTRO_SKIP).performScrollTo().performClick()
        rule.waitUntil(5_000) { rule.waitForIdle(); intro.finished }
        assertTrue(intro.settings.introDone)
        assertEquals(0, intro.asked)
    }

    private fun showSettings(start: NotifySettings, canPost: Boolean = true): () -> NotifySettings {
        var current = start
        rule.setContent {
            Themed {
                var s by remember { mutableStateOf(start) }
                NotificationSettingsScreen(
                    settings = s, accounts = accounts, canPost = canPost,
                    onChange = { s = it; current = it }, onSendTest = {}, onOpenSettings = {}, onOpenBatterySettings = {}, onBack = {},
                )
            }
        }
        return { current }
    }

    @Test
    fun s46ListsTheRealAccountsAndTheirSwitchesMuteThem() {
        val now = showSettings(NotifySettings())
        rule.onNodeWithTag(NotifyTags.account(1)).assertExists()
        rule.onNodeWithTag(NotifyTags.account(3)).assertDoesNotExist()
        rule.onNodeWithTag(NotifyTags.account(2)).performScrollTo().performClick()
        assertEquals(setOf(2L), now().mutedAccounts)
    }

    @Test
    fun s46AllowsAtMostThreeButtons() {
        val now = showSettings(NotifySettings(actions = listOf(NotifyAction.ARCHIVE, NotifyAction.MARK_READ)))
        rule.onNodeWithTag(NotifyTags.action(NotifyAction.REPLY)).performScrollTo().performClick()
        assertEquals(listOf(NotifyAction.ARCHIVE, NotifyAction.MARK_READ, NotifyAction.REPLY), now().actions)
        rule.onNodeWithTag(NotifyTags.action(NotifyAction.DELETE)).performScrollTo().assertIsNotEnabled()
        rule.onNodeWithTag(NotifyTags.action(NotifyAction.ARCHIVE)).performScrollTo().assertIsEnabled().performClick()
        rule.onNodeWithTag(NotifyTags.action(NotifyAction.DELETE)).performScrollTo().assertIsEnabled()
    }

    @Test
    fun s46ChangesTheFrequencyAndTheBatteryOption() {
        val now = showSettings(NotifySettings())
        rule.onNodeWithTag(NotifyTags.frequency(CheckFrequency.HOUR_2)).performScrollTo().performClick()
        assertEquals(CheckFrequency.HOUR_2, now().frequency)
        rule.onNodeWithTag(NotifyTags.BATTERY_NOT_LOW).performScrollTo().performClick()
        assertTrue(now().batteryNotLow)
        rule.onNodeWithTag(NotifyTags.frequency(CheckFrequency.MANUAL)).performScrollTo().performClick()
        rule.onNodeWithTag(NotifyTags.BATTERY_NOT_LOW).performScrollTo().assertIsNotEnabled()
    }

    @Test
    fun s46ShowsTheBannerAndNoTestWhileNotificationsAreOff() {
        showSettings(NotifySettings(), canPost = false)
        rule.onNodeWithTag(NotifyTags.DENIED).assertExists()
        rule.onNodeWithTag(NotifyTags.SEND_TEST).performScrollTo().assertIsNotEnabled()
        rule.onNodeWithTag(NotifyTags.BATTERY_SETTINGS).performScrollTo().assertHasClickAction()
    }

    private fun androidx.compose.ui.test.junit4.ComposeContentTestRule.onAllNodesWithTagExists(tag: String) =
        onAllNodes(androidx.compose.ui.test.hasTestTag(tag)).fetchSemanticsNodes().isNotEmpty()
}
