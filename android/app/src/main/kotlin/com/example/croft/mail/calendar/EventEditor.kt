// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.mail.calendar

import java.time.Duration
import java.time.Instant
import java.time.LocalDate
import java.time.LocalDateTime
import java.time.LocalTime
import java.time.ZoneId

/**
 * What S38's fields hold. Timed events have [start] and [end] as wall times in [zone]; all-day ones use their dates only, with
 * [end] the last day (inclusive, as the person sees it).
 */
data class EditorState(
    val title: String,
    val allDay: Boolean,
    val start: LocalDateTime,
    val end: LocalDateTime,
    val repeat: RepeatChoice,
    val reminders: List<Long>,
    val calendar: Long?,
    val account: Long?,
    val location: String,
    val notes: String,
) {
    override fun toString(): String = "EditorState(allDay=$allDay, repeat=$repeat, ${reminders.size} reminders)"
}

/** S38's rules: the starting fields, the 15-minute steps, the length kept when the start moves, Save, and the draft to save. */
class EventEditor(private val zone: ZoneId, private val original: EventData?, private val nowMs: Long, defaultCalendar: Pair<Long, Long>?) {
    /** The fields as S38 opens: the event's, or a new one at the next quarter hour for an hour. */
    val initial: EditorState = original?.let { e ->
        val start = LocalDateTime.ofInstant(Instant.ofEpochMilli(e.startMs), zone)
        val endRaw = LocalDateTime.ofInstant(Instant.ofEpochMilli(e.endMs), zone)
        // An all-day event's end is the midnight after its last day.
        val end = if (e.allDay) endRaw.minusDays(1).coerceAtLeast(start) else endRaw
        EditorState(e.title, e.allDay, start, end, e.repeat, e.reminders.sorted(), e.calendar, e.account, e.location, e.notes)
    } ?: run {
        val start = snap(LocalDateTime.ofInstant(Instant.ofEpochMilli(nowMs), zone).plusMinutes(STEP_MINUTES - 1))
        EditorState("", false, start, start.plusHours(1), RepeatChoice.NONE, emptyList(), defaultCalendar?.second, defaultCalendar?.first, "", "")
    }

    /** Whether [s] differs from [initial] in anything that would be saved. */
    fun changed(s: EditorState): Boolean = normalized(s) != normalized(initial)

    /** Save is enabled: there is a title, a calendar, the end is not before the start, and something changed (or it is new). */
    fun canSave(s: EditorState): Boolean =
        s.title.isNotBlank() && s.calendar != null && s.account != null && !s.end.isBefore(s.start) && (original == null || changed(s))

    /** A new start: the end moves with it, so the length stays. */
    fun withStart(s: EditorState, start: LocalDateTime): EditorState {
        val newStart = if (s.allDay) start.toLocalDate().atStartOfDay() else snap(start)
        val length = Duration.between(s.start, s.end)
        return s.copy(start = newStart, end = newStart.plus(length))
    }

    /** A new end (never before the start). */
    fun withEnd(s: EditorState, end: LocalDateTime): EditorState {
        val newEnd = if (s.allDay) end.toLocalDate().atStartOfDay() else snap(end)
        return s.copy(end = newEnd.coerceAtLeast(s.start))
    }

    /** All day on or off: on keeps the dates; off puts the event at the next quarter hour of its first day for an hour. */
    fun withAllDay(s: EditorState, allDay: Boolean): EditorState = when {
        allDay == s.allDay -> s
        allDay -> s.copy(allDay = true, start = s.start.toLocalDate().atStartOfDay(), end = s.end.toLocalDate().atStartOfDay())
        else -> {
            val start = s.start.toLocalDate().atTime(defaultTime())
            s.copy(allDay = false, start = start, end = start.plusHours(1))
        }
    }

    /** The draft to save. */
    fun draft(s: EditorState): EventDraft {
        val (startMs, endMs) = if (s.allDay) {
            s.start.toLocalDate().atStartOfDay(zone).toInstant().toEpochMilli() to
                s.end.toLocalDate().plusDays(1).atStartOfDay(zone).toInstant().toEpochMilli()
        } else {
            s.start.atZone(zone).toInstant().toEpochMilli() to s.end.atZone(zone).toInstant().toEpochMilli()
        }
        return EventDraft(
            title = s.title.trim(), allDay = s.allDay, startMs = startMs, endMs = endMs,
            // A timed event keeps the zone it was written in; a new one takes the device's.
            zone = if (s.allDay) null else (original?.zone ?: zone.id),
            location = s.location.trim(), notes = s.notes.trim(), repeat = s.repeat, reminders = s.reminders.distinct().sorted(),
        )
    }

    private fun defaultTime(): LocalTime = snap(LocalDateTime.ofInstant(Instant.ofEpochMilli(nowMs), zone).plusMinutes(STEP_MINUTES - 1)).toLocalTime()

    private fun normalized(s: EditorState) = s.copy(title = s.title.trim(), location = s.location.trim(), notes = s.notes.trim(), reminders = s.reminders.distinct().sorted())

    companion object {
        /** S38's time step. */
        const val STEP_MINUTES = 15L

        /** The reminder choices, in minutes before the start. */
        val REMINDER_CHOICES: List<Long> = listOf(0L, 5L, 10L, 15L, 30L, 60L, 1440L)

        /** [t] down to its quarter hour, seconds dropped. */
        fun snap(t: LocalDateTime): LocalDateTime = t.withSecond(0).withNano(0).withMinute(((t.minute / STEP_MINUTES) * STEP_MINUTES).toInt())

        /** The date of [ms] in [zone]. */
        fun dateOf(ms: Long, zone: ZoneId): LocalDate = Instant.ofEpochMilli(ms).atZone(zone).toLocalDate()
    }
}
