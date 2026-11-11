// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.ui.mail

import androidx.compose.animation.core.snap
import androidx.compose.foundation.lazy.LazyItemScope
import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.IntOffset
import com.example.croft.theme.LocalMotionLevel
import com.example.croft.theme.MotionLevel

/**
 * How rows and headers move when the list changes (docs/ux/message-list.md, motion.md): a removed row fades while the rows
 * below slide up and close the gap, new rows at the top slide in and push the rest down (the spatial default spring), all of it
 * at Full. At Reduced the travel is gone: rows appear and disappear in place with a short fade (effects default), and the gap
 * closes at once. At Off nothing animates. The springs come from the motion scheme; every one of these can be interrupted by
 * a touch, because a lazy list item animation is retargeted by the next change.
 */
@Composable
fun LazyItemScope.listItemMotion(): Modifier {
    val level = LocalMotionLevel.current
    val scheme = MaterialTheme.motionScheme
    return when (level) {
        MotionLevel.FULL -> Modifier.animateItem(
            fadeInSpec = scheme.defaultEffectsSpec(),
            placementSpec = scheme.defaultSpatialSpec<IntOffset>(),
            fadeOutSpec = scheme.defaultEffectsSpec(),
        )
        MotionLevel.REDUCED -> Modifier.animateItem(
            fadeInSpec = scheme.defaultEffectsSpec(),
            placementSpec = snap<IntOffset>(),
            fadeOutSpec = scheme.defaultEffectsSpec(),
        )
        MotionLevel.OFF -> Modifier.animateItem(fadeInSpec = null, placementSpec = null, fadeOutSpec = null)
    }
}
