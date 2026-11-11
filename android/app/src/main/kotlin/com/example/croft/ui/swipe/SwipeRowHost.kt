// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.ui.swipe

import androidx.compose.animation.animateColorAsState
import androidx.compose.animation.core.animate
import androidx.compose.animation.core.FiniteAnimationSpec
import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.animation.core.snap
import androidx.compose.foundation.background
import androidx.compose.foundation.gestures.awaitEachGesture
import androidx.compose.foundation.gestures.awaitFirstDown
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.absoluteOffset
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.systemGestures
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.Stable
import androidx.compose.runtime.compositionLocalOf
import androidx.compose.runtime.derivedStateOf
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.setValue
import androidx.compose.runtime.snapshotFlow
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.clipToBounds
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.input.pointer.AwaitPointerEventScope
import androidx.compose.ui.input.pointer.PointerEventPass
import androidx.compose.ui.input.pointer.PointerInputChange
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.input.pointer.util.VelocityTracker
import androidx.compose.ui.input.pointer.util.addPointerInputChange
import androidx.compose.ui.layout.layout
import androidx.compose.ui.layout.onGloballyPositioned
import androidx.compose.ui.layout.onSizeChanged
import androidx.compose.ui.layout.positionInWindow
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalLayoutDirection
import androidx.compose.ui.platform.LocalWindowInfo
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.unit.Constraints
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.IntOffset
import androidx.compose.ui.unit.LayoutDirection
import androidx.compose.ui.unit.dp
import com.example.croft.haptics.HapticEvent
import com.example.croft.haptics.Haptics
import com.example.croft.haptics.LocalHaptics
import com.example.croft.haptics.NoHaptics
import com.example.croft.swipe.SwipeAction
import com.example.croft.swipe.SwipeCancel
import com.example.croft.swipe.SwipeContext
import com.example.croft.swipe.SwipeDirection
import com.example.croft.swipe.SwipeEffect
import com.example.croft.swipe.SwipeGeometry
import com.example.croft.swipe.SwipeHaptic
import com.example.croft.swipe.SwipeInput
import com.example.croft.swipe.SwipeMachine
import com.example.croft.swipe.SwipePhase
import com.example.croft.swipe.SwipeSettings
import com.example.croft.swipe.SwipeSlot
import com.example.croft.swipe.SwipeState
import com.example.croft.theme.LocalMotionLevel
import com.example.croft.theme.LocalThemeSettings
import com.example.croft.theme.ListStyle
import com.example.croft.theme.MotionLevel
import com.example.croft.theme.SwipeLooks
import com.example.croft.theme.swipePillColours
import com.example.croft.ui.components.SwipeCardLook
import com.example.croft.ui.components.motionProbes
import com.example.croft.design.CroftContrast
import kotlin.math.abs
import kotlin.math.max
import kotlin.math.min
import kotlin.math.roundToInt
import kotlin.math.tan
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.coroutineScope
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.launch

/**
 * The width, in dp, of the strip at each side of the window where a drag belongs to the system's back and drawer gestures. Null
 * (the default) reads the real system gesture insets; a test sets a number, because a test window has none.
 */
val LocalBackGestureInset = compositionLocalOf<Dp?> { null }

/** What the host reports to the screen. */
class SwipeCallbacks(
    /** A release committed an action that leaves the row in place (read, star): run it now; the card is springing back. */
    val onKeep: (SwipeAction) -> Unit,
    /** A removing action finished sliding out and collapsing: run it. Call the second argument if it failed, so the row comes back. */
    val onRemoved: (SwipeAction, () -> Unit) -> Unit,
    /** An armed Snooze, Move to or Delete forever parked the card: open the sheet or dialog. */
    val onPark: (SwipeAction) -> Unit,
    /** An action that this row cannot do was asked for: the "Not available here" snackbar and its announcement. */
    val onNotAvailable: (SwipeAction) -> Unit,
)

private fun SwipeHaptic.toEvent(): HapticEvent = when (this) {
    SwipeHaptic.ThresholdArmed -> HapticEvent.ThresholdArmed
    SwipeHaptic.ThresholdDisarmed -> HapticEvent.ThresholdDisarmed
    SwipeHaptic.Committed -> HapticEvent.Committed
    SwipeHaptic.Rejected -> HapticEvent.Rejected
}

