// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.ui.components

import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.EnterTransition
import androidx.compose.animation.ExitTransition
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.scaleIn
import androidx.compose.animation.scaleOut
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.width
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Create
import androidx.compose.material3.ExtendedFloatingActionButton
import androidx.compose.material3.FloatingActionButton
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.unit.dp
import com.example.croft.theme.LocalMotionLevel
import com.example.croft.theme.MotionLevel

/** The extended FAB (Compose): expanded, collapsed to its icon (on scroll down), or hidden (selection mode, keyboard open). */
enum class FabState { EXPANDED, COLLAPSED, HIDDEN }

/** From this combined font scale the label no longer fits beside the icon, so the FAB shows only the icon. */
const val FAB_COLLAPSE_FROM_FONT_SCALE = 1.3f

/**
 * The extended FAB. At a large font scale it collapses to the icon whatever [state] says, so the label is never clipped
 * (docs/ux/accessibility.md). The icon carries the label when the text is hidden. Expand and collapse use the spatial fast
 * spring; at Reduced and Off the label swaps at once. Hidden removes it from the tree (a fade at Reduced, nothing at Off).
 */
@Composable
fun CroftExtendedFab(
    label: String,
    onClick: () -> Unit,
    state: FabState,
    modifier: Modifier = Modifier,
    icon: ImageVector = Icons.Filled.Create,
    interactionSource: MutableInteractionSource = remember { MutableInteractionSource() },
) {
    val level = LocalMotionLevel.current
    val bigText = LocalDensity.current.fontScale > FAB_COLLAPSE_FROM_FONT_SCALE
    val expanded = state == FabState.EXPANDED && !bigText
    val shape = MaterialTheme.shapes.large
    val enter: EnterTransition = when (level) {
        MotionLevel.FULL -> scaleIn(MaterialTheme.motionScheme.fastSpatialSpec()) + fadeIn(MaterialTheme.motionScheme.defaultEffectsSpec())
        MotionLevel.REDUCED -> fadeIn(MaterialTheme.motionScheme.defaultEffectsSpec())
        MotionLevel.OFF -> EnterTransition.None
    }
    val exit: ExitTransition = when (level) {
        MotionLevel.FULL -> scaleOut(MaterialTheme.motionScheme.fastSpatialSpec()) + fadeOut(MaterialTheme.motionScheme.defaultEffectsSpec())
        MotionLevel.REDUCED -> fadeOut(MaterialTheme.motionScheme.defaultEffectsSpec())
        MotionLevel.OFF -> ExitTransition.None
    }
    AnimatedVisibility(visible = state != FabState.HIDDEN, enter = enter, exit = exit) {
        // The button's own name, whatever the layout shows: the label of the expanded Material button is not merged into its node
        // (a screen reader found a button without a name), and a collapsed one has only an icon. The icons below stay silent.
        val ring = modifier.croftFocusRing(interactionSource, shape).semantics { contentDescription = label }
        if (level == MotionLevel.FULL) {
            ExtendedFloatingActionButton(
                text = { Text(label, maxLines = 1) },
                icon = { Icon(icon, contentDescription = null) },
                onClick = onClick,
                modifier = ring,
                expanded = expanded,
                interactionSource = interactionSource,
            )
        } else if (expanded) {
            ExtendedFloatingActionButton(onClick = onClick, modifier = ring, interactionSource = interactionSource) {
                Icon(icon, contentDescription = null)
                Spacer(Modifier.width(8.dp))
                Text(label, maxLines = 1)
            }
        } else {
            FloatingActionButton(onClick = onClick, modifier = ring, interactionSource = interactionSource) {
                Icon(icon, contentDescription = null)
            }
        }
    }
}
