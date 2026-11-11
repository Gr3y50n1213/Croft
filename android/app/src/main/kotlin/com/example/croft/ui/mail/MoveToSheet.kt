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
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Search
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import com.example.croft.R
import com.example.croft.mail.FolderId
import com.example.croft.mail.FolderInfo
import com.example.croft.ui.components.CroftBottomSheet
import com.example.croft.ui.components.CroftTextField

/** Test tags of S30 Move to. */
object MoveTags {
    const val SHEET = "S30"
    const val SEARCH = "S30-search"
    const val RECENTS = "S30-recents"
    const val DECLINE = "S30-decline"

    /** The tag of the entry of folder [id]. */
    fun folder(id: FolderId) = "S30-folder-$id"
}

/** One entry of the folder tree: the folder and how deep it sits (the number of `/` in its path). */
data class MoveEntry(val folder: FolderInfo, val depth: Int)

/**
 * The rules of the sheet as pure functions, so they can be tested without a screen: which folders can be chosen, the recent ones,
 * the tree, and the search.
 */
object MoveRules {
    /**
     * The folders a message can be moved to: the real folders of its account (never a virtual mailbox such as Starred), except
     * the ones it is in already. Ordered by path, so a parent comes before its children.
     */
    fun targets(folders: List<FolderInfo>, excluding: Set<FolderId>): List<FolderInfo> =
        folders.filter { !it.isVirtual && it.id !in excluding }.sortedBy { it.path.lowercase() }

    /** The tree: every target with its depth. */
    fun tree(targets: List<FolderInfo>): List<MoveEntry> = targets.map { MoveEntry(it, it.path.count { c -> c == '/' }) }

    /** The recent destinations that can still be chosen, newest first, at most three. */
    fun recents(targets: List<FolderInfo>, recent: List<FolderId>): List<FolderInfo> =
        recent.mapNotNull { id -> targets.firstOrNull { it.id == id } }.take(MAX_RECENTS_SHOWN)

    /** The targets whose path contains [query] (case-insensitive), as a flat list; everything for a blank query. */
    fun search(targets: List<FolderInfo>, query: String): List<FolderInfo> {
        val q = query.trim()
        return if (q.isEmpty()) targets else targets.filter { it.path.contains(q, ignoreCase = true) }
    }

    /** Recent destinations shown above the tree. */
    const val MAX_RECENTS_SHOWN = 3
}

/**
 * S30 Move to: a bottom sheet with a search field, the recent destinations, and the folder tree of the account. A tap on a folder
 * [onPick]s it. There is no "create folder": the core cannot make folders on a server yet, and docs/ux/screens.md says the entry is
 * left out where the server cannot do it. [footer] adds a last button (the "don't offer Archive" choice of the folder prompt).
 * Back, a tap outside and the drag handle dismiss it ([onDismiss]).
 */
@Composable
fun MoveToSheet(
    title: String,
    folders: List<FolderInfo>,
    recent: List<FolderId>,
    excluding: Set<FolderId>,
    onPick: (FolderInfo) -> Unit,
    onDismiss: () -> Unit,
    modifier: Modifier = Modifier,
    footer: (@Composable () -> Unit)? = null,
) {
    var query by rememberSaveable { mutableStateOf("") }
    val targets = remember(folders, excluding) { MoveRules.targets(folders, excluding) }
    val recents = remember(targets, recent) { MoveRules.recents(targets, recent) }
    val searching = query.isNotBlank()
    val shown = remember(targets, query) { if (searching) MoveRules.search(targets, query).map { MoveEntry(it, 0) } else MoveRules.tree(targets) }
    CroftBottomSheet(onDismiss = onDismiss, modifier = modifier.testTag(MoveTags.SHEET), hasUnsavedInput = false) {
        Text(title, style = MaterialTheme.typography.titleLarge, modifier = Modifier.padding(bottom = 8.dp).semantics { heading() })
        CroftTextField(
            value = query,
            onValueChange = { query = it },
            label = stringResource(R.string.move_search),
            leadingIcon = Icons.Filled.Search,
            modifier = Modifier.testTag(MoveTags.SEARCH),
        )
        LazyColumn(Modifier.fillMaxWidth().heightIn(max = SHEET_LIST_MAX_HEIGHT).padding(top = 8.dp)) {
            if (!searching && recents.isNotEmpty()) {
                item(key = "recent-heading") {
                    Text(stringResource(R.string.move_recent), style = MaterialTheme.typography.labelLarge, color = MaterialTheme.colorScheme.primary, modifier = Modifier.padding(vertical = 8.dp).semantics { heading() })
                }
                items(recents, key = { "recent-${it.id}" }) { f -> FolderEntry(f, depth = 0, onPick = onPick, modifier = Modifier.testTag(MoveTags.RECENTS)) }
                item(key = "all-heading") {
                    Text(stringResource(R.string.move_all_folders), style = MaterialTheme.typography.labelLarge, color = MaterialTheme.colorScheme.primary, modifier = Modifier.padding(vertical = 8.dp).semantics { heading() })
                }
            }
            if (shown.isEmpty()) {
                item(key = "none") { Text(stringResource(R.string.move_none), modifier = Modifier.padding(vertical = 16.dp), style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant) }
            }
            items(shown, key = { "folder-${it.folder.id}" }) { e -> FolderEntry(e.folder, e.depth, onPick) }
        }
        footer?.invoke()
    }
}

private val SHEET_LIST_MAX_HEIGHT = 420.dp

@Composable
private fun FolderEntry(folder: FolderInfo, depth: Int, onPick: (FolderInfo) -> Unit, modifier: Modifier = Modifier) {
    Row(
        modifier.fillMaxWidth().heightIn(min = 48.dp).testTag(MoveTags.folder(folder.id))
            .clickable(role = Role.Button) { onPick(folder) }
            .padding(start = (depth * 16).dp, top = 4.dp, bottom = 4.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(16.dp),
    ) {
        Icon(folder.role.icon(), contentDescription = null, tint = MaterialTheme.colorScheme.onSurfaceVariant)
        Text(folder.displayName(), Modifier.weight(1f), style = MaterialTheme.typography.bodyLarge, maxLines = 2)
    }
}

/** The button under the sheet of the Archive prompt: "Don't offer Archive for this account". */
@Composable
fun DeclineFolderButton(label: String, onClick: () -> Unit, modifier: Modifier = Modifier) {
    TextButton(onClick = onClick, modifier = modifier.fillMaxWidth().heightIn(min = 48.dp).testTag(MoveTags.DECLINE)) { Text(label) }
}
