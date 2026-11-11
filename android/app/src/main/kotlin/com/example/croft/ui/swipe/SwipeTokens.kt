// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.ui.swipe

import com.example.croft.design.CroftTokens
import com.example.croft.swipe.SwipeGeometryConfig
import com.example.croft.swipe.SwipeTuning
import com.example.croft.theme.SwipeLooks

/** The numbers of the pure swipe module, taken from `tokens.json` through the generated `CroftTokens` (never typed here). */
object SwipeTokens {
    /** Thresholds, fling, resistance, parking and the 40 ms guard. */
    val tuning = SwipeTuning(
        armLow = CroftTokens.Swipe.ArmAtFractionOfRowWidth.low.toFloat(),
        armNormal = CroftTokens.Swipe.ArmAtFractionOfRowWidth.normal.toFloat(),
        armHigh = CroftTokens.Swipe.ArmAtFractionOfRowWidth.high.toFloat(),
        flingMinVelocity = CroftTokens.Swipe.flingMinVelocityDpPerSec.toFloat(),
        flingMinTravelFraction = CroftTokens.Swipe.flingMinTravelFraction.toFloat(),
        parkFraction = CroftTokens.Swipe.parkAtFractionOfRowWidth.toFloat(),
        resistanceStartFraction = CroftTokens.Swipe.Resistance.startsAtFraction.toFloat(),
        resistanceFactor = CroftTokens.Swipe.Resistance.factor.toFloat(),
        crossingGuardMs = CroftTokens.Haptics.minGapMs.toLong(),
    )

    /** The pill and card geometry. */
    val geometry = SwipeGeometryConfig(
        gapToCard = CroftTokens.Swipe.Pill.gapToCardDp.toFloat(),
        insetFromEdge = CroftTokens.Swipe.Pill.insetFromEdgeDp.toFloat(),
        widthIsDragMinus = CroftTokens.Swipe.Pill.widthIsDragMinusDp.toFloat(),
        cardEaseInDistance = SwipeLooks.CARD_EASE_IN_DP.toFloat(),
        iconSize = SwipeLooks.ICON_DP.toFloat(),
    )

    /** The touch slop (8 dp) and the angle from horizontal (30 degrees) that start a swipe. */
    const val SLOP_DP = CroftTokens.Swipe.touchSlopDp
    const val MAX_ANGLE_DEG = CroftTokens.Swipe.maxAngleFromHorizontalDeg

    /** The icon scale at the arm point (1.2). */
    val armedIconScale = CroftTokens.Swipe.armedIconScale.toFloat()
}
