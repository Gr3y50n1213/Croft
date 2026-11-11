// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.ui.components

import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.EnterTransition
import androidx.compose.animation.ExitTransition
import androidx.compose.animation.expandVertically
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.shrinkVertically
import androidx.compose.animation.slideInVertically
import androidx.compose.animation.slideOutVertically
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Close
import androidx.compose.material.icons.filled.Info
import androidx.compose.material.icons.filled.Warning
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Snackbar
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.Stable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalAccessibilityManager
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import com.example.croft.R
import com.example.croft.design.CroftTokens
import com.example.croft.theme.CroftTimings
import com.example.croft.theme.LocalMotionLevel
import com.example.croft.theme.MotionLevel
import kotlinx.coroutines.delay

/** The persistent banners (docs/ux/controls.md). */
enum class BannerKind {
    /** The account needs sign-in. The most urgent. */
    NEEDS_SIGN_IN,

    /** No network. */
    OFFLINE,

    /** Remote images are blocked until the person allows them. */
    REMOTE_IMAGES_BLOCKED,

    /** A mailing list message: offer Unsubscribe. */
    UNSUBSCRIBE,
    ;

    /** Lower is more urgent. */
    internal val urgency: Int get() = ordinal

    /** The banner that has an action button. Offline is informational. */
    val hasAction: Boolean get() = this != OFFLINE
}

/** At most one banner at a time: the most urgent of those [active] (docs/ux/controls.md). */
fun mostUrgentBanner(active: Collection<BannerKind>): BannerKind? = active.minByOrNull { it.urgency }

/**
 * Shows the most urgent of [active] banners, one at a time. It slides in and pushes the content below it (spatial default)
 * at Full; it just appears at Reduced and Off (docs/ux/motion.md, Offline banner).
 */
@Composable
fun BannerHost(
    active: Collection<BannerKind>,
    onAction: (BannerKind) -> Unit,
    onDismiss: (BannerKind) -> Unit,
    modifier: Modifier = Modifier,
    accountName: String = "",
) {
    val level = LocalMotionLevel.current
    val shown = mostUrgentBanner(active)
    val enter = if (level == MotionLevel.FULL) expandVertically(MaterialTheme.motionScheme.defaultSpatialSpec()) + fadeIn(MaterialTheme.motionScheme.defaultEffectsSpec()) else EnterTransition.None
    val exit = if (level == MotionLevel.FULL) shrinkVertically(MaterialTheme.motionScheme.defaultSpatialSpec()) + fadeOut(MaterialTheme.motionScheme.defaultEffectsSpec()) else ExitTransition.None
    AnimatedVisibility(visible = shown != null, modifier = modifier, enter = enter, exit = exit) {
        // Keep drawing the last banner while it leaves.
        var last by rememberMutable { shown }
        if (shown != null) last = shown
        last?.let { CroftBanner(it, { onAction(it) }, { onDismiss(it) }, accountName = accountName) }
    }
}

@Composable
private fun <T> rememberMutable(init: () -> T) = androidx.compose.runtime.remember { mutableStateOf(init()) }

/** A banner: icon, words, an action when it has one and a dismiss button. Colour is never the only signal. */
@Composable
fun CroftBanner(
    kind: BannerKind,
    onAction: () -> Unit,
    onDismiss: () -> Unit,
    modifier: Modifier = Modifier,
    accountName: String = "",
) {
    val colors = MaterialTheme.colorScheme
    val urgent = kind == BannerKind.NEEDS_SIGN_IN
    val message = when (kind) {
        BannerKind.NEEDS_SIGN_IN -> stringResource(R.string.banner_needs_sign_in, accountName)
        BannerKind.OFFLINE -> stringResource(R.string.banner_offline)
        BannerKind.REMOTE_IMAGES_BLOCKED -> stringResource(R.string.banner_remote_images)
        BannerKind.UNSUBSCRIBE -> stringResource(R.string.banner_unsubscribe)
    }
    val action = when (kind) {
        BannerKind.NEEDS_SIGN_IN -> stringResource(R.string.banner_action_sign_in)
        BannerKind.REMOTE_IMAGES_BLOCKED -> stringResource(R.string.banner_action_show_images)
        BannerKind.UNSUBSCRIBE -> stringResource(R.string.banner_action_unsubscribe)
        BannerKind.OFFLINE -> null
    }
    // At a large text size the action no longer fits beside the message, so it moves below it instead of squeezing it out.
    val bigText = LocalDensity.current.fontScale > BANNER_STACK_FROM_FONT_SCALE
    val buttons: @Composable () -> Unit = {
        if (action != null) TextButton(onClick = onAction) { Text(action) }
        IconButton(onClick = onDismiss) { Icon(Icons.Filled.Close, contentDescription = stringResource(R.string.banner_dismiss)) }
    }
    Surface(
        modifier = modifier.fillMaxWidth().semantics { liveRegion = LiveRegionMode.Polite },
        color = if (urgent) colors.errorContainer else colors.surfaceContainerHigh,
        contentColor = if (urgent) colors.onErrorContainer else colors.onSurface,
    ) {
        if (bigText) {
            Column(Modifier.padding(start = 16.dp, top = 12.dp, bottom = 4.dp, end = 4.dp)) {
                Row(verticalAlignment = Alignment.Top, horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                    Icon(if (urgent) Icons.Filled.Warning else Icons.Filled.Info, contentDescription = null, Modifier.size(20.dp))
                    Text(message, Modifier.weight(1f), style = MaterialTheme.typography.bodyMedium)
                }
                Row(Modifier.align(Alignment.End), verticalAlignment = Alignment.CenterVertically) { buttons() }
            }
        } else {
            Row(Modifier.padding(start = 16.dp, top = 4.dp, bottom = 4.dp, end = 4.dp), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                Icon(if (urgent) Icons.Filled.Warning else Icons.Filled.Info, contentDescription = null, Modifier.size(20.dp))
                Text(message, Modifier.weight(1f), style = MaterialTheme.typography.bodyMedium)
                buttons()
            }
        }
    }
}

