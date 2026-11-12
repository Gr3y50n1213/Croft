// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.ui.components

import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.EnterTransition
import androidx.compose.animation.ExitTransition
import androidx.compose.animation.expandHorizontally
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.shrinkHorizontally
import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Check
import androidx.compose.material.icons.filled.Info
import androidx.compose.material.icons.filled.Lock
import androidx.compose.material.icons.filled.Warning
import androidx.compose.material3.AssistChip
import androidx.compose.material3.AssistChipDefaults
import androidx.compose.material3.Badge
import androidx.compose.material3.FilterChip
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.SuggestionChip
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import com.example.croft.R
import com.example.croft.haptics.HapticEvent
import com.example.croft.theme.LocalMotionLevel
import com.example.croft.theme.MotionLevel
import com.example.croft.theme.statusColors

/** Enter and exit for something that slides in beside text: spatial fast at Full, instant otherwise (docs/ux/motion.md). */
@Composable
internal fun slideInBesideText(): Pair<EnterTransition, ExitTransition> {
    val level = LocalMotionLevel.current
    return if (level == MotionLevel.FULL) {
        (expandHorizontally(MaterialTheme.motionScheme.fastSpatialSpec()) + fadeIn(MaterialTheme.motionScheme.fastEffectsSpec())) to
            (shrinkHorizontally(MaterialTheme.motionScheme.fastSpatialSpec()) + fadeOut(MaterialTheme.motionScheme.fastEffectsSpec()))
    } else EnterTransition.None to ExitTransition.None
}

/**
 * Filter chip for categories and search filters: unselected, selected (tonal fill and a check that slides in), pressed,
 * focused, disabled; with an optional count badge. The count is read with the label.
 */
@Composable
fun CroftFilterChip(
    label: String,
    selected: Boolean,
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
    enabled: Boolean = true,
    count: Int? = null,
    interactionSource: MutableInteractionSource = remember { MutableInteractionSource() },
) {
    val (enter, exit) = slideInBesideText()
    FilterChip(
        selected = selected,
        onClick = onClick,
        enabled = enabled,
        interactionSource = interactionSource,
        modifier = modifier.croftFocusRing(interactionSource, MaterialTheme.shapes.small).minTouchTarget(),
        label = {
            Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                Text(label, maxLines = 1, overflow = TextOverflow.Ellipsis)
                if (count != null) Badge { Text(count.toString()) }
            }
        },
        leadingIcon = {
            AnimatedVisibility(visible = selected, enter = enter, exit = exit) {
                Icon(Icons.Filled.Check, contentDescription = null, modifier = Modifier.size(18.dp))
            }
        },
    )
}

/** Suggestion chip: normal and pressed. */
@Composable
fun CroftSuggestionChip(
    label: String,
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
    enabled: Boolean = true,
    interactionSource: MutableInteractionSource = remember { MutableInteractionSource() },
) {
    SuggestionChip(
        onClick = onClick,
        enabled = enabled,
        interactionSource = interactionSource,
        modifier = modifier.croftFocusRing(interactionSource, MaterialTheme.shapes.small).minTouchTarget(),
        label = { Text(label, maxLines = 1, overflow = TextOverflow.Ellipsis) },
    )
}

/** The states of an input chip (a recipient). */
enum class RecipientChipState { NORMAL, SELECTED, ERROR }

/**
 * Input chip for a recipient: normal, selected (for delete), error (invalid address: red outline, a warning icon and the
 * words, never colour alone), with a key lock when a key is found. A tap opens the chip menu (the caller shows it); a
 * long-press lifts the chip to drag between To, Cc and Bcc (haptic DragStart).
 */
