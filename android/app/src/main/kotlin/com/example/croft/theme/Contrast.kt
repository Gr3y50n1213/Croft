// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.theme

import kotlin.math.pow

/** WCAG 2.x contrast ratio between two opaque colours (docs/ux/accessibility.md: 4.5 to 1, 3 to 1 for large text). */
object Contrast {
    /** Minimum for normal text and icons. */
    const val NORMAL = 4.5

    /** Minimum for large text and graphics. */
    const val LARGE = 3.0

    /** Minimum at High contrast. */
    const val HIGH = 7.0

    /** Ratio from 1 (identical) to 21 (black on white), for opaque ARGB values (alpha is ignored). */
    fun ratio(a: Long, b: Long): Double {
        val la = luminance(a)
        val lb = luminance(b)
        return (maxOf(la, lb) + 0.05) / (minOf(la, lb) + 0.05)
    }

    private fun luminance(argb: Long): Double {
        fun channel(shift: Int): Double {
            val c = ((argb shr shift) and 0xFF) / 255.0
            return if (c <= 0.04045) c / 12.92 else ((c + 0.055) / 1.055).pow(2.4)
        }
        return 0.2126 * channel(16) + 0.7152 * channel(8) + 0.0722 * channel(0)
    }
}
