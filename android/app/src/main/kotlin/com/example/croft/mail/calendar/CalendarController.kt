// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.mail.calendar

import java.time.DayOfWeek
import java.time.Instant
import java.time.LocalDate
import java.time.YearMonth
import java.time.ZoneId
import java.time.temporal.TemporalAdjusters
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch

/** An account as the calendar screens name it. */
data class CalendarAccount(val id: Long, val label: String, val isDemo: Boolean = false) {
    override fun toString(): String = "CalendarAccount($id)"
}

/** One occurrence on one day of the agenda, and whether it overlaps another timed event that day. */
data class AgendaItem(val occurrence: Occurrence, val overlaps: Boolean)

/** A day of the agenda: its date and its events (empty for today with nothing on). */
data class AgendaDay(val date: LocalDate, val items: List<AgendaItem>)

/** The two views of the calendar this task builds (Week is T39c). */
enum class CalendarView { AGENDA, MONTH }

/** Grouping and overlap rules of S34 and S36, pure. */
object AgendaModel {
    /** The days [occ] covers in [zone]: an event ending at midnight does not cover that day; an empty event covers its start day. */
    fun daysOf(occ: Occurrence, zone: ZoneId): List<LocalDate> {
        val first = Instant.ofEpochMilli(occ.startMs).atZone(zone).toLocalDate()
        val lastInstant = if (occ.endMs > occ.startMs) occ.endMs - 1 else occ.startMs
        val last = Instant.ofEpochMilli(lastInstant).atZone(zone).toLocalDate()
        if (last.isBefore(first)) return listOf(first)
        return generateSequence(first) { it.plusDays(1) }.takeWhile { !it.isAfter(last) }.take(MAX_DAYS_PER_EVENT).toList()
    }

    /**
     * The agenda between [from] and [to] (inclusive): every day with events, and [today] always (with no events, "No events").
     * In a day, all-day events first, then by start; a timed event that overlaps another timed event that day is marked.
     */
    fun days(occurrences: List<Occurrence>, from: LocalDate, to: LocalDate, zone: ZoneId, today: LocalDate): List<AgendaDay> {
        val byDay = sortedMapOf<LocalDate, MutableList<Occurrence>>()
        for (o in occurrences) {
            for (d in daysOf(o, zone)) {
                if (!d.isBefore(from) && !d.isAfter(to)) byDay.getOrPut(d) { mutableListOf() }.add(o)
            }
        }
        if (!today.isBefore(from) && !today.isAfter(to)) byDay.getOrPut(today) { mutableListOf() }
        return byDay.map { (date, list) ->
            val sorted = list.distinctBy { Triple(it.account, it.eventId, it.keyMs) }.sortedWith(compareBy({ !it.allDay }, { it.startMs }, { it.endMs }, { it.eventId }))
            val timed = sorted.filter { !it.allDay }
            AgendaDay(date, sorted.map { o -> AgendaItem(o, !o.allDay && timed.any { it !== o && overlaps(it, o) }) })
        }
    }

    private fun overlaps(a: Occurrence, b: Occurrence): Boolean = a.startMs < b.endMs && b.startMs < a.endMs

    /** The days of [month]'s grid: whole weeks starting on [firstDay], covering the month. */
    fun monthGrid(month: YearMonth, firstDay: DayOfWeek): List<LocalDate> {
        val start = month.atDay(1).with(TemporalAdjusters.previousOrSame(firstDay))
        val end = month.atEndOfMonth().with(TemporalAdjusters.nextOrSame(firstDay.minus(1)))
        return generateSequence(start) { it.plusDays(1) }.takeWhile { !it.isAfter(end) }.toList()
    }

    /** How many events each day has (for S36's dots). */
    fun counts(occurrences: List<Occurrence>, zone: ZoneId): Map<LocalDate, Int> {
        val out = HashMap<LocalDate, Int>()
        for (o in occurrences) for (d in daysOf(o, zone)) out[d] = (out[d] ?: 0) + 1
        return out
    }

    /** The longest stretch of days one event is listed on (a longer one is shown on its first days only). */
    private const val MAX_DAYS_PER_EVENT = 366
}

/** What S34, S36 and S39 show. */
data class CalendarState(
    val view: CalendarView = CalendarView.AGENDA,
    val agendaFrom: LocalDate = LocalDate.MIN,
    val agendaTo: LocalDate = LocalDate.MIN,
    val agenda: List<Occurrence> = emptyList(),
    val month: YearMonth = YearMonth.of(2000, 1),
    val selected: LocalDate = LocalDate.MIN,
    val monthOccurrences: List<Occurrence> = emptyList(),
    /** The accounts that have calendars, and their calendars. */
    val accounts: List<CalendarAccount> = emptyList(),
    val calendars: Map<Long, List<CalendarRef>> = emptyMap(),
    val loaded: Boolean = false,
    val refreshing: Boolean = false,
    val problems: Map<Long, CalendarSync> = emptyMap(),
    val syncedOnce: Boolean = false,
    /** Some events of a window were left out (a very large calendar). */
    val truncated: Boolean = false,
) {
    override fun toString(): String = "CalendarState($view, ${agenda.size} agenda, ${monthOccurrences.size} month, loaded=$loaded)"
}

/**
 * The calendar's state: the agenda (from today, [AGENDA_DAYS] at a time), the month, the calendars, and the syncs. Every
 * account but the sample mailbox is synced on [start] and [refresh], one after another; the windows are read again after each.
 * [onChanged] is called after a sync or a change (the app plans the reminder job again).
 */
