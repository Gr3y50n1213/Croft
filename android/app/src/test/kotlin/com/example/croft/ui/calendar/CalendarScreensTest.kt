// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.ui.calendar

import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.ui.test.assertIsEnabled
import androidx.compose.ui.test.assertIsNotEnabled
import androidx.compose.ui.test.hasTestTag
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollTo
import androidx.compose.ui.test.performTextInput
import com.example.croft.CroftRobolectricRunner
import com.example.croft.mail.calendar.CalendarAccount
import com.example.croft.mail.calendar.CalendarController
import com.example.croft.mail.calendar.CalendarSync
import com.example.croft.mail.calendar.EventData
import com.example.croft.mail.calendar.EventScopeChoice
import com.example.croft.mail.calendar.Occurrence
import com.example.croft.mail.calendar.RepeatChoice
import com.example.croft.theme.AnimationLevel
import com.example.croft.ui.ScreenshotKit
import com.example.croft.ui.components.show
import java.time.LocalDate
import java.time.LocalDateTime
import java.time.ZoneOffset
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.annotation.Config

/** S34, S36, S37, S38 and S39 (Robolectric, in-memory backend, UTC). Made-up events. */
@RunWith(CroftRobolectricRunner::class)
@Config(sdk = [35])
class CalendarScreensTest {
    @get:Rule
    val rule = createComposeRule()

    private val utc = ZoneOffset.UTC
    private fun ms(s: String) = LocalDateTime.parse(s).toInstant(utc).toEpochMilli()
    private val now = ms("2026-11-06T09:00")
    private val today = LocalDate.of(2026, 11, 6)
    private lateinit var controller: CalendarController
    private val opened = ArrayList<Occurrence>()
    private var newOn: LocalDate? = null

    private fun backend() = FakeCalendarBackend().apply {
        calendar(1, 10, "Sam", "#3A7D44")
        calendar(1, 11, "Birthdays")
        occurrence(1, 1, "Lunch with Sam", ms("2026-11-06T12:00"), ms("2026-11-06T13:00"))
        occurrence(1, 2, "Standup", ms("2026-11-06T12:00"), ms("2026-11-06T12:30"))
        occurrence(1, 3, "Bank holiday", ms("2026-11-09T00:00"), ms("2026-11-10T00:00"), allDay = true)
        occurrence(1, 4, "Birthday", ms("2026-11-11T00:00"), ms("2026-11-12T00:00"), allDay = true, calendar = 11)
    }

    private fun openCalendar(b: FakeCalendarBackend) {
        ScreenshotKit.configure(360, 1600, false, 1.0f)
        rule.show(AnimationLevel.OFF) {
            val scope = rememberCoroutineScope()
            val c = remember { CalendarController(b, { listOf(CalendarAccount(1, "Sam"), CalendarAccount(2, "Sample", isDemo = true)) }, scope, { utc }, { now }).also { it.start() } }
            controller = c
            CalendarScreen(c, onOpen = { opened += it }, onNew = { newOn = it }, onCalendars = {}, onMail = {}, onContacts = null, zone = utc)
        }
        rule.waitUntil(10_000) { ::controller.isInitialized && controller.state.value.syncedOnce }
        rule.waitForIdle()
    }

    @Test
    fun theAgendaGroupsByDayMarksOverlapsAndOpensEvents() {
        val b = backend()
        openCalendar(b)
        assertEquals(listOf(1L), b.synced)
        rule.onNodeWithTag(CalendarTags.day(today)).assertExists()
        rule.onNodeWithTag(CalendarTags.day(LocalDate.of(2026, 11, 9))).assertExists()
        assertEquals(2, rule.onAllNodes(hasTestTag(CalendarTags.OVERLAP), useUnmergedTree = true).fetchSemanticsNodes().size)
        rule.onNodeWithText("Bank holiday").assertExists()
        rule.onNodeWithTag(CalendarTags.event(1, ms("2026-11-06T12:00"))).performClick()
        assertEquals(listOf(1L), opened.map { it.eventId })
        rule.onNodeWithTag(CalendarTags.NEW).performClick()
        assertEquals(today, newOn)
    }

