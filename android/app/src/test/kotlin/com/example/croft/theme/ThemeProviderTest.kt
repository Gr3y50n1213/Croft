// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.theme

import android.provider.Settings
import androidx.compose.foundation.shape.CornerBasedShape
import androidx.compose.foundation.shape.CornerSize
import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.SideEffect
import androidx.compose.runtime.snapshots.Snapshot
import androidx.compose.runtime.mutableStateOf
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.unit.Density
import androidx.compose.ui.unit.dp
import androidx.test.core.app.ApplicationProvider
import com.example.croft.CroftTheme
import com.example.croft.design.CroftContrast
import com.example.croft.design.CroftMode
import com.example.croft.design.CroftPreset
import com.example.croft.design.CroftSchemes
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotEquals
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import org.robolectric.annotation.Config

/** The theme provider on a simulated device: settings in one CompositionLocal, cross-fade, reduced motion, shapes, text size. */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35])
class ThemeProviderTest {
    @get:Rule
    val rule = createComposeRule()

    private val settings = mutableStateOf(ThemeSettings.Default)
    private var primary = Color.Unspecified
    private var seenSettings: ThemeSettings? = null
    private var fontScale = 0f
    private var motion: MotionLevel? = null
    private var mediumCorner = 0f

    private fun show(initial: ThemeSettings, systemDark: Boolean = false) {
        settings.value = initial
        rule.setContent {
            CroftTheme(settings.value, systemDark = systemDark) {
                val p = MaterialTheme.colorScheme.primary
                val s = LocalThemeSettings.current
                val f = LocalDensity.current.fontScale
                val m = LocalMotionLevel.current
                val c = (MaterialTheme.shapes.medium as CornerBasedShape).topStart.toPx(Size(1000f, 1000f), Density(1f))
                SideEffect { primary = p; seenSettings = s; fontScale = f; motion = m; mediumCorner = c }
            }
        }
    }

    /** Write the state on the UI thread so the change is seen at the next frame even when the clock is paused. */
    private fun set(value: ThemeSettings) = rule.runOnUiThread { settings.value = value; Snapshot.sendApplyNotifications() }

    private fun table(p: CroftPreset, mode: CroftMode = CroftMode.LIGHT) = Color(CroftSchemes.roles(p, mode, CroftContrast.STANDARD).primary)

    @Test
    fun settingsReachEveryComposableThroughOneCompositionLocal() {
        val s = ThemeSettings.Default.copy(density = DensityLevel.COMPACT, uiFont = UiFont.ATKINSON)
        show(s)
        rule.waitForIdle()
        assertEquals(s, seenSettings)
    }

    @Test
    fun theDefaultIsMossInLightFromTheGeneratedTable() {
        show(ThemeSettings.Default)
        rule.waitForIdle()
        assertEquals(table(CroftPreset.MOSS), primary)
    }

    @Test
    fun aThemeChangeCrossFadesAtFullAnimationAndEndsOnTheNewColours() {
        show(ThemeSettings.Default.copy(animation = AnimationLevel.FULL))
        rule.waitForIdle()
        rule.mainClock.autoAdvance = false
        set(ThemeSettings.Default.copy(animation = AnimationLevel.FULL).withPreset(CroftPreset.WAX))
        rule.mainClock.advanceTimeByFrame()
        rule.mainClock.advanceTimeByFrame()
        rule.mainClock.advanceTimeBy(40)
        val mid = primary
        assertNotEquals("still the old colour in the middle of the fade", table(CroftPreset.MOSS), mid)
        assertNotEquals("already the new colour in the middle of the fade", table(CroftPreset.WAX), mid)
        rule.mainClock.advanceTimeBy(3_000)
        assertEquals(table(CroftPreset.WAX), primary)
    }

    @Test
    fun withReducedAnimationAThemeChangeIsAnInstantSwap() = instantAt(AnimationLevel.REDUCED)

    @Test
    fun withAnimationOffAThemeChangeIsAnInstantSwap() = instantAt(AnimationLevel.OFF)

    private fun instantAt(level: AnimationLevel) {
        show(ThemeSettings.Default.copy(animation = level))
        rule.waitForIdle()
        rule.mainClock.autoAdvance = false
        set(ThemeSettings.Default.copy(animation = level).withPreset(CroftPreset.WAX))
        rule.mainClock.advanceTimeByFrame()
        assertEquals("no cross-fade at $level; seen=${seenSettings?.preset} motion=$motion state=${settings.value.preset}", table(CroftPreset.WAX), primary)
    }