/** The springs the driver animates with, from the motion scheme in force (expressive, standard at Reduced, jumps at Off). */
internal class SwipeSpecs(
    val fastSpatial: FiniteAnimationSpec<Float>,
    val defaultSpatial: FiniteAnimationSpec<Float>,
    val defaultEffects: FiniteAnimationSpec<Float>,
)

/**
 * Everything one row's swipe does apart from reading the finger: the pure machine, the card's offset, the fade of the Reduced
 * motion commit and the collapse of the slot. Created per row by [SwipeRowHost]. Pixels in the animated values, dp in the machine.
 * The offset is a plain state value that the finger writes directly and a spring animation ([animate]) writes between touches;
 * a touch cancels the running animation and takes the card from where it is, so nothing waits for an animation to end, and a
 * release hands its velocity to the spring.
 */
@Stable
internal class SwipeRowDriver(private val scope: CoroutineScope) {
    private val machine = SwipeMachine(
        SwipeTokens.tuning,
        SwipeContext(1f, false, SwipeSettings.Default.sensitivity, SwipeSlot(SwipeAction.NONE), SwipeSlot(SwipeAction.NONE)),
    )

    /** What the machine says now; drawn from, and read by tests through the probes. */
    var state by mutableStateOf(SwipeState())
        private set

    /** The card's offset in px, positive to the right on screen. */
    var offsetPx by mutableFloatStateOf(0f)
        private set

    /** Opacity of the whole row (the Reduced motion commit fades instead of travelling). */
    var fade by mutableFloatStateOf(1f)
        private set

    /** Height of the slot as a fraction of its full height. */
    var collapse by mutableFloatStateOf(1f)
        private set

    var density = 1f
    var widthPx = 0
    var rtl = false
    var level = MotionLevel.FULL
    var haptics: Haptics = NoHaptics
    var specs: SwipeSpecs = SwipeSpecs(snap(), snap(), snap())
    var callbacks: SwipeCallbacks = SwipeCallbacks({}, { _, _ -> }, {}, {})
    private var slid = false

    /** The preview's own demonstration is moving the card: no haptics for it, and a touch ends it. */
    var silent = false

    /** A finger is on the card (a touch ends the demonstration). */
    var touched by mutableStateOf(false)
    private var offsetJob: Job? = null
    private var releaseVelocityPx = 0f

    /** The row's width, layout direction, sensitivity and slots as they are now; asked for at every input, so a drag never uses stale ones. */
    var contextProvider: () -> SwipeContext = { machine.context }

    private fun send(input: SwipeInput) {
        machine.context = contextProvider()
        val effects = machine.send(input)
        state = machine.state
        apply(effects)
    }

    /** True while the card is the finger's or can be taken by it. */
    val canBeGrabbed: Boolean
        get() = state.phase == SwipePhase.IDLE || state.phase == SwipePhase.SETTLING_BACK || state.phase == SwipePhase.DRAGGING || state.phase == SwipePhase.ARMED ||
            // A read or star that is springing back can be swiped again at once, to reverse it.
            (state.phase == SwipePhase.COMMITTING && !state.action.removesRow)

    /** The finger took hold: whatever moved the card stops where it is. Returns the card's place in px. */
    fun takeOver(): Float {
        offsetJob?.cancel()
        silent = false
        touched = true
        return offsetPx
    }

    /** The finger is up or the gesture was left to someone else. */
    fun touchEnded() { touched = false }

