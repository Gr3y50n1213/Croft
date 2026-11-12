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

    // Plain geometric glyphs for the drawer and the selection bar, drawn here from straight lines (no copied artwork).

    /** A tray: the Inbox. */
    val Inbox: ImageVector by lazy {
        materialIcon(name = "Croft.Inbox") {
            materialPath {
                moveTo(3f, 5f); horizontalLineTo(21f); verticalLineTo(19f); horizontalLineTo(3f); close()
                moveTo(5f, 7f); verticalLineTo(13f); horizontalLineTo(9f); verticalLineTo(15f); horizontalLineTo(15f); verticalLineTo(13f); horizontalLineTo(19f); verticalLineTo(7f); close()
            }
        }
    }

    /** A box with a lid and a slot: Archive. */
    val Archive: ImageVector by lazy {
        materialIcon(name = "Croft.Archive") {
            materialPath {
                moveTo(3f, 3f); horizontalLineTo(21f); verticalLineTo(8f); horizontalLineTo(3f); close()
                moveTo(4f, 10f); horizontalLineTo(20f); verticalLineTo(21f); horizontalLineTo(4f); close()
                moveTo(9f, 13f); verticalLineTo(15f); horizontalLineTo(15f); verticalLineTo(13f); close()
            }
        }
    }

    /** A page with a folded corner: Drafts. */
    val Drafts: ImageVector by lazy {
        materialIcon(name = "Croft.Drafts") {
            materialPath {
                moveTo(6f, 3f); horizontalLineTo(14f); lineTo(19f, 8f); verticalLineTo(21f); horizontalLineTo(6f); close()
            }
        }
    }

    /** A picture: a frame with a hill. Attach a photo. */
    val Photo: ImageVector by lazy {
        materialIcon(name = "Croft.Photo") {
            materialPath(pathFillType = androidx.compose.ui.graphics.PathFillType.EvenOdd) {
                moveTo(4f, 4f); horizontalLineTo(20f); verticalLineTo(20f); horizontalLineTo(4f); close()
                moveTo(6f, 6f); verticalLineTo(18f); horizontalLineTo(18f); verticalLineTo(6f); close()
                moveTo(7.5f, 16.5f); lineTo(11f, 11.5f); lineTo(13.5f, 14.5f); lineTo(15.5f, 12.5f); lineTo(17.5f, 16.5f); close()
            }
        }
    }

    /** An arrow leaving a line: Outbox. */
    val Outbox: ImageVector by lazy {
        materialIcon(name = "Croft.Outbox") {
            materialPath {
                moveTo(12f, 3f); lineTo(18f, 10f); horizontalLineTo(14f); verticalLineTo(17f); horizontalLineTo(10f); verticalLineTo(10f); horizontalLineTo(6f); close()
                moveTo(5f, 19f); horizontalLineTo(19f); verticalLineTo(21f); horizontalLineTo(5f); close()
            }
        }
    }

    /** A tag: a label. */
    val Label: ImageVector by lazy {
        materialIcon(name = "Croft.Label") {
            materialPath {
                moveTo(4f, 6f); horizontalLineTo(15f); lineTo(21f, 12f); lineTo(15f, 18f); horizontalLineTo(4f); close()
            }
        }
    }

    /** An open envelope: Mark as read. */
    val EnvelopeOpen: ImageVector by lazy {
        materialIcon(name = "Croft.EnvelopeOpen") {
            materialPath {
                moveTo(4f, 10f); lineTo(12f, 4f); lineTo(20f, 10f); close()
                moveTo(4f, 11.5f); horizontalLineTo(20f); verticalLineTo(20f); horizontalLineTo(4f); close()
            }
        }
    }

    // Glyphs of the swipe pills (T15), plain shapes drawn from straight lines.

    /** A folder: Move to. */
    val Folder: ImageVector by lazy {
        materialIcon(name = "Croft.Folder") {
            materialPath {
                moveTo(3f, 5f); horizontalLineTo(10f); lineTo(12f, 8f); horizontalLineTo(21f); verticalLineTo(19f); horizontalLineTo(3f); close()
            }
        }
    }

    /** A shield: Mark as spam. */
    val Shield: ImageVector by lazy {
        materialIcon(name = "Croft.Shield") {
            materialPath {
                moveTo(12f, 2f); lineTo(20f, 5f); verticalLineTo(12f); curveTo(20f, 16.5f, 16.5f, 20.5f, 12f, 22f)
                curveTo(7.5f, 20.5f, 4f, 16.5f, 4f, 12f); verticalLineTo(5f); close()
            }
        }
    }

    /** A dash: None. */
    val Dash: ImageVector by lazy {
        materialIcon(name = "Croft.Dash") {
            materialPath { moveTo(5f, 11f); horizontalLineTo(19f); verticalLineTo(13f); horizontalLineTo(5f); close() }
        }
    }

    /** An outlined star: Star (the message is not starred yet). */
    val StarOutline: ImageVector by lazy {
        materialIcon(name = "Croft.StarOutline") {
            materialPath(pathFillType = androidx.compose.ui.graphics.PathFillType.EvenOdd) {
                moveTo(12f, 2f); lineTo(14.9f, 8.6f); lineTo(22f, 9.3f); lineTo(16.6f, 14f); lineTo(18.2f, 21f); lineTo(12f, 17.3f)
                lineTo(5.8f, 21f); lineTo(7.4f, 14f); lineTo(2f, 9.3f); lineTo(9.1f, 8.6f); close()
                moveTo(12f, 6.8f); lineTo(10.9f, 10f); lineTo(7.5f, 10.3f); lineTo(10.1f, 12.5f); lineTo(9.3f, 15.8f); lineTo(12f, 14.1f)
                lineTo(14.7f, 15.8f); lineTo(13.9f, 12.5f); lineTo(16.5f, 10.3f); lineTo(13.1f, 10f); close()
            }
        }
    }
}
