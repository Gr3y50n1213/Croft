// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.ui.components

import androidx.compose.animation.core.Animatable
import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.interaction.collectIsPressedAsState
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.IntrinsicSize
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Check
import androidx.compose.material.icons.filled.Lock
import androidx.compose.material.icons.filled.Star
import androidx.compose.material.icons.filled.Warning
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.CustomAccessibilityAction
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.customActions
import androidx.compose.ui.semantics.selected
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.res.pluralStringResource
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import com.example.croft.R
import com.example.croft.theme.CroftTimings
import com.example.croft.theme.ListStyle
import com.example.croft.theme.LocalMotionLevel
import com.example.croft.theme.LocalThemeSettings
import com.example.croft.theme.MotionLevel
import com.example.croft.theme.UnreadEmphasis
import com.example.croft.theme.AvatarStyle
import com.example.croft.theme.linearFadeSpec
import com.example.croft.theme.rowStarColour

/** The lock beside a message: encrypted, or signed with a signature that does not check out (a warning triangle instead). */
enum class EncryptionMark { NONE, ENCRYPTED, SIGNED_INVALID }

/**
 * Everything about a message row that changes how it looks (docs/ux/controls.md, "Message row"). Pressed and focused are not
 * here: they come from the interaction source, as they do from real input.
 */
data class MessageRowState(
    val unread: Boolean = false,
    val selected: Boolean = false,
    val starred: Boolean = false,
    val encryption: EncryptionMark = EncryptionMark.NONE,
    /** A small clock beside the time: the action waits for the network. */
    val pendingOffline: Boolean = false,
    /** A permanent refusal: the row returns with a red edge. */
    val failed: Boolean = false,
    /** Just arrived: a tint that fades over 1.2 s. The caller sets it only while the row is on screen. */
    val isNew: Boolean = false,
    val hasAttachment: Boolean = false,
    /** Messages in the conversation; a count chip shows when there is more than one. */
    val threadCount: Int = 1,
)

/** How a row looks while the swipe engine holds it as a card (T15): its shape, and the High contrast outline. */
class SwipeCardLook(val shape: androidx.compose.ui.graphics.Shape, val outlined: Boolean)

/** One extra action of a row for screen readers and switch users (the configured swipe actions fill the list). */
class RowAction(val label: String, val onClick: () -> Unit)

/** Whether swiping the row is on. Selection mode turns it off; the swipe engine (T15) reads it. Tests read it too. */
val SwipeEnabled = androidx.compose.ui.semantics.SemanticsPropertyKey<Boolean>("SwipeEnabled")
var androidx.compose.ui.semantics.SemanticsPropertyReceiver.swipeEnabled by SwipeEnabled

private val RedEdgeWidth = 4.dp
private const val UNREAD_DOT_DP = 8

/**
 * A message row. Read: regular weight. Unread: bold sender and subject, a dot at the leading edge and the time in the strong
 * colour (as far as S52 "Unread emphasis" allows). Selected: primary container tint, and the avatar turns into a check
 * (a flip with the spatial fast spring; a cross-fade at Reduced; at once at Off). Dragging: the row is a rounded card that
 * follows the finger and a separate pill fills the space it leaves (`SwipeRowHost`, T15, draws both around this row; [card] is the card's look). New: a
 * 1.2 s tint. Starred, encrypted, pending and failed as in the table.
 *
 * Tap opens; long-press selects (haptic SelectionStart); tapping the avatar selects. Screen readers hear the state first,
 * then the content, and get Select and Star as actions, so nobody needs a swipe or a long-press.
 */
