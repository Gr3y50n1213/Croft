// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.ui.mail

import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import com.example.croft.R
import com.example.croft.mail.MailSettings
import com.example.croft.ui.appearance.ChoiceRow
import com.example.croft.ui.appearance.SectionLabel
import com.example.croft.ui.appearance.SettingsFrame

/** Test tags of S49. */
object InboxSettingsTags {
    const val SCREEN = "S49"
    const val UNDO_TIME = "S49-undo-time"
}

/**
 * S49 Inbox and categories, as far as it exists: the Undo time (3, 5 or 10 seconds). It is how long the Undo snackbar of a move, a
 * delete or a spam stays, and how long the core waits before it sends the change; the choice applies at once to everything done from
 * then on and is kept ([com.example.croft.mail.MailSettingsStore]). Categories, conversation view, preview lines, the undo time of
 * Send, the mark-read delay and remote content come with their own tasks.
 */
@Composable
fun InboxSettingsScreen(
    settings: MailSettings,
    onChange: (MailSettings) -> Unit,
    onBack: () -> Unit,
    modifier: Modifier = Modifier,
) {
    SettingsFrame(title = stringResource(R.string.inbox_settings_title), onBack = onBack, modifier = modifier.testTag(InboxSettingsTags.SCREEN)) {
        SectionLabel(stringResource(R.string.inbox_undo_time))
        ChoiceRow(
            options = MailSettings.UndoChoices,
            selected = settings.undoSeconds,
            label = { stringResource(R.string.inbox_undo_seconds, it) },
            onSelect = { onChange(settings.copy(undoSeconds = it)) },
            modifier = Modifier.testTag(InboxSettingsTags.UNDO_TIME),
        )
        Text(
            stringResource(R.string.inbox_undo_time_hint),
            style = MaterialTheme.typography.bodyMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            modifier = Modifier.fillMaxWidth().padding(top = 8.dp),
        )
        Text(
            stringResource(R.string.inbox_settings_more),
            style = MaterialTheme.typography.bodyMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            modifier = Modifier.fillMaxWidth().padding(top = 16.dp),
        )
    }
}
