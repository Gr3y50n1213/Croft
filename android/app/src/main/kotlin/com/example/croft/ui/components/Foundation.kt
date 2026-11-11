// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.ui.components

import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.foundation.LocalIndication
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.focusable
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.hoverable
import androidx.compose.foundation.indication
import androidx.compose.foundation.interaction.FocusInteraction
import androidx.compose.foundation.interaction.HoverInteraction
import androidx.compose.foundation.interaction.PressInteraction
import androidx.compose.foundation.interaction.collectIsFocusedAsState
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.minimumInteractiveComponentSize
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.drawWithContent
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Outline
import androidx.compose.ui.graphics.Shape
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.graphics.drawscope.translate
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.input.key.Key
import androidx.compose.ui.input.key.KeyEventType
import androidx.compose.ui.input.key.key
import androidx.compose.ui.input.key.onKeyEvent
import androidx.compose.ui.input.key.type
import androidx.compose.foundation.gestures.awaitEachGesture
import androidx.compose.foundation.gestures.awaitFirstDown
import androidx.compose.foundation.gestures.waitForUpOrCancellation
import androidx.compose.ui.input.pointer.PointerEventPass
import androidx.compose.ui.input.pointer.PointerEventTimeoutCancellationException
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.SemanticsPropertyKey
import androidx.compose.ui.semantics.SemanticsPropertyReceiver
import androidx.compose.ui.semantics.disabled
import androidx.compose.ui.semantics.onClick
import androidx.compose.ui.semantics.onLongClick
import androidx.compose.ui.semantics.role
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.ui.unit.dp
import com.example.croft.design.CroftTokens
import com.example.croft.haptics.HapticEvent
import com.example.croft.haptics.LocalHaptics
import com.example.croft.theme.LocalMotionLevel
import com.example.croft.theme.MotionLevel

/** The smallest touch target, from the Touch token (48 dp). */
val MinTouchTarget = CroftTokens.Touch.minTargetDp.dp

/** Focus ring: 3 dp stroke in the primary colour with a 2 dp gap (docs/ux/controls.md). */
internal val FocusRingStroke = 3.dp
internal val FocusRingGap = 2.dp

/** Whether the focus ring is drawn on a node. Tests read it; screen readers do not. */
val FocusRingVisible = SemanticsPropertyKey<Boolean>("FocusRingVisible")
var SemanticsPropertyReceiver.focusRingVisible by FocusRingVisible

/**
 * The values an animated control is showing right now (a scale, a progress), by name, read lazily. Tests use them to see that
 * a control moved, did not move (Reduced and Off) or was handed to the finger (interrupt). Screen readers do not see them.
 */
val MotionProbes = SemanticsPropertyKey<Map<String, () -> Float>>("MotionProbes") { parent, child -> (parent ?: emptyMap()) + child }
var SemanticsPropertyReceiver.motionProbes by MotionProbes

/** Expose [read] as the probe [name] of this node. */
fun Modifier.motionProbe(name: String, read: () -> Float): Modifier = semantics { motionProbes = mapOf(name to read) }

/** Expose several probes of this node at once (two `semantics` modifiers on one node do not merge a map). */
fun Modifier.motionProbes(vararg probes: Pair<String, () -> Float>): Modifier = semantics { motionProbes = probes.toMap() }

/** The reason a disabled control is disabled, when it has one. Tests read it. */
val DisabledReason = SemanticsPropertyKey<String>("DisabledReason")
var SemanticsPropertyReceiver.disabledReason by DisabledReason

/** The state a gallery entry forces on a control, through its interaction source. Real input produces the same interactions. */
enum class ForcedState { NONE, PRESSED, FOCUSED, HOVERED }

/** An interaction source that sits in [state] (the gallery shows pressed, focused and hovered controls side by side). */
@Composable
fun rememberForcedInteractionSource(state: ForcedState): MutableInteractionSource {
    val source = remember { MutableInteractionSource() }
    LaunchedEffect(state) {
        when (state) {
            ForcedState.NONE -> Unit
            ForcedState.PRESSED -> source.emit(PressInteraction.Press(Offset.Zero))
            ForcedState.FOCUSED -> source.emit(FocusInteraction.Focus())
            ForcedState.HOVERED -> source.emit(HoverInteraction.Enter())
        }
    }
    return source
}

