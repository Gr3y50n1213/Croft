// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.ui.components

import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.LocalContentColor
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.unit.dp
import com.example.croft.theme.LocalMotionLevel
import com.example.croft.theme.MotionLevel

/**
 * A busy indicator. It turns at Full and Reduced; at animation level Off it stands still as a three-quarter arc
 * (docs/ux/motion.md: static spinner). The meaning ("Busy") is announced by the control that owns it.
 */
@Composable
fun CroftSpinner(modifier: Modifier = Modifier, color: Color = LocalContentColor.current) {
    if (LocalMotionLevel.current == MotionLevel.OFF) {
        CircularProgressIndicator(progress = { STATIC_ARC }, modifier = modifier, color = color, strokeWidth = 2.dp)
    } else {
        CircularProgressIndicator(modifier = modifier, color = color, strokeWidth = 2.dp)
    }
}

private const val STATIC_ARC = 0.75f
