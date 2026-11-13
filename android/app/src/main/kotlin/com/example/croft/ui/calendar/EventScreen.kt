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
import androidx.compose.foundation.layout.navigationBars
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.statusBars
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.filled.DateRange
import androidx.compose.material.icons.filled.Delete
import androidx.compose.material.icons.filled.Edit
import androidx.compose.material.icons.filled.LocationOn
import androidx.compose.material.icons.filled.Notifications
import androidx.compose.material.icons.filled.Person
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.platform.LocalConfiguration
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import com.example.croft.R
import com.example.croft.mail.calendar.CalendarBackend
import com.example.croft.mail.calendar.CalendarRef
import com.example.croft.mail.calendar.EventData
import com.example.croft.mail.calendar.EventPerson
import com.example.croft.mail.calendar.EventScopeChoice
import com.example.croft.ui.components.CroftIconButton
import com.example.croft.ui.components.CroftSnackbarHost
import com.example.croft.ui.components.CroftSnackbarState
import com.example.croft.ui.components.DestructiveDialog
import java.time.Instant
import java.time.ZoneId
import java.time.format.DateTimeFormatter
import java.time.format.FormatStyle
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.launch

/** Test tags of S37. */
object EventTags {
    const val SCREEN = "event-screen"
    const val TITLE = "event-title"
    const val WHEN = "event-when"
    const val EDIT = "event-edit"
    const val DELETE = "event-delete"
    const val GONE = "event-gone"
    const val CONFLICT = "event-conflict"
    const val PENDING = "event-pending"
    const val SCOPE_ONE = "event-scope-one"
    const val SCOPE_ALL = "event-scope-all"
}

/** The words for a reminder [minutes] before the start. */
@Composable
fun reminderText(minutes: Long): String = when {
    minutes <= 0L -> stringResource(R.string.event_reminder_at_start)
    minutes == 60L -> stringResource(R.string.event_reminder_hour)
    minutes == 1440L -> stringResource(R.string.event_reminder_day)
    minutes % 1440L == 0L -> stringResource(R.string.event_reminder_days, (minutes / 1440L).toInt())
    minutes % 60L == 0L -> stringResource(R.string.event_reminder_hours, (minutes / 60L).toInt())
    else -> stringResource(R.string.event_reminder_minutes, minutes.toInt())
}

/** The date and time line of S37: "Fri 6 Nov, 12:00 to 13:00", or the date for a whole day. */
@Composable
fun whenText(e: EventData, zone: ZoneId): String {
    val locale = LocalConfiguration.current.locales[0]
    val start = Instant.ofEpochMilli(e.startMs).atZone(zone)
    val end = Instant.ofEpochMilli(e.endMs).atZone(zone)
    val dateF = DateTimeFormatter.ofLocalizedDate(FormatStyle.MEDIUM).withLocale(locale)
    if (e.allDay) {
        val last = end.minusDays(1).toLocalDate()
        return if (last.isAfter(start.toLocalDate())) stringResource(R.string.calendar_time_range, start.format(dateF), last.format(dateF)) else start.format(dateF)
    }
    val timeF = DateTimeFormatter.ofLocalizedTime(FormatStyle.SHORT).withLocale(locale)
    val endText = if (end.toLocalDate() == start.toLocalDate()) end.format(timeF) else end.format(dateF) + ", " + end.format(timeF)
    return start.format(dateF) + ", " + stringResource(R.string.calendar_time_range, start.format(timeF), endText)
}

/**
 * S37 Event: the header in the calendar's colour with the title and when, then the place, the organiser and the attendees with their
 * answers, the reminders, the calendar and the notes. Edit and Delete in the bar: for a repeating event both ask "This event / All
 * events"; Delete always asks first. RSVP is T40.
 */
