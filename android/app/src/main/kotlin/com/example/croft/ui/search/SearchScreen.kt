// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.ui.search

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.LazyRow
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.filled.Close
import androidx.compose.material.icons.filled.Search
import androidx.compose.material.icons.filled.MoreVert
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextField
import androidx.compose.material3.TextFieldDefaults
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import com.example.croft.R
import com.example.croft.mail.AccountInfo
import com.example.croft.mail.search.FilterChip
import com.example.croft.mail.search.SearchController
import com.example.croft.mail.search.SearchHitData
import com.example.croft.mail.search.SearchPhase
import com.example.croft.mail.search.ServerNote
import com.example.croft.ui.components.ButtonKind
import com.example.croft.ui.components.CroftButton
import com.example.croft.ui.components.CroftFilterChip
import com.example.croft.ui.components.CroftIconButton
import com.example.croft.ui.components.CroftSnackbarHost
import com.example.croft.ui.components.CroftSnackbarState
import com.example.croft.ui.components.MessageRow
import com.example.croft.ui.components.MessageRowState
import java.time.Instant
import java.time.ZoneId
import java.time.format.DateTimeFormatter
import java.time.format.FormatStyle
import java.util.Locale

/** Test tags of S27 and S28. */
object SearchTags {
    const val SCREEN = "search-screen"
    const val FIELD = "search-field"
    const val HISTORY = "search-history"
    fun historyItem(q: String) = "search-history-$q"
    fun historyRemove(q: String) = "search-history-remove-$q"
    const val RESULTS = "search-results"
    fun result(id: Long) = "search-result-$id"
    const val NOTE = "search-note"
    const val EMPTY = "search-empty"
    const val SERVER_BUTTON = "search-server"
    const val SEARCHING = "search-searching"
    const val MENU = "search-menu"
    fun chip(c: FilterChip) = "search-chip-${c.name}"
    fun scope(account: Long?) = "search-scope-${account ?: "all"}"
}

/**
 * S27 Search (recent searches, filter chips) and S28 Search results (highlighted matches, active chips, scope, notes about the server).
 * The field types into [controller] (150 ms pause, then the device's results, then the servers'). Back: from results to S27 with the
 * query cleared, then [onBack] to the Inbox.
 */
@Composable
fun SearchScreen(
    controller: SearchController,
    accounts: List<AccountInfo>,
    onOpenMessage: (Long) -> Unit,
    onBack: () -> Unit,
    modifier: Modifier = Modifier,
    zone: ZoneId = ZoneId.systemDefault(),
    snackbar: CroftSnackbarState = remember { CroftSnackbarState() },
) {
    val state by controller.state.collectAsState()
    val focus = remember { FocusRequester() }
    LaunchedEffect(Unit) { runCatching { focus.requestFocus() } }
    BackHandler { if (state.query.isNotEmpty()) controller.clear() else onBack() }
    val removedText = stringResource(R.string.search_history_removed)
    val undo = stringResource(R.string.search_undo)
    Surface(modifier.fillMaxSize().testTag(SearchTags.SCREEN), color = MaterialTheme.colorScheme.surface) {
        Box(Modifier.fillMaxSize()) {
            Column(Modifier.fillMaxSize()) {
                Row(Modifier.fillMaxWidth().padding(horizontal = 4.dp, vertical = 8.dp), verticalAlignment = Alignment.CenterVertically) {
                    CroftIconButton(Icons.AutoMirrored.Filled.ArrowBack, stringResource(R.string.search_back), onClick = {
                        if (state.query.isNotEmpty()) controller.clear() else onBack()
                    })
                    TextField(
                        value = state.query,
                        onValueChange = controller::onQuery,
                        modifier = Modifier.weight(1f).focusRequester(focus).testTag(SearchTags.FIELD),
                        placeholder = { Text(stringResource(R.string.search_hint)) },
                        singleLine = true,
                        keyboardOptions = KeyboardOptions(imeAction = ImeAction.Search),
                        keyboardActions = KeyboardActions(onSearch = { controller.submit() }),
                        colors = TextFieldDefaults.colors(
                            focusedContainerColor = MaterialTheme.colorScheme.surfaceContainerHigh,
                            unfocusedContainerColor = MaterialTheme.colorScheme.surfaceContainerHigh,
                        ),
                        shape = MaterialTheme.shapes.extraLarge,
                    )
                    if (state.query.isNotEmpty()) {
                        CroftIconButton(Icons.Filled.Close, stringResource(R.string.search_clear), onClick = controller::clear)
                    }
                    var menu by remember { mutableStateOf(false) }
                    Box {
                        CroftIconButton(Icons.Filled.MoreVert, stringResource(R.string.search_menu), onClick = { menu = true }, modifier = Modifier.testTag(SearchTags.MENU))
                        DropdownMenu(expanded = menu, onDismissRequest = { menu = false }) {
                            DropdownMenuItem(
                                text = { Text(stringResource(R.string.search_clear_history)) },
                                onClick = { menu = false; controller.clearHistory() },
                                enabled = state.history.isNotEmpty(),
                            )
                        }
                    }
                }
                ChipsRow(
                    query = state.query,
                    onToggle = controller::toggle,
                    accounts = accounts,
                    account = state.account,
                    onAccount = controller::setAccount,
                )
                if (state.phase == SearchPhase.IDLE) {
                    History(
                        items = state.history,
                        onPick = { controller.submit(it) },
                        onRemove = { q ->
                            val at = controller.removeHistory(q)
                            if (at >= 0) snackbar.show(removedText, undo) { controller.undoRemove(q, at) }
                        },
                    )
                } else {
                    Results(
                        hits = state.hits,
                        phase = state.phase,
                        query = state.query,
                        bodiesPartial = state.bodiesPartial,
                        note = state.serverNote,
                        badDate = state.badDate,
                        onOpen = onOpenMessage,
                        onSearchServer = controller::searchServerAgain,
                        zone = zone,
                    )
                }
            }
            CroftSnackbarHost(snackbar, Modifier.align(Alignment.BottomCenter).padding(16.dp))
        }
    }
}