    /**
     * The looped demonstration of S47: each of [directions] in turn, the card is dragged past the arm point, held, released, and
     * the commit plays (a removing action collapses and then the row is restored, so the loop can go on). It makes no sound or
     * vibration, waits [pauseMs] between stages, gives way at once to a finger, and never runs at Animation level Off.
     */
    suspend fun runDemo(directions: List<SwipeDirection>, pauseMs: Long) {
        if (level == MotionLevel.OFF || directions.isEmpty()) return
        while (true) {
            for (d in directions) {
                kotlinx.coroutines.delay(pauseMs)
                if (touched || state.phase != SwipePhase.IDLE || widthPx == 0) continue
                silent = true
                var t = 0L
                val sign = d.physicalSign(rtl)
                val reach = DEMO_REACH * widthPx / density
                try {
                    animate(0f, reach, 0f, specs.defaultSpatial) { v, _ ->
                        if (touched) throw kotlinx.coroutines.CancellationException("a finger took the card")
                        t += FRAME_MS
                        drag(sign * v, t)
                    }
                    kotlinx.coroutines.delay(pauseMs)
                    if (!touched) release(0f, t + FRAME_MS)
                    snapshotFlow { state.phase == SwipePhase.IDLE }.first { it }
                } catch (e: kotlinx.coroutines.CancellationException) {
                    if (!touched) throw e
                } finally {
                    silent = false
                }
            }
        }
    }

    /** A drag event: [dxDp] is the finger's distance from where it took hold of the card, in dp. */
    fun drag(dxDp: Float, timeMs: Long) {
        send(SwipeInput.Drag(dxDp, timeMs))
        if (state.phase == SwipePhase.DRAGGING || state.phase == SwipePhase.ARMED || state.phase == SwipePhase.IDLE) offsetPx = state.offset * density
    }

    /** The finger lifted at [velocityPx] px per second. */
    fun release(velocityPx: Float, timeMs: Long) {
        releaseVelocityPx = velocityPx
        send(SwipeInput.Release(velocityPx / density, timeMs))
    }

    /** Cancels what the finger was doing without an action. */
    fun cancel(reason: SwipeCancel) { send(SwipeInput.Cancel(reason)) }

    /** The sheet or dialog of a parked card closed. */
    fun sheetResult(chosen: Boolean) { if (state.phase == SwipePhase.PARKED) send(SwipeInput.SheetResult(chosen)) }

    private fun apply(effects: List<SwipeEffect>) {
        for (e in effects) when (e) {
            is SwipeEffect.Haptic -> if (!silent) haptics.perform(e.kind.toEvent())
            is SwipeEffect.Commit -> commit(e.action)
            is SwipeEffect.Park -> { park(); callbacks.onPark(e.action) }
            SwipeEffect.SpringBack -> springBack()
            is SwipeEffect.NotAvailable -> callbacks.onNotAvailable(e.action)
            is SwipeEffect.Completed -> completed(e.action)
        }
    }

    private suspend fun animateOffset(target: Float, spec: FiniteAnimationSpec<Float>, velocity: Float = 0f) {
        animate(offsetPx, target, velocity, spec) { v, _ -> offsetPx = v }
    }

    private fun springBack() {
        val v = releaseVelocityPx
        offsetJob?.cancel()
        offsetJob = scope.launch {
            animateOffset(0f, specs.fastSpatial, v)
            send(SwipeInput.AnimationEnd)
        }
    }

    private fun park() {
        val target = state.offset * density
        offsetJob?.cancel()
        offsetJob = scope.launch { animateOffset(target, specs.defaultSpatial, releaseVelocityPx) }
    }

    private fun commit(action: SwipeAction) {
        if (action.removesRow) {
            offsetJob?.cancel()
            offsetJob = scope.launch {
                slideOutAndCollapse()
                send(SwipeInput.AnimationEnd)
            }
        } else {
            callbacks.onKeep(action)
            springBack()
        }
    }

    private fun completed(action: SwipeAction) {
        if (!action.removesRow) { send(SwipeInput.Reset); return }
        if (slid) callbacks.onRemoved(action, ::restore)
        else {
            // The sheet or dialog of a parked card was answered: the card leaves now.
            offsetJob?.cancel()
            offsetJob = scope.launch {
                slideOutAndCollapse()
                callbacks.onRemoved(action, ::restore)
            }
        }
    }

