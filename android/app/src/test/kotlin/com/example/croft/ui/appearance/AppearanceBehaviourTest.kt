// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.ui.appearance

import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.snapshots.Snapshot
import androidx.compose.ui.geometry.Rect
import androidx.compose.ui.semantics.SemanticsActions
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.test.SemanticsNodeInteraction
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.assertIsEnabled
import androidx.compose.ui.test.assertIsNotEnabled
import androidx.compose.ui.test.assertIsOff
import androidx.compose.ui.test.assertIsOn
import androidx.compose.ui.test.assert
import androidx.compose.ui.test.click
import androidx.compose.ui.semantics.getOrNull
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollTo
import androidx.compose.ui.test.performSemanticsAction
import androidx.compose.ui.test.performTouchInput
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.unit.toSize
import androidx.compose.ui.unit.dp
import com.example.croft.CroftTheme
import com.example.croft.design.CroftContrast
import com.example.croft.design.CroftPreset
import com.example.croft.theme.AnimationLevel
import com.example.croft.theme.ColourSource
import com.example.croft.theme.DensityLevel
import com.example.croft.theme.HapticsLevel
import com.example.croft.theme.McuSchemes
import com.example.croft.theme.ModePreference
import com.example.croft.theme.Roundness
import com.example.croft.theme.ThemeJson
import com.example.croft.theme.ThemeSettings
import com.example.croft.theme.ThemeStyle
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config

/**
 * What the screens do, on a simulated device and fake data. Test names start with the screen ID (S08, S50 to S53).
 * The words in the assertions are the strings of `strings_appearance.xml`.
 */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35])
class AppearanceBehaviourTest {
    @get:Rule
    val rule = createComposeRule()

    private val settings = mutableStateOf(ThemeSettings.Default)
    private fun change(s: ThemeSettings) { settings.value = s }

    /** A change made by the test itself (not by the UI): on the UI thread, so a paused clock still sees it. */
    private fun testChange(s: ThemeSettings) = rule.runOnUiThread { settings.value = s; Snapshot.sendApplyNotifications() }

    // Instant motion keeps the tests about behaviour; the cross-fade has its own tests in ThemeProviderTest.
    private val off = ThemeSettings.Default.copy(animation = AnimationLevel.OFF)

    private fun s50(
        onReset: () -> Unit = {},
        wallpaper: Boolean = true,
        initial: ThemeSettings = off,
    ) {
        settings.value = initial
        rule.setContent {
            CroftTheme(settings.value, systemDark = false) {
                AppearanceScreen(
                    settings = settings.value, onChange = ::change, onBack = {}, onOpenCustomColour = {}, onOpenLayout = {},
                    onOpenMotion = {}, onReset = onReset, wallpaperAvailable = wallpaper,
                )
            }
        }
    }

    private fun node(tag: String): SemanticsNodeInteraction = rule.onNodeWithTag(tag)

    // ---- S50 ----

    @Test
    fun S50_tappingAPaletteAppliesItAndOffersUndoForFiveSeconds() {
        s50()
        node(AppearanceTags.PALETTE_PREFIX + "wax").performScrollTo()
        rule.mainClock.autoAdvance = false
        node(AppearanceTags.PALETTE_PREFIX + "wax").performClick()
        rule.mainClock.advanceTimeBy(300)
        assertEquals(CroftPreset.WAX, settings.value.preset)
        assertEquals(CroftPreset.WAX.seed, settings.value.seed)
        rule.onNodeWithText("Wax applied").assertIsDisplayed()
        rule.onNodeWithText("Undo").assertIsDisplayed()

        rule.mainClock.advanceTimeBy(4_000) // 4.3 s since the tap: still there
        rule.onNodeWithText("Wax applied").assertExists()
        rule.mainClock.advanceTimeBy(1_500) // past 5 s, plus the fade-out
        rule.onNodeWithText("Wax applied").assertDoesNotExist()
        assertEquals("the palette stays applied", CroftPreset.WAX, settings.value.preset)
    }

