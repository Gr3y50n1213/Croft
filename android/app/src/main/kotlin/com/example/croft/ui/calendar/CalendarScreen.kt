// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.ui.calendar

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.sizeIn
import androidx.compose.foundation.layout.statusBars
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.KeyboardArrowLeft
import androidx.compose.material.icons.automirrored.filled.KeyboardArrowRight
import androidx.compose.material.icons.filled.Add
import androidx.compose.material.icons.filled.Menu
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.pulltorefresh.pullToRefresh
import androidx.compose.material3.pulltorefresh.rememberPullToRefreshState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.setValue
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.platform.LocalConfiguration
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.pluralStringResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.ui.semantics.selected
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import com.example.croft.R
import com.example.croft.haptics.LocalHaptics
import com.example.croft.mail.calendar.AgendaModel
import com.example.croft.mail.calendar.CalendarController
import com.example.croft.mail.calendar.CalendarSync
import com.example.croft.mail.calendar.CalendarView
import com.example.croft.mail.calendar.Occurrence
import com.example.croft.ui.components.CroftExtendedFab
import com.example.croft.ui.components.CroftFilterChip
import com.example.croft.ui.components.CroftIconButton
import com.example.croft.ui.components.FabState
import com.example.croft.ui.mail.MailBottomBar
import com.example.croft.ui.mail.MailDestination
import com.example.croft.ui.mail.PULL_ARM_DP
import com.example.croft.ui.mail.PullIndicator
import com.example.croft.ui.mail.PullRefreshHaptics
import java.time.LocalDate
import java.time.ZoneId
import java.time.format.DateTimeFormatter
import java.time.format.FormatStyle
import java.time.format.TextStyle
import kotlinx.coroutines.launch

private const val WIDE_DP = 600

/**
 * S34 Agenda and S36 Month: the header (the month, the view switch, Calendars), the sync problems, the agenda or the month grid with
 * the selected day's events, New event, and the bottom bar with Calendar selected. Pull down syncs again.
 */
