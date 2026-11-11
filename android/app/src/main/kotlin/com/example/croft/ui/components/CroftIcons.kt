// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.ui.components

import androidx.compose.material.icons.materialIcon
import androidx.compose.material.icons.materialPath
import androidx.compose.ui.graphics.vector.ImageVector

/**
 * The few glyphs the core icon set lacks, drawn from the Material Symbols paths (Apache-2.0). Kept here instead of adding
 * the large extended icon library for three shapes.
 */
object CroftIcons {
    /** A clock: an action that waits for the network, and a scheduled send. */
    val Schedule: ImageVector by lazy {
        materialIcon(name = "Croft.Schedule") {
            materialPath {
                moveTo(12f, 2f)
                curveTo(6.5f, 2f, 2f, 6.5f, 2f, 12f)
                reflectiveCurveToRelative(4.5f, 10f, 10f, 10f)
                reflectiveCurveToRelative(10f, -4.5f, 10f, -10f)
                reflectiveCurveTo(17.5f, 2f, 12f, 2f)
                close()
                moveToRelative(4.2f, 14.2f)
                lineTo(11f, 13f)
                verticalLineTo(7f)
                horizontalLineToRelative(1.5f)
                verticalLineToRelative(5.2f)
                lineToRelative(4.5f, 2.7f)
                lineToRelative(-0.8f, 1.3f)
                close()
            }
        }
    }

    /** A paperclip: an attachment. */
    val Attachment: ImageVector by lazy {
        materialIcon(name = "Croft.Attachment") {
            materialPath {
                moveTo(16.5f, 6f)
                verticalLineToRelative(11.5f)
                curveToRelative(0f, 2.21f, -1.79f, 4f, -4f, 4f)
                reflectiveCurveToRelative(-4f, -1.79f, -4f, -4f)
                verticalLineTo(5f)
                curveToRelative(0f, -1.38f, 1.12f, -2.5f, 2.5f, -2.5f)
                reflectiveCurveToRelative(2.5f, 1.12f, 2.5f, 2.5f)
                verticalLineToRelative(10.5f)
                curveToRelative(0f, 0.55f, -0.45f, 1f, -1f, 1f)
                reflectiveCurveToRelative(-1f, -0.45f, -1f, -1f)
                verticalLineTo(6f)
                horizontalLineTo(10f)
                verticalLineToRelative(9.5f)
                curveToRelative(0f, 1.38f, 1.12f, 2.5f, 2.5f, 2.5f)
                reflectiveCurveToRelative(2.5f, -1.12f, 2.5f, -2.5f)
                verticalLineTo(5f)
                curveToRelative(0f, -2.21f, -1.79f, -4f, -4f, -4f)
                reflectiveCurveTo(7f, 2.79f, 7f, 5f)
                verticalLineToRelative(12.5f)
                curveToRelative(0f, 3.04f, 2.46f, 5.5f, 5.5f, 5.5f)
                reflectiveCurveToRelative(5.5f, -2.46f, 5.5f, -5.5f)
                verticalLineTo(6f)
                horizontalLineToRelative(-1.5f)
                close()
            }
        }
    }
}
