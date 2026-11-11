// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.ui.appearance

import androidx.compose.animation.core.Animatable
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.selection.selectableGroup
import androidx.compose.foundation.layout.Column
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import com.example.croft.R
import com.example.croft.design.CroftMotion
import com.example.croft.theme.AnimationLevel
import com.example.croft.theme.HapticsLevel
import com.example.croft.theme.LocalMotionLevel
import com.example.croft.theme.MotionLevel
import com.example.croft.theme.ThemeSettings

/** The haptic events S53 lets the person try (docs/ux/haptics.md). The `Haptics` interface that plays them arrives in T13. */
enum class TryItEvent { THRESHOLD, SUCCESS, ERROR, TOGGLE, TICK, DRAG_START }

/** Test tags on S53. */
object MotionTags {
    const val SCREEN = "S53"
    const val ANIMATION_PREFIX = "S53-animation-"
    const val HAPTICS_PREFIX = "S53-haptics-"
    const val TRY_PREFIX = "S53-try-"
    const val STAGE = "S53-stage"
}

/**
 * A small square that plays the motion that goes with a Try it button, at the animation level now in force: a springy
 * pop at Full (Spatial default), a short fade at Reduced (Effects default, no overshoot, no travel), nothing at Off.
 * [trigger] changes each time a button is pressed.
 */
@Composable
internal fun TryItStage(trigger: Int, modifier: Modifier = Modifier) {
    val level = LocalMotionLevel.current
    val scale = remember { Animatable(1f) }
    val alpha = remember { Animatable(1f) }
    LaunchedEffect(trigger) {
        if (trigger == 0) return@LaunchedEffect
        when (level) {
            MotionLevel.FULL -> { scale.snapTo(POP_FROM); scale.animateTo(1f, CroftMotion.spatialDefault.spec()) }
            MotionLevel.REDUCED -> { alpha.snapTo(FADE_FROM); alpha.animateTo(1f, CroftMotion.effectsDefault.spec()) }
            MotionLevel.OFF -> Unit
        }
    }
    val description = stringResource(R.string.motion_try_stage_description)
    Box(modifier.heightIn(min = 72.dp).fillMaxWidth().semantics { contentDescription = description }, contentAlignment = Alignment.Center) {
        Box(
            Modifier.size(56.dp).graphicsLayer { scaleX = scale.value; scaleY = scale.value; this.alpha = alpha.value }
                .background(MaterialTheme.colorScheme.primaryContainer, MaterialTheme.shapes.large),
        )
    }
}

private const val POP_FROM = 0.7f
private const val FADE_FROM = 0.4f

/**
 * S53 Motion and haptics (settings only). Animation level and haptics level, each with a one-line description, and Try it
 * buttons: each calls [onTry] (the haptic, once the `Haptics` interface exists) and plays the matching motion on the stage.
 */
@Composable
fun MotionHapticsScreen(
    settings: ThemeSettings,
    onChange: (ThemeSettings) -> Unit,
    onBack: () -> Unit,
    modifier: Modifier = Modifier,
    onTry: (TryItEvent) -> Unit = {},
) {
    var tries by remember { mutableIntStateOf(0) }
    SettingsFrame(
        title = stringResource(R.string.motion_title),
        onBack = onBack,
        modifier = modifier.testTag(MotionTags.SCREEN),
    ) {
        SectionLabel(stringResource(R.string.motion_section_animation))
        Column(Modifier.selectableGroup()) {
            AnimationLevel.entries.forEach { level ->
                val (title, hint) = when (level) {
                    AnimationLevel.SYSTEM -> R.string.motion_anim_system to R.string.motion_anim_system_hint
                    AnimationLevel.FULL -> R.string.motion_anim_full to R.string.motion_anim_full_hint
                    AnimationLevel.REDUCED -> R.string.motion_anim_reduced to R.string.motion_anim_reduced_hint
                    AnimationLevel.OFF -> R.string.motion_anim_off to R.string.motion_anim_off_hint
                }
                RadioRow(
                    stringResource(title), stringResource(hint), settings.animation == level, { onChange(settings.copy(animation = level)) },
                    Modifier.testTag(MotionTags.ANIMATION_PREFIX + level.id),
                )
            }
        }

        SectionLabel(stringResource(R.string.motion_section_haptics))
        Column(Modifier.selectableGroup()) {
            HapticsLevel.entries.forEach { level ->
                val (title, hint) = when (level) {
                    HapticsLevel.OFF -> R.string.motion_haptics_off to R.string.motion_haptics_off_hint
                    HapticsLevel.SUBTLE -> R.string.motion_haptics_subtle to R.string.motion_haptics_subtle_hint
                    HapticsLevel.FULL -> R.string.motion_haptics_full to R.string.motion_haptics_full_hint
                }
                RadioRow(
                    stringResource(title), stringResource(hint), settings.haptics == level, { onChange(settings.copy(haptics = level)) },
                    Modifier.testTag(MotionTags.HAPTICS_PREFIX + level.id),
                )
            }
        }

        SectionLabel(stringResource(R.string.motion_section_try))
        Wrap {
            TryItEvent.entries.forEach { event ->
                OutlinedButton(
                    onClick = { tries++; onTry(event) },
                    modifier = Modifier.heightIn(min = 48.dp).testTag(MotionTags.TRY_PREFIX + event.name.lowercase()),
                ) {
                    Text(
                        stringResource(
                            when (event) {
                                TryItEvent.THRESHOLD -> R.string.motion_try_threshold
                                TryItEvent.SUCCESS -> R.string.motion_try_success
                                TryItEvent.ERROR -> R.string.motion_try_error
                                TryItEvent.TOGGLE -> R.string.motion_try_toggle
                                TryItEvent.TICK -> R.string.motion_try_tick
                                TryItEvent.DRAG_START -> R.string.motion_try_drag_start
                            },
                        ),
                    )
                }
            }
        }
        TryItStage(tries, Modifier.testTag(MotionTags.STAGE))
        Text(stringResource(R.string.motion_try_hint), style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
    }
}