    /**
     * The commit sequence: the card slides off with the fast spatial spring while the pill widens to the full row width (the pill's
     * width follows the card), and the slot collapses with the default spatial spring from the moment the card is half way, so the two
     * overlap. At Reduced Motion the card does not travel: the row fades (effects default, about 100 ms) and the slot closes with
     * a fade-and-close; at Off every spring jumps to its end.
     */
    private suspend fun slideOutAndCollapse() {
        slid = true
        val sign = (state.direction ?: SwipeDirection.START_TO_END).physicalSign(rtl)
        val target = sign * widthPx.toFloat()
        if (level == MotionLevel.REDUCED) {
            animate(fade, 0f, 0f, specs.defaultEffects) { v, _ -> fade = v }
            animate(collapse, 0f, 0f, specs.defaultEffects) { v, _ -> collapse = v }
        } else coroutineScope {
            launch { animateOffset(target, specs.fastSpatial, releaseVelocityPx) }
            launch {
                snapshotFlow { abs(offsetPx) }.first { it >= 0.5f * widthPx }
                animate(collapse, 0f, 0f, specs.defaultSpatial) { v, _ -> collapse = v }
            }
        }
    }

    /** The row is back in its place (the action failed): reopen the slot and start over. */
    fun restore() {
        offsetJob?.cancel()
        slid = false
        send(SwipeInput.Reset)
        offsetPx = 0f
        fade = 1f
        scope.launch { animate(collapse, 1f, 0f, specs.defaultSpatial) { v, _ -> collapse = v } }
    }
}

/**
 * One message row with its swipe: the row as a lifted card, the pill that fills the space it leaves, the gesture, and the commit
 * sequence (docs/ux/swipe.md). It wraps the row ([content], which receives the card look while the card is lifted) and a [trailing]
 * part (the divider) that collapses with it.
 *
 * The gesture is read in the Initial pass so that a swipe takes the touch from the row's own tap and the list's scroll: it starts
 * after 8 dp of horizontal movement within 30 degrees of horizontal; a drag that begins as a scroll never becomes a swipe; a drag that
 * starts in the system back-gesture strip is left to the system; a second finger cancels; nothing starts while the list is still
 * scrolling or when [enabled] is false (selection mode, pull to refresh, fast scrolling). A removed row ([alive] false) ends the
 * drag without an action.
 */