@Composable
fun RecipientChip(
    name: String,
    state: RecipientChipState,
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
    hasKey: Boolean = false,
    onLongPress: (() -> Unit)? = null,
    forceLifted: Boolean = false,
    interactionSource: MutableInteractionSource = remember { MutableInteractionSource() },
) {
    val colors = MaterialTheme.colorScheme
    val shape = MaterialTheme.shapes.small
    val errorWord = stringResource(R.string.recipient_invalid)
    val selectedWord = stringResource(R.string.control_selected)
    val keyWord = stringResource(R.string.recipient_has_key)
    val invalidState = stringResource(R.string.recipient_invalid_state)
    val container = if (state == RecipientChipState.SELECTED) colors.secondaryContainer else androidx.compose.ui.graphics.Color.Transparent
    val content = when (state) {
        RecipientChipState.ERROR -> colors.error
        RecipientChipState.SELECTED -> colors.onSecondaryContainer
        RecipientChipState.NORMAL -> colors.onSurfaceVariant
    }
    val border = BorderStroke(1.dp, if (state == RecipientChipState.ERROR) colors.error else colors.outline)
    Row(
        modifier
            .croftFocusRing(interactionSource, shape)
            .clip(shape)
            .background(container)
            .border(border, shape)
            .croftClickable(
                interactionSource = interactionSource,
                role = Role.Button,
                onLongClick = onLongPress,
                longClickEvent = HapticEvent.DragStart,
                forceLifted = forceLifted,
                onClick = onClick,
            )
            .semantics(mergeDescendants = true) {
                contentDescription = buildString {
                    append(name)
                    if (hasKey) append(", ").append(keyWord)
                }
                if (state == RecipientChipState.ERROR) stateDescription = invalidState
                if (state == RecipientChipState.SELECTED) stateDescription = selectedWord
            }
            .heightIn(min = 32.dp)
            .padding(horizontal = 12.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(6.dp),
    ) {
        if (state == RecipientChipState.ERROR) Icon(Icons.Filled.Warning, null, Modifier.size(16.dp), tint = content)
        // Middle ellipsis: a long chip keeps its end, where an address ends.
        Text(name, style = MaterialTheme.typography.labelLarge, color = content, maxLines = 1, overflow = TextOverflow.MiddleEllipsis)
        if (state == RecipientChipState.ERROR) Text(errorWord, style = MaterialTheme.typography.labelSmall, color = content)
        if (hasKey) Icon(Icons.Filled.Lock, null, Modifier.size(14.dp), tint = content)
    }
}

/** The status of an account (docs/ux/controls.md, "Account status chip"). */
enum class AccountStatus { CONNECTED, SYNCING, OFFLINE, NEEDS_SIGN_IN, SERVER_PROBLEM }

/**
 * Account status chip: Connected shows nothing; Syncing, Offline (grey), Needs sign-in (red) and Server problem (amber) each
 * have words and an icon, so colour is never the only signal. Read as "Account Sam, Needs sign-in".
 */
@Composable
fun AccountStatusChip(
    accountName: String,
    status: AccountStatus,
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
    interactionSource: MutableInteractionSource = remember { MutableInteractionSource() },
) {
    if (status == AccountStatus.CONNECTED) return
    val colors = MaterialTheme.colorScheme
    val amber = statusColors()
    val text = stringResource(
        when (status) {
            AccountStatus.SYNCING -> R.string.status_syncing
            AccountStatus.OFFLINE -> R.string.status_offline
            AccountStatus.NEEDS_SIGN_IN -> R.string.status_needs_sign_in
            else -> R.string.status_server_problem
        },
    )
    val description = stringResource(R.string.status_chip_description, accountName, text)
    val (container, content) = when (status) {
        AccountStatus.OFFLINE -> colors.surfaceContainerHighest to colors.onSurfaceVariant
        AccountStatus.NEEDS_SIGN_IN -> colors.errorContainer to colors.onErrorContainer
        AccountStatus.SERVER_PROBLEM -> amber.amberContainer to amber.onAmberContainer
        else -> colors.secondaryContainer to colors.onSecondaryContainer
    }
    AssistChip(
        onClick = onClick,
        interactionSource = interactionSource,
        modifier = modifier.croftFocusRing(interactionSource, MaterialTheme.shapes.small).minTouchTarget().semantics { contentDescription = description },
        label = { Text(text, maxLines = 1) },
        leadingIcon = {
            when (status) {
                AccountStatus.SYNCING -> CroftSpinner(Modifier.size(16.dp), color = content)
                AccountStatus.OFFLINE -> Icon(Icons.Filled.Info, null, Modifier.size(16.dp))
                else -> Icon(Icons.Filled.Warning, null, Modifier.size(16.dp))
            }
        },
        colors = AssistChipDefaults.assistChipColors(containerColor = container, labelColor = content, leadingIconContentColor = content),
        border = null,
    )
}
