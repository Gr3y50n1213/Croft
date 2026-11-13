// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.ui.mail

import com.example.croft.notify.SnoozeSchedule
import java.time.LocalDateTime
import java.time.ZoneId
import java.util.Locale
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/** T36: S29's presets by time of day and weekday, a picked date and time, the labels, and the wake job's delay. */
class SnoozePresetsTest {
    private val utc = ZoneId.of("UTC")

    private fun ms(text: String, zone: ZoneId = utc) = LocalDateTime.parse(text).atZone(zone).toInstant().toEpochMilli()

    private fun presets(now: String, zone: ZoneId = utc) =
        SnoozePresets.presets(ms(now, zone), zone).associate { it.kind to LocalDateTime.ofInstant(java.time.Instant.ofEpochMilli(it.atMs), zone).toString() }

    @Test
    fun aFridayMorning_laterTodayIsSixInTheEvening_andTheWeekendIsTomorrow() {
        val p = presets("2027-01-15T08:00")
        assertEquals("2027-01-15T18:00", p[SnoozePresets.Kind.LATER_TODAY])
        assertEquals("2027-01-16T08:00", p[SnoozePresets.Kind.TOMORROW])
        assertEquals("2027-01-16T09:00", p[SnoozePresets.Kind.THIS_WEEKEND])
        assertEquals("2027-01-18T08:00", p[SnoozePresets.Kind.NEXT_WEEK])
        assertEquals("in the sheet's order", SnoozePresets.Kind.entries, SnoozePresets.presets(ms("2027-01-15T08:00"), utc).map { it.kind })
    }

    @Test
    fun fromThreeInTheAfternoon_laterTodayIsInThreeHours_evenPastMidnight() {
        assertEquals("2027-01-13T14:59 is before the cutoff", "2027-01-13T18:00", presets("2027-01-13T14:59")[SnoozePresets.Kind.LATER_TODAY])
        assertEquals("2027-01-13T18:00", presets("2027-01-13T15:00")[SnoozePresets.Kind.LATER_TODAY])
        assertEquals("2027-01-13T19:20", presets("2027-01-13T16:20:45")[SnoozePresets.Kind.LATER_TODAY])
        assertEquals("2027-01-14T01:30", presets("2027-01-13T22:30")[SnoozePresets.Kind.LATER_TODAY])
    }

    @Test
    fun onAWeekend_thisWeekendIsLeftOut_andNextWeekIsTheComingMonday() {
        for (day in listOf("2027-01-16T10:00", "2027-01-17T10:00")) {
            val p = presets(day)
            assertNull("no This weekend on $day", p[SnoozePresets.Kind.THIS_WEEKEND])
            assertEquals("2027-01-18T08:00", p[SnoozePresets.Kind.NEXT_WEEK])
        }
        assertEquals("on a Monday, next week is the Monday after", "2027-01-25T08:00", presets("2027-01-18T07:00")[SnoozePresets.Kind.NEXT_WEEK])
        assertEquals("2027-01-23T09:00", presets("2027-01-18T07:00")[SnoozePresets.Kind.THIS_WEEKEND])
    }

    @Test
    fun theTimesAreTheLocalClocks_acrossADaylightSavingChange() {
        // New York moves its clocks forward on Sunday 14 March 2027: Monday 08:00 is still 08:00 there.
        val ny = ZoneId.of("America/New_York")
        val p = presets("2027-03-12T09:00", ny)
        assertEquals("2027-03-13T09:00", p[SnoozePresets.Kind.THIS_WEEKEND])
        assertEquals("2027-03-15T08:00", p[SnoozePresets.Kind.NEXT_WEEK])
        for (kind in SnoozePresets.Kind.entries) assertTrue("$kind is ahead", SnoozePresets.presets(ms("2027-03-12T09:00", ny), ny).first { it.kind == kind }.atMs > ms("2027-03-12T09:00", ny))
    }

    @Test
    fun aPickedDayAndTime_isThatTimeInTheZone() {
        val berlin = ZoneId.of("Europe/Berlin")
        val dayUtc = ms("2027-01-20T00:00")
        assertEquals(ms("2027-01-20T07:30", berlin), SnoozePresets.picked(dayUtc, 7, 30, berlin))
        assertEquals(ms("2027-01-15T00:00"), SnoozePresets.todayUtcMs(ms("2027-01-15T23:59"), utc))
        assertEquals("the date picker's day is the local day", ms("2027-01-16T00:00"), SnoozePresets.todayUtcMs(ms("2027-01-15T23:30"), berlin))
    }

    @Test
    fun theLabelIsTheWeekdayAndTime_withTheDateWhenItIsMoreThanAWeekAway() {
        val now = ms("2027-01-15T08:00")
        val near = SnoozePresets.label(ms("2027-01-16T09:00"), now, utc, Locale.US)
        assertTrue(near, near.startsWith("Sat ") && near.contains("9:00"))
        val far = SnoozePresets.label(ms("2027-02-01T08:00"), now, utc, Locale.US)
        assertTrue(far, far.startsWith("Feb 1, 2027"))
    }

    @Test
    fun theWakeJobWaitsUntilTheSoonestTime_atOnceWhenItIsDue_andNotAtAllWithNothingSnoozed() {
        assertEquals(60_000L, SnoozeSchedule.delayMs(1_060_000L, 1_000_000L))
        assertEquals(0L, SnoozeSchedule.delayMs(999_000L, 1_000_000L))
        assertNull(SnoozeSchedule.delayMs(null, 1_000_000L))
    }
}