    @Test
    fun S50_undoPutsTheOldLookBack() {
        s50()
        node(AppearanceTags.PALETTE_PREFIX + "tide").performScrollTo().performClick()
        assertEquals(CroftPreset.TIDE, settings.value.preset)
        rule.onNodeWithText("Undo").performClick()
        rule.waitForIdle()
        assertEquals(ThemeSettings.Default.copy(animation = AnimationLevel.OFF), settings.value)
    }

    @Test
    fun S50_resetAsksFirst() {
        var resets = 0
        s50(onReset = { resets++ })
        rule.onNodeWithContentDescription("More options").performClick()
        rule.onNodeWithText("Reset appearance").performClick()
        rule.onNodeWithText("Reset appearance?").assertIsDisplayed()
        assertEquals("nothing is reset before the answer", 0, resets)
        rule.onNodeWithText("Cancel").performClick()
        rule.waitForIdle()
        assertEquals(0, resets)
        rule.onNodeWithText("Reset appearance?").assertDoesNotExist()

        rule.onNodeWithContentDescription("More options").performClick()
        rule.onNodeWithText("Reset appearance").performClick()
        node(AppearanceTags.RESET_CONFIRM).performClick()
        rule.waitForIdle()
        assertEquals(1, resets)
    }

    @Test
    fun S50_pureBlackIsOnlyAvailableWhenDarkCanApply() {
        s50(initial = off.copy(mode = ModePreference.LIGHT))
        node(AppearanceTags.PURE_BLACK).performScrollTo().assertIsNotEnabled()
        rule.onNodeWithText("Dark", useUnmergedTree = false).performClick()
        node(AppearanceTags.PURE_BLACK).assertIsEnabled().assertIsOff().performClick()
        assertTrue(settings.value.pureBlack)
        assertEquals(ModePreference.DARK, settings.value.mode)
        node(AppearanceTags.PURE_BLACK).assertIsOn()
    }

    @Test
    fun S50_modeStyleAndContrastChangeTheSettings() {
        s50()
        rule.onNodeWithText("Dark").performScrollTo().performClick()
        assertEquals(ModePreference.DARK, settings.value.mode)
        rule.onNodeWithText("Vibrant").performScrollTo().performClick()
        assertEquals(ThemeStyle.VIBRANT, settings.value.style)
        rule.onNodeWithText("High").performScrollTo().performClick()
        assertEquals(CroftContrast.HIGH, settings.value.contrast)
    }

    @Test
    fun S50_wallpaperColoursAreHiddenBelowAndroid12() {
        s50(wallpaper = false)
        rule.onNodeWithText("Wallpaper").assertDoesNotExist()
        rule.onNodeWithText("Croft").assertExists()
        rule.onNodeWithText("Custom").assertExists()
    }

    @Test
    fun S50_wallpaperColoursAreOfferedOnAndroid12AndLater() {
        s50(wallpaper = true)
        rule.onNodeWithText("Wallpaper").performScrollTo().performClick()
        assertEquals(ColourSource.WALLPAPER, settings.value.source)
    }

    @Test
    fun S50_aThemeChangeDoesNotMoveTheLayoutOfThePreview() {
        // Full animation on purpose: bounds must hold at every frame of the cross-fade, not just at the end.
        s50(initial = ThemeSettings.Default.copy(animation = AnimationLevel.FULL))
        rule.waitForIdle()
        val tags = listOf(PREVIEW_TAG, AppearanceTags.PURE_BLACK, AppearanceTags.PALETTE_PREFIX + "wax")
        val before = tags.associateWith { boundsOf(node(it)) }
        rule.mainClock.autoAdvance = false
        val changes = listOf<(ThemeSettings) -> ThemeSettings>(
            { it.withPreset(CroftPreset.ROWAN) },
            { it.copy(mode = ModePreference.DARK) },
            { it.copy(pureBlack = true) },
            { it.copy(contrast = CroftContrast.HIGH) },
            { it.copy(style = ThemeStyle.MONOCHROME) },
            { it.withCustomSeed(0xFF8A4FFF) },
        )
        var frames = 0
        for (c in changes) {
            testChange(c(settings.value))
            for (frame in 0 until 4) {
                rule.mainClock.advanceTimeByFrame()
                frames++
                tags.forEach { assertEquals("$it moved during the fade (frame $frame)", before.getValue(it), boundsOf(node(it))) }
            }
            rule.mainClock.advanceTimeBy(2_000)
            tags.forEach { assertEquals("$it moved after the change", before.getValue(it), boundsOf(node(it))) }
        }
        assertEquals(changes.size * 4, frames)
    }

