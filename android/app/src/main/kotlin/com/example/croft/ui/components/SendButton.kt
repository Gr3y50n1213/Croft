// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.ui.components

import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.Send
import androidx.compose.material.icons.filled.Warning
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.ui.unit.dp
import com.example.croft.R

/** The situations of the Send button (docs/ux/controls.md, "Send button"). */
enum class SendCondition {
    /** No recipient: disabled; tap explains Add a recipient. */
    NO_RECIPIENT,

    /** A recipient chip has an invalid address: disabled; tap explains Fix the address. */
    INVALID_RECIPIENT,

    /** Recipient but empty subject and body: enabled; tap asks Send without subject or text? */
    EMPTY_CONFIRM,

    /** Everything present: enabled. */
    READY,

    /** Attachments over the server's size limit: disabled; the attachment chip shows the warning. */
    OVER_SIZE_LIMIT,

    /** An attachment is still uploading: enabled, the message waits in the Outbox, with a progress ring. */
    UPLOADING,

    /** Offline, or the account needs sign-in: enabled; the Outbox shows Queued. */
    OFFLINE_OR_SIGN_IN,

    /** Tapped: the send animation plays, then the Undo snackbar. The button is busy and not tappable meanwhile. */
    SENDING,

    /** A scheduled send: clock icon, label Schedule. */
    SCHEDULED,

    /** Encryption required but a recipient has no key: enabled with a warning icon; tap opens S24. */
    ENCRYPTION_NO_KEY,
    ;

    /** Whether the button takes a tap. */
    val enabled: Boolean get() = when (this) {
        NO_RECIPIENT, INVALID_RECIPIENT, OVER_SIZE_LIMIT, SENDING -> false
        else -> true
    }

    /** What a tap does. */
    val tap: SendTap get() = when (this) {
        NO_RECIPIENT, INVALID_RECIPIENT, OVER_SIZE_LIMIT -> SendTap.EXPLAIN
        SENDING -> SendTap.NOTHING
        EMPTY_CONFIRM -> SendTap.ASK_EMPTY
        ENCRYPTION_NO_KEY -> SendTap.OPEN_ENCRYPTION_CHECK
        else -> SendTap.SEND
    }

    /** Whether a long-press opens Schedule send (S25). Not when the button is disabled. */
    val longPressSchedules: Boolean get() = enabled
}

/** What a tap on Send leads to. */
enum class SendTap { SEND, ASK_EMPTY, OPEN_ENCRYPTION_CHECK, EXPLAIN, NOTHING }

/** What the Send button tells its screen. */
class SendActions(
    /** Send now (or queue it). */
    val onSend: () -> Unit,
    /** The message is empty: ask Send without subject or text? */
    val onAskEmpty: () -> Unit,
    /** Open S24, the encryption check. */
    val onEncryptionCheck: () -> Unit,
    /** Long-press: open Schedule send (S25). */
    val onSchedule: () -> Unit,
    /** A tap on a disabled Send: show the reason once as a snackbar. */
    val onExplain: (String) -> Unit,
)

/**
 * The Send button with every condition of the table. A disabled Send stays in the semantics tree as disabled with its
 * reason as the state, and a tap or long-press shows the reason. Long-press on an enabled Send opens Schedule send.
 */
@Composable
fun SendButton(
    condition: SendCondition,
    actions: SendActions,
    modifier: Modifier = Modifier,
    sizeLimitMb: Int = DEFAULT_SIZE_LIMIT_MB,
    interactionSource: MutableInteractionSource = remember { MutableInteractionSource() },
) {
    val enabled = condition.enabled
    val reason = when (condition) {
        SendCondition.NO_RECIPIENT -> stringResource(R.string.send_reason_no_recipient)
        SendCondition.INVALID_RECIPIENT -> stringResource(R.string.send_reason_invalid_recipient)
        SendCondition.OVER_SIZE_LIMIT -> stringResource(R.string.send_reason_over_limit, sizeLimitMb)
        else -> null
    }
    val label = stringResource(if (condition == SendCondition.SCHEDULED) R.string.send_schedule else R.string.send_label)
    val busy = stringResource(R.string.control_busy)
    val sending = stringResource(R.string.send_sending)
    val uploadingState = stringResource(R.string.send_state_uploading)
    val queuedState = stringResource(R.string.send_state_queued)
    val encryptionState = stringResource(R.string.send_state_no_key)
    val colors = MaterialTheme.colorScheme
    val container = if (enabled) colors.primary else colors.onSurface.copy(alpha = DISABLED_CONTAINER)
    val content = if (enabled) colors.onPrimary else colors.onSurface.copy(alpha = DISABLED_CONTENT)
    val shape = CircleShape
    val tap = condition.tap
    Surface(
        color = container,
        contentColor = content,
        shape = shape,
        modifier = modifier
            .croftFocusRing(interactionSource, shape)
            .explainsWhenDisabled(enabled || condition == SendCondition.SENDING, reason, actions.onExplain)
            .semantics {
                when (condition) {
                    SendCondition.SENDING -> stateDescription = "$sending, $busy"
                    SendCondition.UPLOADING -> stateDescription = uploadingState
                    SendCondition.OFFLINE_OR_SIGN_IN -> stateDescription = queuedState
                    SendCondition.ENCRYPTION_NO_KEY -> stateDescription = encryptionState
                    else -> Unit
                }
            }
            .clip(shape)
            .croftClickable(
                interactionSource = interactionSource,
                enabled = enabled,
                role = Role.Button,
                onLongClickLabel = stringResource(R.string.send_schedule_action),
                onLongClick = if (condition.longPressSchedules) actions.onSchedule else null,
            ) {
                when (tap) {
                    SendTap.SEND -> actions.onSend()
                    SendTap.ASK_EMPTY -> actions.onAskEmpty()
                    SendTap.OPEN_ENCRYPTION_CHECK -> actions.onEncryptionCheck()
                    SendTap.EXPLAIN, SendTap.NOTHING -> Unit
                }
            },
    ) {
        Row(
            Modifier.heightIn(min = 40.dp).padding(horizontal = 24.dp, vertical = 8.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            Box(Modifier.size(18.dp), contentAlignment = Alignment.Center) {
                when (condition) {
                    SendCondition.SCHEDULED -> Icon(CroftIcons.Schedule, null, Modifier.size(18.dp))
                    SendCondition.ENCRYPTION_NO_KEY -> Icon(Icons.Filled.Warning, null, Modifier.size(18.dp))
                    SendCondition.UPLOADING, SendCondition.SENDING ->
                        CroftSpinner(Modifier.size(18.dp), color = content)
                    else -> Icon(Icons.AutoMirrored.Filled.Send, null, Modifier.size(18.dp))
                }
            }
            Text(
                if (condition == SendCondition.SENDING) sending else label,
                style = MaterialTheme.typography.labelLarge,
                maxLines = 1,
            )
        }
    }
}

/** The size limit when the server gives none (docs/ux/controls.md). */
const val DEFAULT_SIZE_LIMIT_MB = 25

// Material 3 disabled state: content at 38 percent, container at 12 percent (docs/ux/controls.md).
internal const val DISABLED_CONTENT = 0.38f
internal const val DISABLED_CONTAINER = 0.12f
