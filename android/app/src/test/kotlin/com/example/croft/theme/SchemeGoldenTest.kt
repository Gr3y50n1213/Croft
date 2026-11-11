// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.theme

import com.example.croft.design.CroftColorRoles
import com.example.croft.design.CroftContrast
import com.example.croft.design.CroftMode
import com.example.croft.design.CroftPreset
import com.example.croft.design.CroftSchemes
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertSame
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * Cross-check (docs/tasks/T12): the Material Color Utilities run at runtime must give exactly the colours of the
 * generated tables, which the TypeScript library produced (T08's golden values). A difference is a bug in this task.
 */
class SchemeGoldenTest {
    private fun diff(expected: CroftColorRoles, actual: CroftColorRoles): List<String> =
        CroftColorRoles::class.java.declaredFields.filter { !it.isSynthetic && !java.lang.reflect.Modifier.isStatic(it.modifiers) }.mapNotNull { f ->
            f.isAccessible = true
            val e = f.get(expected) as Long
            val a = f.get(actual) as Long
            if (e != a) "${f.name}: table %06X, runtime %06X".format(e and 0xFFFFFF, a and 0xFFFFFF) else null
        }

    @Test
    fun runtimeGenerationReproducesTheGeneratedTableForEveryPresetModeAndContrast() {
        var checked = 0
        val problems = mutableListOf<String>()
        for (preset in CroftPreset.entries) for (mode in CroftMode.entries) for (contrast in CroftContrast.entries) {
            val runtime = ThemeEngine.runtimeRoles(preset.seed, ThemeEngine.styleOf(preset), mode, contrast)
            val d = diff(CroftSchemes.roles(preset, mode, contrast), runtime)
            if (d.isNotEmpty()) problems += "$preset $mode $contrast -> $d"
            checked++
        }
        assertEquals(8 * 3 * 3, checked)
        assertTrue("runtime differs from the generated table:\n" + problems.joinToString("\n"), problems.isEmpty())
    }

    @Test
    fun aPresetWithItsOwnStyleReadsTheTableAndOtherChoicesAreComputed() {
        val moss = ThemeSettings.Default
        assertTrue(ThemeEngine.isPrecomputed(moss))
        assertSame(CroftSchemes.roles(CroftPreset.MOSS, CroftMode.DARK, CroftContrast.STANDARD), ThemeEngine.roles(moss, CroftMode.DARK))

        val vibrant = moss.copy(style = ThemeStyle.VIBRANT)
        assertTrue(!ThemeEngine.isPrecomputed(vibrant))
        assertNotEquals(ThemeEngine.roles(moss, CroftMode.LIGHT).primary, ThemeEngine.roles(vibrant, CroftMode.LIGHT).primary)

        val custom = moss.withCustomSeed(0xFF8A4FFF)
        assertNotEquals(ThemeEngine.roles(moss, CroftMode.LIGHT).primary, ThemeEngine.roles(custom, CroftMode.LIGHT).primary)
        assertEquals(0xFF8A4FFF, custom.effectiveSeed)
    }

    @Test
    fun everyStyleProducesOpaqueRolesForALowChromaSeed() {
        // Very low chroma seeds are allowed (docs/ux/theme.md); the library must still produce a scheme.
        for (style in ThemeStyle.entries) for (mode in CroftMode.entries) {
            val r = ThemeEngine.runtimeRoles(0xFF808080, style, mode, CroftContrast.STANDARD)
            assertEquals("$style $mode primary opaque", 0xFFL, r.primary ushr 24)
            assertEquals(0xFFL, r.surface ushr 24)
        }
    }

    @Test
    fun pureBlackIsTheDarkSchemeWithFourBlackBaseSurfaces() {
        for (preset in CroftPreset.entries) {
            val dark = ThemeEngine.runtimeRoles(preset.seed, ThemeStyle.TONAL_SPOT, CroftMode.DARK, CroftContrast.STANDARD)
            val black = ThemeEngine.runtimeRoles(preset.seed, ThemeStyle.TONAL_SPOT, CroftMode.PURE_BLACK, CroftContrast.STANDARD)
            assertEquals(0xFF000000, black.background)
            assertEquals(0xFF000000, black.surface)
            assertEquals(0xFF000000, black.surfaceDim)
            assertEquals(0xFF000000, black.surfaceContainerLowest)
            assertEquals(dark.surfaceContainer, black.surfaceContainer)
            assertEquals(dark.primary, black.primary)
        }
    }

    @Test
    fun modeResolvesFromTheSettingAndTheSystem() {
        val s = ThemeSettings.Default
        assertEquals(CroftMode.LIGHT, ThemeEngine.resolveMode(s, systemDark = false))
        assertEquals(CroftMode.DARK, ThemeEngine.resolveMode(s, systemDark = true))
        assertEquals(CroftMode.LIGHT, ThemeEngine.resolveMode(s.copy(mode = ModePreference.LIGHT, pureBlack = true), systemDark = true))
        assertEquals(CroftMode.DARK, ThemeEngine.resolveMode(s.copy(mode = ModePreference.DARK), systemDark = false))
        assertEquals(CroftMode.PURE_BLACK, ThemeEngine.resolveMode(s.copy(mode = ModePreference.DARK, pureBlack = true), systemDark = false))
        // Pure black waits for dark: it does nothing while the system is light.
        assertEquals(CroftMode.LIGHT, ThemeEngine.resolveMode(s.copy(pureBlack = true), systemDark = false))
        assertEquals(CroftMode.PURE_BLACK, ThemeEngine.resolveMode(s.copy(pureBlack = true), systemDark = true))
    }

    @Test
    fun aPhotoSeedComesFromTheColourLibrary() {
        val green = IntArray(400) { 0xFF2E7D32.toInt() }
        val seed = McuSchemes.seedFromPixels(green)
        assertNotNull(seed)
        assertEquals(0xFF2E7D32, seed)
        assertNull(McuSchemes.seedFromPixels(IntArray(10) { 0 }))
        assertNull(McuSchemes.seedFromPixels(IntArray(0)))
    }

    @Test
    fun hctRoundTripKeepsTheHue() {
        val hct = McuSchemes.hctOf(CroftPreset.WAX.seed)
        val again = McuSchemes.hctOf(McuSchemes.fromHct(hct.hue, hct.chroma, hct.tone))
        assertEquals(hct.hue, again.hue, 1.5)
    }

    @Test
    fun contrastRatioIsTheWcagFormula() {
        assertEquals(21.0, Contrast.ratio(0xFF000000, 0xFFFFFFFF), 1e-9)
        assertEquals(1.0, Contrast.ratio(0xFF336699, 0xFF336699), 1e-9)
        assertEquals(4.0, Contrast.ratio(0xFF777777, 0xFFFFFFFF), 0.6)
    }
}