    /** Position and size in the root, not clipped by the scroll area, so a node scrolled out of view still has bounds. */
    private fun boundsOf(n: SemanticsNodeInteraction): Rect {
        val node = n.fetchSemanticsNode()
        return Rect(node.positionInRoot, node.size.toSize())
    }

    // ---- S50 flow: files, drill-down ----

    private class FakeFiles(var toImport: String?) : ThemeFiles {
        var exported: String? = null
        override fun export(text: String, onDone: (Boolean) -> Unit) { exported = text; onDone(true) }
        override fun import(onText: (String?) -> Unit) = onText(toImport)
    }

    private fun flow(files: ThemeFiles, initial: ThemeSettings = off, start: AppearanceRoute = AppearanceRoute.S50) {
        settings.value = initial
        rule.setContent {
            CroftTheme(settings.value, systemDark = false) {
                AppearanceFlow(settings.value, ::change, onBack = {}, startAt = start, files = files, photoPicker = null)
            }
        }
    }

    private fun openMenu(item: String) {
        rule.onNodeWithContentDescription("More options").performClick()
        rule.onNodeWithText(item).performClick()
        rule.waitForIdle()
    }

    @Test
    fun S50_exportWritesTheCanonicalFile() {
        val files = FakeFiles(null)
        flow(files, off.withPreset(CroftPreset.HEATHER))
        openMenu("Export theme")
        assertEquals(ThemeJson.encode(settings.value), files.exported)
        rule.onNodeWithText("Theme saved as croft-theme.json").assertIsDisplayed()
    }

    @Test
    fun S50_importAppliesAGoodFile() {
        val incoming = off.copy(mode = ModePreference.DARK, density = DensityLevel.COMPACT).withPreset(CroftPreset.PEAT)
        flow(FakeFiles(ThemeJson.encode(incoming)))
        openMenu("Import theme")
        assertEquals(incoming, settings.value)
        rule.onNodeWithText("Theme imported").assertIsDisplayed()
    }

    private fun importRejects(file: String, message: String) {
        val before = off.withPreset(CroftPreset.OCHRE)
        flow(FakeFiles(file), before)
        openMenu("Import theme")
        rule.onNodeWithText(message).assertIsDisplayed()
        assertEquals("changed by <$file>", before, settings.value)
    }

    @Test
    fun S50_importRejectsAnEmptyFile() = importRejects("", "That file is empty. Choose a croft-theme.json file.")

    @Test
    fun S50_importRejectsACorruptFile() = importRejects("{ not json", "That file is not a theme file. Choose a croft-theme.json file.")

    @Test
    fun S50_importRejectsJsonThatIsNotATheme() = importRejects("[1,2]", "That file is not a Croft theme, so nothing was changed.")

    @Test
    fun S50_importRejectsAnOptionCroftDoesNotHave() = importRejects("""{"mode":"neon"}""", "That file is not a Croft theme, so nothing was changed.")

    @Test
    fun S50_importRejectsAFileFromANewerCroft() = importRejects("""{"version":9}""", "That theme was saved by a newer version of Croft. Update Croft to use it.")

