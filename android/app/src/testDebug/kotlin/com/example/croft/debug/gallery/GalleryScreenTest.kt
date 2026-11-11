// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.debug.gallery

import androidx.compose.ui.test.assertCountEquals
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.assertTextContains
import androidx.compose.ui.test.hasAnyAncestor
import androidx.compose.ui.test.hasTestTag
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.longClick
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollToNode
import androidx.compose.ui.test.performTouchInput
import com.example.croft.CroftTheme
import com.example.croft.haptics.HapticEvent
import com.example.croft.haptics.LevelFilteredHaptics
import com.example.croft.haptics.LocalHaptics
import com.example.croft.haptics.RecordingHaptics
import com.example.croft.theme.AnimationLevel
import com.example.croft.theme.HapticsLevel
import com.example.croft.theme.ThemeSettings
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config

/** The gallery screen: sections, the haptic calls from switches, long-press and the destructive dialog, the snackbar rule. */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35])
class GalleryScreenTest {
    @get:Rule
    val rule = createComposeRule()

    private val recording = RecordingHaptics()
    private var backs = 0

    private fun show(level: HapticsLevel = HapticsLevel.FULL) {
        val settings = ThemeSettings.Default.copy(animation = AnimationLevel.OFF, haptics = level)
        rule.setContent {
            CroftTheme(settings, systemDark = false) {
                androidx.compose.runtime.CompositionLocalProvider(LocalHaptics provides LevelFilteredHaptics({ level }, { System.nanoTime() / 1_000_000L * 1_000L }, recording)) {
                    GalleryScreen(onBack = { backs++ })
                }
            }
        }
    }

    private fun scrollToSection(section: String) {
        rule.onNodeWithTag(GalleryTags.LIST).performScrollToNode(hasTestTag(GalleryTags.SECTION_PREFIX + section))
    }

    private fun open(section: String) {
        scrollToSection(section)
        rule.onNodeWithTag(GalleryTags.SECTION_PREFIX + section).performClick()
        rule.waitForIdle()
    }

    @Test
    fun theScreenListsEverySection_andASectionOpensAndGoesBack() {
        show()
        for (s in GalleryCatalog.sections) { scrollToSection(s.id); rule.onNodeWithTag(GalleryTags.SECTION_PREFIX + s.id).assertExists() }
        open("buttons")
        rule.onNodeWithTag(galleryTag("buttons.filled")).assertIsDisplayed()
        rule.onNodeWithContentDescription("Back").performClick()
        rule.waitForIdle()
        rule.onNodeWithTag(GalleryTags.LIST).assertIsDisplayed()
        assertEquals(0, backs)
        rule.onNodeWithContentDescription("Back").performClick()
        assertEquals("Back on the list leaves the gallery", 1, backs)
    }

    @Test
    fun aSwitchInTheGalleryPlaysTheToggleHaptic() {
        show()
        open("toggles")
        rule.onNode(hasAnyAncestor(hasTestTag(galleryTag("toggles.switch_on"))) and androidx.compose.ui.test.isToggleable()).performClick()
        assertEquals(listOf(HapticEvent.ToggleOff), recording.events)
        rule.onNodeWithTag(GalleryTags.HAPTIC_LOG).assertTextContains("ToggleOff", substring = true)
    }

    @Test
    fun theGalleryMarksAnEventTheLevelWouldNotPlay() {
        show(HapticsLevel.SUBTLE)
        open("toggles")
        rule.onNode(hasAnyAncestor(hasTestTag(galleryTag("toggles.switch_on"))) and androidx.compose.ui.test.isToggleable()).performClick()
        assertTrue("dropped by the level filter", recording.events.isEmpty())
        rule.onNodeWithTag(GalleryTags.HAPTIC_LOG).assertTextContains("not at subtle", substring = true)
    }

    @Test
    fun aLongPressInTheGalleryLiftsAndPlaysTheLongPressHaptic_beforeTheResult() {
        show()
        open("longpress")
        rule.onNodeWithText("Hold me").performTouchInput { longClick() }
        rule.onNodeWithText("Result opened at the moment of the long-press").assertExists()
        assertEquals(listOf(HapticEvent.SelectionStart), recording.events)
    }

    @Test
    fun confirmingTheDestructiveDialogInTheGalleryPlaysTheHaptic() {
        show()
        open("dialog")
        rule.onNodeWithText("Delete forever").performClick()
        rule.waitForIdle()
        rule.onNodeWithText("Delete forever?").assertIsDisplayed()
        rule.onAllNodesDialogConfirm()
        assertEquals(listOf(HapticEvent.Success), recording.events)
        rule.onNodeWithText("Confirmed (haptic played)").assertExists()
    }

    private fun androidx.compose.ui.test.junit4.ComposeContentTestRule.onAllNodesDialogConfirm() {
        // Two nodes say "Delete forever": the button behind the dialog and the dialog's confirm; the confirm is the one in the dialog window.
        val nodes = onAllNodesWithText("Delete forever").fetchSemanticsNodes()
        assertEquals(2, nodes.size)
        onAllNodesWithText("Delete forever")[1].performClick()
    }

    @Test
    fun aNewSnackbarInTheGalleryReplacesTheOldOne() {
        show()
        open("snackbar")
        rule.onNodeWithText("Delete message").performClick()
        rule.onNodeWithText("Message 1 deleted").assertIsDisplayed()
        rule.onNodeWithText("Archive message").performClick()
        rule.waitForIdle()
        rule.onNodeWithText("Message 2 archived").assertIsDisplayed()
        rule.onAllNodesWithText("Message 1 deleted").assertCountEquals0()
    }

    private fun androidx.compose.ui.test.SemanticsNodeInteractionCollection.assertCountEquals0() = assertCountEquals(0)

    @Test
    fun theMotionRowOverridesTheAnimationLevelForTheGalleryAlone() {
        show()
        rule.onNodeWithText("Off").performClick()
        rule.onNodeWithText("Reduced").performClick()
        rule.onNodeWithText("App").performClick()
        rule.waitForIdle()
        rule.onNodeWithText("Component gallery").assertIsDisplayed()
    }
}