class CalendarController(
    private val backend: CalendarBackend,
    private val accounts: suspend () -> List<CalendarAccount>,
    private val scope: CoroutineScope,
    private val zone: () -> ZoneId = ZoneId::systemDefault,
    private val clock: () -> Long = System::currentTimeMillis,
    private val onChanged: () -> Unit = {},
) {
    private val mutable = MutableStateFlow(CalendarState())

    /** What the screens draw. */
    val state: StateFlow<CalendarState> get() = mutable

    private var syncJob: Job? = null
    private var all: List<CalendarAccount> = emptyList()

    /** Today in the device's zone. */
    fun today(): LocalDate = Instant.ofEpochMilli(clock()).atZone(zone()).toLocalDate()

    /** Reads the calendars and the windows, then syncs every account. Called when Calendar opens. */
    fun start() {
        val today = today()
        mutable.update {
            if (it.agendaFrom == LocalDate.MIN) it.copy(agendaFrom = today, agendaTo = today.plusDays(AGENDA_DAYS - 1), month = YearMonth.from(today), selected = today) else it
        }
        scope.launch {
            all = runCatching { accounts() }.getOrDefault(emptyList())
            readCalendars()
            reload()
            refresh()
        }
    }

    /** Pull to refresh: syncs every account again (nothing when a sync is running). */
    fun refresh() {
        if (syncJob?.isActive == true) return
        syncJob = scope.launch {
            mutable.update { it.copy(refreshing = true) }
            try {
                for (a in all.filterNot { it.isDemo }) {
                    val result = try { backend.sync(a.id) } catch (e: CancellationException) { throw e } catch (e: Exception) { CalendarSync.UNREACHABLE }
                    noteSync(a.id, result)
                    readCalendars()
                    reload()
                }
            } finally {
                mutable.update { it.copy(refreshing = false, syncedOnce = true) }
                onChanged()
            }
        }
    }

    /** Syncs one account in the background (after a save or a delete), then reads the windows again. */
    fun syncInBackground(account: Long) {
        scope.launch {
            val result = try { backend.sync(account) } catch (e: CancellationException) { throw e } catch (e: Exception) { CalendarSync.UNREACHABLE }
            noteSync(account, result)
            reload()
            onChanged()
        }
    }

    /** Reads the windows again (after S38 saved, or S37 deleted). */
    fun reloadNow() {
        scope.launch { reload(); onChanged() }
    }

    /** Agenda or month. */
    fun setView(view: CalendarView) = mutable.update { it.copy(view = view) }

    /** The agenda reached its end: [AGENDA_DAYS] more days. */
    fun loadMore() {
        mutable.update { it.copy(agendaTo = it.agendaTo.plusDays(AGENDA_DAYS)) }
        scope.launch { reload() }
    }

    /** Shows [month] in S36 (its first day selected, or today when it is this month). */
    fun showMonth(month: YearMonth) {
        val today = today()
        mutable.update { it.copy(month = month, selected = if (YearMonth.from(today) == month) today else month.atDay(1)) }
        scope.launch { reload() }
    }

    /** A day was tapped in S36. */
    fun select(date: LocalDate) {
        val moved = YearMonth.from(date) != mutable.value.month
        mutable.update { it.copy(selected = date, month = YearMonth.from(date)) }
        if (moved) scope.launch { reload() }
    }

    /** S39: shows or hides a calendar's events. */
    fun setShown(account: Long, id: Long, shown: Boolean) {
        scope.launch {
            val done = try { backend.setShown(account, id, shown) } catch (e: CancellationException) { throw e } catch (e: Exception) { false }
            if (done) {
                readCalendars()
                reload()
                onChanged()
            }
        }
    }

    private fun noteSync(account: Long, result: CalendarSync) = mutable.update { s ->
        s.copy(problems = if (result == CalendarSync.UNREACHABLE || result == CalendarSync.SIGN_IN_NEEDED) s.problems + (account to result) else s.problems - account)
    }

    private suspend fun readCalendars() {
        val map = HashMap<Long, List<CalendarRef>>()
        for (a in all) {
            val c = try { backend.calendars(a.id) } catch (e: CancellationException) { throw e } catch (e: Exception) { emptyList() }
            if (c.isNotEmpty()) map[a.id] = c
        }
        mutable.update { it.copy(calendars = map, accounts = all.filter { a -> a.id in map }) }
    }

    private suspend fun reload() {
        val s = mutable.value
        val z = zone()
        val zoneName = z.id
        val dayMs = { d: LocalDate -> d.atStartOfDay(z).toInstant().toEpochMilli() }
        try {
            val agenda = backend.between(dayMs(s.agendaFrom), dayMs(s.agendaTo.plusDays(1)), zoneName)
            val grid = AgendaModel.monthGrid(s.month, firstDayOfWeek())
            val month = backend.between(dayMs(grid.first()), dayMs(grid.last().plusDays(1)), zoneName)
            mutable.update {
                // A newer request moved the windows meanwhile: its own reload sets them.
                if (it.agendaFrom != s.agendaFrom || it.agendaTo != s.agendaTo || it.month != s.month) it.copy(loaded = true)
                else it.copy(agenda = agenda.occurrences, monthOccurrences = month.occurrences, truncated = agenda.truncated || month.truncated, loaded = true)
            }
        } catch (e: CancellationException) {
            throw e
        } catch (e: Exception) {
            mutable.update { it.copy(loaded = true) }
        }
    }

    companion object {
        /** How many days the agenda reads at a time. */
        const val AGENDA_DAYS = 60L

        /** The first day of the week where the person is (Monday unless the locale says otherwise). */
        fun firstDayOfWeek(): DayOfWeek = java.time.temporal.WeekFields.of(java.util.Locale.getDefault()).firstDayOfWeek
    }
}
