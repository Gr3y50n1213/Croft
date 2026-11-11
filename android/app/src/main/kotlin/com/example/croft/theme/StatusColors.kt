// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.theme

import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.graphics.Color
import com.example.croft.design.CroftContrast
import com.example.croft.design.CroftMode
import com.example.croft.design.CroftStatus

/**
 * The amber of a starred message and of the Server problem chip (docs/ux/message-list.md, wireframes: "amber = star and
 * snooze"). The Material scheme has no amber role, so the colours come from `tokens.json` (`status.amber`: an HCT hue and
 * chroma at set tones per mode and contrast, generated as [CroftStatus]).
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

/** The status colours for the mode and contrast now showing. */
@Composable
fun statusColors(mode: CroftMode = LocalCroftMode.current, contrast: CroftContrast = LocalThemeSettings.current.contrast): StatusColors =
    remember(mode, contrast) {
        val roles = CroftStatus.amber(mode, contrast)
        StatusColors(amber = Color(roles.color), onAmberContainer = Color(roles.onContainer), amberContainer = Color(roles.container))
    }

/**
 * The colour of the star on a message row (docs/ux/controls.md, "Message row"). Amber on the list's own surfaces, where it
 * holds 3 to 1 in every theme; on a selected row the background is `primaryContainer`, which at Medium and High contrast
 * is close to amber in tone (1.0 to 2.2 to 1 against it, recorded in docs/DECISIONS.md), so the star takes the row's
 * content colour, `onPrimaryContainer`, which is at least 4.5 to 1 there. The shape of the star carries the meaning, not the colour.
 */
fun starColourFor(selected: Boolean, amber: Color, onPrimaryContainer: Color): Color = if (selected) onPrimaryContainer else amber

/** [starColourFor] with the colours of the theme now showing. */
@Composable
fun rowStarColour(selected: Boolean): Color =
    starColourFor(selected, statusColors().amber, androidx.compose.material3.MaterialTheme.colorScheme.onPrimaryContainer)
