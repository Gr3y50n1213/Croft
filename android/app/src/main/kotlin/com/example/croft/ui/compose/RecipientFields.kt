// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.ui.compose

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.KeyboardArrowDown
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.SideEffect
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.focus.onFocusChanged
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.platform.LocalFocusManager
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.focus.FocusDirection
import androidx.compose.ui.unit.dp
import com.example.croft.R
import com.example.croft.mail.compose.ComposeController
import com.example.croft.mail.compose.RecipientChipData
import com.example.croft.mail.compose.RecipientField
import com.example.croft.ui.components.CroftIconButton
import com.example.croft.ui.components.CroftMenu
import com.example.croft.ui.components.MenuItemSpec
import com.example.croft.ui.components.RecipientChip
import com.example.croft.ui.components.RecipientChipState
import kotlinx.coroutines.launch
import uniffi.croft_ffi.FfiAddress

/**
 * One recipient field of S23 (To, Cc or Bcc): the input chips, a text input after them and the suggestions under it.
 *
 * - Typed or pasted text becomes chips on a comma, a semicolon or Enter, on a space after something that looks like an address, on the
 *   keyboard's Next key and when the field loses focus. The chip is made by the core's parser ([ComposeController.commitRecipients]);
 *   an address the core does not accept is an error chip with an icon and the words "Invalid address", never colour alone.
 * - A tap on a chip opens its menu (Move to Cc, Move to Bcc, Remove).
 * - Suggestions come from the account's contacts, then the addresses of mail sent from it ([ComposeController.suggest]). No contacts permission:
 *   the contacts are Croft's own (T38b), never the phone's.
 */