    @Test
    fun theMonthShowsTheSelectedDaysEvents() {
        openCalendar(backend())
        rule.onNodeWithTag(CalendarTags.VIEW_MONTH).performClick()
        rule.onNodeWithTag(CalendarTags.cell(LocalDate.of(2026, 11, 9))).performClick()
        rule.waitUntil(5_000) { rule.onAllNodes(hasText("Bank holiday")).fetchSemanticsNodes().isNotEmpty() }
        assertTrue(rule.onAllNodes(hasText("Lunch with Sam")).fetchSemanticsNodes().isEmpty())
        rule.onNodeWithTag(CalendarTags.NEW).performClick()
        assertEquals(LocalDate.of(2026, 11, 9), newOn)
    }

    @Test
    fun hidingACalendarHidesItsEvents_andSyncProblemsAreNamed() {
        val b = backend().apply { syncResults[1] = CalendarSync.UNREACHABLE }
        openCalendar(b)
        rule.onNodeWithText("Couldn't reach Sam. Pull down to try again.").assertExists()
        rule.waitUntil(5_000) { rule.onAllNodes(hasText("Birthday")).fetchSemanticsNodes().isNotEmpty() }
        controller.setShown(1, 11, false)
        rule.waitUntil(5_000) { rule.onAllNodes(hasText("Birthday")).fetchSemanticsNodes().isEmpty() }
        assertTrue(controller.state.value.calendars[1L]!!.first { it.id == 11L }.shown.not())
    }

    @Test
    fun theCalendarsScreenHidesAndShowsACalendar() {
        val b = backend()
        ScreenshotKit.configure(360, 1600, false, 1.0f)
        rule.show(AnimationLevel.OFF) {
            val scope = rememberCoroutineScope()
            val c = remember { CalendarController(b, { listOf(CalendarAccount(1, "Sam")) }, scope, { utc }, { now }).also { it.start() } }
            controller = c
            CalendarsScreen(c, onBack = {})
        }
        rule.waitUntil(10_000) { ::controller.isInitialized && controller.state.value.syncedOnce }
        rule.onNodeWithText("Birthdays").assertExists()
        rule.onNodeWithTag(CalendarsTags.calendar(11)).performClick()
        rule.waitUntil(5_000) { b.cals[1L]!!.first { it.id == 11L }.shown.not() }
        rule.onNodeWithTag(CalendarsTags.calendar(11)).performClick()
        rule.waitUntil(5_000) { b.cals[1L]!!.first { it.id == 11L }.shown }
    }

    // ---- S37 ----

    private var editedWith: EventScopeChoice? = null
    private var deletedFrom: Long? = null

    private fun openEvent(b: FakeCalendarBackend, id: Long, key: Long?) {
        ScreenshotKit.configure(360, 1600, false, 1.0f)
        rule.show(AnimationLevel.OFF) {
            EventScreen(b, 1, id, key, b.cals[1L].orEmpty(), onBack = {}, onEdit = { _, s -> editedWith = s }, onDeleted = { deletedFrom = it }, zone = utc)
        }
        rule.waitForIdle()
    }

    @Test
    fun anEventShowsItsFields_andARepeatingOneAsksForTheScope() {
        val b = backend()
        val key = ms("2026-11-06T12:00")
        b.event(
            FakeCalendarBackend.data(1, 1, "Lunch with Sam", key, ms("2026-11-06T13:00"), RepeatChoice.WEEKLY, key = key).copy(
                location = "The Mill, 14 Water Lane", reminders = listOf(30),
                organizer = com.example.croft.mail.calendar.EventPerson("sam@example.org", "Sam Rivera", null),
                attendees = listOf(com.example.croft.mail.calendar.EventPerson("mira@example.org", "Mira Okafor", "ACCEPTED")),
            ),
        )
        openEvent(b, 1, key)
        rule.onNodeWithTag(EventTags.TITLE).assertExists()
        rule.onNodeWithText("The Mill, 14 Water Lane").assertExists()
        rule.onNodeWithText("Sam Rivera (organiser)").assertExists()
        rule.onNodeWithText("Mira Okafor, going").assertExists()
        rule.onNodeWithText("30 minutes before").assertExists()
        rule.onNodeWithTag(EventTags.EDIT).performClick()
        rule.onNodeWithTag(EventTags.SCOPE_ONE).performClick()
        assertEquals(EventScopeChoice.One(key), editedWith)
        // Delete asks the scope, then asks again before deleting.
        rule.onNodeWithTag(EventTags.DELETE).performClick()
        rule.onNodeWithTag(EventTags.SCOPE_ALL).performClick()
        rule.onNodeWithText("Delete this event?").assertExists()
        rule.onNodeWithText("Delete").performClick()
        rule.waitUntil(5_000) { deletedFrom != null }
        assertEquals(listOf(1L to EventScopeChoice.All), b.deleted)
    }

