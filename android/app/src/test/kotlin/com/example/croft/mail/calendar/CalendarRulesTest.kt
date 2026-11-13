// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.mail.calendar

import com.example.croft.notify.CalendarReminderPlan
import com.example.croft.ui.calendar.FakeCalendarBackend
import java.time.DayOfWeek
import java.time.LocalDate
import java.time.LocalDateTime
import java.time.YearMonth
import java.time.ZoneId
import java.time.ZoneOffset
import kotlinx.coroutines.runBlocking
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/** The pure rules of S34, S36 and S38, and the reminder job's plan. Made-up events. */
class CalendarRulesTest {
    private val utc: ZoneId = ZoneOffset.UTC
    private fun ms(s: String) = LocalDateTime.parse(s).toInstant(ZoneOffset.UTC).toEpochMilli()
    private fun occ(id: Long, start: String, end: String, allDay: Boolean = false) =
        Occurrence(id, 1, 10, null, ms(start), ms(end), allDay, ms(start), "E$id", null, false, false, false, false)

    @Test
    fun theAgendaGroupsByDay_alwaysShowsToday_andListsAllDayFirst() {
        val today = LocalDate.of(2026, 11, 6)
        val days = AgendaModel.days(
            listOf(occ(1, "2026-11-09T09:00", "2026-11-09T10:00"), occ(2, "2026-11-09T00:00", "2026-11-10T00:00", allDay = true), occ(3, "2026-11-10T12:00", "2026-11-10T13:00")),
            today, today.plusDays(10), utc, today,
        )
        assertEquals(listOf(today, LocalDate.of(2026, 11, 9), LocalDate.of(2026, 11, 10)), days.map { it.date })
        assertTrue(days[0].items.isEmpty())
        assertEquals(listOf(2L, 1L), days[1].items.map { it.occurrence.eventId })
    }

    @Test
    fun overlappingTimedEventsAreMarked_butAllDayOnesNeverAre() {
        val day = LocalDate.of(2026, 11, 6)
        val items = AgendaModel.days(
            listOf(occ(1, "2026-11-06T12:00", "2026-11-06T13:00"), occ(2, "2026-11-06T12:00", "2026-11-06T12:30"), occ(3, "2026-11-06T15:30", "2026-11-06T16:30"), occ(4, "2026-11-06T00:00", "2026-11-07T00:00", allDay = true)),
            day, day, utc, day,
        ).single().items
        assertEquals(mapOf(4L to false, 1L to true, 2L to true, 3L to false), items.associate { it.occurrence.eventId to it.overlaps })
    }

    @Test
    fun aMultiDayEventIsOnEachDayItCovers_andMidnightEndsDoNotCountTheNextDay() {
        assertEquals(3, AgendaModel.daysOf(occ(1, "2026-11-06T00:00", "2026-11-09T00:00", allDay = true), utc).size)
        assertEquals(listOf(LocalDate.of(2026, 11, 6)), AgendaModel.daysOf(occ(2, "2026-11-06T23:00", "2026-11-07T00:00"), utc))
        assertEquals(listOf(LocalDate.of(2026, 11, 6)), AgendaModel.daysOf(occ(3, "2026-11-06T09:00", "2026-11-06T09:00"), utc))
    }

    @Test
    fun theMonthGridIsWholeWeeksFromTheFirstDay_andCountsDots() {
        val grid = AgendaModel.monthGrid(YearMonth.of(2026, 11), DayOfWeek.MONDAY)
        assertEquals(LocalDate.of(2026, 10, 26), grid.first())
        assertEquals(LocalDate.of(2026, 12, 6), grid.last())
        assertEquals(0, grid.size % 7)
        assertEquals(LocalDate.of(2026, 11, 1), AgendaModel.monthGrid(YearMonth.of(2026, 11), DayOfWeek.SUNDAY).first())
        val counts = AgendaModel.counts(listOf(occ(1, "2026-11-06T12:00", "2026-11-06T13:00"), occ(2, "2026-11-06T15:00", "2026-11-06T16:00")), utc)
        assertEquals(2, counts[LocalDate.of(2026, 11, 6)])
    }

