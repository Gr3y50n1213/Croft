// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.ui.calendar

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.navigationBars
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.statusBars
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.ArrowDropDown
import androidx.compose.material.icons.filled.Close
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.DatePicker
import androidx.compose.material3.DatePickerDialog
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TimePicker
import androidx.compose.material3.rememberDatePickerState
import androidx.compose.material3.rememberTimePickerState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.saveable.Saver
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalConfiguration
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import com.example.croft.R
import com.example.croft.mail.calendar.CalendarAccount
import com.example.croft.mail.calendar.CalendarBackend
import com.example.croft.mail.calendar.CalendarRef
import com.example.croft.mail.calendar.EditorState
import com.example.croft.mail.calendar.EventData
import com.example.croft.mail.calendar.EventEditor
import com.example.croft.mail.calendar.EventScopeChoice
import com.example.croft.mail.calendar.RepeatChoice
import com.example.croft.ui.components.ButtonKind
import com.example.croft.ui.components.ButtonProgress
import com.example.croft.ui.components.CroftButton
import com.example.croft.ui.components.CroftFilterChip
import com.example.croft.ui.components.CroftIconButton
import com.example.croft.ui.components.CroftMenu
import com.example.croft.ui.components.CroftSnackbarHost
import com.example.croft.ui.components.CroftSnackbarState
import com.example.croft.ui.components.CroftSwitchRow
import com.example.croft.ui.components.CroftTextField
import com.example.croft.ui.components.DestructiveDialog
import com.example.croft.ui.components.MenuItemSpec
import java.time.LocalDate
import java.time.LocalDateTime
import java.time.LocalTime
import java.time.ZoneId
import java.time.ZoneOffset
import java.time.format.DateTimeFormatter
import java.time.format.FormatStyle
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.launch

/** Test tags of S38. */
object EditorTags {
    const val SCREEN = "editor-screen"
    const val CLOSE = "editor-close"
    const val SAVE = "editor-save"
    const val TITLE = "editor-title"
    const val ALL_DAY = "editor-all-day"
    const val START_DATE = "editor-start-date"
    const val START_TIME = "editor-start-time"
    const val END_DATE = "editor-end-date"
    const val END_TIME = "editor-end-time"
    const val REPEAT = "editor-repeat"
    const val CALENDAR = "editor-calendar"
    const val LOCATION = "editor-location"
    const val NOTES = "editor-notes"
    fun reminder(minutes: Long) = "editor-reminder-$minutes"
    fun repeat(choice: RepeatChoice) = "editor-repeat-${choice.name.lowercase()}"
}

/**
 * S38 Event editor, for event [id] of [account] with [scope] (the first occurrence's times for every occurrence, that occurrence's for
 * one), or a new event on [newDate] in the first calendar (chosen under Calendar) when [id] is null. Save needs a title and, for a
 * change, something changed. Close with changes asks first.
 */