@Composable
fun MessageRow(
    sender: String,
    subject: String,
    preview: String,
    time: String,
    state: MessageRowState,
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
    onLongClick: (() -> Unit)? = null,
    onAvatarClick: (() -> Unit)? = null,
    onToggleStar: (() -> Unit)? = null,
    /**
     * Set while the swipe engine lifts the row as a card (T15): the card's shape (its corners ease from the row's own to the
     * swipe card radius), and whether it gets the 1 dp outline of High contrast. The card colour is the lowest surface container.
     */
    card: SwipeCardLook? = null,
    forceLifted: Boolean = false,
    failedEdgeHoldMs: Long? = CroftTimings.FAILED_EDGE_MS,
    interactionSource: MutableInteractionSource = remember { MutableInteractionSource() },
    /** How the time is read aloud ("2 hours ago"); the shown [time] when null. */
    timeDescription: String? = null,
    /** A Sent or Drafts row: [sender] holds the recipients, so it is read as "To ...". */
    outgoing: Boolean = false,
    /** Swiping is on (off in selection mode). The row only reports it; the swipe engine is T15. */
    swipeEnabled: Boolean = true,
    /** More actions for screen readers, after Select and Star (Archive, Delete, Mark read...). */
    extraActions: List<RowAction> = emptyList(),
    /** Lets the screen move focus to this row (after a removal). */
    focusRequester: androidx.compose.ui.focus.FocusRequester? = null,
) {
    val settings = LocalThemeSettings.current
    val colors = MaterialTheme.colorScheme
    val level = LocalMotionLevel.current
    val density = settings.density
    // At a large text size rows grow and wrap instead of cutting words (docs/ux/accessibility.md).
    val bigText = androidx.compose.ui.platform.LocalDensity.current.fontScale > BIG_TEXT_FROM_FONT_SCALE
    val lineCap = if (bigText) Int.MAX_VALUE else 1
    val shape = RoundedCornerShape(settings.roundness.swipeCardRadiusDp.dp)
    val pressed by interactionSource.collectIsPressedAsState()
    val boldUnread = state.unread && settings.unreadEmphasis != UnreadEmphasis.DOT
    val dotUnread = state.unread && settings.unreadEmphasis != UnreadEmphasis.BOLD
    val weight = if (boldUnread) FontWeight.Bold else FontWeight.Normal

    val newTint = remember { Animatable(0f) }
    LaunchedEffect(state.isNew, level) {
        if (state.isNew && level != MotionLevel.OFF) {
            newTint.snapTo(1f)
            newTint.animateTo(0f, linearFadeSpec(CroftTimings.NEW_ROW_TINT_MS))
        } else newTint.snapTo(0f)
    }

    val unreadWord = stringResource(if (state.unread) R.string.row_unread else R.string.row_read)
    val description = buildString {
        append(unreadWord).append(". ")
        if (state.selected) append(stringResource(R.string.control_selected)).append(". ")
        append(stringResource(if (outgoing) R.string.mail_to else R.string.row_from, sender)).append(". ").append(subject).append(". ").append(timeDescription ?: time).append(". ")
        if (state.threadCount > 1) append(pluralStringResource(R.plurals.mail_thread_count, state.threadCount, state.threadCount)).append(". ")
        if (state.hasAttachment) append(stringResource(R.string.row_has_attachment)).append(". ")
        if (state.starred) append(stringResource(R.string.row_starred)).append(". ")
        when (state.encryption) {
            EncryptionMark.ENCRYPTED -> append(stringResource(R.string.row_encrypted)).append(". ")
            EncryptionMark.SIGNED_INVALID -> append(stringResource(R.string.row_signature_invalid)).append(". ")
            EncryptionMark.NONE -> Unit
        }
        if (state.pendingOffline) append(stringResource(R.string.row_pending)).append(". ")
        if (state.failed) append(stringResource(R.string.row_failed)).append(". ")
        append(preview)
    }
    val selectLabel = stringResource(if (state.selected) R.string.row_action_deselect else R.string.row_action_select)
    val starLabel = stringResource(if (state.starred) R.string.row_action_unstar else R.string.row_action_star)
    val failedState = stringResource(R.string.row_failed_state)

    Box(modifier.fillMaxWidth().height(IntrinsicSize.Min).heightIn(min = density.rowDp.dp).motionProbe("new-tint") { newTint.value }) {
        val container = when {
            card != null -> if (state.selected) colors.primaryContainer else colors.surfaceContainerLowest
            state.selected -> colors.primaryContainer
            settings.listStyle == ListStyle.FLAT -> colors.surface
            else -> colors.surfaceContainerLow
        }
        Surface(
            color = container,
            contentColor = if (state.selected) colors.onPrimaryContainer else colors.onSurface,
            shape = card?.shape ?: if (settings.listStyle == ListStyle.FLAT) androidx.compose.ui.graphics.RectangleShape else shape,
            shadowElevation = if (pressed || card != null) 1.dp else 0.dp,
            border = if (card?.outlined == true) androidx.compose.foundation.BorderStroke(1.dp, colors.outline) else null,
            modifier = Modifier
                .fillMaxWidth()
                .croftFocusRing(interactionSource, shape)
                .clip(card?.shape ?: shape)
                .then(if (focusRequester != null) Modifier.focusRequester(focusRequester) else Modifier)
                .croftClickable(
                    interactionSource = interactionSource,
                    role = Role.Button,
                    onLongClick = onLongClick,
                    forceLifted = forceLifted,
                    onClick = onClick,
                )
                .semantics(mergeDescendants = true) {
                    contentDescription = description
                    this.swipeEnabled = swipeEnabled
                    selected = state.selected
                    if (state.failed) stateDescription = failedState
                    customActions = buildList {
                        if (onAvatarClick != null || onLongClick != null) add(CustomAccessibilityAction(selectLabel) { (onAvatarClick ?: onLongClick)?.invoke(); true })
                        if (onToggleStar != null) add(CustomAccessibilityAction(starLabel) { onToggleStar(); true })
                        for (a in extraActions) add(CustomAccessibilityAction(a.label) { a.onClick(); true })
                    }
                },
        ) {
            Box {
                if (newTint.value > 0f) Box(Modifier.matchParentSizeTint(colors.primary.copy(alpha = NEW_TINT_ALPHA * newTint.value)))
                Row(
                    Modifier.padding(start = 12.dp, end = 16.dp, top = 8.dp, bottom = 8.dp),
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(12.dp),
                ) {
                    // Leading edge: the unread dot takes its own column so the avatars line up in every row.
                    Box(Modifier.size(UNREAD_DOT_DP.dp), contentAlignment = Alignment.Center) {
                        if (dotUnread) Box(Modifier.size(UNREAD_DOT_DP.dp).clip(CircleShape).background(colors.primary))
                    }
                    if (settings.avatars != AvatarStyle.HIDDEN) {
                        Avatar(sender, state.selected, density.avatarDp.dp, onAvatarClick)
                    }
                    Column(Modifier.weight(1f)) {
                        Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                            Text(sender, Modifier.weight(1f), style = MaterialTheme.typography.titleMedium, fontWeight = weight, maxLines = lineCap, overflow = TextOverflow.Ellipsis)
                            if (state.threadCount > 1) {
                                Surface(shape = MaterialTheme.shapes.small, color = colors.surfaceContainerHighest, contentColor = colors.onSurfaceVariant) {
                                    Text(state.threadCount.toString(), Modifier.padding(horizontal = 6.dp, vertical = 1.dp), style = MaterialTheme.typography.labelSmall, maxLines = 1)
                                }
                            }
                            if (state.pendingOffline) Icon(CroftIcons.Schedule, null, Modifier.size(14.dp), tint = colors.onSurfaceVariant)
                            Text(
                                time,
                                style = MaterialTheme.typography.labelMedium,
                                color = if (state.unread) colors.primary else colors.onSurfaceVariant,
                                fontWeight = if (state.unread) FontWeight.Bold else FontWeight.Normal,
                                maxLines = 1,
                            )
                        }
                        Text(subject, style = MaterialTheme.typography.bodyMedium, fontWeight = weight, maxLines = lineCap, overflow = TextOverflow.Ellipsis)
                        if (density.previewLines > 0) {
                            Text(preview, Modifier.testTag(ROW_PREVIEW_TAG), style = MaterialTheme.typography.bodyMedium, color = colors.onSurfaceVariant, maxLines = density.previewLines, overflow = TextOverflow.Ellipsis)
                        }
                    }
                    if (state.starred || state.hasAttachment || state.encryption != EncryptionMark.NONE) {
                        Column(horizontalAlignment = Alignment.CenterHorizontally, verticalArrangement = Arrangement.spacedBy(4.dp)) {
                            if (state.hasAttachment) Icon(CroftIcons.Attachment, null, Modifier.size(18.dp), tint = colors.onSurfaceVariant)
                            if (state.starred) Icon(Icons.Filled.Star, null, Modifier.size(20.dp), tint = rowStarColour(state.selected))
                            when (state.encryption) {
                                EncryptionMark.ENCRYPTED -> Icon(Icons.Filled.Lock, null, Modifier.size(16.dp), tint = colors.onSurfaceVariant)
                                EncryptionMark.SIGNED_INVALID -> Icon(Icons.Filled.Warning, null, Modifier.size(16.dp), tint = colors.error)
                                EncryptionMark.NONE -> Unit
                            }
                        }
                    }
                }
                if (state.failed) FailedEdge(failedEdgeHoldMs, Modifier.align(Alignment.CenterStart))
            }
        }
    }
}