    @Test
    fun S50_importOfAFileThatCannotBeReadSaysSo() {
        flow(FakeFiles(null))
        openMenu("Import theme")
        rule.onNodeWithText("Croft could not read that file.").assertIsDisplayed()
    }

    @Test
    fun S50_resetInTheFlowRestoresMossStandardSystemAfterAsking() {
        flow(FakeFiles(null), off.withPreset(CroftPreset.WAX).copy(density = DensityLevel.COMPACT, textScale = 1.4f))
        openMenu("Reset appearance")
        node(AppearanceTags.RESET_CONFIRM).performClick()
        rule.waitForIdle()
        assertEquals(ThemeSettings.Default, settings.value)
        rule.onNodeWithText("Appearance reset").assertIsDisplayed()
    }

    @Test
    fun S50_drillDownToTheThreeSubScreensAndBack() {
        flow(FakeFiles(null))
        for ((link, tag) in listOf("Layout and density" to LayoutTags.SCREEN, "Motion and haptics" to MotionTags.SCREEN, "Custom colour" to CustomColourTags.SCREEN)) {
            rule.onNodeWithText(link).performScrollTo().performClick()
            rule.waitForIdle()
            node(tag).assertExists()
            node(AppearanceTags.SCREEN).assertDoesNotExist()
            rule.onNodeWithContentDescription("Back").performClick()
            rule.waitForIdle()
            node(AppearanceTags.SCREEN).assertExists()
        }
    }

    // ---- S51 ----

    private fun s51(picker: PhotoSeedPicker? = null, applied: MutableList<ThemeSettings> = mutableListOf()): MutableList<ThemeSettings> {
        settings.value = off
        rule.setContent {
            CroftTheme(settings.value, systemDark = false) {
                CustomColourScreen(settings.value, onApply = { applied += it; change(it) }, onBack = {}, photoPicker = picker)
            }
        }
        return applied
    }

    @Test
    fun S51_hexValueSetsTheSeedAndNothingChangesUntilApply() {
        val applied = s51()
        setHex("8a4fff")
        rule.waitForIdle()
        assertEquals("the app keeps its look until Apply", off, settings.value)
        assertTrue(applied.isEmpty())
        node(CustomColourTags.APPLY).performScrollTo().performClick()
        assertEquals(1, applied.size)
        assertEquals(ColourSource.CUSTOM, applied[0].source)
        assertEquals(0xFF8A4FFF, applied[0].seed)
        assertEquals(0xFF8A4FFF, applied[0].effectiveSeed)
    }

    @Test
    fun S51_aBadHexValueShowsAMessageAndBlocksApply() {
        val applied = s51()
        setHex("12G")
        rule.onNodeWithText("Use six digits, like 4F6F52.").assertExists()
        node(CustomColourTags.APPLY).performScrollTo().assertIsNotEnabled()
        assertTrue(applied.isEmpty())
    }

    @Test
    fun S51_styleAndContrastAreAppliedWithTheSeed() {
        val applied = s51()
        rule.onNodeWithText("Vibrant").performScrollTo().performClick()
        rule.onNodeWithText("High").performScrollTo().performClick()
        node(CustomColourTags.APPLY).performScrollTo().performClick()
        assertEquals(ThemeStyle.VIBRANT, applied.single().style)
        assertEquals(CroftContrast.HIGH, applied.single().contrast)
    }

    @Test
    fun S51_tappingTheWheelPicksThatHueAndKeepsItVisible() {
        val applied = s51()
        node(CustomColourTags.WHEEL).performScrollTo().performTouchInput { click(Offset(width / 2f, 18f)) } // top of the ring: hue 0
        rule.waitForIdle()
        node(CustomColourTags.APPLY).performScrollTo().performClick()
        val hue = McuSchemes.hctOf(applied.single().seed).hue
        assertTrue("hue $hue", hue < 8.0 || hue > 352.0)
        node(CustomColourTags.WHEEL).performScrollTo().performTouchInput { click(Offset(width - 18f, height / 2f)) } // right of the ring: hue 90
        node(CustomColourTags.APPLY).performScrollTo().performClick()
        val hue90 = McuSchemes.hctOf(applied.last().seed).hue
        assertEquals(90.0, hue90, 8.0)
    }

