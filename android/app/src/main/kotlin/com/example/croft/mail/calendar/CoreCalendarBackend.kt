// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.mail.calendar

import kotlinx.coroutines.CoroutineDispatcher
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import uniffi.croft_ffi.CalendarSyncResult
import uniffi.croft_ffi.Core
import uniffi.croft_ffi.EventInput
import uniffi.croft_ffi.EventScope
import uniffi.croft_ffi.RepeatPreset
import uniffi.croft_ffi.EventPerson as CorePerson

/** [CalendarBackend] over the core (FFI v20). Database calls run off the main thread; the sync is the core's own async call. */
class CoreCalendarBackend(private val core: Core, private val io: CoroutineDispatcher = Dispatchers.IO) : CalendarBackend {
    override suspend fun sync(account: Long): CalendarSync = when (core.syncCalendars(account).result) {
        CalendarSyncResult.DONE -> CalendarSync.DONE
        CalendarSyncResult.NOT_OFFERED -> CalendarSync.NOT_OFFERED
        CalendarSyncResult.UNREACHABLE -> CalendarSync.UNREACHABLE
        CalendarSyncResult.SIGN_IN_NEEDED -> CalendarSync.SIGN_IN_NEEDED
    }

    override suspend fun calendars(account: Long): List<CalendarRef> = withContext(io) {
        core.calendars(account).map { CalendarRef(it.id, it.account, it.name, it.color, it.shown) }
    }

    override suspend fun setShown(account: Long, id: Long, shown: Boolean): Boolean = withContext(io) { core.setCalendarShown(account, id, shown) }

    override suspend fun between(fromMs: Long, toMs: Long, zone: String): EventsInWindow = withContext(io) {
        val w = core.eventsBetween(fromMs, toMs, zone)
        EventsInWindow(
            w.occurrences.map {
                Occurrence(
                    it.eventId, it.account, it.calendar, it.color, it.startMs, it.endMs, it.allDay, it.keyMs, it.title, it.location,
                    it.repeats, it.repeatsUnsupported, it.zoneUnknown, it.pending,
                )
            },
            w.truncated,
        )
    }

    override suspend fun get(account: Long, id: Long, keyMs: Long?, zone: String): EventData? = withContext(io) {
        core.getEvent(account, id, keyMs, zone)?.let { d ->
            EventData(
                id = d.id, account = d.account, calendar = d.calendar, keyMs = d.keyMs, title = d.title, location = d.location,
                notes = d.notes, startMs = d.startMs, endMs = d.endMs, allDay = d.allDay, zone = d.zone, zoneUnknown = d.zoneUnknown,
                repeat = d.repeat.toApp(), reminders = d.reminders, organizer = d.organizer?.toApp(), attendees = d.attendees.map { it.toApp() },
                status = d.status, conflict = d.conflict, pending = d.pending,
            )
        }
    }

    override suspend fun save(account: Long, id: Long?, calendar: Long, draft: EventDraft, scope: EventScopeChoice, zone: String): Long = withContext(io) {
        core.saveEvent(account, id, calendar, draft.toInput(), scope.toCore(), zone)
    }

    override suspend fun delete(account: Long, id: Long, scope: EventScopeChoice, zone: String): Boolean = withContext(io) {
        core.deleteEvent(account, id, scope.toCore(), zone)
    }

    override suspend fun reminders(fromMs: Long, toMs: Long, zone: String): List<Reminder> = withContext(io) {
        core.nextReminders(fromMs, toMs, zone).map { Reminder(it.eventId, it.account, it.keyMs, it.atMs, it.startMs, it.title) }
    }

    companion object {
        internal fun RepeatPreset.toApp(): RepeatChoice = when (this) {
            RepeatPreset.NONE -> RepeatChoice.NONE
            RepeatPreset.DAILY -> RepeatChoice.DAILY
            RepeatPreset.WEEKLY -> RepeatChoice.WEEKLY
            RepeatPreset.MONTHLY -> RepeatChoice.MONTHLY
            RepeatPreset.YEARLY -> RepeatChoice.YEARLY
            RepeatPreset.CUSTOM -> RepeatChoice.CUSTOM
        }

        internal fun RepeatChoice.toCore(): RepeatPreset = when (this) {
            RepeatChoice.NONE -> RepeatPreset.NONE
            RepeatChoice.DAILY -> RepeatPreset.DAILY
            RepeatChoice.WEEKLY -> RepeatPreset.WEEKLY
            RepeatChoice.MONTHLY -> RepeatPreset.MONTHLY
            RepeatChoice.YEARLY -> RepeatPreset.YEARLY
            RepeatChoice.CUSTOM -> RepeatPreset.CUSTOM
        }

        private fun CorePerson.toApp() = EventPerson(address, name, status)

        private fun EventDraft.toInput() = EventInput(
            title = title, allDay = allDay, startMs = startMs, endMs = endMs, zone = zone, location = location, notes = notes,
            repeat = repeat.toCore(), reminders = reminders,
        )

        private fun EventScopeChoice.toCore(): EventScope = when (this) {
            EventScopeChoice.All -> EventScope.All
            is EventScopeChoice.One -> EventScope.One(keyMs)
        }
    }
}
