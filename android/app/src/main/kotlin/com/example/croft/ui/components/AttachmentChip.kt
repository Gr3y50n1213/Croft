// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.ui.components

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Refresh
import androidx.compose.material.icons.filled.Warning
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import com.example.croft.R
import com.example.croft.theme.statusColors

/** The states of an attachment chip (docs/ux/controls.md). */
enum class AttachmentState {
    /** Icon, name, size. Tap downloads. */
    NOT_DOWNLOADED,

    /** A progress ring. Tap cancels. */
    DOWNLOADING,

    /** Tap opens. */
    DOWNLOADED,

    /** Red. Tap retries. */
    FAILED,

    /** An executable or script: flagged, opens only after a warning. */
    BLOCKED,

    /** In compose, over the server's size limit: a warning with the limit. */
    OVER_LIMIT,
}

/**
 * An attachment chip. Every state has an icon and words as well as colour. A tap does what the state says (download, cancel,
 * open, retry, warn); a long-press opens the menu (Save to device, Share, Open with), with SelectionStart haptic.
 * [progress] is 0 to 1 while downloading.
 */
@Composable
fun AttachmentChip(
    name: String,
    size: String,
    state: AttachmentState,
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
    progress: Float = 0f,
    limitText: String? = null,
    onLongClick: (() -> Unit)? = null,
    interactionSource: MutableInteractionSource = remember { MutableInteractionSource() },
) {
    val colors = MaterialTheme.colorScheme
    val amber = statusColors()
    val shape = MaterialTheme.shapes.small
    // At a large text size the name and the size line wrap instead of ending in an ellipsis.
    val lineCap = if (LocalDensity.current.fontScale > BIG_TEXT_FROM_FONT_SCALE) Int.MAX_VALUE else 1
    val tapLabel = stringResource(
        when (state) {
            AttachmentState.NOT_DOWNLOADED -> R.string.attachment_tap_download
            AttachmentState.DOWNLOADING -> R.string.attachment_tap_cancel
            AttachmentState.DOWNLOADED -> R.string.attachment_tap_open
            AttachmentState.FAILED -> R.string.attachment_tap_retry
            AttachmentState.BLOCKED -> R.string.attachment_tap_warning
            AttachmentState.OVER_LIMIT -> R.string.attachment_tap_remove
        },
    )
    val stateWord = stringResource(
        when (state) {
            AttachmentState.NOT_DOWNLOADED -> R.string.attachment_state_not_downloaded
            AttachmentState.DOWNLOADING -> R.string.attachment_state_downloading
            AttachmentState.DOWNLOADED -> R.string.attachment_state_downloaded
            AttachmentState.FAILED -> R.string.attachment_state_failed
            AttachmentState.BLOCKED -> R.string.attachment_state_blocked
            AttachmentState.OVER_LIMIT -> R.string.attachment_state_over_limit
        },
    )
    val (container, content) = when (state) {
        AttachmentState.FAILED -> colors.errorContainer to colors.onErrorContainer
        AttachmentState.BLOCKED, AttachmentState.OVER_LIMIT -> amber.amberContainer to amber.onAmberContainer
        else -> colors.surfaceContainerHigh to colors.onSurface
    }
    val outline = if (state == AttachmentState.FAILED) colors.error else colors.outlineVariant
    val description = buildString {
        append(name).append(", ").append(size).append(", ").append(stateWord)
        if (limitText != null) append(", ").append(limitText)
    }
    Row(
        modifier
            .croftFocusRing(interactionSource, shape)
            .clip(shape)
            .background(container)
            .border(1.dp, outline, shape)
            .croftClickable(
                interactionSource = interactionSource,
                role = Role.Button,
                onClickLabel = tapLabel,
                onLongClick = onLongClick,
                onClick = onClick,
            )
            .semantics(mergeDescendants = true) {
                contentDescription = description
                stateDescription = stateWord
            }
            .heightIn(min = 40.dp)
            .padding(horizontal = 12.dp, vertical = 6.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        when (state) {
            AttachmentState.DOWNLOADING -> CircularProgressIndicator(progress = { progress }, modifier = Modifier.size(20.dp), color = content, strokeWidth = 2.dp)
            AttachmentState.FAILED -> Icon(Icons.Filled.Refresh, null, Modifier.size(20.dp), tint = content)
            AttachmentState.BLOCKED, AttachmentState.OVER_LIMIT -> Icon(Icons.Filled.Warning, null, Modifier.size(20.dp), tint = content)
            else -> Icon(CroftIcons.Attachment, null, Modifier.size(20.dp), tint = content)
        }
        Column {
            Text(name, style = MaterialTheme.typography.labelLarge, color = content, maxLines = lineCap, overflow = TextOverflow.Ellipsis)
            Text(
                when (state) {
                    AttachmentState.FAILED -> stringResource(R.string.attachment_failed_line, size)
                    AttachmentState.BLOCKED -> stringResource(R.string.attachment_blocked_line, size)
                    AttachmentState.OVER_LIMIT -> limitText ?: size
                    AttachmentState.DOWNLOADING -> stringResource(R.string.attachment_downloading_line, (progress * 100).toInt())
                    else -> size
                },
                style = MaterialTheme.typography.bodySmall,
                color = content,
                maxLines = lineCap,
                overflow = TextOverflow.Ellipsis,
            )
        }
    }
}

private const val BIG_TEXT_FROM_FONT_SCALE = 1.3f