    @Test
    fun S51_aLowChromaSeedOrAGreyStyleShowsTheGreyNote() {
        s51()
        setHex("808080")
        rule.onNodeWithText("This style makes the scheme look nearly grey. Try Vibrant or Expressive for more colour.").performScrollTo().assertExists()
        setHex("8A4FFF")
        rule.onNodeWithText("Colours below 4.5 to 1 contrast are adjusted automatically.").performScrollTo().assertExists()
    }

    @Test
    fun S51_fromPhotoTakesTheSeedFromThePickerOrSaysItFailed() {
        var result: Long? = 0xFF2E7D32
        s51(PhotoSeedPicker { it(result) })
        node(CustomColourTags.FROM_PHOTO).performScrollTo().performClick()
        rule.waitForIdle()
        node(CustomColourTags.HEX).assert(hasEditableText("2E7D32"))
        result = null
        node(CustomColourTags.FROM_PHOTO).performClick()
        rule.onNodeWithText("Croft could not find a colour in that picture.").assertExists()
    }

    // SetText writes the field's value without focusing it, so the blinking cursor never runs in the test.
    private fun setHex(text: String) {
        node(CustomColourTags.HEX).performSemanticsAction(SemanticsActions.SetText) { it(androidx.compose.ui.text.AnnotatedString(text)) }
        rule.waitForIdle()
    }

    private fun hasEditableText(text: String) = androidx.compose.ui.test.SemanticsMatcher("editable text is $text") {
        it.config.getOrNull(SemanticsProperties.EditableText)?.text == text
    }

    // ---- S52 ----

    private val changes = mutableListOf<ThemeSettings>()

    private fun s52(initial: ThemeSettings = off) {
        settings.value = initial
        rule.setContent {
            CroftTheme(settings.value, systemDark = false) {
                LayoutDensityScreen(settings.value, onChange = { changes += it; change(it) }, onBack = {})
            }
        }
    }

    @Test
    fun S52_densityRoundnessListStyleAvatarsAndUnreadChange() {
        s52()
        node(LayoutTags.DENSITY_PREFIX + "compact").performScrollTo().performClick()
        assertEquals(DensityLevel.COMPACT, settings.value.density)
        rule.onNodeWithText("Round").performScrollTo().performClick()
        assertEquals(Roundness.ROUND, settings.value.roundness)
        rule.onNodeWithText("Flat").performScrollTo().performClick()
        assertEquals(com.example.croft.theme.ListStyle.FLAT, settings.value.listStyle)
        rule.onNodeWithText("Hidden").performScrollTo().performClick()
        assertEquals(com.example.croft.theme.AvatarStyle.HIDDEN, settings.value.avatars)
        rule.onNodeWithText("Dot").performScrollTo().performClick()
        assertEquals(com.example.croft.theme.UnreadEmphasis.DOT, settings.value.unreadEmphasis)
    }

    @Test
    fun S52_textSizeSliderRunsFrom85To150PercentInWholeStepsAndShowsThePercent() {
        s52()
        rule.onNodeWithText("100 percent").performScrollTo().assertExists()
        node(LayoutTags.TEXT_SIZE).performScrollTo().performSemanticsAction(SemanticsActions.SetProgress) { it(1.5f) }
        rule.waitForIdle()
        assertEquals(1.5f, settings.value.textScale, 0.001f)
        rule.onNodeWithText("150 percent").assertExists()
        node(LayoutTags.TEXT_SIZE).performSemanticsAction(SemanticsActions.SetProgress) { it(0.1f) }
        rule.waitForIdle()
        assertEquals(0.85f, settings.value.textScale, 0.001f)
        rule.onNodeWithText("85 percent").assertExists()
        node(LayoutTags.TEXT_SIZE).performSemanticsAction(SemanticsActions.SetProgress) { it(1.12f) }
        rule.waitForIdle()
        val pct = Math.round(settings.value.textScale * 100f)
        assertEquals("snaps to a 5 percent step", 0, pct % 5)
    }

