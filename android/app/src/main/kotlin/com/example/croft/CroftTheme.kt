// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft

import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.Composable
import com.example.croft.design.CroftMode
import com.example.croft.design.CroftPreset
import com.example.croft.design.croftColorScheme
import com.example.croft.design.croftMotionScheme

/** The app theme: colours and motion from the generated design tokens, light or dark with the system. */
@Composable
fun CroftTheme(
    preset: CroftPreset = CroftPreset.DEFAULT,
    content: @Composable () -> Unit,
) {
    val mode = if (isSystemInDarkTheme()) CroftMode.DARK else CroftMode.LIGHT
    MaterialTheme(
        colorScheme = croftColorScheme(preset, mode),
        motionScheme = croftMotionScheme(),
        content = content,
    )
}