@Composable
fun SwipeRowHost(
    settings: SwipeSettings,
    enabled: Boolean,
    alive: Boolean,
    slotOf: (SwipeDirection) -> SwipeSlot,
    iconOf: (SwipeAction) -> ImageVector,
    listScrolling: () -> Boolean,
    callbacks: SwipeCallbacks,
    parkResolution: Boolean?,
    onParkResolutionHandled: () -> Unit,
    modifier: Modifier = Modifier,
    trailing: @Composable () -> Unit = {},
    /** The directions to demonstrate in a loop (S47's preview), or null. */
    demo: List<SwipeDirection>? = null,
    content: @Composable (card: SwipeCardLook?) -> Unit,
) {
    val scope = rememberCoroutineScope()
    val driver = remember { SwipeRowDriver(scope) }
    val density = LocalDensity.current
    val rtl = LocalLayoutDirection.current == LayoutDirection.Rtl
    val level = LocalMotionLevel.current
    val scheme = MaterialTheme.motionScheme
    val theme = LocalThemeSettings.current
    val haptics = LocalHaptics.current

    // The driver reads the latest of everything, so the gesture below never has to restart.
    driver.density = density.density
    driver.rtl = rtl
    driver.level = level
    driver.haptics = haptics
    driver.callbacks = callbacks
    driver.specs = SwipeSpecs(scheme.fastSpatialSpec(), scheme.defaultSpatialSpec(), scheme.defaultEffectsSpec())
    val settingsNow by rememberUpdatedState(settings)
    val slotOfNow by rememberUpdatedState(slotOf)
    driver.contextProvider = {
        SwipeContext(max(driver.widthPx / driver.density, 1f), driver.rtl, settingsNow.sensitivity, slotOfNow(SwipeDirection.START_TO_END), slotOfNow(SwipeDirection.END_TO_START))
    }

    val enabledNow by rememberUpdatedState(enabled)
    val aliveNow by rememberUpdatedState(alive)
    val scrollingNow by rememberUpdatedState(listScrolling)
    var heightPx by remember { mutableStateOf(0) }
    var leftInWindow by remember { mutableStateOf(0f) }
    val windowWidth = LocalWindowInfo.current.containerSize.width
    val insetOverride = LocalBackGestureInset.current
    val systemInsets = WindowInsets.systemGestures
    val layoutDirection = LocalLayoutDirection.current
    val leftInset = insetOverride?.let { with(density) { it.toPx() } } ?: systemInsets.getLeft(density, layoutDirection).toFloat()
    val rightInset = insetOverride?.let { with(density) { it.toPx() } } ?: systemInsets.getRight(density, layoutDirection).toFloat()
    val guard by rememberUpdatedState(BackGuard(leftInset, rightInset, windowWidth))

    LaunchedEffect(demo, level, settings) { if (demo != null) driver.runDemo(demo, SwipeLooks.PREVIEW_PAUSE_MS) }

    // A row that leaves while it is being dragged ends the drag without an action.
    LaunchedEffect(alive) { if (!alive) driver.cancel(SwipeCancel.ROW_REMOVED) }
    LaunchedEffect(parkResolution) {
        if (parkResolution != null) { driver.sheetResult(parkResolution); onParkResolutionHandled() }
    }

    val phase by remember { derivedStateOf { driver.state.phase } }
    val showPill by remember { derivedStateOf { driver.state.phase != SwipePhase.IDLE } }
    val armed by remember { derivedStateOf { driver.state.phase == SwipePhase.ARMED } }
    val action by remember { derivedStateOf { driver.state.action } }
    val direction by remember { derivedStateOf { driver.state.direction } }
    val available = remember(direction, settings, slotOf) { direction?.let { slotOf(it).available } ?: true }

    val iconScale by animateFloatAsState(if (armed && level == MotionLevel.FULL) SwipeTokens.armedIconScale else 1f, scheme.fastSpatialSpec(), label = "swipe-icon-scale")

    val cardRadiusDp by remember(theme.roundness, theme.listStyle) {
        derivedStateOf {
            val rest = if (theme.listStyle == ListStyle.FLAT) 0f else theme.roundness.swipeCardRadiusDp.toFloat()
            SwipeGeometry.cardRadius(SwipeTokens.geometry, abs(driver.offsetPx) / driver.density, rest, theme.roundness.swipeCardRadiusDp.toFloat()).roundToInt()
        }
    }
    val outlined = theme.contrast == CroftContrast.HIGH
    val pageColour = MaterialTheme.colorScheme.surfaceContainer
    val cardLook = if (showPill) SwipeCardLook(RoundedCornerShape(cardRadiusDp.dp), outlined) else null

    Column(
        modifier
            .layout { measurable, constraints ->
                val p = measurable.measure(constraints)
                val h = (p.height * driver.collapse.coerceIn(0f, 1f)).roundToInt()
                layout(p.width, h) { p.placeRelative(0, 0) }
            }
            .clipToBounds()
            .graphicsLayer { alpha = driver.fade }
            .motionProbes(
                "swipe-offset" to { driver.offsetPx / driver.density },
                "swipe-collapse" to { driver.collapse },
                "swipe-fade" to { driver.fade },
                "swipe-icon-scale" to { iconScale },
                "swipe-phase" to { driver.state.phase.ordinal.toFloat() },
            ),
    ) {
        Box(
            Modifier
                .fillMaxWidth()
                .onSizeChanged { driver.widthPx = it.width; heightPx = it.height }
                .onGloballyPositioned { leftInWindow = it.positionInWindow().x }
                // The page behind the card (the container tone) shows only while the card is lifted. Drawn, not a changing modifier, so the gesture below is never rebuilt.
                .drawBehind { if (showPill) drawRect(pageColour) }
                .pointerInput(Unit) {
                    val dens = this.density
                    val slop = SwipeTokens.SLOP_DP * dens
                    val tangent = tan(Math.toRadians(SwipeTokens.MAX_ANGLE_DEG.toDouble())).toFloat()
                    awaitEachGesture {
                        val down = awaitFirstDown(requireUnconsumed = false, pass = PointerEventPass.Initial)
                        val x = leftInWindow + down.position.x
                        val g = guard
                        val inBackStrip = x < g.left || x > g.windowWidth - g.right
                        if (!enabledNow || !aliveNow || inBackStrip || scrollingNow() || !driver.canBeGrabbed) {
                            ignoreRestOfGesture()
                            return@awaitEachGesture
                        }
                        // Undecided: scroll, swipe or tap? The first axis to pass the slop decides, and a scroll never becomes a swipe.
                        var origin = 0f
                        var decided = false
                        while (!decided) {
                            val event = awaitPointerEvent(PointerEventPass.Initial)
                            val change = event.changes.firstOrNull { it.id == down.id }
                            if (change == null || !change.pressed) return@awaitEachGesture
                            if (event.changes.count { it.pressed } > 1 || change.isConsumed) { ignoreRestOfGesture(); return@awaitEachGesture }
                            val dx = change.position.x - down.position.x
                            val dy = change.position.y - down.position.y
                            if (abs(dx) >= slop || abs(dy) >= slop) {
                                if (abs(dx) >= slop && abs(dy) <= abs(dx) * tangent) { decided = true; origin = change.position.x; change.consume() }
                                else { ignoreRestOfGesture(); return@awaitEachGesture }
                            }
                        }
                        // A swipe: from here the card follows the finger one to one, from wherever it is.
                        val tracker = VelocityTracker()
                        val base = driver.takeOver()
                        try {
                        while (true) {
                            val event = awaitPointerEvent(PointerEventPass.Initial)
                            if (!aliveNow || !enabledNow) {
                                driver.cancel(if (!aliveNow) SwipeCancel.ROW_REMOVED else SwipeCancel.SELECTION)
                                ignoreRestOfGesture()
                                return@awaitEachGesture
                            }
                            if (event.changes.count { it.pressed } > 1) {
                                driver.cancel(SwipeCancel.SECOND_POINTER)
                                ignoreRestOfGesture()
                                return@awaitEachGesture
                            }
                            val change = event.changes.firstOrNull { it.id == down.id } ?: continue
                            tracker.addPointerInputChange(change)
                            if (!change.pressed) {
                                val velocityPx = tracker.calculateVelocity().x
                                change.consume()
                                driver.release(velocityPx, change.uptimeMillis)
                                return@awaitEachGesture
                            }
                            val dxDp = (base + (change.position.x - origin)) / dens
                            change.consume()
                            driver.drag(dxDp, change.uptimeMillis)
                        }
                        } finally { driver.touchEnded() }
                    }
                },
        ) {
            if (showPill) {
                SwipePill(
                    offsetPx = { driver.offsetPx },
                    rowWidthPx = { driver.widthPx },
                    density = driver.density,
                    action = action,
                    available = available,
                    armed = armed,
                    icon = iconOf(action),
                    modifier = Modifier.matchParentSize(),
                    iconScale = { iconScale },
                )
            }
            Box(Modifier.absoluteOffset { IntOffset(driver.offsetPx.roundToInt(), 0) }) { content(cardLook) }
        }
        trailing()
    }
}

