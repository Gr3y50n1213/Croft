// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.theme

import androidx.compose.animation.core.FiniteAnimationSpec
import androidx.compose.animation.core.InfiniteRepeatableSpec
import androidx.compose.animation.core.LinearEasing
import androidx.compose.animation.core.RepeatMode
import androidx.compose.animation.core.infiniteRepeatable
import androidx.compose.animation.core.TweenSpec
import com.example.croft.design.CroftTokens

/**
 * Fixed times from docs/ux/controls.md and motion.md that are waits, not animations. Durations are never typed in a
 * component; they come from `tokens.json` (`timing`, generated as `CroftTokens.Timing`) through here, or from the spring
 * tokens for movement.
 */
object CroftTimings {
    /** A button that finished its work shows a check this long, then its label again. */
    const val BUTTON_SUCCESS_HOLD_MS = CroftTokens.Timing.buttonSuccessHoldMs.toLong()

    /** A loading button offers Cancel after this long. */
    const val LOADING_OFFERS_CANCEL_MS = CroftTokens.Timing.loadingOffersCancelMs.toLong()

    /** A snackbar without an action stays this long (a longer system accessibility timeout wins). */
    const val SNACKBAR_MIN_MS = CroftTokens.Timing.snackbarMinMs.toLong()

    /** The tint on a row that just arrived fades over this long. */
    const val NEW_ROW_TINT_MS = CroftTokens.Timing.newRowTintMs.toLong()

    /** The red edge on a row the server refused fades after this long. */
    const val FAILED_EDGE_MS = CroftTokens.Timing.failedEdgeMs.toLong()

    /** One loop of the skeleton rows' shimmer on a first load (docs/ux/motion.md: slow shimmer, 1.2 s, linear). */
    const val SKELETON_SHIMMER_MS = CroftTokens.Timing.skeletonShimmerMs.toLong()

    /** The fast scroller's thumb stays this long after the list stops moving. */
    const val FAST_SCROLLER_LINGER_MS = CroftTokens.Timing.fastScrollerLingerMs.toLong()

    /** A row that arrived keeps its "new" mark this long (the tint fade plus a margin), then the mark is dropped. */
    const val NEW_ROW_MARK_MS = CroftTokens.Timing.newRowMarkMs.toLong()
}

/** The skeleton shimmer: a linear loop of [CroftTimings.SKELETON_SHIMMER_MS]. */
fun shimmerSpec(): InfiniteRepeatableSpec<Float> =
    infiniteRepeatable(TweenSpec(CroftTimings.SKELETON_SHIMMER_MS.toInt(), easing = LinearEasing), RepeatMode.Restart)

/** A linear fade of [ms], for the waits above (a tint that fades over 1.2 s). Durations are only ever typed in this package. */
fun <T> linearFadeSpec(ms: Long): FiniteAnimationSpec<T> = TweenSpec(ms.toInt(), easing = LinearEasing)