/** From this combined font scale a banner puts its buttons under the message. */
private const val BANNER_STACK_FROM_FONT_SCALE = 1.3f

/** One snackbar message. */
@Stable
class SnackbarMessage internal constructor(
    internal val id: Long,
    /** The text. */
    val text: String,
    /** The action's label (Undo, Retry), or null. */
    val actionLabel: String?,
    internal val onAction: (() -> Unit)?,
    /** How long it stays when there is no accessibility timeout to honour. Null: [CroftTimings.SNACKBAR_MIN_MS]. */
    val stayMs: Long?,
)

/**
 * The snackbar rules (docs/ux/controls.md): one at a time, and a new one replaces the old one at once. The earlier action
 * is not run, not cancelled and not undone by the replacement: only its button goes away, so what it would have undone
 * stays queued and runs on schedule (the caller owns that queue; deleted mail is in Trash, a queued send in the Outbox).
 */
@Stable
class CroftSnackbarState {
    private var nextId = 0L

    /** The one showing now, or null. */
    var current by mutableStateOf<SnackbarMessage?>(null)
        private set

    /** Show [text]; whatever showed before is replaced. [stayMs] is the Undo window for an Undo snackbar. */
    fun show(text: String, actionLabel: String? = null, stayMs: Long? = null, onAction: (() -> Unit)? = null): SnackbarMessage {
        val message = SnackbarMessage(++nextId, text, actionLabel, onAction, stayMs)
        current = message
        return message
    }

    /** The action button was pressed (it can be pressed once, and only on the message that is showing). */
    fun perform(message: SnackbarMessage) {
        if (current?.id != message.id) return
        current = null
        message.onAction?.invoke()
    }

    /** Remove [message] if it is still the one showing. */
    fun dismiss(message: SnackbarMessage) {
        if (current?.id == message.id) current = null
    }

    /** The undo window of the default setting, in ms (`tokens.json`, swipe.undoWindowSeconds.default). */
    companion object {
        /** Five seconds unless S47 says otherwise. */
        val DefaultUndoWindowMs: Long = CroftTokens.Swipe.UndoWindowSeconds.default * 1000L
    }
}

/**
 * Draws the snackbar of [state]. It stays [SnackbarMessage.stayMs] (or 4 s without an action); where the system reports a
 * longer accessibility timeout the longer one wins. It rises with the spatial fast spring and fades (a fade at Reduced,
 * nothing at Off), is a polite live region, and sits where the caller puts the host (above the bottom bar).
 */
@Composable
fun CroftSnackbarHost(state: CroftSnackbarState, modifier: Modifier = Modifier) {
    val level = LocalMotionLevel.current
    val manager = LocalAccessibilityManager.current
    val message = state.current
    val stateNow by rememberUpdatedState(state)
    LaunchedEffect(message?.id) {
        val m = message ?: return@LaunchedEffect
        val base = m.stayMs ?: CroftTimings.SNACKBAR_MIN_MS
        val wait = manager?.calculateRecommendedTimeoutMillis(base, containsIcons = false, containsText = true, containsControls = m.actionLabel != null) ?: base
        delay(maxOf(base, wait))
        stateNow.dismiss(m)
    }
    val enter = when (level) {
        MotionLevel.FULL -> slideInVertically(MaterialTheme.motionScheme.fastSpatialSpec()) { it } + fadeIn(MaterialTheme.motionScheme.defaultEffectsSpec())
        MotionLevel.REDUCED -> fadeIn(MaterialTheme.motionScheme.defaultEffectsSpec())
        MotionLevel.OFF -> EnterTransition.None
    }
    val exit = when (level) {
        MotionLevel.FULL -> slideOutVertically(MaterialTheme.motionScheme.fastSpatialSpec()) { it } + fadeOut(MaterialTheme.motionScheme.defaultEffectsSpec())
        MotionLevel.REDUCED -> fadeOut(MaterialTheme.motionScheme.defaultEffectsSpec())
        MotionLevel.OFF -> ExitTransition.None
    }
    var last by rememberMutable { message }
    if (message != null) last = message
    AnimatedVisibility(visible = message != null, modifier = modifier, enter = enter, exit = exit) {
        val shown = last
        if (shown != null) {
            Snackbar(
                modifier = Modifier.padding(12.dp).semantics { liveRegion = LiveRegionMode.Polite },
                action = shown.actionLabel?.let { label -> { TextButton(onClick = { state.perform(shown) }) { Text(label) } } },
            ) { Text(shown.text) }
        }
    }
}