@Composable
private fun ChipsRow(query: String, onToggle: (FilterChip) -> Unit, accounts: List<AccountInfo>, account: Long?, onAccount: (Long?) -> Unit) {
    LazyRow(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(8.dp), contentPadding = androidx.compose.foundation.layout.PaddingValues(horizontal = 16.dp)) {
        items(FilterChip.entries.toList()) { chip ->
            val label = when (chip) {
                FilterChip.UNREAD -> stringResource(R.string.search_chip_unread)
                FilterChip.STARRED -> stringResource(R.string.search_chip_starred)
                FilterChip.ATTACHMENT -> stringResource(R.string.search_chip_attachment)
            }
            CroftFilterChip(label, selected = chip.isOn(query), onClick = { onToggle(chip) }, modifier = Modifier.testTag(SearchTags.chip(chip)))
        }
        if (accounts.size > 1) {
            item {
                CroftFilterChip(stringResource(R.string.search_scope_all), selected = account == null, onClick = { onAccount(null) }, modifier = Modifier.testTag(SearchTags.scope(null)))
            }
            items(accounts) { a ->
                CroftFilterChip(a.label, selected = account == a.id, onClick = { onAccount(a.id) }, modifier = Modifier.testTag(SearchTags.scope(a.id)))
            }
        }
    }
}

