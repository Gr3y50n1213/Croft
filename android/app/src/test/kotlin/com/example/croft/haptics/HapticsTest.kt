// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.haptics

import com.example.croft.theme.HapticsLevel
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

/** The `Haptics` rules of docs/ux/haptics.md: the level filter, the 40 ms guard and the recording fake. */
class HapticsTest {
    private var now = 1_000L
    private val recording = RecordingHaptics()
    private fun at(level: HapticsLevel) = LevelFilteredHaptics(level = { level }, clockMs = { now }, next = recording)

    /** Every event, spaced far apart so the guard never interferes. */
    private fun playAll(h: Haptics) {
        for (e in HapticEvent.entries) { h.perform(e); now += 1_000 }
    }

    @Test
    fun theInterfaceHasTheTwelveSemanticEvents() {
        assertEquals(
            listOf(
                "ThresholdArmed", "ThresholdDisarmed", "Committed", "Rejected", "ToggleOn", "ToggleOff", "SelectionStart",
                "SelectionTick", "Tick", "DragStart", "Success", "Failure",
            ),
            HapticEvent.entries.map { it.name },
        )
    }

    @Test
    fun offDropsEveryEvent() {
        playAll(at(HapticsLevel.OFF))
        assertTrue(recording.events.isEmpty())
    }

    @Test
    fun subtleKeepsOnlyThreshold_errors_send_andLongPress() {
        playAll(at(HapticsLevel.SUBTLE))
        // Haptics map: arm, action not available, send accepted or failed, long-press (selection, menu, schedule), dialog confirmed.
        assertEquals(
            listOf(HapticEvent.ThresholdArmed, HapticEvent.Rejected, HapticEvent.SelectionStart, HapticEvent.Success, HapticEvent.Failure),
            recording.events,
        )
    }

    @Test
    fun fullAddsDisarmCommitTogglesSelectionTicksAndDrag() {
        playAll(at(HapticsLevel.FULL))
        assertEquals(HapticEvent.entries.toList(), recording.events)
    }

    @Test
    fun theLevelCanChangeWhileRunning() {
        var level = HapticsLevel.OFF
        val h = LevelFilteredHaptics({ level }, { now }, recording)
        h.perform(HapticEvent.ToggleOn)
        level = HapticsLevel.FULL
        now += 1_000
        h.perform(HapticEvent.ToggleOn)
        assertEquals(listOf(HapticEvent.ToggleOn), recording.events)
    }

    @Test
    fun theGuardDropsTheSameEventWithin40ms_andAllowsItAfter() {
        val h = at(HapticsLevel.FULL)
        h.perform(HapticEvent.Tick)          // t = 0, goes through
        now += 39
        h.perform(HapticEvent.Tick)          // 39 ms later: dropped
        now += 1
        h.perform(HapticEvent.Tick)          // 40 ms after the one that went through: allowed
        assertEquals(listOf(HapticEvent.Tick, HapticEvent.Tick), recording.events)
    }

    @Test
    fun aDroppedRepeatDoesNotExtendTheWindow() {
        val h = at(HapticsLevel.FULL)
        h.perform(HapticEvent.Tick)
        now += 30
        h.perform(HapticEvent.Tick) // dropped
        now += 15                   // 45 ms after the first
        h.perform(HapticEvent.Tick)
        assertEquals(2, recording.events.size)
    }

    @Test
    fun theGuardIsPerEvent_aDifferentEventIsNotDropped() {
        val h = at(HapticsLevel.FULL)
        h.perform(HapticEvent.ThresholdArmed)
        now += 1
        h.perform(HapticEvent.ThresholdDisarmed)
        assertEquals(listOf(HapticEvent.ThresholdArmed, HapticEvent.ThresholdDisarmed), recording.events)
    }

    @Test
    fun anEventTheLevelDropsDoesNotStartTheGuard() {
        val h = at(HapticsLevel.SUBTLE)
        h.perform(HapticEvent.Tick) // dropped by the level
        now += 1
        h.perform(HapticEvent.ThresholdArmed)
        assertEquals(listOf(HapticEvent.ThresholdArmed), recording.events)
    }

    @Test
    fun aSwipeThatCrossesComesBackAndCrossesAgainRecordsArmedDisarmedArmedCommitted() {
        val h = at(HapticsLevel.FULL)
        for (e in listOf(HapticEvent.ThresholdArmed, HapticEvent.ThresholdDisarmed, HapticEvent.ThresholdArmed, HapticEvent.Committed)) {
            h.perform(e)
            now += 100 // the person's finger is slower than 40 ms between moments
        }
        assertEquals(
            listOf(HapticEvent.ThresholdArmed, HapticEvent.ThresholdDisarmed, HapticEvent.ThresholdArmed, HapticEvent.Committed),
            recording.events,
        )
    }

    @Test
    fun theRecordingFakeKeepsOrderAndCanBeCleared() {
        val r = RecordingHaptics()
        r.perform(HapticEvent.Failure); r.perform(HapticEvent.Success); r.perform(HapticEvent.Failure)
        assertEquals(listOf(HapticEvent.Failure, HapticEvent.Success, HapticEvent.Failure), r.events)
        r.clear()
        assertTrue(r.events.isEmpty())
    }

    @Test
    fun theGuardIs40msFromTheTokens() {
        assertEquals(40L, SameEventGuardMs)
    }

    @Test
    fun everyEventHasALevelThatMatchesTheEventMap() {
        val subtle = HapticEvent.entries.filter { it.minLevel == HapticsLevel.SUBTLE }.map { it.name }.toSet()
        assertEquals(setOf("ThresholdArmed", "Rejected", "SelectionStart", "Success", "Failure"), subtle)
        assertTrue(HapticEvent.entries.none { it.minLevel == HapticsLevel.OFF })
    }
}
