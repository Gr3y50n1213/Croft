// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.ui.calendar

import com.example.croft.mail.calendar.CalendarBackend
import com.example.croft.mail.calendar.CalendarRef
import com.example.croft.mail.calendar.CalendarSync
import com.example.croft.mail.calendar.EventData
import com.example.croft.mail.calendar.EventDraft
import com.example.croft.mail.calendar.EventScopeChoice
import com.example.croft.mail.calendar.EventsInWindow
import com.example.croft.mail.calendar.Occurrence
import com.example.croft.mail.calendar.Reminder
import com.example.croft.mail.calendar.RepeatChoice

/**
 * An in-memory [CalendarBackend]: occurrences and event details set by the test, calendars by account, scripted sync results, and
 * what was saved and deleted. Made-up events only.
 */
class FakeCalendarBackend : CalendarBackend {
    val occurrences = ArrayList<Occurrence>()
    val events = HashMap<Pair<Long, Long>, EventData>()
    val cals = HashMap<Long, MutableList<CalendarRef>>()
    val syncResults = HashMap<Long, CalendarSync>()
    val synced = ArrayList<Long>()
    val saved = ArrayList<Triple<Long, Long?, EventScopeChoice>>()
    val drafts = ArrayList<EventDraft>()
    val deleted = ArrayList<Pair<Long, EventScopeChoice>>()
    val reminderList = ArrayList<Reminder>()
    var nextId = 500L

    fun calendar(account: Long, id: Long, name: String, color: String? = null, shown: Boolean = true) {
        cals.getOrPut(account) { mutableListOf() }.add(CalendarRef(id, account, name, color, shown))
    }

    fun occurrence(account: Long, id: Long, title: String, startMs: Long, endMs: Long, allDay: Boolean = false, calendar: Long = 10, key: Long = startMs, repeats: Boolean = false) {
        occurrences += Occurrence(id, account, calendar, null, startMs, endMs, allDay, key, title, null, repeats, false, false, false)
    }

    fun event(e: EventData) { events[e.account to e.id] = e }

    override suspend fun sync(account: Long): CalendarSync { synced += account; return syncResults[account] ?: CalendarSync.DONE }
    override suspend fun calendars(account: Long): List<CalendarRef> = cals[account].orEmpty()
    override suspend fun setShown(account: Long, id: Long, shown: Boolean): Boolean {
        val list = cals[account] ?: return false
        val i = list.indexOfFirst { it.id == id }
        if (i < 0) return false
        list[i] = list[i].copy(shown = shown)
        return true
    }
    override suspend fun between(fromMs: Long, toMs: Long, zone: String): EventsInWindow {
        val hidden = cals.values.flatten().filter { !it.shown }.map { it.account to it.id }.toSet()
        return EventsInWindow(occurrences.filter { it.startMs < toMs && (it.endMs > fromMs || it.startMs >= fromMs) && (it.account to it.calendar) !in hidden }, false)
    }
    override suspend fun get(account: Long, id: Long, keyMs: Long?, zone: String): EventData? = events[account to id]
    override suspend fun save(account: Long, id: Long?, calendar: Long, draft: EventDraft, scope: EventScopeChoice, zone: String): Long {
        saved += Triple(account, id, scope)
        drafts += draft
        return id ?: nextId++
    }
    override suspend fun delete(account: Long, id: Long, scope: EventScopeChoice, zone: String): Boolean {
        deleted += id to scope
        return events.remove(account to id) != null
    }
    override suspend fun reminders(fromMs: Long, toMs: Long, zone: String): List<Reminder> = reminderList.filter { it.atMs in fromMs until toMs }.sortedBy { it.atMs }

    companion object {
        fun data(account: Long, id: Long, title: String, startMs: Long, endMs: Long, repeat: RepeatChoice = RepeatChoice.NONE, key: Long? = null, calendar: Long = 10) = EventData(
            id = id, account = account, calendar = calendar, keyMs = key, title = title, location = "", notes = "", startMs = startMs, endMs = endMs,
            allDay = false, zone = "UTC", zoneUnknown = false, repeat = repeat, reminders = emptyList(), organizer = null, attendees = emptyList(),
            status = null, conflict = false, pending = false,
        )
    }
}
