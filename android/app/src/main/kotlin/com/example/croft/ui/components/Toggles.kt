// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.ui.components

import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.selection.selectable
import androidx.compose.foundation.selection.toggleable
import androidx.compose.foundation.selection.triStateToggleable
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.RadioButton
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.state.ToggleableState
import androidx.compose.ui.unit.dp
import androidx.compose.ui.draw.alpha
import com.example.croft.haptics.HapticEvent
import com.example.croft.haptics.LocalHaptics

private val RowShapeMinHeight = 48.dp

@Composable
private fun RowLabel(title: String, hint: String?, enabled: Boolean, modifier: Modifier = Modifier) {
    Column(modifier.alpha(if (enabled) 1f else DISABLED_CONTENT)) {
        Text(title, style = MaterialTheme.typography.bodyLarge, color = MaterialTheme.colorScheme.onSurface)
        if (hint != null) Text(hint, style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
    }
}

/**
 * A switch row: the whole row is the target (at least 48 dp), the thumb grows while pressed and moves with the spatial fast
 * spring (instant at Reduced and Off, through the motion scheme). Toggling applies at once and plays the ToggleOn or
 * ToggleOff haptic through `Haptics`. A setting that needs a restart says so in [hint], under the label.
 */
@Composable
fun CroftSwitchRow(
    title: String,
    checked: Boolean,
    onCheckedChange: (Boolean) -> Unit,
    modifier: Modifier = Modifier,
    hint: String? = null,
    enabled: Boolean = true,
    interactionSource: MutableInteractionSource = remember { MutableInteractionSource() },
) {
    val haptics = LocalHaptics.current
    Row(
        modifier
            .fillMaxWidth()
            .croftFocusRing(interactionSource, MaterialTheme.shapes.small)
            .toggleable(
                value = checked,
                enabled = enabled,
                role = Role.Switch,
                interactionSource = interactionSource,
                indication = androidx.compose.foundation.LocalIndication.current,
                onValueChange = { haptics.perform(if (it) HapticEvent.ToggleOn else HapticEvent.ToggleOff); onCheckedChange(it) },
            )
            .heightIn(min = RowShapeMinHeight)
            .padding(vertical = 4.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(16.dp),
    ) {
        RowLabel(title, hint, enabled, Modifier.weight(1f))
        Switch(checked = checked, onCheckedChange = null, enabled = enabled, interactionSource = interactionSource)
    }
}

/** A radio row: the whole row selects. */
@Composable
fun CroftRadioRow(
    title: String,
    selected: Boolean,
    onSelect: () -> Unit,
    modifier: Modifier = Modifier,
    hint: String? = null,
    enabled: Boolean = true,
    interactionSource: MutableInteractionSource = remember { MutableInteractionSource() },
) {
    Row(
        modifier
            .fillMaxWidth()
            .croftFocusRing(interactionSource, MaterialTheme.shapes.small)
            .selectable(selected = selected, enabled = enabled, role = Role.RadioButton, interactionSource = interactionSource, indication = androidx.compose.foundation.LocalIndication.current, onClick = onSelect)
            .heightIn(min = RowShapeMinHeight)
            .padding(vertical = 4.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(16.dp),
    ) {
        RadioButton(selected = selected, onClick = null, enabled = enabled, interactionSource = interactionSource)
        RowLabel(title, hint, enabled, Modifier.weight(1f))
    }
}

/**
 * A checkbox row with the three states of Select all: off, on and indeterminate (some selected). A tap on an
 * indeterminate box selects all, which is what the caller's [onToggle] gets as `true`.
 */
@Composable
fun CroftCheckboxRow(
    title: String,
    state: ToggleableState,
    onToggle: (Boolean) -> Unit,
    modifier: Modifier = Modifier,
    hint: String? = null,
    enabled: Boolean = true,
    interactionSource: MutableInteractionSource = remember { MutableInteractionSource() },
) {
    val haptics = LocalHaptics.current
    Row(
        modifier
            .fillMaxWidth()
            .croftFocusRing(interactionSource, MaterialTheme.shapes.small)
            .triStateToggleable(
                state = state,
                enabled = enabled,
                role = Role.Checkbox,
                interactionSource = interactionSource,
                indication = androidx.compose.foundation.LocalIndication.current,
                onClick = { haptics.perform(HapticEvent.SelectionTick); onToggle(state != ToggleableState.On) },
            )
            .heightIn(min = RowShapeMinHeight)
            .padding(vertical = 4.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(16.dp),
    ) {
        androidx.compose.material3.TriStateCheckbox(state = state, onClick = null, enabled = enabled, interactionSource = interactionSource)
        RowLabel(title, hint, enabled, Modifier.weight(1f))
    }
}
