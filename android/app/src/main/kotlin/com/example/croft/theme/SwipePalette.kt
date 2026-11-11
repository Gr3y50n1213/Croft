// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.theme

import androidx.compose.material3.ColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.toArgb
import com.example.croft.swipe.SwipeAction

/**
 * The colours of one swipe pill (docs/ux/swipe.md, "The pill" and "Contrast"): below the arm point the pill is the action's
 * tonal container with its content colour; armed it is the full colour with the colour for content on it. The icon takes
 * the content colour (dark on a light pill, light on a dark pill), never white on a pale pill, so it holds 3 to 1 against the
 * pill in every theme.
 */
class PillColours(val container: Color, val onContainer: Color, val full: Color, val onFull: Color)

/**
 * Which theme roles each action uses: Delete and Spam the error roles (wax red in the wireframes), Archive the primary roles
 * (green), Mark read the tertiary roles and Move to the secondary roles (the two "blue" ones), Star and Snooze the amber of
 * [StatusColors] (amber = star and snooze), None and an unavailable action the neutral surface-variant roles. Every pair
 * is a Material container pair, so the theme contrast test already covers the tonal pill; the full pills and amber are
 * covered by `SwipePaletteTest` for every preset, mode and contrast.
 */
object SwipePalette {
    /** The pill colours of [action] in [scheme]; [available] false gives the neutral grey. */
    fun of(action: SwipeAction, available: Boolean, scheme: ColorScheme, amber: StatusColors): PillColours {
        if (!available || action == SwipeAction.NONE) {
            return PillColours(scheme.surfaceVariant, scheme.onSurfaceVariant, scheme.surfaceVariant, scheme.onSurfaceVariant)
        }
        return when (action) {
            SwipeAction.DELETE, SwipeAction.SPAM -> PillColours(scheme.errorContainer, scheme.onErrorContainer, scheme.error, scheme.onError)
            SwipeAction.ARCHIVE -> PillColours(scheme.primaryContainer, scheme.onPrimaryContainer, scheme.primary, scheme.onPrimary)
            SwipeAction.TOGGLE_READ -> PillColours(scheme.tertiaryContainer, scheme.onTertiaryContainer, scheme.tertiary, scheme.onTertiary)
            SwipeAction.MOVE_TO -> PillColours(scheme.secondaryContainer, scheme.onSecondaryContainer, scheme.secondary, scheme.onSecondary)
            SwipeAction.TOGGLE_STAR, SwipeAction.SNOOZE -> {
                // Amber has no "on" role for its full colour: of the two ends of the amber family take the one that stands out more.
                val full = amber.amber
                val dark = amber.onAmberContainer
                val light = amber.amberContainer
                val on = if (ratio(dark, full) >= ratio(light, full)) dark else light
                PillColours(amber.amberContainer, amber.onAmberContainer, full, on)
            }
            SwipeAction.NONE -> error("handled above")
        }
    }

    private fun ratio(a: Color, b: Color): Double = Contrast.ratio(a.toArgb().toLong() and 0xFFFFFFFFL, b.toArgb().toLong() and 0xFFFFFFFFL)
}

/** [SwipePalette.of] for the theme now showing. */
@Composable
fun swipePillColours(action: SwipeAction, available: Boolean): PillColours {
    val scheme = androidx.compose.material3.MaterialTheme.colorScheme
    val amber = statusColors()
    return remember(action, available, scheme, amber) { SwipePalette.of(action, available, scheme, amber) }
}
