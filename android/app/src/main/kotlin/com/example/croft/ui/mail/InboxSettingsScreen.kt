// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.ui.mail

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Close
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import com.example.croft.R
import com.example.croft.mail.MailSession
import com.example.croft.mail.MailSettings
import com.example.croft.mail.reading.MarkReadDelay
import com.example.croft.mail.reading.ReadFailure
import com.example.croft.mail.reading.RemoteImages
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.launch
import com.example.croft.ui.appearance.ChoiceRow
import com.example.croft.ui.appearance.SectionLabel
import com.example.croft.ui.appearance.SettingsFrame
import com.example.croft.ui.components.CroftCheckboxRow
import com.example.croft.ui.components.CroftSwitchRow
import com.example.croft.mail.MailCategory
import androidx.compose.ui.state.ToggleableState

/** Test tags of S49. */
object InboxSettingsTags {
    const val SCREEN = "S49"
    const val UNDO_TIME = "S49-undo-time"
    const val REMOTE_IMAGES = "S49-remote-images"
    const val MARK_READ = "S49-mark-read"
    const val KNOWN_SENDERS = "S49-known-senders"
    const val CATEGORIES = "S49-categories"
    fun category(c: MailCategory) = "S49-category-${c.key}"
    fun knownSender(address: String) = "S49-known-$address"
}

/**
 * S49 Inbox and categories, as far as it exists: the Undo time (3, 5 or 10 seconds). It is how long the Undo snackbar of a move, a
 * delete or a spam stays, and how long the core waits before it sends the change; the choice applies at once to everything done from
 * then on and is kept ([com.example.croft.mail.MailSettingsStore]).
 *
 * T18b adds the two settings of reading and the list that belongs to one of them: **Remote images** (Ask, Known senders, Never; default
 * Ask), **Mark as read when opened** (Immediately, After 2 s, Never; default After 2 s) and, when [session] is given, the **known senders**
 * with a button to remove each. Categories, conversation view, preview lines and the undo time of Send come with their own tasks.
 */
@Composable
fun InboxSettingsScreen(
    settings: MailSettings,
    onChange: (MailSettings) -> Unit,
    onBack: () -> Unit,
    modifier: Modifier = Modifier,
    session: MailSession? = null,
) {
    SettingsFrame(title = stringResource(R.string.inbox_settings_title), onBack = onBack, modifier = modifier.testTag(InboxSettingsTags.SCREEN)) {
        // T35: the category chips above the Inbox, and which of them show.
        SectionLabel(stringResource(R.string.inbox_categories_title))
        CroftSwitchRow(
            title = stringResource(R.string.inbox_categories_switch),
            checked = settings.categoriesOn,
            onCheckedChange = { onChange(settings.copy(categoriesOn = it)) },
            hint = stringResource(R.string.inbox_categories_hint),
            modifier = Modifier.testTag(InboxSettingsTags.CATEGORIES),
        )
        if (settings.categoriesOn) {
            for (c in MailCategory.entries) {
                CroftCheckboxRow(
                    title = categoryName(c),
                    state = ToggleableState(c in settings.shownCategories),
                    onToggle = { on ->
                        onChange(settings.copy(shownCategories = if (on) settings.shownCategories + c else settings.shownCategories - c))
                    },
                    modifier = Modifier.testTag(InboxSettingsTags.category(c)),
                )
            }
        }
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
        SectionLabel(stringResource(R.string.reading_settings_remote_title))
        ChoiceRow(
            options = RemoteImages.entries,
            selected = settings.remoteImages,
            label = {
                stringResource(
                    when (it) {
                        RemoteImages.ASK -> R.string.reading_settings_remote_ask
                        RemoteImages.KNOWN_SENDERS -> R.string.reading_settings_remote_known
                        RemoteImages.NEVER -> R.string.reading_settings_remote_never
                    },
                )
            },
            onSelect = { onChange(settings.copy(remoteImages = it)) },
            modifier = Modifier.testTag(InboxSettingsTags.REMOTE_IMAGES),
        )
        Text(
            stringResource(R.string.reading_settings_remote_hint),
            style = MaterialTheme.typography.bodyMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            modifier = Modifier.fillMaxWidth().padding(top = 8.dp),
        )
        SectionLabel(stringResource(R.string.reading_settings_markread_title))
        ChoiceRow(
            options = MarkReadDelay.entries,
            selected = settings.markRead,
            label = {
                stringResource(
                    when (it) {
                        MarkReadDelay.IMMEDIATELY -> R.string.reading_settings_markread_now
                        MarkReadDelay.AFTER_2_SECONDS -> R.string.reading_settings_markread_2s
                        MarkReadDelay.NEVER -> R.string.reading_settings_markread_never
                    },
                )
            },
            onSelect = { onChange(settings.copy(markRead = it)) },
            modifier = Modifier.testTag(InboxSettingsTags.MARK_READ),
        )
        Text(
            stringResource(R.string.reading_settings_markread_hint),
            style = MaterialTheme.typography.bodyMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            modifier = Modifier.fillMaxWidth().padding(top = 8.dp),
        )
        if (session?.reading != null) KnownSenders(session)
        Text(
            stringResource(R.string.inbox_settings_more),
            style = MaterialTheme.typography.bodyMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            modifier = Modifier.fillMaxWidth().padding(top = 16.dp),
        )
    }
}

/** The known senders (the people whose images show without asking under "Known senders"), one row each with a button that removes it. */
@Composable
private fun KnownSenders(session: MailSession) {
    val reading = session.reading ?: return
    var entries by remember { mutableStateOf<List<Pair<Long, String>>?>(null) }
    val scope = rememberCoroutineScope()
    LaunchedEffect(session) {
        entries = try {
            session.repository.listAccounts().flatMap { a ->
                try { reading.trustedSenders(a.id).map { a.id to it } } catch (e: ReadFailure) { emptyList() }
            }
        } catch (e: CancellationException) {
            throw e
        } catch (e: Exception) {
            emptyList()
        }
    }
    SectionLabel(stringResource(R.string.reading_settings_known_title))
    Column(Modifier.fillMaxWidth().testTag(InboxSettingsTags.KNOWN_SENDERS)) {
        val list = entries
        when {
            list == null -> Unit
            list.isEmpty() -> Text(stringResource(R.string.reading_settings_known_empty), style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
            else -> list.forEach { (account, address) ->
                Row(Modifier.fillMaxWidth().heightIn(min = 48.dp).testTag(InboxSettingsTags.knownSender(address)), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    Text(address, Modifier.weight(1f), style = MaterialTheme.typography.bodyLarge)
                    IconButton(onClick = {
                        scope.launch {
                            try { reading.untrustSender(account, address) } catch (e: ReadFailure) { return@launch }
                            entries = entries?.filter { it != account to address }
                        }
                    }) { Icon(Icons.Filled.Close, contentDescription = stringResource(R.string.reading_settings_known_remove, address)) }
                }
            }
        }
    }
}
