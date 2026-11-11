// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.ui.components

import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.RowScope
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Warning
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.autofill.ContentType
import androidx.compose.ui.focus.onFocusChanged
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.contentType
import androidx.compose.ui.semantics.error
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.text.input.VisualTransformation
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.unit.dp
import com.example.croft.R

/**
 * When a field is checked (docs/ux/controls.md): validation runs when the field loses focus, not while typing; once it has
 * shown an error it updates live, so the error clears as soon as the text is fixed. [validate] returns the message, or
 * null when the text is fine.
 */
class FieldValidation(initial: String = "", private val validate: (String) -> String?) {
    /** The text. */
    var value by mutableStateOf(initial)
        private set

    /** The message to show, or null. */
    var error by mutableStateOf<String?>(null)
        private set

    private var hadFocus = false
    private var checked = false

    /** The text changed. */
    fun onValueChange(v: String) {
        value = v
        if (checked) error = validate(v)
    }

    /** Focus moved into or out of the field. */
    fun onFocusChange(focused: Boolean) {
        if (focused) hadFocus = true
        else if (hadFocus) {
            error = validate(value)
            checked = error != null
        }
    }
}

/**
 * Outlined text field with every state of the control table: empty with label, focused (the label floats and the outline
 * is 2 dp in the primary colour), filled, error ([errorText] with a warning icon and the words, never colour alone),
 * disabled, read-only, leading and trailing icons, supporting text and a counter. A password field has a Show or Hide
 * button whose state is announced. [contentType] lets autofill and password managers recognise the field.
 */
@Composable
fun CroftTextField(
    value: String,
    onValueChange: (String) -> Unit,
    label: String,
    modifier: Modifier = Modifier,
    enabled: Boolean = true,
    readOnly: Boolean = false,
    errorText: String? = null,
    supportingText: String? = null,
    maxLength: Int? = null,
    leadingIcon: ImageVector? = null,
    trailingIcon: ImageVector? = null,
    trailingIconDescription: String? = null,
    onTrailingClick: (() -> Unit)? = null,
    password: Boolean = false,
    contentType: ContentType? = null,
    keyboardType: KeyboardType = if (password) KeyboardType.Password else KeyboardType.Text,
    onFocusChange: (Boolean) -> Unit = {},
    interactionSource: MutableInteractionSource = remember { MutableInteractionSource() },
) {
    var shown by remember { mutableStateOf(false) }
    val isError = errorText != null
    val show = stringResource(R.string.field_show_password)
    val hide = stringResource(R.string.field_hide_password)
    val shownState = stringResource(R.string.field_password_shown)
    val hiddenState = stringResource(R.string.field_password_hidden)
    val counterWord = if (maxLength != null) stringResource(R.string.field_counter, value.length, maxLength) else null
    OutlinedTextField(
        value = value,
        onValueChange = { if (maxLength == null || it.length <= maxLength) onValueChange(it) },
        modifier = modifier
            .fillMaxWidth()
            .onFocusChanged { onFocusChange(it.isFocused) }
            .semantics {
                if (contentType != null) this.contentType = contentType
                if (errorText != null) error(errorText)
            },
        enabled = enabled,
        readOnly = readOnly,
        label = { Text(label) },
        isError = isError,
        interactionSource = interactionSource,
        singleLine = true,
        visualTransformation = if (password && !shown) PasswordVisualTransformation() else VisualTransformation.None,
        keyboardOptions = KeyboardOptions(keyboardType = keyboardType),
        leadingIcon = leadingIcon?.let { { Icon(it, contentDescription = null) } },
        trailingIcon = when {
            password -> {
                {
                    TextButton(
                        onClick = { shown = !shown },
                        enabled = enabled,
                        modifier = Modifier.semantics {
                            contentDescription = if (shown) hide else show
                            stateDescription = if (shown) shownState else hiddenState
                        },
                    ) { Text(stringResource(if (shown) R.string.field_hide else R.string.field_show)) }
                }
            }
            onTrailingClick != null -> {
                { IconButton(onClick = onTrailingClick, enabled = enabled) { Icon(trailingIcon ?: Icons.Filled.Warning, contentDescription = trailingIconDescription) } }
            }
            trailingIcon != null -> { { Icon(trailingIcon, contentDescription = null) } }
            else -> null
        },
        supportingText = if (errorText != null || supportingText != null || counterWord != null) {
            {
                Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    SupportingMessage(errorText, supportingText)
                    if (counterWord != null) Text(counterWord, style = MaterialTheme.typography.bodySmall)
                }
            }
        } else null,
    )
}

@Composable
private fun RowScope.SupportingMessage(errorText: String?, supportingText: String?) {
    when {
        errorText != null -> Row(Modifier.weight(1f), horizontalArrangement = Arrangement.spacedBy(4.dp)) {
            Icon(Icons.Filled.Warning, contentDescription = null, modifier = Modifier.size(16.dp))
            Text(errorText)
        }
        supportingText != null -> Text(supportingText, Modifier.weight(1f))
        else -> androidx.compose.foundation.layout.Spacer(Modifier.weight(1f))
    }
}
