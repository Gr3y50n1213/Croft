// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.ui.components

import androidx.compose.animation.core.Animatable
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.BottomSheetDefaults
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.SheetValue
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import com.example.croft.R
import com.example.croft.haptics.HapticEvent
import com.example.croft.haptics.LocalHaptics
import com.example.croft.theme.LocalMotionLevel
import com.example.croft.theme.MotionLevel
import kotlinx.coroutines.launch

/**
 * A dialog for a destructive or irreversible choice only. The destructive button is on the right, text style, in the error
 * colour; Cancel is on the left. Back and a tap on the scrim cancel. Confirming plays the Success haptic (the destructive
 * dialog confirmed, Subtle) before [onConfirm]. It opens from 90 percent size with a fade (Effects default); at Reduced
 * with the fade only; at Off at once.
 */
@Composable
fun DestructiveDialog(
    title: String,
    message: String,
    confirmLabel: String,
    onConfirm: () -> Unit,
    onDismiss: () -> Unit,
    modifier: Modifier = Modifier,
    /** More content under [message], for a dialog that lists what will be deleted (Remove account, Erase all data). */
    body: (@Composable () -> Unit)? = null,
) {
    val haptics = LocalHaptics.current
    val level = LocalMotionLevel.current
    val enter = remember { Animatable(if (level == MotionLevel.OFF) 1f else 0f) }
    val spec = MaterialTheme.motionScheme.defaultEffectsSpec<Float>()
    LaunchedEffect(Unit) { if (level != MotionLevel.OFF) enter.animateTo(1f, spec) }
    AlertDialog(
        onDismissRequest = onDismiss,
        modifier = modifier
            .motionProbes(
                "enter" to { enter.value },
                "enter-scale" to { if (level == MotionLevel.FULL) DIALOG_START_SCALE + (1f - DIALOG_START_SCALE) * enter.value else 1f },
            )
            .graphicsLayer {
                alpha = enter.value
                if (level == MotionLevel.FULL) {
                    val scale = DIALOG_START_SCALE + (1f - DIALOG_START_SCALE) * enter.value
                    scaleX = scale; scaleY = scale
                }
            },
        title = { Text(title) },
        text = {
            if (body == null) Text(message)
            else androidx.compose.foundation.layout.Column(
                Modifier.verticalScroll(androidx.compose.foundation.rememberScrollState()),
                verticalArrangement = androidx.compose.foundation.layout.Arrangement.spacedBy(8.dp),
            ) {
                Text(message)
                body()
            }
        },
        dismissButton = { TextButton(onClick = onDismiss) { Text(stringResource(R.string.control_cancel)) } },
        confirmButton = {
            TextButton(onClick = { haptics.perform(HapticEvent.Success); onConfirm() }) {
                Text(confirmLabel, color = MaterialTheme.colorScheme.error)
            }
        },
    )
}

private const val DIALOG_START_SCALE = 0.9f

/**
 * A bottom sheet: a drag handle, drag or Back or a tap outside to dismiss, content that moves above the keyboard. A sheet that
 * holds unsaved input ([hasUnsavedInput]) asks before it goes (Discard in the error colour, or Keep editing). The sheet's
 * own motion follows the motion scheme: the spatial default spring at Full and Reduced, none at Off.
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun CroftBottomSheet(
    onDismiss: () -> Unit,
    modifier: Modifier = Modifier,
    hasUnsavedInput: Boolean = false,
    content: @Composable () -> Unit,
) {
    var asking by remember { mutableStateOf(false) }
    var discardConfirmed by remember { mutableStateOf(false) }
    val scope = rememberCoroutineScope()
    val state = rememberModalBottomSheetState(
        skipPartiallyExpanded = true,
        confirmValueChange = { target ->
            if (target == SheetValue.Hidden && hasUnsavedInput && !discardConfirmed) { asking = true; false } else true
        },
    )
    ModalBottomSheet(
        onDismissRequest = onDismiss,
        sheetState = state,
        modifier = modifier,
        dragHandle = { BottomSheetDefaults.DragHandle() },
    ) {
        Column(Modifier.fillMaxWidth().imePadding().padding(horizontal = 16.dp).padding(bottom = 16.dp)) { content() }
    }
    if (asking) {
        DestructiveDialog(
            title = stringResource(R.string.sheet_discard_title),
            message = stringResource(R.string.sheet_discard_message),
            confirmLabel = stringResource(R.string.sheet_discard),
            onConfirm = {
                asking = false
                discardConfirmed = true
                scope.launch { state.hide() }.invokeOnCompletion { onDismiss() }
            },
            onDismiss = { asking = false },
        )
    }
}
