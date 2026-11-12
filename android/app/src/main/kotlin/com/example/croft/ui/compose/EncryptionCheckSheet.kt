// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.ui.compose

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Warning
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.pluralStringResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import com.example.croft.R
import com.example.croft.ui.components.ButtonKind
import com.example.croft.ui.components.CroftBottomSheet
import com.example.croft.ui.components.CroftButton

/** Test tags of S24. */
object EncryptionTags {
    const val SHEET = "encryption-sheet"
    const val SEND_UNENCRYPTED = "encryption-send-unencrypted"
    const val CANCEL = "encryption-cancel"
}

/**
 * S24 Encryption check (the seam only; no keys exist before phase 3): shown when the [com.example.croft.mail.compose.EncryptionPolicy]
 * says encryption is required and [missing] (addresses) have no key. It names each one and offers Send unencrypted or Cancel. Dismissing
 * the sheet is Cancel; nothing is sent without the person pressing Send unencrypted. The title is announced when the sheet opens.
 */
@Composable
fun EncryptionCheckSheet(
    missing: List<String>,
    onSendUnencrypted: () -> Unit,
    onCancel: () -> Unit,
    modifier: Modifier = Modifier,
) {
    CroftBottomSheet(onDismiss = onCancel, modifier = modifier.testTag(EncryptionTags.SHEET)) {
        EncryptionCheckContent(missing, onSendUnencrypted, onCancel)
    }
}

/** The content of the S24 sheet, apart from the sheet itself (the screenshots draw it directly). */
@Composable
fun EncryptionCheckContent(missing: List<String>, onSendUnencrypted: () -> Unit, onCancel: () -> Unit) {
    Column(Modifier.fillMaxWidth(), verticalArrangement = Arrangement.spacedBy(12.dp)) {
        Text(
            pluralStringResource(R.plurals.encryption_title, missing.size, missing.size),
            Modifier.semantics { heading(); liveRegion = LiveRegionMode.Assertive },
            style = MaterialTheme.typography.titleLarge,
        )
        Text(stringResource(R.string.encryption_message), style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
        missing.forEach { address ->
            Row(Modifier.fillMaxWidth().padding(vertical = 4.dp), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                Icon(Icons.Filled.Warning, null, Modifier.size(24.dp), tint = MaterialTheme.colorScheme.error)
                Column {
                    Text(address, style = MaterialTheme.typography.bodyLarge)
                    Text(stringResource(R.string.encryption_row_no_key), style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
                }
            }
        }
        CroftButton(stringResource(R.string.encryption_send_unencrypted), onSendUnencrypted, Modifier.fillMaxWidth().testTag(EncryptionTags.SEND_UNENCRYPTED), kind = ButtonKind.OUTLINED)
        CroftButton(stringResource(R.string.encryption_cancel), onCancel, Modifier.fillMaxWidth().testTag(EncryptionTags.CANCEL), kind = ButtonKind.TEXT)
    }
}