@Composable
fun CalendarScreen(
    controller: CalendarController,
    onOpen: (Occurrence) -> Unit,
    onNew: ((LocalDate) -> Unit)?,
    onCalendars: () -> Unit,
    onMail: () -> Unit,
    onContacts: (() -> Unit)?,
    modifier: Modifier = Modifier,
    zone: ZoneId = ZoneId.systemDefault(),
    /** S35: a tap on an empty slot (a new event at that time). */
    onNewAt: ((java.time.LocalDateTime) -> Unit)? = null,
    /** S35: the "Move" action of a block for screen readers (the editor's times). */
    onMoveRequest: (Occurrence) -> Unit = onOpen,
) {
    val state by controller.state.collectAsState()
    val haptics = LocalHaptics.current
    val locale = LocalConfiguration.current.locales[0]
    val today = controller.today()
    val coroutines = androidx.compose.runtime.rememberCoroutineScope()
    // S35: a drag of a repeating event waits for "This event / All events".
    var pendingMove by androidx.compose.runtime.remember { androidx.compose.runtime.mutableStateOf<Pair<Occurrence, Long>?>(null) }
    pendingMove?.let { (o, ms) ->
        androidx.compose.material3.AlertDialog(
            onDismissRequest = { pendingMove = null },
            title = { Text(stringResource(R.string.event_scope_title)) },
            text = {
                Column {
                    androidx.compose.material3.TextButton({ pendingMove = null; coroutines.launch { controller.move(o, ms, com.example.croft.mail.calendar.EventScopeChoice.One(o.keyMs)) } }, Modifier.testTag(CalendarTags.MOVE_ONE)) { Text(stringResource(R.string.event_scope_one)) }
                    androidx.compose.material3.TextButton({ pendingMove = null; coroutines.launch { controller.move(o, ms, com.example.croft.mail.calendar.EventScopeChoice.All) } }, Modifier.testTag(CalendarTags.MOVE_ALL)) { Text(stringResource(R.string.event_scope_all)) }
                }
            },
            confirmButton = {},
            dismissButton = { androidx.compose.material3.TextButton({ pendingMove = null }) { Text(stringResource(R.string.control_cancel)) } },
        )
    }
    Column(modifier.fillMaxSize().background(MaterialTheme.colorScheme.background).testTag(CalendarTags.SCREEN)) {
        Column(Modifier.windowInsetsPadding(WindowInsets.statusBars).padding(horizontal = 12.dp, vertical = 8.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                val title = state.month.month.getDisplayName(TextStyle.FULL_STANDALONE, locale) + " " + state.month.year
                Text(title, Modifier.weight(1f).padding(horizontal = 4.dp).semantics { heading() }, style = MaterialTheme.typography.titleLarge)
                CroftIconButton(Icons.Filled.Menu, stringResource(R.string.calendar_calendars), onCalendars, Modifier.testTag(CalendarTags.CALENDARS))
            }
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                CroftFilterChip(stringResource(R.string.calendar_view_agenda), state.view == CalendarView.AGENDA, { controller.setView(CalendarView.AGENDA) }, Modifier.testTag(CalendarTags.VIEW_AGENDA))
                CroftFilterChip(stringResource(R.string.calendar_view_week), state.view == CalendarView.WEEK, { controller.setView(CalendarView.WEEK) }, Modifier.testTag(CalendarTags.VIEW_WEEK))
                CroftFilterChip(stringResource(R.string.calendar_view_month), state.view == CalendarView.MONTH, { controller.setView(CalendarView.MONTH) }, Modifier.testTag(CalendarTags.VIEW_MONTH))
            }
            val problems = state.problems.mapNotNull { (id, p) ->
                val label = state.accounts.firstOrNull { it.id == id }?.label ?: return@mapNotNull null
                when (p) {
                    CalendarSync.UNREACHABLE -> stringResource(R.string.calendar_unreachable, label)
                    CalendarSync.SIGN_IN_NEEDED -> stringResource(R.string.calendar_sign_in, label)
                    else -> null
                }
            }
            if (problems.isNotEmpty()) {
                Column(Modifier.testTag(CalendarTags.PROBLEMS).semantics { liveRegion = LiveRegionMode.Polite }) {
                    for (p in problems) Text(p, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.error)
                }
            }
            if (state.truncated) Text(stringResource(R.string.calendar_truncated), style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
        }
        val pullState = rememberPullToRefreshState()
        PullRefreshHaptics(pullState, state.refreshing, haptics)
        Box(
            Modifier.weight(1f).fillMaxWidth()
                .pullToRefresh(isRefreshing = state.refreshing, state = pullState, threshold = PULL_ARM_DP, onRefresh = controller::refresh),
        ) {
            when {
                !state.loaded -> Text(stringResource(R.string.calendar_loading), Modifier.padding(24.dp), style = MaterialTheme.typography.bodyMedium)
                state.syncedOnce && state.calendars.isEmpty() && state.agenda.isEmpty() -> Column(
                    Modifier.padding(24.dp).widthIn(max = 480.dp).testTag(CalendarTags.EMPTY),
                    verticalArrangement = Arrangement.spacedBy(8.dp),
                ) {
                    Text(stringResource(R.string.calendar_not_offered_title), style = MaterialTheme.typography.titleMedium, modifier = Modifier.semantics { heading() })
                    Text(stringResource(R.string.calendar_not_offered_message), style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
                }
                state.view == CalendarView.AGENDA -> Agenda(controller, state.agenda, state.agendaFrom, state.agendaTo, today, zone, onOpen)
                state.view == CalendarView.WEEK -> {
                    val wide = LocalConfiguration.current.screenWidthDp >= WIDE_DP
                    androidx.compose.runtime.LaunchedEffect(wide) { controller.setWeekDays(if (wide) 7 else 3) }
                    WeekGrid(
                        occurrences = state.weekOccurrences, start = state.weekStart, days = state.weekDays, today = today, nowMs = controller.nowMs(), zone = zone,
                        onOpen = onOpen,
                        onNewAt = { t -> onNewAt?.invoke(t) },
                        onMove = { o, ms -> if (o.repeats) pendingMove = o to ms else coroutines.launch { controller.move(o, ms, com.example.croft.mail.calendar.EventScopeChoice.All) } },
                        onMoveRequest = onMoveRequest,
                        onShift = controller::moveWeek,
                    )
                }
                else -> Month(controller, today, zone, onOpen)
            }
            PullIndicator(pullState, state.refreshing, Modifier.align(Alignment.TopCenter))
            if (onNew != null && state.calendars.isNotEmpty()) {
                Box(Modifier.align(Alignment.BottomEnd).padding(16.dp)) {
                    CroftExtendedFab(
                        stringResource(R.string.calendar_new_event),
                        { onNew(if (state.view == CalendarView.MONTH) state.selected else today) },
                        FabState.EXPANDED, Modifier.testTag(CalendarTags.NEW), icon = Icons.Filled.Add,
                    )
                }
            }
        }
        MailBottomBar(enabled = true, onMail = onMail, selected = MailDestination.CALENDAR, onContacts = onContacts, onCalendar = { controller.setView(CalendarView.AGENDA) })
    }
}

@Composable
private fun Agenda(controller: CalendarController, occurrences: List<Occurrence>, from: LocalDate, to: LocalDate, today: LocalDate, zone: ZoneId, onOpen: (Occurrence) -> Unit) {
    val days = remember(occurrences, from, to, today, zone) { AgendaModel.days(occurrences, from, to, zone, today) }
    val listState = rememberLazyListState()
    LazyColumn(Modifier.fillMaxSize().testTag(CalendarTags.LIST), state = listState, contentPadding = androidx.compose.foundation.layout.PaddingValues(start = 12.dp, end = 12.dp, bottom = 88.dp)) {
        items(days, key = { it.date.toString() }) { day ->
            Row(Modifier.fillMaxWidth().padding(vertical = 6.dp).testTag(CalendarTags.day(day.date)), horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                DayHeader(day.date, day.date == today)
                Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                    if (day.items.isEmpty()) {
                        Text(stringResource(R.string.calendar_no_events), Modifier.padding(vertical = 12.dp), style = MaterialTheme.typography.bodyLarge, color = MaterialTheme.colorScheme.onSurfaceVariant)
                    }
                    for (item in day.items) {
                        val o = item.occurrence
                        EventCard(o, zone, item.overlaps, onClick = { onOpen(o) }, modifier = Modifier.testTag(CalendarTags.event(o.eventId, o.keyMs)))
                    }
                }
            }
        }
        item(key = "more") {
            com.example.croft.ui.components.CroftButton(
                stringResource(R.string.calendar_load_more), controller::loadMore, Modifier.padding(vertical = 12.dp).testTag(CalendarTags.MORE),
                kind = com.example.croft.ui.components.ButtonKind.TEXT,
            )
        }
    }
}

