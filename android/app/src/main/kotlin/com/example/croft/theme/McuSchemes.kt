// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

@file:Suppress("RestrictedApi")

package com.example.croft.theme

import com.example.croft.design.CroftColorRoles
import com.google.android.material.color.utilities.DynamicScheme
import com.google.android.material.color.utilities.Hct
import com.google.android.material.color.utilities.MaterialDynamicColors
import com.google.android.material.color.utilities.QuantizerCelebi
import com.google.android.material.color.utilities.SchemeExpressive
import com.google.android.material.color.utilities.SchemeFidelity
import com.google.android.material.color.utilities.SchemeMonochrome
import com.google.android.material.color.utilities.SchemeNeutral
import com.google.android.material.color.utilities.SchemeTonalSpot
import com.google.android.material.color.utilities.SchemeVibrant
import com.google.android.material.color.utilities.Score

/**
 * The only place that touches Google's Material Color Utilities (Apache-2.0, the `color.utilities` package of
 * `com.google.android.material:material`; there is no separate artifact). It is the same HCT model the generator
 * uses (`@material/material-color-utilities`, TypeScript) and the one Android's own dynamic colour uses, so no colour
 * maths is written in Croft. Pure functions, no Android classes: they run on the JVM.
 */
internal object McuSchemes {
    private val dynamicColors = MaterialDynamicColors()

    /** Every colour role for a seed, style and contrast level (0, 0.5 or 1; the library accepts -1 to 1). */
    fun roles(seed: Long, style: ThemeStyle, dark: Boolean, contrastLevel: Double): CroftColorRoles {
        val scheme = scheme(seed, style, dark, contrastLevel)
        fun c(role: com.google.android.material.color.utilities.DynamicColor): Long = role.getArgb(scheme).toLong() and 0xFFFFFFFFL
        val m = dynamicColors
        return CroftColorRoles(
            primary = c(m.primary()), onPrimary = c(m.onPrimary()),
            primaryContainer = c(m.primaryContainer()), onPrimaryContainer = c(m.onPrimaryContainer()),
            inversePrimary = c(m.inversePrimary()),
            secondary = c(m.secondary()), onSecondary = c(m.onSecondary()),
            secondaryContainer = c(m.secondaryContainer()), onSecondaryContainer = c(m.onSecondaryContainer()),
            tertiary = c(m.tertiary()), onTertiary = c(m.onTertiary()),
            tertiaryContainer = c(m.tertiaryContainer()), onTertiaryContainer = c(m.onTertiaryContainer()),
            background = c(m.background()), onBackground = c(m.onBackground()),
            surface = c(m.surface()), onSurface = c(m.onSurface()),
            surfaceVariant = c(m.surfaceVariant()), onSurfaceVariant = c(m.onSurfaceVariant()),
            surfaceTint = c(m.surfaceTint()),
            inverseSurface = c(m.inverseSurface()), inverseOnSurface = c(m.inverseOnSurface()),
            error = c(m.error()), onError = c(m.onError()),
            errorContainer = c(m.errorContainer()), onErrorContainer = c(m.onErrorContainer()),
            outline = c(m.outline()), outlineVariant = c(m.outlineVariant()), scrim = c(m.scrim()),
            surfaceBright = c(m.surfaceBright()), surfaceContainer = c(m.surfaceContainer()),
            surfaceContainerHigh = c(m.surfaceContainerHigh()), surfaceContainerHighest = c(m.surfaceContainerHighest()),
            surfaceContainerLow = c(m.surfaceContainerLow()), surfaceContainerLowest = c(m.surfaceContainerLowest()),
            surfaceDim = c(m.surfaceDim()),
            primaryFixed = c(m.primaryFixed()), primaryFixedDim = c(m.primaryFixedDim()),
            onPrimaryFixed = c(m.onPrimaryFixed()), onPrimaryFixedVariant = c(m.onPrimaryFixedVariant()),
            secondaryFixed = c(m.secondaryFixed()), secondaryFixedDim = c(m.secondaryFixedDim()),
            onSecondaryFixed = c(m.onSecondaryFixed()), onSecondaryFixedVariant = c(m.onSecondaryFixedVariant()),
            tertiaryFixed = c(m.tertiaryFixed()), tertiaryFixedDim = c(m.tertiaryFixedDim()),
            onTertiaryFixed = c(m.onTertiaryFixed()), onTertiaryFixedVariant = c(m.onTertiaryFixedVariant()),
        )
    }

    private fun scheme(seed: Long, style: ThemeStyle, dark: Boolean, contrastLevel: Double): DynamicScheme {
        val hct = Hct.fromInt(seed.toInt())
        return when (style) {
            ThemeStyle.TONAL_SPOT -> SchemeTonalSpot(hct, dark, contrastLevel)
            ThemeStyle.NEUTRAL -> SchemeNeutral(hct, dark, contrastLevel)
            ThemeStyle.VIBRANT -> SchemeVibrant(hct, dark, contrastLevel)
            ThemeStyle.EXPRESSIVE -> SchemeExpressive(hct, dark, contrastLevel)
            ThemeStyle.FIDELITY -> SchemeFidelity(hct, dark, contrastLevel)
            ThemeStyle.MONOCHROME -> SchemeMonochrome(hct, dark, contrastLevel)
        }
    }

    /** Hue (0 to 360), chroma and tone of an opaque ARGB colour. */
    data class HctColour(val hue: Double, val chroma: Double, val tone: Double)

    fun hctOf(argb: Long): HctColour = Hct.fromInt(argb.toInt()).let { HctColour(it.hue, it.chroma, it.tone) }

    /** The opaque ARGB colour for a hue, chroma and tone; the library finds the nearest colour a screen can show. */
    fun fromHct(hue: Double, chroma: Double, tone: Double): Long =
        Hct.from(hue.mod(360.0), chroma.coerceAtLeast(0.0), tone.coerceIn(0.0, 100.0)).toInt().toLong() and 0xFFFFFFFFL

    /**
     * The seed for a photo: the library quantizes the pixels and scores the palette for the most pleasant, usable
     * colour. Null for an image with no opaque pixels.
     */
    fun seedFromPixels(argbPixels: IntArray): Long? {
        val opaque = argbPixels.filter { (it ushr 24) == 0xFF }.toIntArray()
        if (opaque.isEmpty()) return null
        val ranked = Score.score(QuantizerCelebi.quantize(opaque, 128))
        return ranked.firstOrNull()?.toLong()?.and(0xFFFFFFFFL)
    }
}
