// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.ui.reading

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.selection.toggleable
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Checkbox
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.pluralStringResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.unit.dp
import com.example.croft.R
import com.example.croft.mail.reading.UnsubscribeInfoData
import com.example.croft.mail.reading.UnsubscribeWay

/** Test tags of S21's unsubscribe banner and S31 (T37). */
object UnsubscribeTags {
    const val BANNER = "S21-unsubscribe"
    const val ACTION = "S21-unsubscribe-action"
    const val DIALOG = "S31"
    const val DELETE_OLDER = "S31-delete-older"
    const val CONFIRM = "S31-confirm"
    const val CANCEL = "S31-cancel"
}

/** T37: S21's banner on a mailing-list message: Unsubscribe, or that the person already did. */
@Composable
fun UnsubscribeBanner(info: UnsubscribeInfoData, onUnsubscribe: () -> Unit, modifier: Modifier = Modifier) {
    Surface(modifier.fillMaxWidth().testTag(UnsubscribeTags.BANNER), shape = MaterialTheme.shapes.medium, color = MaterialTheme.colorScheme.surfaceContainerHigh) {
        Row(Modifier.padding(start = 16.dp, end = 4.dp, top = 4.dp, bottom = 4.dp), verticalAlignment = Alignment.CenterVertically) {
            Text(
                stringResource(if (info.already) R.string.unsubscribe_already else R.string.unsubscribe_banner, info.sender),
                Modifier.weight(1f).padding(vertical = 8.dp),
                style = MaterialTheme.typography.bodyMedium,
            )
            if (!info.already) {
                TextButton(onClick = onUnsubscribe, modifier = Modifier.testTag(UnsubscribeTags.ACTION)) { Text(stringResource(R.string.unsubscribe_action)) }
            }
        }
    }
}

/**
 * T37: S31, the confirmation before Croft contacts the sender: who, how (the host, or the address the email goes to), and "Also delete
 * the N older messages from this sender" when there are any.
 */
@Composable
fun UnsubscribeDialog(info: UnsubscribeInfoData, busy: Boolean, onConfirm: (Boolean) -> Unit, onCancel: () -> Unit) {
    var deleteOlder by rememberSaveable { mutableStateOf(false) }
    AlertDialog(
        onDismissRequest = onCancel,
        modifier = Modifier.testTag(UnsubscribeTags.DIALOG),
        title = { Text(stringResource(R.string.unsubscribe_title, info.sender)) },
        text = {
            Column(verticalArrangement = Arrangement.spacedBy(12.dp)) {
                val how = when (info.way) {
                    UnsubscribeWay.ONE_CLICK -> stringResource(R.string.unsubscribe_one_click, info.host.orEmpty())
                    UnsubscribeWay.EMAIL -> stringResource(R.string.unsubscribe_email, info.address.orEmpty())
                    UnsubscribeWay.WEBSITE -> stringResource(R.string.unsubscribe_website, info.host.orEmpty())
                }
                Text(how)
                if (info.olderCount > 0) {
                    Row(
                        Modifier.fillMaxWidth()
                            .toggleable(value = deleteOlder, role = Role.Checkbox, onValueChange = { deleteOlder = it })
                            .testTag(UnsubscribeTags.DELETE_OLDER),
                        verticalAlignment = Alignment.CenterVertically,
                    ) {
                        Checkbox(checked = deleteOlder, onCheckedChange = null)
                        Text(pluralStringResource(R.plurals.unsubscribe_delete_older, info.olderCount, info.olderCount), Modifier.padding(start = 8.dp))
                    }
                }
            }
        },
        confirmButton = {
            TextButton(onClick = { onConfirm(deleteOlder) }, enabled = !busy, modifier = Modifier.testTag(UnsubscribeTags.CONFIRM)) {
                Text(stringResource(R.string.unsubscribe_action))
            }
        },
        dismissButton = {
            TextButton(onClick = onCancel, enabled = !busy, modifier = Modifier.testTag(UnsubscribeTags.CANCEL)) {
                Text(stringResource(R.string.unsubscribe_cancel))
            }
        },
    )
}
