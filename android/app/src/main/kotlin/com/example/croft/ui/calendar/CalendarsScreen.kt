// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.ui.calendar

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.navigationBars
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBars
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.selection.toggleable
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material3.Checkbox
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import androidx.compose.foundation.horizontalScroll
import androidx.compose.material.icons.filled.Add
import androidx.compose.material.icons.filled.Delete
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Icon
import androidx.compose.material3.TextButton
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.ui.text.input.KeyboardType
import com.example.croft.R
import com.example.croft.mail.calendar.CalendarController
import com.example.croft.mail.calendar.LinkProblem
import com.example.croft.ui.components.CroftIcons
import com.example.croft.ui.components.ButtonKind
import com.example.croft.ui.components.ButtonProgress
import com.example.croft.ui.components.CroftButton
import com.example.croft.ui.components.CroftFilterChip
import com.example.croft.ui.components.CroftIconButton
import com.example.croft.ui.components.CroftTextField
import com.example.croft.ui.components.DestructiveDialog
import kotlinx.coroutines.launch

/** Test tags of S39. */
object CalendarsTags {
    const val SCREEN = "calendars-screen"
    const val SUBSCRIBE = "calendars-subscribe"
    const val LINK_URL = "calendars-link-url"
    const val LINK_NAME = "calendars-link-name"
    const val LINK_ADD = "calendars-link-add"
    const val LINK_ERROR = "calendars-link-error"
    fun calendar(id: Long) = "calendars-calendar-$id"
    fun remove(id: Long) = "calendars-remove-$id"
}

/**
 * S39 Calendars: grouped by account, each with its colour and a check for whether its events show (a hidden one stays in sync).
 * A calendar from a link has a link icon and Remove (which asks first). "Subscribe to a calendar link" (T39c) adds a read-only one.
 */