@Composable
private fun Month(controller: CalendarController, today: LocalDate, zone: ZoneId, onOpen: (Occurrence) -> Unit) {
    val state by controller.state.collectAsState()
    val locale = LocalConfiguration.current.locales[0]
    val firstDay = CalendarController.firstDayOfWeek()
    val grid = remember(state.month, firstDay) { AgendaModel.monthGrid(state.month, firstDay) }
    val counts = remember(state.monthOccurrences, zone) { AgendaModel.counts(state.monthOccurrences, zone) }
    val selectedDay = remember(state.monthOccurrences, state.selected, zone, today) {
        AgendaModel.days(state.monthOccurrences, state.selected, state.selected, zone, state.selected).firstOrNull()?.items.orEmpty()
    }
    LazyColumn(Modifier.fillMaxSize().testTag(CalendarTags.MONTH), contentPadding = androidx.compose.foundation.layout.PaddingValues(start = 12.dp, end = 12.dp, bottom = 88.dp)) {
        item(key = "nav") {
            Row(verticalAlignment = Alignment.CenterVertically) {
                CroftIconButton(Icons.AutoMirrored.Filled.KeyboardArrowLeft, stringResource(R.string.calendar_previous_month), { controller.showMonth(state.month.minusMonths(1)) }, Modifier.testTag(CalendarTags.PREVIOUS))
                Spacer(Modifier.weight(1f))
                CroftIconButton(Icons.AutoMirrored.Filled.KeyboardArrowRight, stringResource(R.string.calendar_next_month), { controller.showMonth(state.month.plusMonths(1)) }, Modifier.testTag(CalendarTags.NEXT))
            }
        }
        item(key = "weekdays") {
            Row(Modifier.fillMaxWidth().clearAndSetSemantics { }) {
                for (i in 0 until 7) {
                    Text(
                        firstDay.plus(i.toLong()).getDisplayName(TextStyle.NARROW, locale),
                        Modifier.weight(1f), textAlign = TextAlign.Center, style = MaterialTheme.typography.labelMedium, color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
            }
        }
        items(grid.chunked(7), key = { it.first().toString() }) { week ->
            Row(Modifier.fillMaxWidth()) {
                for (date in week) {
                    val n = counts[date] ?: 0
                    val spoken = pluralStringResource(R.plurals.calendar_day_events, n, date.format(DateTimeFormatter.ofLocalizedDate(FormatStyle.LONG).withLocale(locale)), n)
                    val selected = date == state.selected
                    val isToday = date == today
                    val inMonth = date.month == state.month.month
                    Column(
                        Modifier.weight(1f).heightIn(min = 52.dp).clip(MaterialTheme.shapes.medium)
                            .clickable(role = Role.Button) { controller.select(date) }
                            .semantics { contentDescription = spoken; this.selected = selected }
                            .testTag(CalendarTags.cell(date)),
                        horizontalAlignment = Alignment.CenterHorizontally,
                        verticalArrangement = Arrangement.Center,
                    ) {
                        Box(
                            Modifier.sizeIn(minWidth = 36.dp, minHeight = 36.dp).clip(CircleShape)
                                .then(if (selected) Modifier.background(MaterialTheme.colorScheme.primary) else Modifier)
                                .then(if (isToday && !selected) Modifier.border(2.dp, MaterialTheme.colorScheme.primary, CircleShape) else Modifier),
                            contentAlignment = Alignment.Center,
                        ) {
                            Text(
                                date.dayOfMonth.toString(),
                                modifier = Modifier.padding(4.dp),
                                style = MaterialTheme.typography.bodyLarge,
                                color = when {
                                    selected -> MaterialTheme.colorScheme.onPrimary
                                    inMonth -> MaterialTheme.colorScheme.onSurface
                                    else -> MaterialTheme.colorScheme.onSurfaceVariant
                                },
                            )
                        }
                        Row(Modifier.height(8.dp), horizontalArrangement = Arrangement.spacedBy(2.dp), verticalAlignment = Alignment.CenterVertically) {
                            repeat(n.coerceAtMost(3)) { Box(Modifier.size(5.dp).clip(CircleShape).background(MaterialTheme.colorScheme.primary)) }
                        }
                    }
                }
            }
        }
        item(key = "selected") {
            Text(
                state.selected.format(DateTimeFormatter.ofLocalizedDate(FormatStyle.FULL).withLocale(locale)),
                Modifier.padding(top = 16.dp, bottom = 8.dp).semantics { heading() },
                style = MaterialTheme.typography.titleMedium,
            )
            if (selectedDay.isEmpty()) Text(stringResource(R.string.calendar_no_events), style = MaterialTheme.typography.bodyLarge, color = MaterialTheme.colorScheme.onSurfaceVariant)
        }
        items(selectedDay, key = { "s-${it.occurrence.eventId}-${it.occurrence.keyMs}" }) { item ->
            EventCard(item.occurrence, zone, item.overlaps, onClick = { onOpen(item.occurrence) }, modifier = Modifier.padding(bottom = 8.dp).testTag(CalendarTags.event(item.occurrence.eventId, item.occurrence.keyMs)))
        }
    }
}
