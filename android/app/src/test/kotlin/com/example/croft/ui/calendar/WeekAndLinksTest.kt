// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.ui.calendar

import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.semantics.SemanticsActions
import androidx.compose.ui.test.hasTestTag
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollTo
import androidx.compose.ui.test.performSemanticsAction
import androidx.compose.ui.test.performTextInput
import androidx.compose.ui.test.performTouchInput
import com.example.croft.CroftRobolectricRunner
import com.example.croft.haptics.HapticEvent
import com.example.croft.haptics.RecordingHaptics
import com.example.croft.mail.calendar.CalendarAccount
import com.example.croft.mail.calendar.CalendarController
import com.example.croft.mail.calendar.CalendarView
import com.example.croft.mail.calendar.EventScopeChoice
import com.example.croft.mail.calendar.LinkFailed
import com.example.croft.mail.calendar.LinkProblem
import com.example.croft.mail.calendar.Occurrence
import com.example.croft.mail.calendar.RepeatChoice
import com.example.croft.mail.calendar.WeekLayout
import com.example.croft.theme.AnimationLevel
import com.example.croft.ui.ScreenshotKit
import com.example.croft.ui.components.show
import java.time.LocalDate
import java.time.LocalDateTime
import java.time.ZoneOffset
import kotlinx.coroutines.runBlocking
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.annotation.Config

/** T39c: S35's layout and gestures, moving a series, read-only events, and S39's calendar links. Made-up events. */
@RunWith(CroftRobolectricRunner::class)
@Config(sdk = [35])
class WeekAndLinksTest {
    @get:Rule
    val rule = createComposeRule()

    private val utc = ZoneOffset.UTC
    private fun ms(s: String) = LocalDateTime.parse(s).toInstant(utc).toEpochMilli()
    private val now = ms("2026-11-06T09:00")
    private val day = LocalDate.of(2026, 11, 6)
    private fun occ(id: Long, start: String, end: String, repeats: Boolean = false, allDay: Boolean = false) =
        Occurrence(id, 1, 10, null, ms(start), ms(end), allDay, ms(start), "E$id", null, repeats, false, false, false)

    // ---- Pure rules ----

    @Test
    fun overlappingEventsShareTheWidth_andOthersTakeAllOfIt() {
        val placed = WeekLayout.place(
            listOf(occ(1, "2026-11-06T12:00", "2026-11-06T13:00"), occ(2, "2026-11-06T12:30", "2026-11-06T13:30"), occ(3, "2026-11-06T15:00", "2026-11-06T16:00"), occ(4, "2026-11-06T09:00", "2026-11-06T09:00")),
            day, utc,
        ).associateBy { it.occurrence.eventId }
        assertEquals(listOf(0, 2), listOf(placed.getValue(1).column, placed.getValue(1).columns))
        assertEquals(listOf(1, 2), listOf(placed.getValue(2).column, placed.getValue(2).columns))
        assertEquals(1, placed.getValue(3).columns)
        // A zero-length event still gets a 15-minute block.
        assertEquals(9 * 60 + WeekLayout.MIN_BLOCK_MINUTES, placed.getValue(4).endMinute)
        // An event across midnight is clipped to each day.
        val late = occ(5, "2026-11-06T23:00", "2026-11-07T01:00")
        assertEquals(WeekLayout.DAY_MINUTES, WeekLayout.place(listOf(late), day, utc).single().endMinute)
        assertEquals(60, WeekLayout.place(listOf(late), day.plusDays(1), utc).single().endMinute)
    }

    @Test
    fun dragsSnapToQuarterHours_andMoveAcrossDays() {
        assertEquals(615, WeekLayout.snapDown(629))
        assertEquals(2, WeekLayout.steps(37f))
        assertEquals(-1, WeekLayout.steps(-10f))
        val o = occ(1, "2026-11-06T12:00", "2026-11-06T13:00")
        assertEquals(ms("2026-11-07T12:30"), WeekLayout.movedStart(o, 2, 1, utc))
    }

