// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.ui.mail

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import com.example.croft.R
import com.example.croft.mail.compose.ComposeClosed
import uniffi.croft_ffi.DraftSummary

/** How a compose screen ended, handed to the Inbox once so that it shows the snackbar. Compared by identity: two equal results are two notices. */
class InboxNotice(val closed: ComposeClosed)

/** Test tags of the local drafts section. */
object DraftTags {
    const val SECTION = "local-drafts"
    fun row(id: Long) = "local-draft-$id"
}

/**
 * The drafts of this device at the top of a Drafts folder: the subject (or "(no subject)"), who it is for and the first words. A tap
 * reopens the draft in compose. The folder's own rows below are the server's copies; those that repeat one of these are hidden.
 */
@Composable
fun LocalDraftsSection(drafts: List<DraftSummary>, onOpen: (DraftSummary) -> Unit, modifier: Modifier = Modifier) {
    Column(modifier.fillMaxWidth().heightIn(max = 260.dp).verticalScroll(rememberScrollState()).testTag(DraftTags.SECTION)) {
        Text(
            stringResource(R.string.drafts_local_title),
            Modifier.padding(horizontal = 16.dp, vertical = 8.dp).semantics { heading() },
            style = MaterialTheme.typography.labelLarge,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        drafts.forEach { d ->
            val subject = d.subject.ifBlank { stringResource(R.string.drafts_local_no_subject) }
            val who = d.toPreview.joinToString(", ")
            val spoken = stringResource(R.string.drafts_local_row, subject, d.snippet)
            Column(
                Modifier.fillMaxWidth().heightIn(min = 56.dp).clickable { onOpen(d) }.padding(horizontal = 16.dp, vertical = 8.dp)
                    .semantics(mergeDescendants = true) { contentDescription = spoken }.testTag(DraftTags.row(d.id)),
                verticalArrangement = Arrangement.spacedBy(2.dp),
            ) {
                Text(subject, style = MaterialTheme.typography.titleMedium, maxLines = 1)
                if (who.isNotBlank()) Text(who, style = MaterialTheme.typography.bodyMedium, maxLines = 1)
                if (d.snippet.isNotBlank()) Text(d.snippet, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant, maxLines = 1)
            }
        }
    }
}