    @Test
    fun aNewEventStartsAtTheNextQuarterHour_needsATitle_andMovingTheStartKeepsTheLength() {
        val editor = EventEditor(utc, null, ms("2026-11-06T09:07"), 1L to 10L)
        val s = editor.initial
        assertEquals(LocalDateTime.parse("2026-11-06T09:15"), s.start)
        assertEquals(LocalDateTime.parse("2026-11-06T10:15"), s.end)
        assertFalse(editor.canSave(s))
        val titled = s.copy(title = "Planning")
        assertTrue(editor.canSave(titled))
        val moved = editor.withStart(titled, LocalDateTime.parse("2026-11-07T14:52"))
        assertEquals(LocalDateTime.parse("2026-11-07T14:45"), moved.start)
        assertEquals(LocalDateTime.parse("2026-11-07T15:45"), moved.end)
        // The end never goes before the start.
        assertEquals(moved.start, editor.withEnd(moved, LocalDateTime.parse("2026-11-07T10:00")).end)
    }

    @Test
    fun allDayDraftsEndAtTheMidnightAfterTheLastDay_andTimedOnesKeepTheirZone() {
        val editor = EventEditor(utc, null, ms("2026-11-06T09:00"), 1L to 10L)
        val allDay = editor.withAllDay(editor.initial.copy(title = "Holiday"), true)
        val d = editor.draft(allDay)
        assertEquals(ms("2026-11-06T00:00"), d.startMs)
        assertEquals(ms("2026-11-07T00:00"), d.endMs)
        assertNull(d.zone)
        val timed = editor.draft(editor.initial.copy(title = " Planning "))
        assertEquals("Planning", timed.title)
        assertEquals("Z", timed.zone)
    }

    @Test
    fun editingNeedsAChange_andAnAllDayEventShowsItsLastDay() {
        val e = FakeCalendarBackend.data(1, 7, "Conference", ms("2026-11-13T00:00"), ms("2026-11-15T00:00")).copy(allDay = true)
        val editor = EventEditor(utc, e, ms("2026-11-01T00:00"), null)
        assertEquals(LocalDate.of(2026, 11, 14), editor.initial.end.toLocalDate())
        assertFalse(editor.canSave(editor.initial))
        assertTrue(editor.canSave(editor.initial.copy(notes = "Bring badges")))
        assertFalse(editor.changed(editor.initial.copy(title = "Conference ")))
        assertEquals(ms("2026-11-15T00:00"), editor.draft(editor.initial).endMs)
    }

    @Test
    fun theReminderJobPostsWhatFellDueOnce_andPlansTheNext() = runBlocking<Unit> {
        val b = FakeCalendarBackend()
        val now = ms("2026-11-06T12:00")
        b.reminderList += Reminder(1, 1, 0, now - 60_000, now + 600_000, "Standup")
        b.reminderList += Reminder(2, 1, 0, now - 3_600_000, now, "Old")
        b.reminderList += Reminder(3, 1, 0, now + 1_800_000, now + 3_600_000, "Lunch")
        val first = CalendarReminderPlan.run(b, lastRunMs = null, nowMs = now, zone = "UTC")
        // A first run posts only the last quarter hour, never an hour-old reminder.
        assertEquals(listOf(1L), first.due.map { it.eventId })
        assertEquals(now + 1_800_000, first.nextRunMs)
        val again = CalendarReminderPlan.run(b, lastRunMs = now, nowMs = now + 60_000, zone = "UTC")
        assertTrue("posted once", again.due.isEmpty())
        val none = CalendarReminderPlan.run(FakeCalendarBackend(), null, now, "UTC")
        assertEquals(now + CalendarReminderPlan.MAX_WAIT_MS, none.nextRunMs)
    }

    @Test
    fun nothingPersonalIsPrinted() {
        val printed = listOf(occ(1, "2026-11-06T12:00", "2026-11-06T13:00").copy(title = "Secret meeting"), Reminder(1, 1, 0, 0, 0, "Secret meeting"), EventDraft("Secret meeting", false, 0, 0, null, "Secret place", "", RepeatChoice.NONE, emptyList()))
        for (p in printed) assertFalse(p.toString(), p.toString().contains("Secret"))
    }
}