@Composable
private fun History(items: List<String>, onPick: (String) -> Unit, onRemove: (String) -> Unit) {
    Column(Modifier.fillMaxWidth().padding(top = 8.dp).testTag(SearchTags.HISTORY)) {
        if (items.isEmpty()) {
            Text(stringResource(R.string.search_no_history), Modifier.padding(16.dp), style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
            return
        }
        Text(stringResource(R.string.search_recent), Modifier.padding(horizontal = 16.dp, vertical = 8.dp), style = MaterialTheme.typography.titleSmall)
        LazyColumn {
            items(items, key = { it }) { q ->
                Row(
                    Modifier.fillMaxWidth().clickable { onPick(q) }.padding(start = 16.dp).testTag(SearchTags.historyItem(q)),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    Icon(Icons.Filled.Search, contentDescription = null, tint = MaterialTheme.colorScheme.onSurfaceVariant)
                    Text(q, Modifier.weight(1f).padding(horizontal = 16.dp), maxLines = 1, overflow = TextOverflow.Ellipsis)
                    CroftIconButton(
                        Icons.Filled.Close,
                        stringResource(R.string.search_history_remove, q),
                        onClick = { onRemove(q) },
                        modifier = Modifier.testTag(SearchTags.historyRemove(q)),
                    )
                }
            }
        }
    }
}

@Composable
private fun Results(
    hits: List<SearchHitData>,
    phase: SearchPhase,
    query: String,
    bodiesPartial: Boolean,
    note: ServerNote,
    badDate: Boolean,
    onOpen: (Long) -> Unit,
    onSearchServer: () -> Unit,
    zone: ZoneId,
) {
    val locale = Locale.getDefault()
    if (badDate) {
        Note(stringResource(R.string.search_bad_date))
        return
    }
    if (hits.isEmpty() && phase == SearchPhase.DONE) {
        Column(Modifier.fillMaxWidth().padding(24.dp).testTag(SearchTags.EMPTY), horizontalAlignment = Alignment.CenterHorizontally) {
            Text(stringResource(R.string.search_no_results, query), style = MaterialTheme.typography.titleMedium)
            NoteFor(note)
            if (note != ServerNote.NONE || bodiesPartial) {
                CroftButton(stringResource(R.string.search_server_button), onClick = onSearchServer, kind = ButtonKind.TONAL, modifier = Modifier.padding(top = 12.dp).testTag(SearchTags.SERVER_BUTTON))
            }
        }
        return
    }
    LazyColumn(Modifier.fillMaxSize().testTag(SearchTags.RESULTS)) {
        items(hits, key = { it.row.id }) { hit ->
            val r = hit.row
            MessageRow(
                sender = r.senderName ?: r.senderAddress ?: "",
                subject = r.subject,
                preview = hit.snippet ?: r.snippet,
                time = formatWhen(r.dateMs, zone, locale),
                state = MessageRowState(unread = !r.seen, starred = r.flagged, hasAttachment = r.hasAttachments, threadCount = r.threadCount, localOnly = r.localOnly),
                onClick = { onOpen(r.id) },
                modifier = Modifier.testTag(SearchTags.result(r.id)),
                subjectMatches = hit.subjectMatches,
                previewMatches = if (hit.snippet != null) hit.snippetMatches else emptyList(),
                swipeEnabled = false,
            )
        }
        item {
            Column(Modifier.fillMaxWidth().padding(16.dp)) {
                if (phase == SearchPhase.SEARCHING_SERVER) {
                    Text(stringResource(R.string.search_searching_server), Modifier.testTag(SearchTags.SEARCHING).semantics { liveRegion = LiveRegionMode.Polite }, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
                } else {
                    if (bodiesPartial && note == ServerNote.NONE) Note(stringResource(R.string.search_partial))
                    NoteFor(note)
                    if (note == ServerNote.FAILED) {
                        CroftButton(stringResource(R.string.search_retry), onClick = onSearchServer, kind = ButtonKind.TEXT, modifier = Modifier.testTag(SearchTags.SERVER_BUTTON))
                    }
                }
            }
        }
    }
}

@Composable
private fun NoteFor(note: ServerNote) {
    when (note) {
        ServerNote.NONE -> Unit
        ServerNote.DEVICE_ONLY -> Note(stringResource(R.string.search_device_only))
        ServerNote.OFFLINE -> Note(stringResource(R.string.search_offline))
        ServerNote.SIGN_IN -> Note(stringResource(R.string.search_sign_in))
        ServerNote.FAILED -> Note(stringResource(R.string.search_failed))
    }
}

/** A note that screen readers announce when it appears. */
@Composable
private fun Note(text: String) {
    Text(
        text,
        Modifier.padding(vertical = 4.dp).testTag(SearchTags.NOTE).semantics { liveRegion = LiveRegionMode.Polite },
        style = MaterialTheme.typography.bodySmall,
        color = MaterialTheme.colorScheme.onSurfaceVariant,
    )
}

/** Today: the time; another day: the date. */
private fun formatWhen(ms: Long, zone: ZoneId, locale: Locale): String {
    val at = Instant.ofEpochMilli(ms).atZone(zone)
    val today = Instant.now().atZone(zone).toLocalDate()
    return if (at.toLocalDate() == today) DateTimeFormatter.ofLocalizedTime(FormatStyle.SHORT).withLocale(locale).format(at)
    else DateTimeFormatter.ofLocalizedDate(FormatStyle.MEDIUM).withLocale(locale).format(at)
}