    @Test
    fun movingAnOccurrenceOfASeriesMovesThatOneOrTheWholeSeriesByTheSameAmount() = runBlocking<Unit> {
        val b = FakeCalendarBackend().apply {
            calendar(1, 10, "Sam")
            event(FakeCalendarBackend.data(1, 1, "Standup", ms("2026-11-02T09:00"), ms("2026-11-02T09:15"), RepeatChoice.WEEKLY))
        }
        val c = CalendarController(b, { listOf(CalendarAccount(1, "Sam")) }, kotlinx.coroutines.CoroutineScope(kotlinx.coroutines.Dispatchers.Unconfined), { utc }, { now })
        val third = occ(1, "2026-11-16T09:00", "2026-11-16T09:15", repeats = true)
        assertTrue(c.move(third, ms("2026-11-16T10:00"), EventScopeChoice.All))
        // The first occurrence's times, moved by the same hour.
        assertEquals(ms("2026-11-02T10:00"), b.drafts.last().startMs)
        assertEquals(ms("2026-11-02T10:15"), b.drafts.last().endMs)
        assertEquals(EventScopeChoice.All, b.saved.last().third)
    }

    // ---- S35 ----

    private lateinit var controller: CalendarController
    private var newAt: LocalDateTime? = null
    private var moveRequested: Occurrence? = null

    private fun openWeek(b: FakeCalendarBackend, haptics: RecordingHaptics = RecordingHaptics()) {
        ScreenshotKit.configure(360, 1600, false, 1.0f)
        rule.show(AnimationLevel.OFF, haptics) {
            val scope = rememberCoroutineScope()
            val c = remember { CalendarController(b, { listOf(CalendarAccount(1, "Sam")) }, scope, { utc }, { now }).also { it.setView(CalendarView.WEEK); it.start() } }
            controller = c
            CalendarScreen(c, onOpen = {}, onNew = null, onCalendars = {}, onMail = {}, onContacts = null, zone = utc, onNewAt = { newAt = it }, onMoveRequest = { moveRequested = it })
        }
        rule.waitUntil(10_000) { ::controller.isInitialized && controller.state.value.syncedOnce }
        rule.waitForIdle()
    }

    @Test
    fun theWeekShowsThreeDaysTheTimeLineAndOpensTheEditorOnAnEmptySlot() {
        val b = FakeCalendarBackend().apply { calendar(1, 10, "Sam"); occurrence(1, 1, "Lunch", ms("2026-11-06T12:00"), ms("2026-11-06T13:00")) }
        openWeek(b)
        assertEquals(3, controller.state.value.weekDays)
        rule.onNodeWithTag(WeekTags.day(day.plusDays(2))).assertExists()
        rule.onNodeWithTag(WeekTags.NOW, useUnmergedTree = true).assertExists()
        rule.onNodeWithTag(WeekTags.block(1, ms("2026-11-06T12:00"))).assertExists()
        // 56 dp an hour at mdpi: y = 14.6 hours is 14:30 (rounded down to the quarter).
        rule.onNodeWithTag(WeekTags.day(day.plusDays(1))).performTouchInput { click(Offset(centerX, 56f * 14.6f)) }
        assertEquals(LocalDateTime.parse("2026-11-07T14:30"), newAt)
        rule.onNodeWithTag(WeekTags.NEXT).performClick()
        rule.waitUntil(5_000) { controller.state.value.weekStart == day.plusDays(3) }
    }

    @Test
    fun aLongPressDragMovesInQuarterHoursWithTicks_andARepeatingEventAsksFirst() {
        val b = FakeCalendarBackend().apply {
            calendar(1, 10, "Sam")
            occurrence(1, 1, "Standup", ms("2026-11-06T09:00"), ms("2026-11-06T09:15"), repeats = true)
            event(FakeCalendarBackend.data(1, 1, "Standup", ms("2026-11-06T09:00"), ms("2026-11-06T09:15"), RepeatChoice.WEEKLY, key = ms("2026-11-06T09:00")))
        }
        val haptics = RecordingHaptics()
        openWeek(b, haptics)
        rule.onNodeWithTag(WeekTags.block(1, ms("2026-11-06T09:00"))).performTouchInput {
            down(center)
            advanceEventTime(1_000)
            moveBy(Offset(0f, 28f))
            moveBy(Offset(0f, 28f))
            up()
        }
        rule.waitForIdle()
        assertEquals(HapticEvent.DragStart, haptics.events.first())
        assertTrue(haptics.events.count { it == HapticEvent.Tick } >= 2)
        rule.onNodeWithTag(CalendarTags.MOVE_ONE).performClick()
        rule.waitUntil(5_000) { b.saved.isNotEmpty() }
        // Two half hours: 56 dp = 60 minutes, so 56 px is four steps.
        assertEquals(EventScopeChoice.One(ms("2026-11-06T09:00")), b.saved.single().third)
        assertEquals(ms("2026-11-06T10:00"), b.drafts.single().startMs)
    }

