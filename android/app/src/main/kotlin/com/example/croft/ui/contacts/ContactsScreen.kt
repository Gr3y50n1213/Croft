// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.ui.contacts

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBars
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Add
import androidx.compose.material.icons.filled.Close
import androidx.compose.material.icons.filled.Search
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.pulltorefresh.pullToRefresh
import androidx.compose.material3.pulltorefresh.rememberPullToRefreshState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import com.example.croft.R
import com.example.croft.haptics.LocalHaptics
import com.example.croft.mail.contacts.ContactRow
import com.example.croft.mail.contacts.ContactSections
import com.example.croft.mail.contacts.ContactsController
import com.example.croft.mail.contacts.ContactsItem
import com.example.croft.mail.contacts.ContactsSync
import com.example.croft.theme.LocalMotionLevel
import com.example.croft.theme.MotionLevel
import com.example.croft.ui.components.CroftExtendedFab
import com.example.croft.ui.components.CroftFilterChip
import com.example.croft.ui.components.CroftTextField
import com.example.croft.ui.components.FabState
import com.example.croft.ui.mail.MailBottomBar
import com.example.croft.ui.mail.MailDestination
import com.example.croft.ui.mail.PULL_ARM_DP
import com.example.croft.ui.mail.PullIndicator
import com.example.croft.ui.mail.PullRefreshHaptics
import kotlinx.coroutines.launch

/** Test tags of S40. */
object ContactsTags {
    const val SCREEN = "contacts-screen"
    const val SEARCH = "contacts-search"
    const val LIST = "contacts-list"
    const val ADD = "contacts-add"
    const val EMPTY = "contacts-empty"
    const val PROBLEMS = "contacts-problems"
    fun chip(account: Long?) = "contacts-chip-${account ?: "all"}"
    fun header(letter: String) = "contacts-header-$letter"
    fun row(account: Long, id: Long) = "contacts-row-$account-$id"
}

/**
 * S40 Contacts: the search, the account chips (with more than one account that has an address book), the list A to Z with its
 * headings and the index on the trailing edge, the Add button and the bottom bar with Contacts selected. Pull down syncs again.
 */