    @Test
    fun followSystemMeansOffWhenAndroidRemovesAnimations() {
        assertEquals(MotionLevel.FULL, resolveMotionLevel(AnimationLevel.SYSTEM, 1f))
        assertEquals(MotionLevel.FULL, resolveMotionLevel(AnimationLevel.SYSTEM, 0.5f))
        assertEquals(MotionLevel.OFF, resolveMotionLevel(AnimationLevel.SYSTEM, 0f))
        assertEquals(MotionLevel.FULL, resolveMotionLevel(AnimationLevel.FULL, 0f))
        assertEquals(MotionLevel.REDUCED, resolveMotionLevel(AnimationLevel.REDUCED, 1f))
        assertEquals(MotionLevel.OFF, resolveMotionLevel(AnimationLevel.OFF, 1f))

        val context = ApplicationProvider.getApplicationContext<android.content.Context>()
        Settings.Global.putFloat(context.contentResolver, Settings.Global.ANIMATOR_DURATION_SCALE, 0f)
        show(ThemeSettings.Default)
        rule.waitForIdle()
        assertEquals(MotionLevel.OFF, motion)
    }

    @Test
    fun aChangeInTheMiddleOfAFadeStartsFromWhatIsOnScreen() {
        val base = ThemeSettings.Default.copy(animation = AnimationLevel.FULL)
        show(base)
        rule.waitForIdle()
        rule.mainClock.autoAdvance = false
        set(base.withPreset(CroftPreset.WAX))
        rule.mainClock.advanceTimeByFrame()
        rule.mainClock.advanceTimeBy(60)
        val before = primary
        set(base.withPreset(CroftPreset.SLATE))
        rule.mainClock.advanceTimeByFrame()
        val after = primary
        // One frame after the second tap the colour is where it was, not back at Moss and not already Slate.
        assertTrue("jumped: %06X -> %06X".format(before.value.toLong() ushr 32, after.value.toLong() ushr 32), distance(before, after) < distance(table(CroftPreset.MOSS), table(CroftPreset.SLATE)) * 0.3f)
        rule.mainClock.advanceTimeBy(3_000)
        assertEquals(table(CroftPreset.SLATE), primary)
    }

    private fun distance(a: Color, b: Color): Float =
        Math.abs(a.red - b.red) + Math.abs(a.green - b.green) + Math.abs(a.blue - b.blue)

    @Test
    fun textSizeIsMultipliedWithTheSystemFontSize() {
        RuntimeEnvironment.setFontScale(1.3f)
        show(ThemeSettings.Default.copy(textScale = 1.5f))
        rule.waitForIdle()
        assertEquals(1.95f, fontScale, 0.001f)
        // 85 percent of the system size at the smallest, 200 percent combined at the largest the design checks.
        set(ThemeSettings.Default.copy(textScale = 0.85f))
        rule.waitForIdle()
        assertEquals(1.105f, fontScale, 0.001f)
    }

    @Test
    fun roundnessScalesTheMaterialCornersByTheShapeTokens() {
        show(ThemeSettings.Default.copy(roundness = Roundness.STANDARD))
        rule.waitForIdle()
        val standard = mediumCorner
        assertTrue(standard > 0f)
        for (r in Roundness.entries) {
            set(ThemeSettings.Default.copy(roundness = r))
            rule.waitForIdle()
            assertEquals("roundness $r", standard * r.scale, mediumCorner, 0.01f)
        }
        // The swipe card (17 dp at Standard) follows the same scale: 10, 17 and 24 dp from the swipe tokens.
        assertEquals(10, Roundness.TIGHT.swipeCardRadiusDp)
        assertEquals(17, Roundness.STANDARD.swipeCardRadiusDp)
        assertEquals(24, Roundness.ROUND.swipeCardRadiusDp)
        assertEquals(Roundness.TIGHT.swipeCardRadiusDp.toFloat(), 17 * Roundness.TIGHT.scale, 0.5f)
        assertEquals(Roundness.ROUND.swipeCardRadiusDp.toFloat(), 17 * Roundness.ROUND.scale, 0.5f)
    }

    @Test
    fun aScaledCornerIsNeverMoreThanHalfTheComponentHeight() {
        val corner = ScaledCornerSize(CornerSize(28.dp), Roundness.ROUND.scale)
        val density = Density(1f)
        assertEquals(24f, corner.toPx(Size(200f, 48f), density), 0.001f) // 28 x 1.4 = 39.2, capped at 24
        assertEquals(39.2f, corner.toPx(Size(200f, 200f), density), 0.001f)
        assertEquals(16.8f, ScaledCornerSize(CornerSize(28.dp), Roundness.TIGHT.scale).toPx(Size(200f, 200f), density), 0.001f)
    }

    @Test
    fun wallpaperColoursAreOnlyOfferedOnAndroid12AndLater() {
        assertTrue(!wallpaperColoursAvailable(25))
        assertTrue(!wallpaperColoursAvailable(30))
        assertTrue(wallpaperColoursAvailable(31))
        assertTrue(wallpaperColoursAvailable(36))
    }

    @Test
    fun pureBlackAppliesInDarkAndTheBackgroundIsBlack() {
        show(ThemeSettings.Default.copy(mode = ModePreference.DARK, pureBlack = true))
        rule.waitForIdle()
        assertEquals(Color(CroftSchemes.roles(CroftPreset.MOSS, CroftMode.PURE_BLACK, CroftContrast.STANDARD).primary), primary)
    }
}