    @Test
    fun screenReadersMoveThroughTheEditor() {
        val b = FakeCalendarBackend().apply { calendar(1, 10, "Sam"); occurrence(1, 1, "Lunch", ms("2026-11-06T12:00"), ms("2026-11-06T13:00")) }
        openWeek(b)
        val node = rule.onNodeWithTag(WeekTags.block(1, ms("2026-11-06T12:00"))).fetchSemanticsNode()
        val action = node.config[androidx.compose.ui.semantics.SemanticsProperties.CustomActions].single { it.label == "Move" }
        rule.runOnIdle { action.action() }
        assertEquals(1L, moveRequested?.eventId)
    }

    // ---- Read-only events and S39 ----

    @Test
    fun aReadOnlyEventHasNoEditOrDelete() {
        val b = FakeCalendarBackend().apply {
            calendar(1, 900, "Holidays")
            event(FakeCalendarBackend.data(1, 3, "Christmas", ms("2026-12-25T00:00"), ms("2026-12-26T00:00"), calendar = 900).copy(readOnly = true))
        }
        ScreenshotKit.configure(360, 1600, false, 1.0f)
        rule.show(AnimationLevel.OFF) { EventScreen(b, 1, 3, null, b.cals[1L].orEmpty(), onBack = {}, onEdit = { _, _ -> }, onDeleted = {}, zone = utc) }
        rule.waitForIdle()
        rule.onNodeWithTag(EventTags.TITLE).assertExists()
        assertTrue(rule.onAllNodes(hasTestTag(EventTags.EDIT)).fetchSemanticsNodes().isEmpty())
        assertTrue(rule.onAllNodes(hasTestTag(EventTags.DELETE)).fetchSemanticsNodes().isEmpty())
    }

    private fun openCalendars(b: FakeCalendarBackend) {
        ScreenshotKit.configure(360, 1600, false, 1.0f)
        rule.show(AnimationLevel.OFF) {
            val scope = rememberCoroutineScope()
            val c = remember { CalendarController(b, { listOf(CalendarAccount(1, "Sam")) }, scope, { utc }, { now }).also { it.start() } }
            controller = c
            CalendarsScreen(c, onBack = {})
        }
        rule.waitUntil(10_000) { ::controller.isInitialized && controller.state.value.syncedOnce }
        rule.waitForIdle()
    }

    @Test
    fun aLinkIsAddedFromTheDialog_andItsProblemsAreSaid() {
        val b = FakeCalendarBackend().apply { calendar(1, 10, "Sam"); linkResult = Result.failure(LinkFailed(LinkProblem.NOT_A_CALENDAR)) }
        openCalendars(b)
        rule.onNodeWithTag(CalendarsTags.SUBSCRIBE).performScrollTo().performClick()
        rule.onNodeWithTag(CalendarsTags.LINK_URL).performTextInput("webcal://cal.example.org/holidays.ics")
        rule.onNodeWithTag(CalendarsTags.LINK_ADD).performClick()
        rule.waitUntil(5_000) { rule.onAllNodes(hasText("This link doesn't lead to a calendar file.")).fetchSemanticsNodes().isNotEmpty() }
        b.linkResult = Result.success(900L)
        rule.onNodeWithTag(CalendarsTags.LINK_NAME).performTextInput("Holidays")
        rule.onNodeWithTag(CalendarsTags.LINK_ADD).performClick()
        rule.waitUntil(5_000) { rule.onAllNodes(hasTestTag(CalendarsTags.remove(900))).fetchSemanticsNodes().isNotEmpty() }
        assertEquals(Triple(1L, "webcal://cal.example.org/holidays.ics", "Holidays"), b.subscribed.last())
        // Remove asks first.
        rule.onNodeWithTag(CalendarsTags.remove(900)).performClick()
        rule.onNodeWithText("Remove this calendar?").assertExists()
        rule.onNodeWithText("Remove").performClick()
        rule.waitUntil(5_000) { b.unsubscribed == listOf(900L) }
    }
}