/**
 * The focus ring, drawn outside the control's bounds (never clipped by it): 3 dp in the primary colour with a 2 dp gap.
 * It follows the keyboard focus of the control through [interactionSource].
 */
@Composable
fun Modifier.croftFocusRing(interactionSource: MutableInteractionSource, shape: Shape): Modifier {
    val focused by interactionSource.collectIsFocusedAsState()
    val colour = MaterialTheme.colorScheme.primary
    return this
        .semantics { focusRingVisible = focused }
        .drawWithContent {
            drawContent()
            if (focused) {
                val grow = (FocusRingGap + FocusRingStroke / 2).toPx()
                val stroke = FocusRingStroke.toPx()
                val outline = shape.createOutline(Size(size.width + 2 * grow, size.height + 2 * grow), layoutDirection, this)
                translate(-grow, -grow) { drawRing(outline, colour, Stroke(stroke)) }
            }
        }
}

/** Whether an element is lifted by a long-press right now. */
@androidx.compose.runtime.Stable
class LiftState {
    /** True from the long-press until the finger leaves. */
    var lifted by mutableStateOf(false)
}

/**
 * The lift look: 1 dp of elevation and 102 percent size (spatial fast) while [state] is lifted (or [force], for the gallery).
 * At Reduced the highlight only (no scale); at Off nothing moves. The spring runs from the value it has, so a new touch
 * during the animation takes it over.
 */
@Composable
internal fun Modifier.liftVisual(state: LiftState, force: Boolean = false): Modifier {
    val level = LocalMotionLevel.current
    val showLift = (state.lifted || force) && level == MotionLevel.FULL
    val progress by animateFloatAsState(if (showLift) 1f else 0f, MaterialTheme.motionScheme.fastSpatialSpec(), label = "long-press-lift")
    val liftPx = with(androidx.compose.ui.platform.LocalDensity.current) { LIFT.toPx() }
    return this
        .motionProbe("lift") { progress }
        .graphicsLayer {
            val scale = 1f + (LONG_PRESS_SCALE - 1f) * progress
            scaleX = scale; scaleY = scale
            shadowElevation = liftPx * progress
        }
}

private val LIFT = 1.dp
private const val LONG_PRESS_SCALE = 1.02f

/**
 * Taps, long-presses and keyboard activation for controls Material does not have (rows, input chips). Material's own
 * `combinedClickable` plays the platform long-press haptic by itself, which would bypass `Haptics`; this does not.
 * Press, focus and hover are reported through [interactionSource], so the ripple, the state layers and the focus ring work.
 * The action fires on release, so dragging away cancels it. A long-press fires at the system's long-press time (moving past
 * the touch slop first cancels it): the element lifts (see [liftVisual]), the [longClickEvent] haptic plays, then
 * [onLongClick] runs, all before the finger leaves.
 */
@Composable
fun Modifier.croftClickable(
    interactionSource: MutableInteractionSource,
    enabled: Boolean = true,
    role: Role? = null,
    onClickLabel: String? = null,
    onLongClickLabel: String? = null,
    onLongClick: (() -> Unit)? = null,
    longClickEvent: HapticEvent = HapticEvent.SelectionStart,
    forceLifted: Boolean = false,
    onClick: () -> Unit,
): Modifier {
    val onClickNow = rememberUpdated(onClick)
    val onLongNow = rememberUpdated(onLongClick)
    val haptics = LocalHaptics.current
    val lift = remember { LiftState() }
    return this
        .liftVisual(lift, forceLifted)
        .semantics(mergeDescendants = true) {
            if (role != null) this.role = role
            if (!enabled) disabled() else {
                onClick(label = onClickLabel) { onClickNow.value(); true }
                if (onLongClick != null) onLongClick(label = onLongClickLabel) { onLongNow.value?.invoke(); true }
            }
        }
        .indication(interactionSource, LocalIndication.current)
        .hoverable(interactionSource, enabled)
        .focusable(enabled, interactionSource)
        .onKeyEvent { e ->
            if (enabled && e.type == KeyEventType.KeyUp && (e.key == Key.Enter || e.key == Key.NumPadEnter || e.key == Key.DirectionCenter)) {
                onClickNow.value(); true
            } else false
        }
        .pointerInput(enabled, onLongClick != null) {
            if (enabled) detectTapGestures(
                onPress = { offset ->
                    val press = PressInteraction.Press(offset)
                    interactionSource.emit(press)
                    val released = try { tryAwaitRelease() } finally { lift.lifted = false }
                    interactionSource.emit(if (released) PressInteraction.Release(press) else PressInteraction.Cancel(press))
                },
                onLongPress = if (onLongClick != null) {
                    {
                        lift.lifted = true
                        haptics.perform(longClickEvent)
                        onLongNow.value?.invoke()
                    }
                } else null,
                onTap = { onClickNow.value() },
            )
        }
}