    @Test
    fun aGoneEventSaysSo() {
        openEvent(backend(), 99, null)
        rule.onNodeWithTag(EventTags.GONE).assertExists()
    }

    // ---- S38 ----

    private var savedAs: Pair<Long, Long>? = null
    private var closed = 0

    private fun openEditor(b: FakeCalendarBackend, id: Long?, scope: EventScopeChoice = EventScopeChoice.All) {
        ScreenshotKit.configure(360, 2000, false, 1.0f)
        rule.show(AnimationLevel.OFF) {
            EventEditorScreen(
                b, if (id == null) null else 1L, id, scope, if (id == null) LocalDate.of(2026, 11, 9) else null,
                listOf(CalendarAccount(1, "Sam")), mapOf(1L to b.cals[1L].orEmpty()),
                onSaved = { a, i -> savedAs = a to i }, onClose = { closed++ }, zone = utc, clock = { now },
            )
        }
        rule.waitForIdle()
    }

    @Test
    fun aNewEventNeedsATitle_andGoesToTheFirstCalendarOnTheChosenDay() {
        val b = backend()
        openEditor(b, null)
        rule.onNodeWithTag(EditorTags.SAVE).assertIsNotEnabled()
        rule.onNodeWithTag(EditorTags.TITLE).performTextInput("Planning")
        rule.onNodeWithTag(EditorTags.reminder(15)).performScrollTo().performClick()
        rule.onNodeWithTag(EditorTags.SAVE).assertIsEnabled().performClick()
        rule.waitUntil(5_000) { savedAs != null }
        val draft = b.drafts.single()
        assertEquals("Planning", draft.title)
        assertEquals(ms("2026-11-09T09:00"), draft.startMs)
        assertEquals(ms("2026-11-09T10:00"), draft.endMs)
        assertEquals(listOf(15L), draft.reminders)
        assertEquals(Triple(1L, null, EventScopeChoice.All), b.saved.single())
    }

    @Test
    fun aChangeToOneOccurrenceIsSavedForThatOccurrence_andClosingWithChangesAsks() {
        val b = backend()
        val key = ms("2026-11-06T12:00")
        b.event(FakeCalendarBackend.data(1, 1, "Lunch with Sam", key, ms("2026-11-06T13:00"), RepeatChoice.WEEKLY, key = key))
        openEditor(b, 1, EventScopeChoice.One(key))
        // Nothing changed: Save is off, and Close just closes.
        rule.onNodeWithTag(EditorTags.SAVE).assertIsNotEnabled()
        rule.onNodeWithTag(EditorTags.CLOSE).performClick()
        assertEquals(1, closed)
        rule.onNodeWithTag(EditorTags.LOCATION).performScrollTo().performTextInput("Café")
        rule.onNodeWithTag(EditorTags.CLOSE).performClick()
        rule.onNodeWithText("Discard changes?").assertExists()
        rule.onNodeWithText("Cancel").performClick()
        rule.onNodeWithTag(EditorTags.SAVE).performClick()
        rule.waitUntil(5_000) { savedAs != null }
        assertEquals(Triple(1L, 1L, EventScopeChoice.One(key)), b.saved.single())
        assertEquals("Café", b.drafts.single().location)
        assertEquals(RepeatChoice.WEEKLY, b.drafts.single().repeat)
    }
}
