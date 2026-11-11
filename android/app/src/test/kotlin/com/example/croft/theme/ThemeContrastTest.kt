// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.theme

import com.example.croft.design.CroftColorRoles
import com.example.croft.design.CroftContrast
import com.example.croft.design.CroftMode
import com.example.croft.design.CroftPreset
import com.example.croft.design.CroftSchemes
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.junit.runners.Parameterized

/**
 * Text and icon colours against their containers, for every preset (8) x mode (light, dark, pure black) x contrast
 * (standard, medium, high): 4.5 to 1 for text and icons, 3 to 1 for graphics, and 7 to 1 for text at High contrast
 * (docs/ux/theme.md, "Acceptance checks"). Each case is its own test, named by preset, mode and contrast; a failure
 * message lists every pair that is below its minimum.
 */
@RunWith(Parameterized::class)
class ThemeContrastTest(private val preset: CroftPreset, private val mode: CroftMode, private val contrast: CroftContrast) {
    companion object {
        @JvmStatic
        @Parameterized.Parameters(name = "{0} {1} {2}")
        fun cases(): List<Array<Any>> = CroftPreset.entries.flatMap { p ->
            CroftMode.entries.flatMap { m -> CroftContrast.entries.map { c -> arrayOf<Any>(p, m, c) } }
        }
    }

    private class Pair(val name: String, val fg: (CroftColorRoles) -> Long, val bg: (CroftColorRoles) -> Long, val graphic: Boolean = false)

    private val pairs: List<Pair> = buildList {
        fun text(name: String, fg: (CroftColorRoles) -> Long, bg: (CroftColorRoles) -> Long) = add(Pair(name, fg, bg))
        text("onPrimary on primary", { it.onPrimary }, { it.primary })
        text("onPrimaryContainer on primaryContainer", { it.onPrimaryContainer }, { it.primaryContainer })
        text("onSecondary on secondary", { it.onSecondary }, { it.secondary })
        text("onSecondaryContainer on secondaryContainer", { it.onSecondaryContainer }, { it.secondaryContainer })
        text("onTertiary on tertiary", { it.onTertiary }, { it.tertiary })
        text("onTertiaryContainer on tertiaryContainer", { it.onTertiaryContainer }, { it.tertiaryContainer })
        text("onError on error", { it.onError }, { it.error })
        text("onErrorContainer on errorContainer", { it.onErrorContainer }, { it.errorContainer })
        text("onBackground on background", { it.onBackground }, { it.background })
        text("onSurface on surface", { it.onSurface }, { it.surface })
        text("onSurface on surfaceContainerLowest", { it.onSurface }, { it.surfaceContainerLowest })
        text("onSurface on surfaceContainerLow", { it.onSurface }, { it.surfaceContainerLow })
        text("onSurface on surfaceContainer", { it.onSurface }, { it.surfaceContainer })
        text("onSurface on surfaceContainerHigh", { it.onSurface }, { it.surfaceContainerHigh })
        text("onSurface on surfaceContainerHighest", { it.onSurface }, { it.surfaceContainerHighest })
        text("onSurface on surfaceDim", { it.onSurface }, { it.surfaceDim })
        text("onSurface on surfaceBright", { it.onSurface }, { it.surfaceBright })
        text("onSurfaceVariant on surface", { it.onSurfaceVariant }, { it.surface })
        text("onSurfaceVariant on surfaceVariant", { it.onSurfaceVariant }, { it.surfaceVariant })
        text("onSurfaceVariant on surfaceContainerLowest", { it.onSurfaceVariant }, { it.surfaceContainerLowest })
        text("onSurfaceVariant on surfaceContainerLow", { it.onSurfaceVariant }, { it.surfaceContainerLow })
        text("onSurfaceVariant on surfaceContainer", { it.onSurfaceVariant }, { it.surfaceContainer })
        text("onSurfaceVariant on surfaceContainerHigh", { it.onSurfaceVariant }, { it.surfaceContainerHigh })
        text("onSurfaceVariant on surfaceContainerHighest", { it.onSurfaceVariant }, { it.surfaceContainerHighest })
        text("inverseOnSurface on inverseSurface", { it.inverseOnSurface }, { it.inverseSurface })
        text("inversePrimary on inverseSurface", { it.inversePrimary }, { it.inverseSurface })
        text("primary on surface", { it.primary }, { it.surface })
        text("primary on surfaceContainerLowest", { it.primary }, { it.surfaceContainerLowest })
        text("error on surface", { it.error }, { it.surface })
        text("onPrimaryFixed on primaryFixed", { it.onPrimaryFixed }, { it.primaryFixed })
        text("onPrimaryFixedVariant on primaryFixed", { it.onPrimaryFixedVariant }, { it.primaryFixed })
        text("onSecondaryFixed on secondaryFixed", { it.onSecondaryFixed }, { it.secondaryFixed })
        text("onSecondaryFixedVariant on secondaryFixed", { it.onSecondaryFixedVariant }, { it.secondaryFixed })
        text("onTertiaryFixed on tertiaryFixed", { it.onTertiaryFixed }, { it.tertiaryFixed })
        text("onTertiaryFixedVariant on tertiaryFixed", { it.onTertiaryFixedVariant }, { it.tertiaryFixed })
        // Graphics: borders and outlines of controls.
        add(Pair("outline on surface (graphic)", { it.outline }, { it.surface }, graphic = true))
        add(Pair("outline on background (graphic)", { it.outline }, { it.background }, graphic = true))
    }

    private fun check(roles: CroftColorRoles, label: String) {
        val failures = pairs.mapNotNull { p ->
            val ratio = Contrast.ratio(p.fg(roles), p.bg(roles))
            val min = when {
                p.graphic -> Contrast.LARGE
                contrast == CroftContrast.HIGH -> Contrast.HIGH
                else -> Contrast.NORMAL
            }
            if (ratio + 1e-9 < min) "${p.name}: %.2f < %.1f".format(ratio, min) else null
        }
        assertTrue("$label $preset $mode $contrast below minimum: $failures", failures.isEmpty())
    }

    @Test
    fun generatedTableMeetsContrast() = check(CroftSchemes.roles(preset, mode, contrast), "generated")

    @Test
    fun runtimeSchemeMeetsContrast() =
        check(ThemeEngine.runtimeRoles(preset.seed, ThemeEngine.styleOf(preset), mode, contrast), "runtime")
}