@Composable
private fun <T> rememberUpdated(v: T) = androidx.compose.runtime.rememberUpdatedState(v)

/**
 * Long-press feedback on an element that has no tap of its own (see [croftClickable] for one that has): the lift of
 * [liftVisual], the haptic [event], then [onLongPress]. [forceLifted] shows the lifted look (gallery).
 */
@Composable
fun Modifier.longPressLift(
    enabled: Boolean = true,
    event: HapticEvent = HapticEvent.SelectionStart,
    forceLifted: Boolean = false,
    onLongPress: () -> Unit,
): Modifier {
    val haptics = LocalHaptics.current
    val lift = remember { LiftState() }
    val longNow = rememberUpdated(onLongPress)
    return this
        .liftVisual(lift, forceLifted)
        .pointerInput(enabled) {
            if (enabled) detectTapGestures(
                onPress = { try { tryAwaitRelease() } finally { lift.lifted = false } },
                onLongPress = { lift.lifted = true; haptics.perform(event); longNow.value() },
            )
        }
}

/**
 * A disabled control still receives the touch so it can say why: a tap or long-press calls [onExplain] with [reason] (the
 * screen shows it once as a snackbar), and the reason is on the node as its state description for screen readers. Put it
 * before the control's own modifiers so the control stays disabled and announced as disabled.
 */
@Composable
fun Modifier.explainsWhenDisabled(enabled: Boolean, reason: String?, onExplain: (String) -> Unit): Modifier {
    if (enabled || reason == null) return this
    val haptics = LocalHaptics.current
    val explain = rememberUpdated(onExplain)
    return this
        .semantics { stateDescription = reason; disabledReason = reason }
        // Watches on the Initial pass without consuming, so it sees the touch even though the disabled control (and any
        // scroll container around it) takes no part in it.
        .pointerInput(reason) {
            awaitEachGesture {
                val down = awaitFirstDown(requireUnconsumed = false, pass = PointerEventPass.Initial)
                try {
                    val up = withTimeout(viewConfiguration.longPressTimeoutMillis) { waitForUpOrCancellation(PointerEventPass.Initial) }
                    if (up != null && (up.position - down.position).getDistance() <= viewConfiguration.touchSlop) {
                        haptics.perform(HapticEvent.Rejected)
                        explain.value(reason)
                    }
                } catch (e: PointerEventTimeoutCancellationException) {
                    explain.value(reason)
                    waitForUpOrCancellation(PointerEventPass.Initial)
                }
            }
        }
}

/** At least 48 by 48 dp for the touch target, whatever the drawn size. */
fun Modifier.minTouchTarget(): Modifier = minimumInteractiveComponentSize()

private fun androidx.compose.ui.graphics.drawscope.DrawScope.drawRing(outline: Outline, colour: androidx.compose.ui.graphics.Color, style: Stroke) {
    when (outline) {
        is Outline.Rectangle -> drawRect(colour, outline.rect.topLeft, outline.rect.size, style = style)
        is Outline.Rounded -> {
            val r = outline.roundRect
            drawRoundRect(colour, androidx.compose.ui.geometry.Offset(r.left, r.top), androidx.compose.ui.geometry.Size(r.width, r.height), androidx.compose.ui.geometry.CornerRadius(r.topLeftCornerRadius.x, r.topLeftCornerRadius.y), style = style)
        }
        is Outline.Generic -> drawPath(outline.path, colour, style = style)
    }
}
