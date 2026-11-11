// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.theme

import androidx.compose.ui.graphics.toArgb
import com.example.croft.design.CroftContrast
import com.example.croft.design.CroftMode
import com.example.croft.design.CroftPreset
import com.example.croft.design.CroftSchemes
import com.example.croft.design.CroftStatus
import com.example.croft.design.toColorScheme
import com.example.croft.swipe.SwipeAction
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.junit.runners.Parameterized

/**
 * docs/ux/swipe.md, "Contrast": the icon takes the colour for content on its pill, so it meets 3 to 1 against the pill in every
 * theme: below the arm point (the tonal pill) and armed (the full colour), for every action, every preset, every mode and every
 * contrast level. Each case is its own test; a failure lists the pairs.
 */
@RunWith(Parameterized::class)
class SwipePaletteTest(private val preset: CroftPreset, private val mode: CroftMode, private val contrast: CroftContrast) {
    companion object {
        @JvmStatic
        @Parameterized.Parameters(name = "{0} {1} {2}")
        fun cases(): List<Array<Any>> = CroftPreset.entries.flatMap { p ->
            CroftMode.entries.flatMap { m -> CroftContrast.entries.map { c -> arrayOf<Any>(p, m, c) } }
        }
    }

    private fun argb(c: androidx.compose.ui.graphics.Color) = c.toArgb().toLong() and 0xFFFFFFFFL

    @Test
    fun theIconMeetsThreeToOneOnItsPillAtEveryStage() {
        val scheme = CroftSchemes.roles(preset, mode, contrast).toColorScheme(mode)
        val a = CroftStatus.amber(mode, contrast)
        val amber = StatusColors(androidx.compose.ui.graphics.Color(a.color), androidx.compose.ui.graphics.Color(a.onContainer), androidx.compose.ui.graphics.Color(a.container))
        val failures = ArrayList<String>()
        for (action in SwipeAction.entries) for (available in listOf(true, false)) {
            val p = SwipePalette.of(action, available, scheme, amber)
            val tonal = Contrast.ratio(argb(p.onContainer), argb(p.container))
            val full = Contrast.ratio(argb(p.onFull), argb(p.full))
            if (tonal + 1e-9 < Contrast.LARGE) failures += "$action available=$available tonal: %.2f".format(tonal)
            // The armed pill is drawn only for an available action, but the pair must hold anyway.
            if (full + 1e-9 < Contrast.LARGE) failures += "$action available=$available armed: %.2f".format(full)
        }
        assertTrue("swipe pill $preset $mode $contrast: $failures", failures.isEmpty())
    }

    @Test
    fun theTonalAndFullPillsOfAnActionDiffer_soArmingIsSeenInColour() {
        val scheme = CroftSchemes.roles(preset, mode, contrast).toColorScheme(mode)
        val a = CroftStatus.amber(mode, contrast)
        val amber = StatusColors(androidx.compose.ui.graphics.Color(a.color), androidx.compose.ui.graphics.Color(a.onContainer), androidx.compose.ui.graphics.Color(a.container))
        for (action in SwipeAction.entries.filter { it != SwipeAction.NONE }) {
            val p = SwipePalette.of(action, true, scheme, amber)
            assertTrue("$action: tonal and full are the same colour", p.container != p.full)
        }
        val grey = SwipePalette.of(SwipeAction.ARCHIVE, false, scheme, amber)
        assertTrue("an unavailable action never changes colour", grey.container == grey.full)
    }
}