@Composable
fun CalendarsScreen(controller: CalendarController, onBack: () -> Unit, modifier: Modifier = Modifier) {
    val state by controller.state.collectAsState()
    var subscribing by remember { mutableStateOf(false) }
    var removing by remember { mutableStateOf<com.example.croft.mail.calendar.CalendarRef?>(null) }
    BackHandler { onBack() }
    Column(
        modifier.fillMaxSize().background(MaterialTheme.colorScheme.background).testTag(CalendarsTags.SCREEN)
            .windowInsetsPadding(WindowInsets.statusBars).windowInsetsPadding(WindowInsets.navigationBars),
    ) {
        Row(Modifier.fillMaxWidth().padding(horizontal = 4.dp), verticalAlignment = Alignment.CenterVertically) {
            CroftIconButton(Icons.AutoMirrored.Filled.ArrowBack, stringResource(R.string.event_back), onBack)
            Text(stringResource(R.string.calendars_title), Modifier.padding(start = 8.dp).semantics { heading() }, style = MaterialTheme.typography.titleLarge)
        }
        Column(Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(horizontal = 16.dp, vertical = 8.dp), verticalArrangement = Arrangement.spacedBy(4.dp)) {
            for (a in state.accounts) {
                Text(a.label, Modifier.padding(top = 12.dp, bottom = 4.dp).semantics { heading() }, style = MaterialTheme.typography.titleSmall, color = MaterialTheme.colorScheme.primary)
                for (c in state.calendars[a.id].orEmpty()) {
                    val color = calendarColor(c.color, MaterialTheme.colorScheme.primary)
                    Row(
                        Modifier.fillMaxWidth().heightIn(min = 56.dp).clip(MaterialTheme.shapes.medium)
                            .toggleable(value = c.shown, role = Role.Checkbox, onValueChange = { controller.setShown(a.id, c.id, it) })
                            .padding(horizontal = 8.dp)
                            .testTag(CalendarsTags.calendar(c.id)),
                        verticalAlignment = Alignment.CenterVertically,
                        horizontalArrangement = Arrangement.spacedBy(16.dp),
                    ) {
                        Checkbox(checked = c.shown, onCheckedChange = null)
                        Text(c.name, Modifier.weight(1f), style = MaterialTheme.typography.bodyLarge)
                        if (c.linked) {
                            Icon(CroftIcons.Link, contentDescription = stringResource(R.string.calendars_linked), tint = MaterialTheme.colorScheme.onSurfaceVariant)
                            CroftIconButton(Icons.Filled.Delete, stringResource(R.string.calendars_remove, c.name), { removing = c }, Modifier.testTag(CalendarsTags.remove(c.id)))
                        }
                        Box(Modifier.size(16.dp).clip(CircleShape).background(color).clearAndSetSemantics { })
                    }
                }
            }
            if (state.knownAccounts.isNotEmpty()) {
                CroftButton(
                    stringResource(R.string.calendars_subscribe), { subscribing = true }, Modifier.padding(top = 12.dp).testTag(CalendarsTags.SUBSCRIBE),
                    kind = ButtonKind.TONAL, leadingIcon = Icons.Filled.Add,
                )
            }
            Text(stringResource(R.string.calendars_note), Modifier.padding(top = 16.dp), style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
        }
    }
    if (subscribing) SubscribeDialog(controller, onDone = { subscribing = false })
    removing?.let { c ->
        DestructiveDialog(
            title = stringResource(R.string.calendars_remove_title),
            message = stringResource(R.string.calendars_remove_message),
            confirmLabel = stringResource(R.string.calendars_remove_confirm),
            onConfirm = { removing = null; controller.unsubscribe(c.account, c.id) },
            onDismiss = { removing = null },
        )
    }
}

/** "Subscribe to a calendar link": the address, a name, and for which account; it says the calendar is read-only and fetched from time to time. */
@Composable
private fun SubscribeDialog(controller: CalendarController, onDone: () -> Unit) {
    val state by controller.state.collectAsState()
    val accounts = state.knownAccounts
    var url by remember { mutableStateOf("") }
    var name by remember { mutableStateOf("") }
    var account by remember { mutableStateOf(accounts.firstOrNull()?.id) }
    var busy by remember { mutableStateOf(false) }
    var problem by remember { mutableStateOf<LinkProblem?>(null) }
    val scope = rememberCoroutineScope()
    AlertDialog(
        onDismissRequest = { if (!busy) onDone() },
        title = { Text(stringResource(R.string.calendars_subscribe)) },
        text = {
            Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                Text(stringResource(R.string.calendars_subscribe_note), style = MaterialTheme.typography.bodyMedium)
                CroftTextField(url, { url = it; problem = null }, stringResource(R.string.calendars_link_address), Modifier.fillMaxWidth().testTag(CalendarsTags.LINK_URL), keyboardType = KeyboardType.Uri)
                CroftTextField(name, { name = it }, stringResource(R.string.calendars_link_name), Modifier.fillMaxWidth().testTag(CalendarsTags.LINK_NAME))
                if (accounts.size > 1) {
                    Row(Modifier.horizontalScroll(rememberScrollState()), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                        for (a in accounts) CroftFilterChip(a.label, account == a.id, { account = a.id })
                    }
                }
                problem?.let { p ->
                    Text(
                        stringResource(
                            when (p) {
                                LinkProblem.BAD_ADDRESS -> R.string.calendars_link_bad_address
                                LinkProblem.NOT_A_CALENDAR -> R.string.calendars_link_not_calendar
                                LinkProblem.UNREACHABLE -> R.string.calendars_link_unreachable
                                LinkProblem.REFUSED -> R.string.calendars_link_refused
                                LinkProblem.ALREADY_ADDED -> R.string.calendars_link_already
                                LinkProblem.TOO_MANY -> R.string.calendars_link_too_many
                            },
                        ),
                        Modifier.testTag(CalendarsTags.LINK_ERROR).semantics { liveRegion = LiveRegionMode.Polite },
                        color = MaterialTheme.colorScheme.error, style = MaterialTheme.typography.bodyMedium,
                    )
                }
            }
        },
        confirmButton = {
            CroftButton(
                stringResource(R.string.calendars_link_add),
                {
                    val a = account ?: return@CroftButton
                    busy = true
                    scope.launch {
                        val p = controller.subscribe(a, url.trim(), name.trim())
                        busy = false
                        if (p == null) onDone() else problem = p
                    }
                },
                Modifier.testTag(CalendarsTags.LINK_ADD),
                enabled = url.isNotBlank() && account != null && !busy,
                progress = if (busy) ButtonProgress.LOADING else ButtonProgress.IDLE,
                kind = ButtonKind.TEXT,
            )
        },
        dismissButton = { TextButton({ if (!busy) onDone() }) { Text(stringResource(R.string.control_cancel)) } },
    )
}
