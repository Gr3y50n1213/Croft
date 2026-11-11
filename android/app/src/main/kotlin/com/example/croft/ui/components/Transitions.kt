// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.ui.components

import androidx.compose.animation.ContentTransform
import androidx.compose.animation.EnterTransition
import androidx.compose.animation.ExitTransition
import androidx.compose.animation.core.FiniteAnimationSpec
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.slideInHorizontally
import androidx.compose.animation.slideOutHorizontally
import androidx.compose.animation.togetherWith
import androidx.compose.ui.unit.IntOffset
import com.example.croft.design.CroftMotion
import com.example.croft.theme.MotionLevel

/**
 * The shared axis X of a settings drill-down (docs/ux/motion.md): the spatial default spring for the move and the effects
 * default for the fade at Full, a cross-fade at Reduced, nothing at Off. [forward] opens a deeper page; [rtl] mirrors it.
 */
fun sharedAxisX(level: MotionLevel, forward: Boolean, rtl: Boolean): ContentTransform {
    val sign = if (forward != rtl) 1 else -1
    return when (level) {
        MotionLevel.FULL -> {
            val move: FiniteAnimationSpec<IntOffset> = CroftMotion.spatialDefault.spec()
            val fade: FiniteAnimationSpec<Float> = CroftMotion.effectsDefault.spec()
            (slideInHorizontally(move) { sign * it / 4 } + fadeIn(fade)) togetherWith (slideOutHorizontally(move) { -sign * it / 4 } + fadeOut(fade))
        }
        MotionLevel.REDUCED -> fadeIn(CroftMotion.effectsDefault.spec()) togetherWith fadeOut(CroftMotion.effectsDefault.spec())
        MotionLevel.OFF -> EnterTransition.None togetherWith ExitTransition.None
    }
}