@Composable
fun EventScreen(
    backend: CalendarBackend,
    account: Long,
    id: Long,
    keyMs: Long?,
    calendars: List<CalendarRef>,
    onBack: () -> Unit,
    onEdit: (EventData, EventScopeChoice) -> Unit,
    onDeleted: (account: Long) -> Unit,
    modifier: Modifier = Modifier,
    zone: ZoneId = ZoneId.systemDefault(),
) {
    var event by remember(account, id, keyMs) { mutableStateOf<EventData?>(null) }
    var gone by remember(account, id, keyMs) { mutableStateOf(false) }
    // What the scope question is for: editing or deleting.
    var askScope by remember { mutableStateOf<Boolean?>(null) }
    var confirmDelete by remember { mutableStateOf<EventScopeChoice?>(null) }
    val snackbar = remember { CroftSnackbarState() }
    val scope = rememberCoroutineScope()
    val failed = stringResource(R.string.event_delete_failed)
    BackHandler { onBack() }
    LaunchedEffect(account, id, keyMs) {
        val e = try { backend.get(account, id, keyMs, zone.id) } catch (e: CancellationException) { throw e } catch (e: Exception) { null }
        event = e
        gone = e == null
    }
    val e = event
    val cal = e?.let { ev -> calendars.firstOrNull { it.id == ev.calendar } }
    val accent = calendarColor(cal?.color, MaterialTheme.colorScheme.primaryContainer)
    Box(modifier.fillMaxSize().background(MaterialTheme.colorScheme.background).testTag(EventTags.SCREEN)) {
        Column(Modifier.fillMaxSize().windowInsetsPadding(WindowInsets.navigationBars)) {
            Column(Modifier.fillMaxWidth().background(accent.copy(alpha = HEADER_ALPHA)).windowInsetsPadding(WindowInsets.statusBars)) {
                Row(Modifier.fillMaxWidth().padding(horizontal = 4.dp), verticalAlignment = Alignment.CenterVertically) {
                    CroftIconButton(Icons.AutoMirrored.Filled.ArrowBack, stringResource(R.string.event_back), onBack)
                    Box(Modifier.weight(1f))
                    if (e != null) {
                        CroftIconButton(Icons.Filled.Edit, stringResource(R.string.event_edit), { if (e.repeats) askScope = true else onEdit(e, EventScopeChoice.All) }, Modifier.testTag(EventTags.EDIT))
                        CroftIconButton(Icons.Filled.Delete, stringResource(R.string.event_delete), { if (e.repeats) askScope = false else confirmDelete = EventScopeChoice.All }, Modifier.testTag(EventTags.DELETE))
                    }
                }
                if (e != null) {
                    Column(Modifier.padding(start = 20.dp, end = 20.dp, bottom = 20.dp), verticalArrangement = Arrangement.spacedBy(4.dp)) {
                        Text(e.title.ifBlank { stringResource(R.string.calendar_untitled) }, Modifier.testTag(EventTags.TITLE).semantics { heading() }, style = MaterialTheme.typography.headlineSmall)
                        Text(whenText(e, zone), Modifier.testTag(EventTags.WHEN), style = MaterialTheme.typography.bodyLarge)
                        when (e.status) {
                            "TENTATIVE" -> Text(stringResource(R.string.event_status_tentative), style = MaterialTheme.typography.labelLarge)
                            "CANCELLED" -> Text(stringResource(R.string.event_status_cancelled), style = MaterialTheme.typography.labelLarge, color = MaterialTheme.colorScheme.error)
                        }
                    }
                }
            }
            when {
                gone -> Text(stringResource(R.string.event_gone), Modifier.padding(24.dp).testTag(EventTags.GONE), style = MaterialTheme.typography.bodyLarge)
                e != null -> Column(
                    Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(horizontal = 16.dp, vertical = 8.dp),
                    verticalArrangement = Arrangement.spacedBy(12.dp),
                ) {
                    if (e.conflict) Text(stringResource(R.string.event_conflict), Modifier.testTag(EventTags.CONFLICT), style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
                    if (e.pending) Text(stringResource(R.string.event_pending), Modifier.testTag(EventTags.PENDING), style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
                    if (e.zoneUnknown) Text(stringResource(R.string.calendar_zone_unknown), style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
                    if (e.location.isNotBlank()) Detail(Icons.Filled.LocationOn, e.location)
                    e.organizer?.let { o ->
                        Detail(Icons.Filled.Person, stringResource(R.string.event_organizer, o.name ?: o.address), e.attendees.map { personText(it) }.joinToString(", ").ifBlank { null })
                    } ?: run {
                        if (e.attendees.isNotEmpty()) Detail(Icons.Filled.Person, e.attendees.map { personText(it) }.joinToString(", "))
                    }
                    Detail(Icons.Filled.Notifications, if (e.reminders.isEmpty()) stringResource(R.string.event_reminder_none) else e.reminders.map { reminderText(it) }.joinToString(", "))
                    cal?.let { Detail(Icons.Filled.DateRange, it.name) }
                    if (e.notes.isNotBlank()) Text(e.notes, Modifier.widthIn(max = 640.dp), style = MaterialTheme.typography.bodyLarge)
                }
            }
        }
        CroftSnackbarHost(snackbar, Modifier.align(Alignment.BottomCenter).windowInsetsPadding(WindowInsets.navigationBars))
    }
    val ev = event
    when (val forEdit = askScope) {
        null -> {}
        else -> if (ev != null) {
            AlertDialog(
                onDismissRequest = { askScope = null },
                title = { Text(stringResource(R.string.event_scope_title)) },
                text = {
                    Column {
                        TextButton({ askScope = null; val s = EventScopeChoice.One(ev.keyMs ?: ev.startMs); if (forEdit) onEdit(ev, s) else confirmDelete = s }, Modifier.testTag(EventTags.SCOPE_ONE)) { Text(stringResource(R.string.event_scope_one)) }
                        TextButton({ askScope = null; if (forEdit) onEdit(ev, EventScopeChoice.All) else confirmDelete = EventScopeChoice.All }, Modifier.testTag(EventTags.SCOPE_ALL)) { Text(stringResource(R.string.event_scope_all)) }
                    }
                },
                confirmButton = {},
                dismissButton = { TextButton({ askScope = null }) { Text(stringResource(R.string.control_cancel)) } },
            )
        }
    }
    val del = confirmDelete
    if (del != null && ev != null) {
        DestructiveDialog(
            title = stringResource(R.string.event_delete_title),
            message = stringResource(R.string.event_delete_message),
            confirmLabel = stringResource(R.string.event_delete),
            onConfirm = {
                confirmDelete = null
                scope.launch {
                    val done = try { backend.delete(ev.account, ev.id, del, zone.id) } catch (e: CancellationException) { throw e } catch (e: Exception) { false }
                    if (done) onDeleted(ev.account) else snackbar.show(failed)
                }
            },
            onDismiss = { confirmDelete = null },
        )
    }
}

@Composable
private fun personText(p: EventPerson): String {
    val name = p.name ?: p.address
    val answer = when (p.status) {
        "ACCEPTED" -> stringResource(R.string.event_answer_accepted)
        "DECLINED" -> stringResource(R.string.event_answer_declined)
        "TENTATIVE" -> stringResource(R.string.event_answer_tentative)
        "NEEDS-ACTION" -> stringResource(R.string.event_answer_pending)
        else -> null
    }
    return if (answer == null) name else stringResource(R.string.event_person_answer, name, answer)
}

@Composable
private fun Detail(icon: ImageVector, text: String, secondary: String? = null) {
    Row(Modifier.fillMaxWidth().widthIn(max = 640.dp), horizontalArrangement = Arrangement.spacedBy(16.dp)) {
        Icon(icon, contentDescription = null, tint = MaterialTheme.colorScheme.onSurfaceVariant)
        Column(Modifier.weight(1f)) {
            Text(text, style = MaterialTheme.typography.bodyLarge)
            if (secondary != null) Text(secondary, style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
        }
    }
}

private const val HEADER_ALPHA = 0.25f
