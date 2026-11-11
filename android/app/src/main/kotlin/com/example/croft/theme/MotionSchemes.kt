// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.theme

import androidx.compose.animation.core.FiniteAnimationSpec
import androidx.compose.animation.core.snap
import androidx.compose.material3.MotionScheme
import com.example.croft.design.croftMotionScheme

/**
 * Animation level Off: every spring becomes a jump to the end value. Material components read their springs from
 * `MaterialTheme.motionScheme` (switch thumb, chips, FAB, sheets, menus), so providing this scheme makes them instant
 * too. Ripples and state layers are press feedback, not travel, and are left to the platform.
 */
internal object OffMotionScheme : MotionScheme {
    override fun <T> defaultSpatialSpec(): FiniteAnimationSpec<T> = snap()
    override fun <T> fastSpatialSpec(): FiniteAnimationSpec<T> = snap()
    override fun <T> slowSpatialSpec(): FiniteAnimationSpec<T> = snap()
    override fun <T> defaultEffectsSpec(): FiniteAnimationSpec<T> = snap()
    override fun <T> fastEffectsSpec(): FiniteAnimationSpec<T> = snap()
    override fun <T> slowEffectsSpec(): FiniteAnimationSpec<T> = snap()
}

/** The motion scheme for a resolved level: expressive springs, the standard set, or none. */
internal fun motionSchemeFor(level: MotionLevel): MotionScheme = when (level) {
    MotionLevel.FULL -> croftMotionScheme(reducedMotion = false)
    MotionLevel.REDUCED -> croftMotionScheme(reducedMotion = true)
    MotionLevel.OFF -> OffMotionScheme
}

/** The level an Animation setting means, for places that resolve it without a device (the gallery's own override). */
fun resolveMotionLevelForGallery(pref: AnimationLevel): MotionLevel = resolveMotionLevel(pref, systemAnimatorScale = 1f)