private const val NEW_TINT_ALPHA = 0.16f

/** From this combined font scale a row's sender and subject wrap instead of ending in an ellipsis. */
private const val BIG_TEXT_FROM_FONT_SCALE = 1.3f

/** The preview is cut to the Show previews setting on purpose; tests that look for clipped text skip it. */
const val ROW_PREVIEW_TAG = "row-preview"

private fun Modifier.matchParentSizeTint(colour: androidx.compose.ui.graphics.Color): Modifier =
    this.fillMaxWidth().fillMaxHeight().background(colour)

/** The red edge of a refused change: solid, then (after [holdMs]) fading at Full and Reduced Motion's plain removal. */
@Composable
private fun FailedEdge(holdMs: Long?, modifier: Modifier) {
    val level = LocalMotionLevel.current
    var gone by remember { mutableStateOf(false) }
    LaunchedEffect(holdMs) {
        gone = false
        if (holdMs != null) { kotlinx.coroutines.delay(holdMs); gone = true }
    }
    val alpha by animateFloatAsState(
        if (gone) 0f else 1f,
        // Full: effects default fade. Reduced: the edge goes without fading (docs/ux/motion.md). Off: at once.
        if (level == MotionLevel.FULL) MaterialTheme.motionScheme.defaultEffectsSpec() else androidx.compose.animation.core.snap(),
        label = "failed-edge",
    )
    Box(modifier.width(RedEdgeWidth).fillMaxHeight().motionProbe("failed-edge") { alpha }.alpha(alpha).background(MaterialTheme.colorScheme.error))
}