@Composable
fun ContactsScreen(
    controller: ContactsController,
    onOpen: (ContactRow) -> Unit,
    onAdd: (() -> Unit)?,
    onMail: () -> Unit,
    modifier: Modifier = Modifier,
    /** Calendar (S34) from the bottom bar; null leaves it disabled. */
    onCalendar: (() -> Unit)? = null,
) {
    val state by controller.state.collectAsState()
    val scope = rememberCoroutineScope()
    val haptics = LocalHaptics.current
    val level = LocalMotionLevel.current
    val listState = rememberLazyListState()
    val listItems = state.items
    Column(modifier.fillMaxSize().background(MaterialTheme.colorScheme.background).testTag(ContactsTags.SCREEN)) {
        Column(Modifier.windowInsetsPadding(WindowInsets.statusBars).padding(horizontal = 12.dp, vertical = 8.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
            Text(stringResource(R.string.contacts_title), Modifier.padding(horizontal = 4.dp).semantics { heading() }, style = MaterialTheme.typography.titleLarge)
            CroftTextField(
                value = state.query,
                onValueChange = controller::setQuery,
                label = stringResource(R.string.contacts_search_hint),
                modifier = Modifier.fillMaxWidth().testTag(ContactsTags.SEARCH),
                leadingIcon = Icons.Filled.Search,
                trailingIcon = if (state.query.isNotEmpty()) Icons.Filled.Close else null,
                trailingIconDescription = stringResource(R.string.contacts_search_clear),
                onTrailingClick = { controller.setQuery("") },
            )
            if (state.accounts.size > 1) {
                Row(Modifier.horizontalScroll(rememberScrollState()), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    CroftFilterChip(stringResource(R.string.contacts_filter_all), state.filter == null, { controller.setFilter(null) }, Modifier.testTag(ContactsTags.chip(null)))
                    for (a in state.accounts) CroftFilterChip(a.label, state.filter == a.id, { controller.setFilter(a.id) }, Modifier.testTag(ContactsTags.chip(a.id)))
                }
            }
            val problems = state.problems.mapNotNull { (id, p) ->
                val label = state.accounts.firstOrNull { it.id == id }?.label ?: return@mapNotNull null
                when (p) {
                    ContactsSync.UNREACHABLE -> stringResource(R.string.contacts_unreachable, label)
                    ContactsSync.SIGN_IN_NEEDED -> stringResource(R.string.contacts_sign_in, label)
                    else -> null
                }
            }
            if (problems.isNotEmpty()) {
                Column(Modifier.testTag(ContactsTags.PROBLEMS).semantics { liveRegion = LiveRegionMode.Polite }) {
                    for (p in problems) Text(p, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.error)
                }
            }
        }
        val pullState = rememberPullToRefreshState()
        PullRefreshHaptics(pullState, state.refreshing, haptics)
        Box(
            Modifier
                .weight(1f)
                .fillMaxWidth()
                .pullToRefresh(isRefreshing = state.refreshing, state = pullState, threshold = PULL_ARM_DP, onRefresh = controller::refresh),
        ) {
            when {
                !state.loaded -> Text(stringResource(R.string.contacts_loading), Modifier.padding(24.dp), style = MaterialTheme.typography.bodyMedium)
                state.rows.isEmpty() -> EmptyContacts(
                    when {
                        state.query.isNotBlank() -> EmptyKind.NO_MATCH
                        state.syncedOnce && state.accounts.isEmpty() -> EmptyKind.NOT_OFFERED
                        else -> EmptyKind.NONE_YET
                    },
                )
                else -> LazyColumn(Modifier.fillMaxSize().padding(start = 12.dp, end = 28.dp).testTag(ContactsTags.LIST), state = listState) {
                    items(listItems, key = { if (it is ContactsItem.Header) "h-${it.letter}" else (it as ContactsItem.Row).row.let { r -> "r-${r.account}-${r.id}" } }) { item ->
                        when (item) {
                            is ContactsItem.Header -> Text(
                                item.letter,
                                Modifier.padding(start = 8.dp, top = 12.dp, bottom = 4.dp).testTag(ContactsTags.header(item.letter)).semantics { heading() },
                                style = MaterialTheme.typography.titleSmall,
                                color = MaterialTheme.colorScheme.primary,
                            )
                            is ContactsItem.Row -> ContactRowView(item.row, onClick = { onOpen(item.row) })
                        }
                    }
                }
            }
            if (state.rows.isNotEmpty()) {
                AlphabetIndex(
                    letters = ContactSections.INDEX,
                    present = state.letters,
                    onLetter = { letter ->
                        val target = nearestSection(letter, ContactSections.INDEX, state.letters) ?: return@AlphabetIndex
                        val index = listItems.indexOfFirst { it is ContactsItem.Header && it.letter == target }
                        if (index >= 0) scope.launch { listState.scrollToItem(index) }
                    },
                    modifier = Modifier.align(Alignment.CenterEnd).padding(vertical = 8.dp, horizontal = 2.dp),
                    haptics = haptics,
                )
            }
            PullIndicator(pullState, state.refreshing, Modifier.align(Alignment.TopCenter))
            if (onAdd != null) {
                Box(Modifier.align(Alignment.BottomEnd).padding(end = 36.dp, bottom = 16.dp)) {
                    CroftExtendedFab(
                        stringResource(R.string.contacts_add),
                        onAdd,
                        if (listState.canScrollBackward && level != MotionLevel.OFF) FabState.COLLAPSED else FabState.EXPANDED,
                        Modifier.testTag(ContactsTags.ADD),
                        icon = Icons.Filled.Add,
                    )
                }
            }
        }
        MailBottomBar(enabled = true, onMail = onMail, selected = MailDestination.CONTACTS, onContacts = { scope.launch { listState.scrollToItem(0) } }, onCalendar = onCalendar)
    }
}

private enum class EmptyKind { NONE_YET, NO_MATCH, NOT_OFFERED }

@Composable
private fun EmptyContacts(kind: EmptyKind) {
    Column(Modifier.padding(24.dp).widthIn(max = 480.dp).testTag(ContactsTags.EMPTY), verticalArrangement = Arrangement.spacedBy(8.dp)) {
        val (title, message) = when (kind) {
            EmptyKind.NONE_YET -> R.string.contacts_empty_title to R.string.contacts_empty_message
            EmptyKind.NO_MATCH -> R.string.contacts_no_match to null
            EmptyKind.NOT_OFFERED -> R.string.contacts_not_offered_title to R.string.contacts_not_offered_message
        }
        Text(stringResource(title), style = MaterialTheme.typography.titleMedium, modifier = Modifier.semantics { heading() })
        if (message != null) Text(stringResource(message), style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
    }
}

/** One contact: the letter avatar, the name and the first address. */
@Composable
private fun ContactRowView(row: ContactRow, onClick: () -> Unit) {
    Row(
        Modifier
            .fillMaxWidth()
            .heightIn(min = 56.dp)
            .clip(MaterialTheme.shapes.medium)
            .clickable(onClick = onClick)
            .padding(horizontal = 8.dp, vertical = 6.dp)
            .testTag(ContactsTags.row(row.account, row.id)),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(16.dp),
    ) {
        LetterAvatar(row.name, AVATAR)
        Column(Modifier.weight(1f)) {
            Text(row.name, style = MaterialTheme.typography.bodyLarge, maxLines = 1, overflow = TextOverflow.Ellipsis)
            if (!row.email.isNullOrBlank() && row.email != row.name) {
                Text(row.email, style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant, maxLines = 1, overflow = TextOverflow.Ellipsis)
            }
        }
    }
}

/** The first letter of [name] in a circle (hidden from screen readers: the name is read next to it). */
@Composable
fun LetterAvatar(name: String, size: Dp, modifier: Modifier = Modifier) {
    val letter = name.trim().firstOrNull()?.uppercaseChar()?.toString() ?: "?"
    Box(
        modifier.size(size).clip(CircleShape).background(MaterialTheme.colorScheme.secondaryContainer).clearAndSetSemantics { },
        contentAlignment = Alignment.Center,
    ) {
        Text(
            letter,
            style = if (size > AVATAR) MaterialTheme.typography.displaySmall else MaterialTheme.typography.titleMedium,
            color = MaterialTheme.colorScheme.onSecondaryContainer,
            maxLines = 1,
        )
    }
}

private val AVATAR = 40.dp
