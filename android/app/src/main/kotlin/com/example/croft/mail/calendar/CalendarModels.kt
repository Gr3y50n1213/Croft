// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.mail.calendar

// Titles, places, notes and people never reach a log: every class here prints ids and counts only (CLAUDE.md, security rules).

/** A calendar of an account. [color] is `#RRGGBB` when the server gives one. */
data class CalendarRef(val id: Long, val account: Long, val name: String, val color: String?, val shown: Boolean) {
    override fun toString(): String = "CalendarRef($account/$id, shown=$shown)"
}

/** One occurrence of an event in a window. Times are Unix ms; [keyMs] is the occurrence's original start. */
data class Occurrence(
    val eventId: Long,
    val account: Long,
    val calendar: Long,
    val color: String?,
    val startMs: Long,
    val endMs: Long,
    val allDay: Boolean,
    val keyMs: Long,
    val title: String,
    val location: String?,
    val repeats: Boolean,
    val repeatsUnsupported: Boolean,
    val zoneUnknown: Boolean,
    val pending: Boolean,
) {
    override fun toString(): String = "Occurrence($account/$eventId@$keyMs)"
}

/** Someone on an event, with their answer (`ACCEPTED`, `DECLINED`, `TENTATIVE`, `NEEDS-ACTION`) when known. */
data class EventPerson(val address: String, val name: String?, val status: String?) {
    override fun toString(): String = "EventPerson($status)"
}

/** How an event repeats, as S38 offers it; [CUSTOM] is a rule Croft did not write (kept as it is). */
enum class RepeatChoice { NONE, DAILY, WEEKLY, MONTHLY, YEARLY, CUSTOM }

/** One event (or one occurrence of it) for S37 and S38. */
data class EventData(
    val id: Long,
    val account: Long,
    val calendar: Long,
    /** The occurrence shown, for a repeating event; null for one that does not repeat. */
    val keyMs: Long?,
    val title: String,
    val location: String,
    val notes: String,
    val startMs: Long,
    val endMs: Long,
    val allDay: Boolean,
    val zone: String?,
    val zoneUnknown: Boolean,
    val repeat: RepeatChoice,
    val reminders: List<Long>,
    val organizer: EventPerson?,
    val attendees: List<EventPerson>,
    val status: String?,
    val conflict: Boolean,
    val pending: Boolean,
) {
    /** It repeats (Edit and Delete ask "This event / All events"). */
    val repeats: Boolean get() = keyMs != null

    override fun toString(): String = "EventData($account/$id, repeats=$repeats)"
}

/** What S38 saves. For an all-day event [startMs] is the first day's midnight and [endMs] the midnight after the last day. */
data class EventDraft(
    val title: String,
    val allDay: Boolean,
    val startMs: Long,
    val endMs: Long,
    val zone: String?,
    val location: String,
    val notes: String,
    val repeat: RepeatChoice,
    val reminders: List<Long>,
) {
    override fun toString(): String = "EventDraft(allDay=$allDay, repeat=$repeat, ${reminders.size} reminders)"
}

/** Which occurrences a change or a delete is for. */
sealed interface EventScopeChoice {
    /** Every occurrence. */
    data object All : EventScopeChoice

    /** Only the occurrence whose original start is [keyMs]. */
    data class One(val keyMs: Long) : EventScopeChoice
}

/** A reminder to post. */
data class Reminder(val eventId: Long, val account: Long, val keyMs: Long, val atMs: Long, val startMs: Long, val title: String) {
    override fun toString(): String = "Reminder($account/$eventId@$atMs)"
}

/** The occurrences in a window; [truncated] when some were left out. */
data class EventsInWindow(val occurrences: List<Occurrence>, val truncated: Boolean)

/** How a calendar sync of one account went. */
enum class CalendarSync { DONE, NOT_OFFERED, UNREACHABLE, SIGN_IN_NEEDED }

/** The calendar calls of the core (FFI v20). [zone] is the device's IANA zone. */
interface CalendarBackend {
    suspend fun sync(account: Long): CalendarSync
    suspend fun calendars(account: Long): List<CalendarRef>
    suspend fun setShown(account: Long, id: Long, shown: Boolean): Boolean
    suspend fun between(fromMs: Long, toMs: Long, zone: String): EventsInWindow
    suspend fun get(account: Long, id: Long, keyMs: Long?, zone: String): EventData?
    suspend fun save(account: Long, id: Long?, calendar: Long, draft: EventDraft, scope: EventScopeChoice, zone: String): Long
    suspend fun delete(account: Long, id: Long, scope: EventScopeChoice, zone: String): Boolean
    suspend fun reminders(fromMs: Long, toMs: Long, zone: String): List<Reminder>
}