/** The avatar: the first letter in a circle, or a check when selected, with the flip (Full) or a cross-fade (Reduced, Off). */
@Composable
private fun Avatar(sender: String, selected: Boolean, size: Dp, onClick: (() -> Unit)?) {
    val level = LocalMotionLevel.current
    val colors = MaterialTheme.colorScheme
    val spec = if (level == MotionLevel.REDUCED) MaterialTheme.motionScheme.defaultEffectsSpec<Float>() else MaterialTheme.motionScheme.fastSpatialSpec<Float>()
    val flip by animateFloatAsState(if (selected) 1f else 0f, spec, label = "avatar-flip")
    val flips = level == MotionLevel.FULL
    val showCheck = if (flips) flip > 0.5f else false
    val letter = sender.trim().firstOrNull()?.uppercaseChar()?.toString() ?: "?"
    Box(
        Modifier
            .size(size)
            .motionProbes("avatar-flip" to { flip }, "avatar-rotation" to { if (flips) flip * 180f else 0f })
            .graphicsLayer { if (flips) rotationY = flip * 180f }
            .clip(CircleShape)
            .background(
                when {
                    flips -> if (showCheck) colors.primary else colors.secondaryContainer
                    else -> androidx.compose.ui.graphics.lerp(colors.secondaryContainer, colors.primary, flip.coerceIn(0f, 1f))
                },
            )
            // Screen readers reach Select through the row's custom actions, so the avatar is not a second element for them.
            .clearAndSetSemantics { }
            .then(if (onClick != null) Modifier.clickable(onClick = onClick) else Modifier),
        contentAlignment = Alignment.Center,
    ) {
        if (flips) {
            Box(Modifier.graphicsLayer { if (showCheck) rotationY = 180f }, contentAlignment = Alignment.Center) {
                if (showCheck) Icon(Icons.Filled.Check, null, tint = colors.onPrimary) else Initial(letter)
            }
        } else {
            Box(Modifier.alpha(1f - flip)) { Initial(letter) }
            Icon(Icons.Filled.Check, null, Modifier.alpha(flip), tint = colors.onPrimary)
        }
    }
}

/** The letter in the avatar. The circle has a fixed size, so the letter does not grow with the text size setting. */
@Composable
private fun Initial(letter: String) {
    val outer = androidx.compose.ui.platform.LocalDensity.current
    androidx.compose.runtime.CompositionLocalProvider(
        androidx.compose.ui.platform.LocalDensity provides androidx.compose.ui.unit.Density(outer.density, fontScale = minOf(outer.fontScale, 1f)),
    ) {
        Text(letter, style = MaterialTheme.typography.titleMedium, color = MaterialTheme.colorScheme.onSecondaryContainer)
    }
}
