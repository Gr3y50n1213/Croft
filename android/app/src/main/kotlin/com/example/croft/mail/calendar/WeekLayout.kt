// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.mail.calendar

import java.time.Instant
import java.time.LocalDate
import java.time.ZoneId

/** One timed event placed in a day of S35: minutes from that day's midnight, and its column among the events it overlaps. */
data class PlacedEvent(val occurrence: Occurrence, val startMinute: Int, val endMinute: Int, val column: Int, val columns: Int)

/** S35's layout rules, pure: what each day shows and where, and the 15-minute steps of a drag. */
object WeekLayout {
    /** A day in minutes. */
    const val DAY_MINUTES = 24 * 60

    /** The shortest block drawn, in minutes (a zero-length event is still tappable). */
    const val MIN_BLOCK_MINUTES = 15

    /** The step a tap or a drag snaps to. */
    const val STEP_MINUTES = 15

    /**
     * The timed events of [day] (all-day ones are listed apart), clipped to the day, each with a column: events that overlap share
     * the width, side by side, in order of start.
     */
    fun place(occurrences: List<Occurrence>, day: LocalDate, zone: ZoneId): List<PlacedEvent> {
        val dayStart = day.atStartOfDay(zone).toInstant().toEpochMilli()
        val dayEnd = day.plusDays(1).atStartOfDay(zone).toInstant().toEpochMilli()
        val items = occurrences
            .filter { !it.allDay && it.startMs < dayEnd && (it.endMs > dayStart || (it.endMs == it.startMs && it.startMs >= dayStart)) }
            .map { o ->
                val s = ((maxOf(o.startMs, dayStart) - dayStart) / 60_000L).toInt()
                val e = ((minOf(maxOf(o.endMs, o.startMs), dayEnd) - dayStart) / 60_000L).toInt()
                Triple(o, s, maxOf(e, minOf(s + MIN_BLOCK_MINUTES, DAY_MINUTES)))
            }
            .sortedWith(compareBy({ it.second }, { -it.third }, { it.first.eventId }))
        // Clusters of events that overlap one another; in each, every event takes the first free column.
        val out = ArrayList<PlacedEvent>()
        var cluster = ArrayList<Triple<Occurrence, Int, Int>>()
        var clusterEnd = -1
        fun flush() {
            val ends = ArrayList<Int>()
            val cols = IntArray(cluster.size)
            cluster.forEachIndexed { i, (_, s, e) ->
                val free = ends.indexOfFirst { it <= s }
                if (free >= 0) { cols[i] = free; ends[free] = e } else { cols[i] = ends.size; ends.add(e) }
            }
            cluster.forEachIndexed { i, (o, s, e) -> out += PlacedEvent(o, s, e, cols[i], ends.size) }
            cluster = ArrayList()
        }
        for (item in items) {
            if (cluster.isNotEmpty() && item.second >= clusterEnd) flush()
            cluster.add(item)
            clusterEnd = maxOf(clusterEnd.takeIf { cluster.size > 1 } ?: item.third, item.third)
        }
        if (cluster.isNotEmpty()) flush()
        return out
    }

    /** The all-day events that cover [day]. */
    fun allDay(occurrences: List<Occurrence>, day: LocalDate, zone: ZoneId): List<Occurrence> =
        occurrences.filter { it.allDay && day in AgendaModel.daysOf(it, zone) }

    /** [minutes] down to its 15-minute step, within the day. */
    fun snapDown(minutes: Int): Int = (minutes.coerceIn(0, DAY_MINUTES - STEP_MINUTES) / STEP_MINUTES) * STEP_MINUTES

    /** A drag of [minutes] as whole 15-minute steps (to the nearest). */
    fun steps(minutes: Float): Int = Math.round(minutes / STEP_MINUTES)

    /** Where [o] starts after a drag of [steps] 15-minute steps and [days] days. */
    fun movedStart(o: Occurrence, steps: Int, days: Int, zone: ZoneId): Long {
        val start = Instant.ofEpochMilli(o.startMs).atZone(zone)
        return start.plusDays(days.toLong()).plusMinutes(steps.toLong() * STEP_MINUTES).toInstant().toEpochMilli()
    }
}