@Composable
fun EventEditorScreen(
    backend: CalendarBackend,
    account: Long?,
    id: Long?,
    scope: EventScopeChoice,
    newDate: LocalDate?,
    accounts: List<CalendarAccount>,
    calendars: Map<Long, List<CalendarRef>>,
    onSaved: (account: Long, id: Long) -> Unit,
    onClose: () -> Unit,
    modifier: Modifier = Modifier,
    zone: ZoneId = ZoneId.systemDefault(),
    clock: () -> Long = System::currentTimeMillis,
    /** S35: the time of the slot tapped, for a new event (else the next quarter hour). */
    newTime: LocalTime? = null,
) {
    val isNew = id == null || account == null
    var original by remember(account, id, scope) { mutableStateOf<EventData?>(null) }
    var loaded by remember(account, id, scope) { mutableStateOf(isNew) }
    LaunchedEffect(account, id, scope) {
        if (!isNew) {
            val key = (scope as? EventScopeChoice.One)?.keyMs
            original = try { backend.get(account!!, id!!, key, zone.id) } catch (e: CancellationException) { throw e } catch (e: Exception) { null }
            loaded = true
        }
    }
    if (!loaded) return
    if (!isNew && original == null) {
        LaunchedEffect(Unit) { onClose() }
        return
    }
    // A new event goes to a calendar that can take it (never one from a link).
    val firstCalendar = accounts.firstNotNullOfOrNull { a -> calendars[a.id]?.firstOrNull { it.shown && !it.readOnly }?.let { a.id to it.id } }
        ?: accounts.firstNotNullOfOrNull { a -> calendars[a.id]?.firstOrNull { !it.readOnly }?.let { a.id to it.id } }
    val editor = remember(original) { EventEditor(zone, original, clock(), firstCalendar) }
    val start = remember(editor, newDate) {
        val s = editor.initial
        if (original == null && newDate != null) editor.withStart(s, newDate.atTime(newTime ?: s.start.toLocalTime())) else s
    }
    var fields by rememberSaveable(original?.id, stateSaver = EditorStateSaver) { mutableStateOf(start) }
    var saving by remember { mutableStateOf(false) }
    var askDiscard by remember { mutableStateOf(false) }
    var picker by remember { mutableStateOf<Pick?>(null) }
    var repeatOpen by remember { mutableStateOf(false) }
    var calendarOpen by remember { mutableStateOf(false) }
    val snackbar = remember { CroftSnackbarState() }
    val coroutines = rememberCoroutineScope()
    val failed = stringResource(R.string.editor_save_failed)
    val changed = original == null || editor.changed(fields)
    val canSave = editor.canSave(fields) && !saving
    val close: () -> Unit = {
        if (original != null && editor.changed(fields) || original == null && (fields.title.isNotBlank() || fields.location.isNotBlank() || fields.notes.isNotBlank())) {
            askDiscard = true
        } else {
            onClose()
        }
    }
    BackHandler { close() }
    val locale = LocalConfiguration.current.locales[0]
    val dateF = DateTimeFormatter.ofLocalizedDate(FormatStyle.MEDIUM).withLocale(locale)
    val timeF = DateTimeFormatter.ofLocalizedTime(FormatStyle.SHORT).withLocale(locale)
    Box(modifier.fillMaxSize().background(MaterialTheme.colorScheme.background).testTag(EditorTags.SCREEN)) {
        Column(Modifier.fillMaxSize().windowInsetsPadding(WindowInsets.statusBars).windowInsetsPadding(WindowInsets.navigationBars).imePadding()) {
            Row(Modifier.fillMaxWidth().padding(4.dp), verticalAlignment = Alignment.CenterVertically) {
                CroftIconButton(Icons.Filled.Close, stringResource(R.string.editor_close), close, Modifier.testTag(EditorTags.CLOSE))
                Text(
                    stringResource(if (original == null) R.string.editor_title_new else R.string.editor_title_edit),
                    Modifier.weight(1f).padding(start = 8.dp).semantics { heading() }, style = MaterialTheme.typography.titleLarge,
                )
                CroftButton(
                    label = stringResource(R.string.editor_save),
                    onClick = {
                        val acc = fields.account ?: return@CroftButton
                        val cal = fields.calendar ?: return@CroftButton
                        saving = true
                        coroutines.launch {
                            try {
                                val saved = backend.save(acc, original?.id, cal, editor.draft(fields), scope, zone.id)
                                onSaved(acc, saved)
                            } catch (e: CancellationException) {
                                throw e
                            } catch (e: Exception) {
                                snackbar.show(failed)
                            } finally {
                                saving = false
                            }
                        }
                    },
                    modifier = Modifier.padding(end = 8.dp).testTag(EditorTags.SAVE),
                    enabled = canSave || saving,
                    progress = if (saving) ButtonProgress.LOADING else ButtonProgress.IDLE,
                    disabledReason = when {
                        fields.title.isBlank() -> stringResource(R.string.editor_save_needs_title)
                        !changed -> stringResource(R.string.editor_save_unchanged)
                        else -> null
                    },
                    onExplain = { snackbar.show(it) },
                )
            }
            Column(
                Modifier.weight(1f).fillMaxWidth().verticalScroll(rememberScrollState()).padding(horizontal = 16.dp, vertical = 8.dp),
                verticalArrangement = Arrangement.spacedBy(12.dp),
            ) {
                val w = Modifier.fillMaxWidth().widthIn(max = 640.dp)
                CroftTextField(fields.title, { fields = fields.copy(title = it) }, stringResource(R.string.editor_field_title), w.testTag(EditorTags.TITLE))
                CroftSwitchRow(stringResource(R.string.editor_all_day), fields.allDay, { fields = editor.withAllDay(fields, it) }, w.testTag(EditorTags.ALL_DAY))
                for ((isStart, value) in listOf(true to fields.start, false to fields.end)) {
                    Row(w, verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                        Text(stringResource(if (isStart) R.string.editor_start else R.string.editor_end), Modifier.weight(1f), style = MaterialTheme.typography.bodyLarge)
                        CroftButton(value.toLocalDate().format(dateF), { picker = Pick(isStart, date = true) }, Modifier.testTag(if (isStart) EditorTags.START_DATE else EditorTags.END_DATE), kind = ButtonKind.TONAL)
                        if (!fields.allDay) {
                            CroftButton(value.toLocalTime().format(timeF), { picker = Pick(isStart, date = false) }, Modifier.testTag(if (isStart) EditorTags.START_TIME else EditorTags.END_TIME), kind = ButtonKind.TONAL)
                        }
                    }
                }
                Column(w) {
                    Text(stringResource(R.string.editor_repeat), style = MaterialTheme.typography.titleSmall, color = MaterialTheme.colorScheme.primary)
                    Box {
                        CroftButton(repeatText(fields.repeat), { if (fields.repeat != RepeatChoice.CUSTOM) repeatOpen = true }, Modifier.testTag(EditorTags.REPEAT), kind = ButtonKind.TONAL, leadingIcon = Icons.Filled.ArrowDropDown)
                        CroftMenu(
                            expanded = repeatOpen,
                            onDismiss = { repeatOpen = false },
                            items = RepeatChoice.entries.filter { it != RepeatChoice.CUSTOM }.map { r ->
                                MenuItemSpec(repeatText(r), { fields = fields.copy(repeat = r); repeatOpen = false })
                            },
                        )
                    }
                }
                Column(w, verticalArrangement = Arrangement.spacedBy(4.dp)) {
                    Text(stringResource(R.string.editor_reminders), style = MaterialTheme.typography.titleSmall, color = MaterialTheme.colorScheme.primary)
                    ReminderChips(fields.reminders) { m ->
                        fields = fields.copy(reminders = if (m in fields.reminders) fields.reminders - m else (fields.reminders + m).sorted())
                    }
                }
                if (original == null) {
                    Column(w) {
                        Text(stringResource(R.string.editor_calendar), style = MaterialTheme.typography.titleSmall, color = MaterialTheme.colorScheme.primary)
                        val label = fields.calendar?.let { c -> calendarLabel(accounts, calendars, fields.account, c) }.orEmpty()
                        Box {
                            CroftButton(label, { calendarOpen = true }, Modifier.testTag(EditorTags.CALENDAR), kind = ButtonKind.TONAL, leadingIcon = Icons.Filled.ArrowDropDown)
                            CroftMenu(
                                expanded = calendarOpen,
                                onDismiss = { calendarOpen = false },
                                items = accounts.flatMap { a ->
                                    calendars[a.id].orEmpty().filter { !it.readOnly }.map { c ->
                                        MenuItemSpec(calendarLabel(accounts, calendars, a.id, c.id), { fields = fields.copy(account = a.id, calendar = c.id); calendarOpen = false })
                                    }
                                },
                            )
                        }
                    }
                }
                CroftTextField(fields.location, { fields = fields.copy(location = it) }, stringResource(R.string.editor_location), w.testTag(EditorTags.LOCATION))
                CroftTextField(fields.notes, { fields = fields.copy(notes = it) }, stringResource(R.string.editor_notes), w.testTag(EditorTags.NOTES))
            }
        }
        CroftSnackbarHost(snackbar, Modifier.align(Alignment.BottomCenter).windowInsetsPadding(WindowInsets.navigationBars))
    }
    picker?.let { p ->
        val value = if (p.start) fields.start else fields.end
        if (p.date) {
            DateDialog(value.toLocalDate(), onDismiss = { picker = null }) { d ->
                picker = null
                val t = value.withYear(d.year).withMonth(d.monthValue).withDayOfMonth(d.dayOfMonth)
                fields = if (p.start) editor.withStart(fields, t) else editor.withEnd(fields, t)
            }
        } else {
            TimeDialog(value.toLocalTime(), onDismiss = { picker = null }) { time ->
                picker = null
                val t = value.toLocalDate().atTime(time)
                fields = if (p.start) editor.withStart(fields, t) else editor.withEnd(fields, t)
            }
        }
    }
    if (askDiscard) {
        DestructiveDialog(
            title = stringResource(R.string.editor_discard_title),
            message = stringResource(R.string.editor_discard_message),
            confirmLabel = stringResource(R.string.editor_discard),
            onConfirm = { askDiscard = false; onClose() },
            onDismiss = { askDiscard = false },
        )
    }
}

