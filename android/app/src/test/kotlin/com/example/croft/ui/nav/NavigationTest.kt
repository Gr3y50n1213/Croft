// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.ui.nav

import androidx.activity.ComponentActivity
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.test.SemanticsMatcher
import androidx.compose.ui.test.assert
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.hasTestTag
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollTo
import com.example.croft.RustInfo
import com.example.croft.ui.mail.InboxFixture
import com.example.croft.ui.mail.InboxTags
import com.example.croft.ui.nav.CroftApp
import com.example.croft.debug.DebugGallery
import com.example.croft.design.CroftPreset
import com.example.croft.theme.AnimationLevel
import com.example.croft.theme.ThemeJson
import com.example.croft.theme.ThemeSettings
import com.example.croft.theme.ThemeStore
import com.example.croft.ui.appearance.AppearanceTags
import com.example.croft.ui.appearance.MotionTags
import java.io.File
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.runBlocking
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config

/**
 * The simple app navigation: Home, Settings, Appearance (S50 and its drill-down) and, in debug builds, the gallery. Back is the
 * system's: the activity's back dispatcher (which is what the back gesture and button call).
 */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35])
class NavigationTest {
    @get:Rule
    val rule = createAndroidComposeRule<ComponentActivity>()

    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
    private val file = File.createTempFile("croft-theme", ".json").also { it.delete() }

    @After
    fun tearDown() { scope.cancel(); file.delete() }

    private val instant = ThemeSettings.Default.copy(animation = AnimationLevel.OFF)

    private fun store(initial: ThemeSettings = instant): ThemeStore {
        file.writeText(ThemeJson.encode(initial))
        return ThemeStore.onFile(file, scope)
    }

    private val mail = InboxFixture(60)

    /** The app starts on the Inbox (S10): the first row is on screen. */
    private fun showApp(store: ThemeStore = store()) {
        rule.setContent { CroftApp(store, RustInfo("9.8.7", 1u), mail.session) }
        rule.waitUntil(10_000) { rule.onAllNodes(hasTestTag(InboxTags.row(mail.topId))).fetchSemanticsNodes().isNotEmpty() }
    }

    /** Settings stay reachable: from the drawer of the Inbox. */
    private fun openSettings() {
        tag(InboxTags.MENU_BUTTON).performClick()
        rule.waitForIdle()
        tag("drawer-settings").performScrollTo().performClick()
        rule.waitUntil(5_000) { rule.onAllNodes(hasTestTag(SettingsTags.SCREEN)).fetchSemanticsNodes().isNotEmpty() }
    }

    private fun back() {
        rule.runOnUiThread { rule.activity.onBackPressedDispatcher.onBackPressed() }
        rule.waitForIdle()
    }

    private fun tag(t: String) = rule.onNodeWithTag(t)

    @Test
    fun theStartScreenIsTheInbox_andSettingsStayReachableFromItsDrawer() {
        showApp()
        tag(InboxTags.SCREEN).assertIsDisplayed()
        openSettings()
        tag(SettingsTags.SCREEN).assertIsDisplayed()
        tag(SettingsTags.APPEARANCE).assertIsDisplayed()
        // The core version moved from the start screen to Settings, About.
        rule.onNodeWithText("Core version 9.8.7. FFI API version 1").assertIsDisplayed()
    }

    @Test
    fun appearanceIsReachableFromSettings_andBackWalksOutOneStepAtATime() {
        showApp()
        openSettings()
        tag(SettingsTags.APPEARANCE).performClick()
        tag(AppearanceTags.SCREEN).assertIsDisplayed()

        rule.onNodeWithText("Motion and haptics").performScrollTo().performClick()
        rule.waitForIdle()
        tag(MotionTags.SCREEN).assertIsDisplayed()

        back() // S53 to S50 first (the flow handles it)
        tag(AppearanceTags.SCREEN).assertIsDisplayed()
        back() // S50 to Settings
        tag(SettingsTags.SCREEN).assertIsDisplayed()
        back() // Settings to Home (the Inbox)
        tag(InboxTags.SCREEN).assertIsDisplayed()
        assertTrue("on Home Back is left to the system", !rule.activity.isFinishing)
        back()
        assertTrue("and the system closes the app", rule.activity.isFinishing)
    }

    @Test
    fun theBackArrowOnTheSettingsListGoesHome() {
        showApp()
        openSettings()
        rule.onNodeWithContentDescription("Back").performClick()
        rule.waitForIdle()
        tag(InboxTags.SCREEN).assertIsDisplayed()
    }

    @Test
    fun theGalleryIsListedInSettingsOnlyWhereItExists() {
        showApp()
        openSettings()
        val listed = rule.onAllNodes(hasTestTag(SettingsTags.GALLERY)).fetchSemanticsNodes().isNotEmpty()
        assertEquals("debug builds list it, release builds do not", DebugGallery.available, listed)
    }

    @Test
    fun aChangeInAppearanceIsAppliedAndWrittenToTheStore() {
        val store = store(instant.withPreset(CroftPreset.WAX))
        showApp(store)
        openSettings()
        tag(SettingsTags.APPEARANCE).performClick()
        tag(AppearanceTags.PALETTE_PREFIX + "tide").performScrollTo().performClick()
        rule.waitForIdle()
        rule.waitUntil(5_000) { runBlocking { store.settings.first().preset } == CroftPreset.TIDE }
        assertEquals(CroftPreset.TIDE, runBlocking { store.settings.first().preset })
    }

    @Test
    fun theStoredSettingsAreAppliedAtStart() {
        val store = store(instant.withPreset(CroftPreset.ROWAN))
        showApp(store)
        openSettings()
        tag(SettingsTags.APPEARANCE).performClick()
        // The palette the person chose last time is the one S50 shows as selected.
        tag(AppearanceTags.PALETTE_PREFIX + "rowan").performScrollTo().assert(SemanticsMatcher.expectValue(SemanticsProperties.Selected, true))
    }
}
