// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.ui.reading

import androidx.compose.material.icons.materialIcon
import androidx.compose.material.icons.materialPath
import androidx.compose.ui.graphics.vector.ImageVector

/**
 * The glyphs of the reading screens that the core icon set lacks, from the Material Symbols paths (Apache-2.0), like
 * [com.example.croft.ui.components.CroftIcons]. Reply and Reply all point backwards and Forward forwards: they mirror in right-to-left.
 */
object ReadingIcons {
    /** Reply: an arrow turning back. */
    val Reply: ImageVector by lazy {
        materialIcon(name = "Croft.Reply", autoMirror = true) {
            materialPath {
                moveTo(10f, 9f)
                verticalLineTo(5f)
                lineToRelative(-7f, 7f)
                lineToRelative(7f, 7f)
                verticalLineToRelative(-4.1f)
                curveToRelative(5f, 0f, 8.5f, 1.6f, 11f, 5.1f)
                curveToRelative(-1f, -5f, -4f, -10f, -11f, -11f)
                close()
            }
        }
    }

    /** Reply all: two arrows turning back. */
    val ReplyAll: ImageVector by lazy {
        materialIcon(name = "Croft.ReplyAll", autoMirror = true) {
            materialPath {
                moveTo(7f, 8f)
                verticalLineTo(5f)
                lineToRelative(-7f, 7f)
                lineToRelative(7f, 7f)
                verticalLineToRelative(-3f)
                lineToRelative(-4f, -4f)
                close()
                moveTo(13f, 9f)
                verticalLineTo(5f)
                lineToRelative(-7f, 7f)
                lineToRelative(7f, 7f)
                verticalLineToRelative(-4.1f)
                curveToRelative(5f, 0f, 8.5f, 1.6f, 11f, 5.1f)
                curveToRelative(-1f, -5f, -4f, -10f, -11f, -11f)
                close()
            }
        }
    }

    /** Forward: an arrow turning forwards. */
    val Forward: ImageVector by lazy {
        materialIcon(name = "Croft.Forward", autoMirror = true) {
            materialPath {
                moveTo(12f, 8f)
                verticalLineTo(4f)
                lineToRelative(8f, 8f)
                lineToRelative(-8f, 8f)
                verticalLineToRelative(-4f)
                horizontalLineTo(4f)
                verticalLineTo(8f)
                close()
            }
        }
    }

    /** Three dots on a column: the overflow menu. */
    val More: ImageVector by lazy {
        materialIcon(name = "Croft.More") {
            materialPath {
                moveTo(12f, 8f)
                curveToRelative(1.1f, 0f, 2f, -0.9f, 2f, -2f)
                reflectiveCurveToRelative(-0.9f, -2f, -2f, -2f)
                reflectiveCurveToRelative(-2f, 0.9f, -2f, 2f)
                reflectiveCurveToRelative(0.9f, 2f, 2f, 2f)
                close()
                moveToRelative(0f, 2f)
                curveToRelative(-1.1f, 0f, -2f, 0.9f, -2f, 2f)
                reflectiveCurveToRelative(0.9f, 2f, 2f, 2f)
                reflectiveCurveToRelative(2f, -0.9f, 2f, -2f)
                reflectiveCurveToRelative(-0.9f, -2f, -2f, -2f)
                close()
                moveToRelative(0f, 6f)
                curveToRelative(-1.1f, 0f, -2f, 0.9f, -2f, 2f)
                reflectiveCurveToRelative(0.9f, 2f, 2f, 2f)
                reflectiveCurveToRelative(2f, -0.9f, 2f, -2f)
                reflectiveCurveToRelative(-0.9f, -2f, -2f, -2f)
                close()
            }
        }
    }
}