private const val DISC_ALPHA = 0.16f

/** How far the demonstration drags: past the arm point at every sensitivity (as a fraction of the row width). */
private const val DEMO_REACH = 0.5f

/** The step of the demonstration's time stamps. */
private const val FRAME_MS = 16L

/** The strips at the window edges that belong to the system, and the window width (px). */
private class BackGuard(val left: Float, val right: Float, val windowWidth: Int)

/** Waits until every finger is up, consuming nothing: the rest of this gesture is not ours. */
private suspend fun AwaitPointerEventScope.ignoreRestOfGesture(consume: Boolean = false) {
    do {
        val event = awaitPointerEvent(PointerEventPass.Initial)
        if (consume) event.changes.forEach { it.consume() }
    } while (event.changes.any { it.pressed })
}


/**
 * The pill that fills the space the card leaves (docs/ux/swipe.md): row height, [SwipeGeometry]'s width for the card's offset, 4 dp from
 * the card and 8 dp from the row's outer edge, a stadium whose radius is half its smaller side, the action's icon centred once the pill is
 * wide enough, its tonal colour below the arm point and its full colour armed. At Reduced and Off, where the icon does not grow, an armed
 * pill also puts a soft disc behind the icon, so the arm point is still a change of shape and not only of colour. Decorative: screen
 * readers reach the action through the row's custom actions.
 */
