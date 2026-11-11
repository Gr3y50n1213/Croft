// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.theme

import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.graphics.Color
import com.example.croft.design.CroftMode

/**
 * The amber of a starred message and of the Server problem chip (docs/ux/message-list.md, wireframes: "amber = star and
 * snooze"). The Material scheme has no amber role and `tokens.json` has no entry for it yet, so the colour is made
 * with the same HCT model as the rest of the theme, from a hue and chroma, at the tones Material uses for an accent
 * (50 for the icon and 90 for the fill in light, 80 and 30 in dark). The hue and chroma are the only numbers; docs/DECISIONS.md asks for a
 * `tokens.json` entry that replaces them.
 */
@androidx.compose.runtime.Immutable
class StatusColors(
    /** The amber itself: the star, the warning icon. */
    val amber: Color,
    /** Content on [amberContainer]. */
    val onAmberContainer: Color,
    /** A soft amber fill. */
    val amberContainer: Color,
)

private const val AMBER_HUE = 78.0
private const val AMBER_CHROMA = 80.0

/** The status colours for the mode now showing. */
@Composable
fun statusColors(mode: CroftMode = LocalCroftMode.current): StatusColors = remember(mode) {
    val dark = mode != CroftMode.LIGHT
    fun tone(t: Double) = Color(McuSchemes.fromHct(AMBER_HUE, AMBER_CHROMA, t).toInt())
    StatusColors(
        amber = tone(if (dark) 80.0 else 50.0),
        onAmberContainer = tone(if (dark) 90.0 else 10.0),
        amberContainer = tone(if (dark) 30.0 else 90.0),
    )
}
