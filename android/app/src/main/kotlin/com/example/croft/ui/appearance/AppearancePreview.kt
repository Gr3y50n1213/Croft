// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.ui.appearance

import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.DateRange
import androidx.compose.material.icons.filled.Edit
import androidx.compose.material.icons.filled.Email
import androidx.compose.material.icons.filled.Person
import androidx.compose.material3.ExtendedFloatingActionButton
import androidx.compose.material3.FilterChip
import androidx.compose.material3.FloatingActionButton
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.NavigationBar
import androidx.compose.material3.NavigationBarItem
import androidx.compose.material3.Snackbar
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.input.pointer.PointerEventPass
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import com.example.croft.R
import com.example.croft.theme.AvatarStyle
import com.example.croft.theme.ListStyle
import com.example.croft.theme.LocalThemeSettings
import com.example.croft.theme.UnreadEmphasis

/** Test tag of the live preview on S08, S50 and S51, so a test can compare its bounds before and after a theme change. */
const val PREVIEW_TAG = "appearance-preview"

/** Test tag of the S10-style sample. */
const val INBOX_SAMPLE_TAG = "inbox-sample"

/** Fake rows used by the preview and the sample. */
private data class FakeRow(val sender: Int, val subject: Int, val snippet: Int, val time: Int, val unread: Boolean, val letter: String)

private val FakeRows = listOf(
    FakeRow(R.string.preview_sender_one, R.string.preview_subject_one, R.string.preview_snippet_one, R.string.preview_time_one, true, "S"),
    FakeRow(R.string.preview_sender_two, R.string.preview_subject_two, R.string.preview_snippet_two, R.string.preview_time_two, false, "H"),
    FakeRow(R.string.preview_sender_three, R.string.preview_subject_three, R.string.preview_snippet_three, R.string.preview_time_three, false, "D"),
)

/** Draws [content] and swallows every touch on it: the preview is made of real components but is not interactive. */
@Composable
private fun NonInteractive(modifier: Modifier = Modifier, description: String, content: @Composable () -> Unit) {
    Box(modifier.clearAndSetSemantics { contentDescription = description }) {
        content()
        Box(
            Modifier.matchParentSize().pointerInput(Unit) {
                awaitPointerEventScope {
                    while (true) awaitPointerEvent(PointerEventPass.Initial).changes.forEach { it.consume() }
                }
            },
        )
    }
}

@Composable
private fun PreviewSearchPill() {
    Surface(Modifier.fillMaxWidth(), shape = CircleShape, color = MaterialTheme.colorScheme.surfaceContainerHigh) {
        Row(Modifier.padding(start = 20.dp, end = 12.dp).heightIn(min = 48.dp), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp)) {
            Text(stringResource(R.string.preview_search), Modifier.weight(1f), style = MaterialTheme.typography.bodyLarge, color = MaterialTheme.colorScheme.onSurfaceVariant, maxLines = 1, overflow = TextOverflow.Ellipsis)
            Box(Modifier.size(28.dp).background(MaterialTheme.colorScheme.primary, CircleShape))
        }
    }
}

@Composable
private fun PreviewChips() {
    Wrap {
        FilterChip(selected = false, onClick = {}, label = { Text(stringResource(R.string.preview_chip_all)) })
        FilterChip(selected = true, onClick = {}, label = { Text(stringResource(R.string.preview_chip_primary)) })
        FilterChip(selected = false, onClick = {}, label = { Text(stringResource(R.string.preview_chip_updates)) })
    }
}

/** One inbox row at the density, avatar, preview and unread settings in [LocalThemeSettings]. */
@Composable
private fun PreviewRow(row: FakeRow, avatarFilled: Boolean) {
    val s = LocalThemeSettings.current
    val d = s.density
    val bold = row.unread && s.unreadEmphasis != UnreadEmphasis.DOT
    val dot = row.unread && s.unreadEmphasis != UnreadEmphasis.BOLD
    val colours = MaterialTheme.colorScheme
    Row(
        Modifier.fillMaxWidth().heightIn(min = d.rowDp.dp).padding(horizontal = 12.dp, vertical = 8.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        if (s.avatars != AvatarStyle.HIDDEN) {
            Box(
                Modifier.size(d.avatarDp.dp).background(if (avatarFilled) colours.primary else colours.secondaryContainer, CircleShape),
                contentAlignment = Alignment.Center,
            ) {
                Text(row.letter, style = MaterialTheme.typography.labelLarge, color = if (avatarFilled) colours.onPrimary else colours.onSecondaryContainer)
            }
        }
        Column(Modifier.weight(1f)) {
            Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                if (dot) Box(Modifier.size(8.dp).background(colours.primary, CircleShape))
                Text(stringResource(row.sender), Modifier.weight(1f), style = MaterialTheme.typography.titleSmall, fontWeight = if (bold) FontWeight.Bold else FontWeight.Normal, maxLines = 1, overflow = TextOverflow.Ellipsis)
                Text(stringResource(row.time), style = MaterialTheme.typography.labelSmall, color = colours.onSurfaceVariant)
            }
            Text(stringResource(row.subject), style = MaterialTheme.typography.bodyMedium, fontWeight = if (bold) FontWeight.Bold else FontWeight.Normal, maxLines = 1, overflow = TextOverflow.Ellipsis)
            if (d.previewLines > 0) {
                Text(stringResource(row.snippet), style = MaterialTheme.typography.bodySmall, color = colours.onSurfaceVariant, maxLines = d.previewLines, overflow = TextOverflow.Ellipsis)
            }
        }
    }
}

