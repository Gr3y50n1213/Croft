// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft

import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.hapticfeedback.HapticFeedback
import androidx.compose.ui.hapticfeedback.HapticFeedbackType
import androidx.compose.ui.platform.LocalHapticFeedback
import com.example.croft.haptics.ProvideHaptics
import com.example.croft.theme.CroftThemeProvider
import com.example.croft.theme.ThemeSettings

/**
 * The app theme. The work is in `com.example.croft.theme` (T12): colours from the generated tokens or the runtime
 * Material Color Utilities, shape, font, text size and motion from [settings]. The defaults are Moss, Standard and
 * System, which is also what a stored file that cannot be read falls back to.
 */
@Composable
fun CroftTheme(
    settings: ThemeSettings = ThemeSettings.Default,
    systemDark: Boolean = isSystemInDarkTheme(),
    content: @Composable () -> Unit,
) {
    CroftThemeProvider(settings = settings, systemDark = systemDark) {
        // Every haptic goes through `Haptics` (design/CLAUDE.md). Compose and Material also vibrate by themselves on a
        // long-press; that path is switched off so the level filter, the 40 ms guard and the recording test see all of them.
        CompositionLocalProvider(LocalHapticFeedback provides NoPlatformHaptics) {
            ProvideHaptics(settings.haptics, content)
        }
    }
}

/** Compose's own haptic feedback, silenced: the app plays haptics only through `com.example.croft.haptics.Haptics`. */
private object NoPlatformHaptics : HapticFeedback {
    override fun performHapticFeedback(hapticFeedbackType: HapticFeedbackType) = Unit
}
