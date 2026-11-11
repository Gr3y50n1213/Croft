// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.theme

import androidx.compose.animation.core.FiniteAnimationSpec
import androidx.compose.animation.core.InfiniteRepeatableSpec
import androidx.compose.animation.core.LinearEasing
import androidx.compose.animation.core.RepeatMode
import androidx.compose.animation.core.infiniteRepeatable
import androidx.compose.animation.core.tween

/**
 * Fixed times from docs/ux/controls.md and motion.md that are waits, not animations. Durations are never typed in a
 * component; they come from here (or from the spring tokens for movement). `tokens.json` does not hold them yet:
 * they are listed in docs/DECISIONS.md as candidates to move there.
 */
object CroftTimings {
    /** A button that finished its work shows a check this long, then its label again. */
    const val BUTTON_SUCCESS_HOLD_MS = 600L

    /** A loading button offers Cancel after this long. */
    const val LOADING_OFFERS_CANCEL_MS = 10_000L

    /** A snackbar without an action stays this long (a longer system accessibility timeout wins). */
    const val SNACKBAR_MIN_MS = 4_000L

    /** The tint on a row that just arrived fades over this long. */
    const val NEW_ROW_TINT_MS = 1_200L

    /** The red edge on a row the server refused fades after this long. */
    const val FAILED_EDGE_MS = 2_000L

    /** One loop of the skeleton rows' shimmer on a first load (docs/ux/motion.md: slow shimmer, 1.2 s, linear). */
    const val SKELETON_SHIMMER_MS = 1_200L

    /** The fast scroller's thumb stays this long after the list stops moving. */
    const val FAST_SCROLLER_LINGER_MS = 1_500L

    /** A row that arrived keeps its "new" mark this long (the tint fade plus a margin), then the mark is dropped. */
    const val NEW_ROW_MARK_MS = NEW_ROW_TINT_MS + 100L
}

/** The skeleton shimmer: a linear loop of [CroftTimings.SKELETON_SHIMMER_MS]. */
fun shimmerSpec(): InfiniteRepeatableSpec<Float> =
    infiniteRepeatable(tween(CroftTimings.SKELETON_SHIMMER_MS.toInt(), easing = LinearEasing), RepeatMode.Restart)

/** A linear fade of [ms], for the waits above (a tint that fades over 1.2 s). Durations are only ever typed in this package. */
fun <T> linearFadeSpec(ms: Long): FiniteAnimationSpec<T> = tween(ms.toInt(), easing = LinearEasing)
