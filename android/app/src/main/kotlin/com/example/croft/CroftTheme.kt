// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft

import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.runtime.Composable
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
    CroftThemeProvider(settings = settings, systemDark = systemDark, content = content)
}