@OptIn(ExperimentalLayoutApi::class)
@Composable
internal fun RecipientRow(
    label: String,
    field: RecipientField,
    chips: List<RecipientChipData>,
    controller: ComposeController,
    modifier: Modifier = Modifier,
    focusRequester: FocusRequester? = null,
    /** Counts up when Send or Back turned the typed text into chips: this field clears its text then. */
    clearTick: Int = 0,
    trailing: (@Composable () -> Unit)? = null,
) {
    val scope = rememberCoroutineScope()
    var text by rememberSaveable(field.name) { mutableStateOf("") }
    var suggestions by remember { mutableStateOf<List<FfiAddress>>(emptyList()) }
    var menuChip by remember { mutableStateOf<RecipientChipData?>(null) }
    val focus = LocalFocusManager.current
    val chosen = chips.map { it.address.address.lowercase() }.toSet()

    LaunchedEffect(text) { suggestions = if (text.isBlank()) emptyList() else controller.suggest(text) }
    // What is typed and not a chip yet is also held by the controller, so that Send and Back turn it into a chip instead of losing it.
    SideEffect { controller.setPendingRecipientText(field, text) }
    var seenTick by rememberSaveable(field.name + "-tick") { mutableIntStateOf(clearTick) }
    LaunchedEffect(clearTick) { if (clearTick != seenTick) { seenTick = clearTick; text = "" } }

    fun commit(raw: String) {
        if (raw.isBlank()) return
        scope.launch { controller.commitRecipients(field, raw) }
    }

    Column(modifier.fillMaxWidth()) {
        Row(Modifier.fillMaxWidth().heightIn(min = 56.dp), verticalAlignment = Alignment.Top) {
            Text(
                label,
                Modifier.padding(top = 16.dp, end = 12.dp).widthIn(min = 40.dp),
                style = MaterialTheme.typography.bodyLarge,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
            FlowRow(
                Modifier.weight(1f).padding(vertical = 8.dp),
                horizontalArrangement = Arrangement.spacedBy(8.dp),
                verticalArrangement = Arrangement.spacedBy(6.dp),
                itemVerticalAlignment = Alignment.CenterVertically,
            ) {
                chips.forEach { chip ->
                    Box {
                        RecipientChip(
                            name = chip.label,
                            state = if (chip.valid) RecipientChipState.NORMAL else RecipientChipState.ERROR,
                            onClick = { menuChip = chip },
                        )
                        CroftMenu(
                            expanded = menuChip == chip,
                            onDismiss = { menuChip = null },
                            items = chipMenu(field, chip, controller, scope),
                            // The menu always shows the address, whatever name the chip carries.
                            header = { Text(chip.address.address, Modifier.padding(horizontal = 16.dp, vertical = 8.dp), style = MaterialTheme.typography.labelLarge) },
                        )
                    }
                }
                BasicTextField(
                    value = text,
                    onValueChange = { new ->
                        val cut = maxOf(new.lastIndexOf(','), new.lastIndexOf(';'), new.lastIndexOf('\n'))
                        when {
                            cut >= 0 -> { commit(new.substring(0, cut)); text = new.substring(cut + 1) }
                            new.endsWith(' ') && new.contains('@') && !new.contains('<') -> { commit(new); text = "" }
                            else -> text = new
                        }
                    },
                    modifier = Modifier
                        .weight(1f)
                        .widthIn(min = 120.dp)
                        .heightIn(min = 40.dp)
                        .then(if (focusRequester != null) Modifier.focusRequester(focusRequester) else Modifier)
                        .onFocusChanged { if (!it.isFocused && text.isNotBlank()) { commit(text); text = "" } }
                        .semantics { contentDescription = label },
                    singleLine = true,
                    textStyle = MaterialTheme.typography.bodyLarge.copy(color = MaterialTheme.colorScheme.onSurface),
                    cursorBrush = SolidColor(MaterialTheme.colorScheme.primary),
                    keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Email, imeAction = ImeAction.Next),
                    keyboardActions = KeyboardActions(onNext = { commit(text); text = ""; focus.moveFocus(FocusDirection.Down) }),
                    decorationBox = { inner ->
                        Box(Modifier.heightIn(min = 40.dp), contentAlignment = Alignment.CenterStart) {
                            if (text.isEmpty() && chips.isEmpty()) {
                                Text(stringResource(R.string.compose_recipient_hint), style = MaterialTheme.typography.bodyLarge, color = MaterialTheme.colorScheme.onSurfaceVariant)
                            }
                            inner()
                        }
                    },
                )
            }
            trailing?.invoke()
        }
        val shown = suggestions.filter { it.address.lowercase() !in chosen }
        if (shown.isNotEmpty()) {
            Column(Modifier.fillMaxWidth().padding(start = 40.dp)) {
                Text(
                    stringResource(R.string.compose_suggestions),
                    Modifier.padding(vertical = 4.dp).semantics { heading() },
                    style = MaterialTheme.typography.labelMedium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
                shown.forEach { s ->
                    Column(
                        Modifier.fillMaxWidth().heightIn(min = 48.dp).clickable { text = ""; suggestions = emptyList(); scope.launch { controller.addRecipient(field, s) } }.padding(vertical = 6.dp, horizontal = 4.dp),
                    ) {
                        s.name?.takeIf { it.isNotBlank() }?.let { Text(it, style = MaterialTheme.typography.bodyLarge, maxLines = 1) }
                        Text(s.address, style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant, maxLines = 1)
                    }
                }
            }
        }
        HorizontalDivider(color = MaterialTheme.colorScheme.outlineVariant)
    }
}

@Composable
private fun chipMenu(field: RecipientField, chip: RecipientChipData, controller: ComposeController, scope: kotlinx.coroutines.CoroutineScope): List<MenuItemSpec> {
    val toCc = stringResource(R.string.compose_chip_move_to_cc)
    val toBcc = stringResource(R.string.compose_chip_move_to_bcc)
    val toTo = stringResource(R.string.compose_chip_move_to_to)
    val remove = stringResource(R.string.compose_chip_remove)
    fun move(to: RecipientField): () -> Unit = { scope.launch { controller.moveRecipient(field, chip, to) } }
    return listOf(
        MenuItemSpec(toTo, move(RecipientField.TO), visible = chip.valid && field != RecipientField.TO),
        MenuItemSpec(toCc, move(RecipientField.CC), visible = chip.valid && field != RecipientField.CC),
        MenuItemSpec(toBcc, move(RecipientField.BCC), visible = chip.valid && field != RecipientField.BCC),
        MenuItemSpec(remove, { scope.launch { controller.removeRecipient(field, chip) } }, destructive = true),
    )
}

/** The toggle at the end of the To row that shows Cc and Bcc. */
@Composable
internal fun CcBccToggle(onClick: () -> Unit) {
    CroftIconButton(Icons.Filled.KeyboardArrowDown, stringResource(R.string.compose_show_cc_bcc), onClick)
}