    @Test
    fun S52_fontChoicesChangeTheStoredFonts() {
        s52()
        rule.onNodeWithText("Croft Sans").performScrollTo().performClick()
        assertEquals(com.example.croft.theme.UiFont.CROFT_SANS, settings.value.uiFont)
        rule.onNodeWithText("Serif").performScrollTo().performClick()
        assertEquals(com.example.croft.theme.ReadingFont.SERIF, settings.value.readingFont)
    }

    // ---- S53 ----

    @Test
    fun S53_animationAndHapticsLevelsAndTryItButtons() {
        settings.value = off
        val tried = mutableListOf<TryItEvent>()
        rule.setContent {
            CroftTheme(settings.value, systemDark = false) {
                MotionHapticsScreen(settings.value, onChange = ::change, onBack = {}, onTry = { tried += it })
            }
        }
        node(MotionTags.ANIMATION_PREFIX + "reduced").performScrollTo().performClick()
        assertEquals(AnimationLevel.REDUCED, settings.value.animation)
        node(MotionTags.HAPTICS_PREFIX + "full").performScrollTo().performClick()
        assertEquals(HapticsLevel.FULL, settings.value.haptics)
        for (e in TryItEvent.entries) node(MotionTags.TRY_PREFIX + e.name.lowercase()).performScrollTo().performClick()
        assertEquals(TryItEvent.entries, tried)
        node(MotionTags.STAGE).assertExists()
    }

    // ---- S08 ----

    @Test
    fun S08_everyTapAppliesAtOnceAndContinueMovesOn() {
        settings.value = off
        var continued = 0
        rule.setContent {
            CroftTheme(settings.value, systemDark = false) {
                PickYourLookScreen(settings.value, ::change, onBack = {}, onContinue = { continued++ }, wallpaperAvailable = true)
            }
        }
        node(PickLookTags.PALETTE_PREFIX + "slate").performScrollTo().performClick()
        assertEquals(CroftPreset.SLATE, settings.value.preset)
        rule.onNodeWithText("Dark").performClick()
        assertEquals(ModePreference.DARK, settings.value.mode)
        node(PickLookTags.WALLPAPER).performScrollTo().performClick()
        assertEquals(ColourSource.WALLPAPER, settings.value.source)
        node(PickLookTags.CONTINUE).performScrollTo().performClick()
        assertEquals(1, continued)
    }

    @Test
    fun S08_wallpaperColoursAreNotOfferedBelowAndroid12() {
        settings.value = off
        rule.setContent {
            CroftTheme(settings.value, systemDark = false) {
                PickYourLookScreen(settings.value, ::change, onBack = {}, onContinue = {}, wallpaperAvailable = false)
            }
        }
        node(PickLookTags.WALLPAPER).assertDoesNotExist()
        rule.onNodeWithText("Use my wallpaper colours").assertDoesNotExist()
    }

    @Test
    fun theLivePreviewIsNotInteractive() {
        settings.value = off
        rule.setContent { CroftTheme(settings.value, systemDark = false) { AppearancePreview() } }
        val n = node(PREVIEW_TAG)
        n.assertExists()
        // Its controls are swallowed: a tap on the chips changes nothing and nothing in it is exposed to a screen reader.
        n.performTouchInput { click(center) }
        rule.onNodeWithText("Compose").assertDoesNotExist() // children are hidden from semantics
        assertNotNull(n.fetchSemanticsNode().config.getOrNull(SemanticsProperties.ContentDescription))
        assertFalse(n.fetchSemanticsNode().children.any { it.config.contains(SemanticsActions.OnClick) })
        assertNull(null)
    }
}
