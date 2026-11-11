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
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Check
import androidx.compose.material.icons.filled.Delete
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
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.CustomAccessibilityAction
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.customActions
import androidx.compose.ui.semantics.selected
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.IntOffset
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
import com.example.croft.theme.statusColors

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
)

private val RedEdgeWidth = 4.dp
private val PillGapToCard = 4.dp
private val PillInsetFromEdge = 8.dp
private const val UNREAD_DOT_DP = 8

/**
 * A message row. Read: regular weight. Unread: bold sender and subject, a dot at the leading edge and the time in the strong
 * colour (as far as S52 "Unread emphasis" allows). Selected: primary container tint, and the avatar turns into a check
 * (a flip with the spatial fast spring; a cross-fade at Reduced; at once at Off). Dragging: the row is a rounded card that
 * follows the finger by [dragOffset] and a separate pill fills the space it leaves (the swipe engine, T15, drives it). New: a
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
    dragOffset: Dp = 0.dp,
    forceLifted: Boolean = false,
    failedEdgeHoldMs: Long? = CroftTimings.FAILED_EDGE_MS,
    interactionSource: MutableInteractionSource = remember { MutableInteractionSource() },
) {
    val settings = LocalThemeSettings.current
    val colors = MaterialTheme.colorScheme
    val level = LocalMotionLevel.current
    val density = settings.density
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
        append(stringResource(R.string.row_from, sender)).append(". ").append(subject).append(". ").append(time).append(". ")
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

    Box(modifier.fillMaxWidth().heightIn(min = density.rowDp.dp)) {
        if (dragOffset > 0.dp) {
            // The pill that fills the space the card leaves: row height, 4 dp from the card, 8 dp from the row edge.
            Box(
                Modifier
                    .align(Alignment.CenterEnd)
                    .padding(end = PillInsetFromEdge)
                    .width((dragOffset - PillGapToCard - PillInsetFromEdge).coerceAtLeast(0.dp))
                    .fillMaxHeight()
                    .clip(CircleShape)
                    .background(colors.error),
                contentAlignment = Alignment.Center,
            ) { Icon(Icons.Filled.Delete, contentDescription = null, tint = colors.onError, modifier = Modifier.size(24.dp)) }
        }
        val container = when {
            state.selected -> colors.primaryContainer
            settings.listStyle == ListStyle.FLAT -> colors.surface
            else -> colors.surfaceContainerLow
        }
        Surface(
            color = container,
            contentColor = if (state.selected) colors.onPrimaryContainer else colors.onSurface,
            shape = if (settings.listStyle == ListStyle.FLAT && dragOffset == 0.dp) androidx.compose.ui.graphics.RectangleShape else shape,
            shadowElevation = if (pressed || dragOffset > 0.dp) 1.dp else 0.dp,
            modifier = Modifier
                .fillMaxWidth()
                .offset { IntOffset(-dragOffset.roundToPx(), 0) }
                .croftFocusRing(interactionSource, shape)
                .clip(shape)
                .croftClickable(
                    interactionSource = interactionSource,
                    role = Role.Button,
                    onLongClick = onLongClick,
                    forceLifted = forceLifted,
                    onClick = onClick,
                )
                .semantics(mergeDescendants = true) {
                    contentDescription = description
                    selected = state.selected
                    if (state.failed) stateDescription = failedState
                    customActions = buildList {
                        if (onAvatarClick != null || onLongClick != null) add(CustomAccessibilityAction(selectLabel) { (onAvatarClick ?: onLongClick)?.invoke(); true })
                        if (onToggleStar != null) add(CustomAccessibilityAction(starLabel) { onToggleStar(); true })
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
                            Text(sender, Modifier.weight(1f), style = MaterialTheme.typography.titleMedium, fontWeight = weight, maxLines = 1, overflow = TextOverflow.Ellipsis)
                            if (state.pendingOffline) Icon(CroftIcons.Schedule, null, Modifier.size(14.dp), tint = colors.onSurfaceVariant)
                            Text(
                                time,
                                style = MaterialTheme.typography.labelMedium,
                                color = if (state.unread) colors.primary else colors.onSurfaceVariant,
                                fontWeight = if (state.unread) FontWeight.Bold else FontWeight.Normal,
                                maxLines = 1,
                            )
                        }
                        Text(subject, style = MaterialTheme.typography.bodyMedium, fontWeight = weight, maxLines = 1, overflow = TextOverflow.Ellipsis)
                        if (density.previewLines > 0) {
                            Text(preview, style = MaterialTheme.typography.bodyMedium, color = colors.onSurfaceVariant, maxLines = density.previewLines, overflow = TextOverflow.Ellipsis)
                        }
                    }
                    if (state.starred || state.encryption != EncryptionMark.NONE) {
                        Column(horizontalAlignment = Alignment.CenterHorizontally, verticalArrangement = Arrangement.spacedBy(4.dp)) {
                            if (state.starred) Icon(Icons.Filled.Star, null, Modifier.size(20.dp), tint = statusColors().amber)
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
    Box(modifier.width(RedEdgeWidth).fillMaxHeight().alpha(alpha).background(MaterialTheme.colorScheme.error))
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
            .motionProbe("avatar-flip") { flip }
            .graphicsLayer { if (flips) rotationY = flip * 180f }
            .clip(CircleShape)
            .background(
                when {
                    flips -> if (showCheck) colors.primary else colors.secondaryContainer
                    else -> androidx.compose.ui.graphics.lerp(colors.secondaryContainer, colors.primary, flip.coerceIn(0f, 1f))
                },
            )
            .then(if (onClick != null) Modifier.clickable(onClick = onClick) else Modifier),
        contentAlignment = Alignment.Center,
    ) {
        if (flips) {
            Box(Modifier.graphicsLayer { if (showCheck) rotationY = 180f }, contentAlignment = Alignment.Center) {
                if (showCheck) Icon(Icons.Filled.Check, null, tint = colors.onPrimary) else Text(letter, style = MaterialTheme.typography.titleMedium, color = colors.onSecondaryContainer)
            }
        } else {
            Text(letter, Modifier.alpha(1f - flip), style = MaterialTheme.typography.titleMedium, color = colors.onSecondaryContainer)
            Icon(Icons.Filled.Check, null, Modifier.alpha(flip), tint = colors.onPrimary)
        }
    }
}
