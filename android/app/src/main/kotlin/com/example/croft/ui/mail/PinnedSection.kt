// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.ui.mail

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import com.example.croft.R
import com.example.croft.mail.MessageId
import com.example.croft.mail.MessageRowData
import com.example.croft.ui.components.CroftIconButton
import com.example.croft.ui.components.CroftIcons

/** Test tags of the Inbox's Pinned section (T36b). */
object PinnedTags {
    const val SECTION = "pinned-section"
    const val TOGGLE = "pinned-toggle"

    fun row(id: MessageId) = "pinned-row-$id"

    fun unpin(id: MessageId) = "pinned-unpin-$id"
}

/**
 * T36b: the pinned messages of the Inbox shown, above its list: a heading, the first [collapsed] rows (all of them after "Show all"),
 * each opening its message on a tap and with its own Unpin. Pinned messages are not in the list below, so nothing shows twice.
 */
@Composable
fun PinnedSection(
    rows: List<MessageRowData>,
    expanded: Boolean,
    collapsed: Int,
    onToggle: () -> Unit,
    onOpen: (MessageId) -> Unit,
    onUnpin: (MessageRowData) -> Unit,
    modifier: Modifier = Modifier,
) {
    if (rows.isEmpty()) return
    val shown = if (expanded) rows else rows.take(collapsed)
    Column(modifier.fillMaxWidth().heightIn(max = 280.dp).verticalScroll(rememberScrollState()).testTag(PinnedTags.SECTION)) {
        Row(Modifier.fillMaxWidth().padding(start = 16.dp, end = 4.dp), verticalAlignment = Alignment.CenterVertically) {
            Text(
                stringResource(R.string.pinned_title),
                Modifier.weight(1f).padding(vertical = 8.dp).semantics { heading() },
                style = MaterialTheme.typography.labelLarge,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
            if (rows.size > collapsed) {
                TextButton(onClick = onToggle, modifier = Modifier.testTag(PinnedTags.TOGGLE)) {
                    Text(stringResource(if (expanded) R.string.pinned_show_less else R.string.pinned_show_all))
                }
            }
        }
        shown.forEach { r ->
            val subject = r.subject.ifBlank { stringResource(R.string.drafts_local_no_subject) }
            val spoken = stringResource(R.string.pinned_row, r.displayName, subject)
            Row(
                Modifier.fillMaxWidth().heightIn(min = 56.dp).testTag(PinnedTags.row(r.id)),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Column(
                    Modifier.weight(1f).clickable { onOpen(r.id) }.padding(start = 16.dp, top = 8.dp, bottom = 8.dp)
                        .semantics(mergeDescendants = true) { contentDescription = spoken },
                    verticalArrangement = Arrangement.spacedBy(2.dp),
                ) {
                    Text(
                        r.displayName,
                        style = MaterialTheme.typography.titleMedium,
                        fontWeight = if (r.seen) FontWeight.Normal else FontWeight.Bold,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                    )
                    Text(subject, style = MaterialTheme.typography.bodyMedium, maxLines = 1, overflow = TextOverflow.Ellipsis)
                }
                CroftIconButton(CroftIcons.Pin, stringResource(R.string.mail_action_unpin), { onUnpin(r) }, Modifier.testTag(PinnedTags.unpin(r.id)))
            }
        }
    }
}