/** Which picker is open: the start or end, its date or its time. */
private data class Pick(val start: Boolean, val date: Boolean)

@Composable
fun repeatText(r: RepeatChoice): String = stringResource(
    when (r) {
        RepeatChoice.NONE -> R.string.editor_repeat_none
        RepeatChoice.DAILY -> R.string.editor_repeat_daily
        RepeatChoice.WEEKLY -> R.string.editor_repeat_weekly
        RepeatChoice.MONTHLY -> R.string.editor_repeat_monthly
        RepeatChoice.YEARLY -> R.string.editor_repeat_yearly
        RepeatChoice.CUSTOM -> R.string.editor_repeat_custom
    },
)

@OptIn(ExperimentalLayoutApi::class)
@Composable
private fun ReminderChips(chosen: List<Long>, onToggle: (Long) -> Unit) {
    FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalArrangement = Arrangement.spacedBy(4.dp)) {
        for (m in EventEditor.REMINDER_CHOICES) {
            CroftFilterChip(reminderText(m), m in chosen, { onToggle(m) }, Modifier.testTag(EditorTags.reminder(m)))
        }
    }
}

@Composable
private fun calendarLabel(accounts: List<CalendarAccount>, calendars: Map<Long, List<CalendarRef>>, account: Long?, calendar: Long): String {
    val a = accounts.firstOrNull { it.id == account }?.label.orEmpty()
    val c = calendars[account]?.firstOrNull { it.id == calendar }?.name.orEmpty()
    return if (c.isBlank()) a else stringResource(R.string.editor_calendar_label, c, a)
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun DateDialog(value: LocalDate, onDismiss: () -> Unit, onPick: (LocalDate) -> Unit) {
    val state = rememberDatePickerState(initialSelectedDateMillis = value.atStartOfDay(ZoneOffset.UTC).toInstant().toEpochMilli())
    DatePickerDialog(
        onDismissRequest = onDismiss,
        confirmButton = {
            TextButton({ state.selectedDateMillis?.let { onPick(LocalDate.ofEpochDay(Math.floorDiv(it, 86_400_000L))) } ?: onDismiss() }) {
                Text(stringResource(R.string.editor_ok))
            }
        },
        dismissButton = { TextButton(onDismiss) { Text(stringResource(R.string.control_cancel)) } },
    ) { DatePicker(state = state) }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun TimeDialog(value: LocalTime, onDismiss: () -> Unit, onPick: (LocalTime) -> Unit) {
    val state = rememberTimePickerState(initialHour = value.hour, initialMinute = value.minute)
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text(stringResource(R.string.editor_pick_time)) },
        text = { TimePicker(state = state) },
        confirmButton = { TextButton({ onPick(LocalTime.of(state.hour, state.minute)) }) { Text(stringResource(R.string.editor_ok)) } },
        dismissButton = { TextButton(onDismiss) { Text(stringResource(R.string.control_cancel)) } },
    )
}

/** Keeps S38's fields across a rotation: a flat list of strings. */
private val EditorStateSaver: Saver<EditorState, Any> = Saver(
    save = { s ->
        arrayListOf(
            s.title, s.allDay.toString(), s.start.toString(), s.end.toString(), s.repeat.name, s.reminders.joinToString(","),
            s.calendar?.toString().orEmpty(), s.account?.toString().orEmpty(), s.location, s.notes,
        )
    },
    restore = { saved ->
        @Suppress("UNCHECKED_CAST")
        val v = saved as List<String>
        EditorState(
            title = v[0], allDay = v[1].toBoolean(), start = LocalDateTime.parse(v[2]), end = LocalDateTime.parse(v[3]),
            repeat = RepeatChoice.valueOf(v[4]), reminders = v[5].split(',').mapNotNull { it.toLongOrNull() },
            calendar = v[6].toLongOrNull(), account = v[7].toLongOrNull(), location = v[8], notes = v[9],
        )
    },
)
