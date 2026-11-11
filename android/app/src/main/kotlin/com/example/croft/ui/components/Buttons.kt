// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.ui.components

import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.size
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Check
import androidx.compose.material.icons.filled.Warning
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.FilledTonalButton
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import com.example.croft.R
import com.example.croft.theme.CroftTimings
import kotlinx.coroutines.delay

/** The button styles of docs/ux/controls.md. Danger always needs a confirmation or Undo from its caller. */
enum class ButtonKind { FILLED, TONAL, OUTLINED, TEXT, DANGER, DANGER_TEXT }

/** Where a button is in its work: the label, a spinner (Busy), or the 600 ms check. */
enum class ButtonProgress { IDLE, LOADING, SUCCESS }

/**
 * Runs a button's work: Loading while it runs, then the check for 600 ms when it succeeded, then back to the label.
 * Hold one with `remember`; give [progress] to the button.
 */
class ButtonProgressController {
    /** What the button should show now. */
    var progress by mutableStateOf(ButtonProgress.IDLE)
        private set

    /** Run [work]; true means it succeeded and the check shows. */
    suspend fun run(work: suspend () -> Boolean) {
        progress = ButtonProgress.LOADING
        val ok = try { work() } catch (e: kotlinx.coroutines.CancellationException) { progress = ButtonProgress.IDLE; throw e }
        if (ok) {
            progress = ButtonProgress.SUCCESS
            delay(CroftTimings.BUTTON_SUCCESS_HOLD_MS)
        }
        progress = ButtonProgress.IDLE
    }
}

/**
 * A button of any [ButtonKind], with its special states. Loading: the label is replaced by a spinner but still takes its
 * space so the width does not change; it is not tappable, announced as Busy, and after 10 s shows Cancel (calls
 * [onCancel]). Success: a check. Error: [errorText] in the error colour with a warning icon (never colour alone). Disabled:
 * Material's 38 percent content and 12 percent container; it stays in the semantics tree as disabled, and when
 * [disabledReason] is given a tap or long-press calls [onExplain] with it.
 */
@Composable
fun CroftButton(
    label: String,
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
    kind: ButtonKind = ButtonKind.FILLED,
    enabled: Boolean = true,
    progress: ButtonProgress = ButtonProgress.IDLE,
    onCancel: (() -> Unit)? = null,
    leadingIcon: ImageVector? = null,
    errorText: String? = null,
    disabledReason: String? = null,
    onExplain: (String) -> Unit = {},
    interactionSource: MutableInteractionSource = remember { MutableInteractionSource() },
) {
    val busy = stringResource(R.string.control_busy)
    var offerCancel by remember { mutableStateOf(false) }
    LaunchedEffect(progress) {
        offerCancel = false
        if (progress == ButtonProgress.LOADING && onCancel != null) {
            delay(CroftTimings.LOADING_OFFERS_CANCEL_MS)
            offerCancel = true
        }
    }
    val hasError = errorText != null
    val shape = ButtonDefaults.shape
    val buttonModifier = modifier
        .minTouchTarget()
        .explainsWhenDisabled(enabled, disabledReason, onExplain)
        .croftFocusRing(interactionSource, shape)
        .then(if (progress == ButtonProgress.LOADING) Modifier.semantics { stateDescription = busy } else Modifier)
    val action = {
        when (progress) {
            ButtonProgress.IDLE -> onClick()
            ButtonProgress.LOADING -> if (offerCancel) onCancel?.invoke()
            ButtonProgress.SUCCESS -> Unit
        }
    }
    val error = MaterialTheme.colorScheme.error
    val content: @Composable () -> Unit = {
        ButtonContent(label, progress, offerCancel, leadingIcon, hasError)
    }
    Column(horizontalAlignment = Alignment.Start, verticalArrangement = Arrangement.spacedBy(4.dp)) {
        when (kind) {
            ButtonKind.FILLED -> Button(action, buttonModifier, enabled, interactionSource = interactionSource, content = { content() })
            ButtonKind.TONAL -> FilledTonalButton(action, buttonModifier, enabled, interactionSource = interactionSource, content = { content() })
            ButtonKind.OUTLINED -> OutlinedButton(
                action, buttonModifier, enabled, interactionSource = interactionSource,
                border = if (hasError) androidx.compose.foundation.BorderStroke(1.dp, error) else ButtonDefaults.outlinedButtonBorder(enabled),
                colors = if (hasError) ButtonDefaults.outlinedButtonColors(contentColor = error) else ButtonDefaults.outlinedButtonColors(),
                content = { content() },
            )
            ButtonKind.TEXT -> TextButton(
                action, buttonModifier, enabled, interactionSource = interactionSource,
                colors = if (hasError) ButtonDefaults.textButtonColors(contentColor = error) else ButtonDefaults.textButtonColors(),
                content = { content() },
            )
            ButtonKind.DANGER -> Button(
                action, buttonModifier, enabled, interactionSource = interactionSource,
                colors = ButtonDefaults.buttonColors(containerColor = error, contentColor = MaterialTheme.colorScheme.onError),
                content = { content() },
            )
            ButtonKind.DANGER_TEXT -> TextButton(
                action, buttonModifier, enabled, interactionSource = interactionSource,
                colors = ButtonDefaults.textButtonColors(contentColor = error),
                content = { content() },
            )
        }
        if (errorText != null) Text(errorText, style = MaterialTheme.typography.bodySmall, color = error)
    }
}

@Composable
private fun ButtonContent(label: String, progress: ButtonProgress, offerCancel: Boolean, icon: ImageVector?, hasError: Boolean) {
    Box(contentAlignment = Alignment.Center) {
        // The label always takes its space, so the width is the same in every state.
        Row(
            Modifier.alpha(if (progress == ButtonProgress.IDLE) 1f else 0f),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            val lead = if (hasError) Icons.Filled.Warning else icon
            if (lead != null) Icon(lead, contentDescription = null, Modifier.size(18.dp))
            Text(label, maxLines = 1)
        }
        when (progress) {
            ButtonProgress.IDLE -> Unit
            ButtonProgress.LOADING ->
                if (offerCancel) Text(stringResource(R.string.control_cancel), maxLines = 1)
                else CroftSpinner(Modifier.size(18.dp).clearAndSetSemantics { })
            ButtonProgress.SUCCESS -> Icon(Icons.Filled.Check, contentDescription = stringResource(R.string.control_done), modifier = Modifier.size(18.dp))
        }
    }
}
