// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.theme

import com.example.croft.design.CroftColorRoles
import com.example.croft.design.CroftContrast
import com.example.croft.design.CroftMode
import com.example.croft.design.CroftPreset
import com.example.croft.design.CroftSchemes

/**
 * Turns [ThemeSettings] into colour roles. A preset with its own style comes from the generated tables
 * (`CroftSchemes`); everything else (custom seed, another style on a preset) is computed at runtime with the
 * Material Color Utilities. [runtimeRoles] is also what the golden test compares with the tables.
 */
object ThemeEngine {
    /** Light, dark or pure black, from the setting and the system's dark mode. Pure black only applies in dark. */
    fun resolveMode(s: ThemeSettings, systemDark: Boolean): CroftMode {
        val dark = when (s.mode) {
            ModePreference.SYSTEM -> systemDark
            ModePreference.LIGHT -> false
            ModePreference.DARK -> true
        }
        return when {
            !dark -> CroftMode.LIGHT
            s.pureBlack -> CroftMode.PURE_BLACK
            else -> CroftMode.DARK
        }
    }

    /** The base surfaces pure black replaces (docs/ux/theme.md; same four roles as the generator). */
    fun withPureBlack(r: CroftColorRoles): CroftColorRoles = r.copy(
        background = BLACK, surface = BLACK, surfaceDim = BLACK, surfaceContainerLowest = BLACK,
    )

    /** True when the generated table holds this exact choice, so it needs no runtime computation. */
    fun isPrecomputed(s: ThemeSettings): Boolean =
        s.source != ColourSource.CUSTOM && s.style.id == s.preset.style

    /** Colour roles for [s] in [mode]. */
    fun roles(s: ThemeSettings, mode: CroftMode): CroftColorRoles =
        if (isPrecomputed(s)) CroftSchemes.roles(s.preset, mode, s.contrast)
        else runtimeRoles(s.effectiveSeed, s.style, mode, s.contrast)

    /** Runtime generation for any seed, style, mode and contrast. */
    fun runtimeRoles(seed: Long, style: ThemeStyle, mode: CroftMode, contrast: CroftContrast): CroftColorRoles {
        val base = McuSchemes.roles(seed, style, mode != CroftMode.LIGHT, contrast.level.toDouble())
        return if (mode == CroftMode.PURE_BLACK) withPureBlack(base) else base
    }

    /** The generated style of a preset as a [ThemeStyle] (its id is stored in the tokens). */
    fun styleOf(p: CroftPreset): ThemeStyle = ThemeStyle.entries.first { it.id == p.style }

    /** Pure black, the one colour the theme defines itself: #000000 (docs/ux/theme.md). */
    const val PURE_BLACK = 0xFF000000L
    private const val BLACK = PURE_BLACK
}