@Composable
private fun PreviewList(rows: List<FakeRow>) {
    val flat = LocalThemeSettings.current.listStyle == ListStyle.FLAT
    val inner = @Composable {
        Column {
            rows.forEachIndexed { i, r -> PreviewRow(r, avatarFilled = i == 0) }
        }
    }
    if (flat) inner() else Surface(Modifier.fillMaxWidth(), shape = MaterialTheme.shapes.large, color = MaterialTheme.colorScheme.surfaceContainerLowest) { inner() }
}

@Composable
private fun PreviewFab() {
    if (LocalDensity.current.fontScale > STACK_FROM_FONT_SCALE) {
        FloatingActionButton(onClick = {}) { Icon(Icons.Filled.Edit, contentDescription = stringResource(R.string.preview_compose)) }
    } else {
        ExtendedFloatingActionButton(onClick = {}) {
            Icon(Icons.Filled.Edit, contentDescription = null)
            Text(stringResource(R.string.preview_compose), Modifier.padding(start = 8.dp))
        }
    }
}

/**
 * The live preview (docs/ux/theme.md): search pill, two rows, chip, FAB and snackbar built from the real components, at
 * preview size, not interactive. It follows the theme and the layout settings around it, so every change shows in it
 * within a frame. Its size depends only on layout settings, never on colours, so a colour change moves nothing.
 */
@Composable
fun AppearancePreview(modifier: Modifier = Modifier, showSnackbar: Boolean = true) {
    NonInteractive(modifier.testTag(PREVIEW_TAG), stringResource(R.string.preview_description)) {
        Surface(
            Modifier.fillMaxWidth(),
            shape = MaterialTheme.shapes.extraLarge,
            color = MaterialTheme.colorScheme.background,
            border = BorderStroke(1.dp, MaterialTheme.colorScheme.outlineVariant),
        ) {
            Column(Modifier.padding(12.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                PreviewSearchPill()
                PreviewChips()
                // The FAB floats over the end of the list, as it does in the app.
                Box(Modifier.fillMaxWidth()) {
                    PreviewList(FakeRows.take(2))
                    Box(Modifier.align(Alignment.BottomEnd).padding(8.dp)) { PreviewFab() }
                }
                if (showSnackbar) {
                    Snackbar(action = { TextButton(onClick = {}) { Text(stringResource(R.string.preview_snackbar_action)) } }) {
                        Text(stringResource(R.string.preview_snackbar))
                    }
                }
            }
        }
    }
}

/**
 * An inbox in the shape of S10 (search pill, category chips, list in a container, extended FAB, bottom bar) on fake data.
 * The real S10 is T13; this is the sample the theme screenshot tests draw for every palette.
 */
@Composable
fun InboxSample(modifier: Modifier = Modifier) {
    NonInteractive(modifier.testTag(INBOX_SAMPLE_TAG), stringResource(R.string.preview_description)) {
        Surface(Modifier.fillMaxWidth(), color = MaterialTheme.colorScheme.background) {
            Column {
                Column(Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
                    PreviewSearchPill()
                    PreviewChips()
                    PreviewList(FakeRows)
                    Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.End) { PreviewFab() }
                }
                NavigationBar {
                    NavigationBarItem(selected = true, onClick = {}, icon = { Icon(Icons.Filled.Email, contentDescription = null) }, label = { Text(stringResource(R.string.preview_nav_inbox)) })
                    NavigationBarItem(selected = false, onClick = {}, icon = { Icon(Icons.Filled.DateRange, contentDescription = null) }, label = { Text(stringResource(R.string.preview_nav_calendar)) })
                    NavigationBarItem(selected = false, onClick = {}, icon = { Icon(Icons.Filled.Person, contentDescription = null) }, label = { Text(stringResource(R.string.preview_nav_contacts)) })
                }
            }
        }
    }
}