@Composable
internal fun SwipePill(
    offsetPx: () -> Float,
    rowWidthPx: () -> Int,
    density: Float,
    action: SwipeAction,
    available: Boolean,
    armed: Boolean,
    icon: ImageVector,
    modifier: Modifier = Modifier,
    iconScale: () -> Float = { 1f },
) {
    val level = LocalMotionLevel.current
    val scheme = MaterialTheme.motionScheme
    val colours = swipePillColours(action, available)
    val pillColour by animateColorAsState(if (armed) colours.full else colours.container, scheme.defaultEffectsSpec(), label = "swipe-pill-colour")
    val iconColour by animateColorAsState(if (armed) colours.onFull else colours.onContainer, scheme.defaultEffectsSpec(), label = "swipe-icon-colour")
    val rightwards by remember { derivedStateOf { offsetPx() >= 0f } }
    var rowHeightPx by remember { mutableStateOf(0) }
    Box(modifier.padding(horizontal = SwipeTokens.geometry.insetFromEdge.dp).onSizeChanged { rowHeightPx = it.height }.clearAndSetSemantics { }) {
        Box(
            Modifier
                .align(if (rightwards) Alignment.CenterStart else Alignment.CenterEnd)
                .layout { measurable, constraints ->
                    val rowH = constraints.maxHeight
                    val geo = SwipeGeometry.pill(SwipeTokens.geometry, abs(offsetPx()) / density, rowH / density)
                    val full = SwipeGeometry.committedPill(SwipeTokens.geometry, rowWidthPx() / density, rowH / density)
                    val w = (min(geo.width, full.width) * density).roundToInt().coerceAtLeast(0)
                    val p = measurable.measure(Constraints.fixed(w, rowH))
                    layout(w, rowH) { p.place(0, 0) }
                }
                .clip(CircleShape)
                .background(pillColour),
            contentAlignment = Alignment.Center,
        ) {
            Box(contentAlignment = Alignment.Center) {
                if (level != MotionLevel.FULL && armed) Box(Modifier.size((SwipeLooks.ICON_DP + 12).dp).clip(CircleShape).background(iconColour.copy(alpha = DISC_ALPHA)))
                Icon(
                    icon,
                    contentDescription = null,
                    tint = iconColour,
                    modifier = Modifier
                        .size(SwipeLooks.ICON_DP.dp)
                        .graphicsLayer {
                            val visible = SwipeGeometry.pill(SwipeTokens.geometry, abs(offsetPx()) / density, rowHeightPx / density).iconVisible
                            alpha = if (visible) 1f else 0f
                            scaleX = iconScale(); scaleY = iconScale()
                        },
                )
            }
        }
    }
}

/**
 * A row held in the middle of a swipe, drawn without a finger (the component gallery, and screenshots): the page behind it, the
 * pill and the card at [offsetDp] from the resting place, to the right when positive. [content] receives the card look.
 */
@Composable
fun SwipeRowFrozen(
    offsetDp: Float,
    action: SwipeAction,
    armed: Boolean,
    modifier: Modifier = Modifier,
    available: Boolean = true,
    icon: ImageVector? = null,
    content: @Composable (card: SwipeCardLook) -> Unit,
) {
    val density = LocalDensity.current
    val theme = LocalThemeSettings.current
    var widthPx by remember { mutableStateOf(0) }
    val offsetPx = offsetDp * density.density
    val rest = if (theme.listStyle == ListStyle.FLAT) 0f else theme.roundness.swipeCardRadiusDp.toFloat()
    val radius = SwipeGeometry.cardRadius(SwipeTokens.geometry, abs(offsetDp), rest, theme.roundness.swipeCardRadiusDp.toFloat())
    Box(modifier.fillMaxWidth().background(MaterialTheme.colorScheme.surfaceContainer).onSizeChanged { widthPx = it.width }) {
        SwipePill(
            offsetPx = { offsetPx },
            rowWidthPx = { widthPx },
            density = density.density,
            action = action,
            available = available,
            armed = armed,
            icon = icon ?: swipeIcon(action, null),
            modifier = Modifier.matchParentSize(),
            iconScale = { if (armed) SwipeTokens.armedIconScale else 1f },
        )
        Box(Modifier.absoluteOffset { IntOffset(offsetPx.roundToInt(), 0) }) {
            content(SwipeCardLook(RoundedCornerShape(radius.dp), theme.contrast == CroftContrast.HIGH))
        }
    }
}
