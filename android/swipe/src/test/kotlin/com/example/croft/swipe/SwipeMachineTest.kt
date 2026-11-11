// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.swipe

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertTrue
import org.junit.Test

/** One test for every row of the transition table in docs/ux/swipe.md, then the special cases. Names start with the row. */
class SwipeMachineTest {
    // ---- The table ----

    @Test
    fun idle_to_dragging_past_the_start_when_an_action_is_set() {
        val m = machine()
        val fx = m.drag(30f, 0)
        assertEquals(SwipePhase.DRAGGING, m.state.phase)
        assertEquals(SwipeAction.TOGGLE_READ, m.state.action)
        assertEquals(30f, m.state.offset, 0.001f)
        assertTrue(fx.isEmpty())
    }

    @Test
    fun idle_stays_idle_and_the_row_does_not_move_when_the_direction_has_no_action() {
        val m = machine(ctx(start = SwipeSlot(SwipeAction.NONE)))
        val fx = m.drag(120f, 0)
        assertEquals(SwipePhase.IDLE, m.state.phase)
        assertEquals(0f, m.state.offset, 0f)
        assertTrue(fx.isEmpty())
        assertTrue(m.release(0f, 100).isEmpty())
    }

    @Test
    fun dragging_to_armed_at_the_threshold_with_the_arm_haptic() {
        val m = machine()
        m.drag(0.34f * W, 0)
        assertEquals(SwipePhase.DRAGGING, m.state.phase)
        val fx = m.drag(0.35f * W, 16)
        assertEquals(SwipePhase.ARMED, m.state.phase)
        assertEquals(listOf(SwipeHaptic.ThresholdArmed), fx.haptics())
    }

    @Test
    fun armed_to_dragging_when_the_distance_falls_back_with_the_disarm_haptic() {
        val m = machine()
        m.drag(0.4f * W, 0)
        val fx = m.drag(0.3f * W, 100)
        assertEquals(SwipePhase.DRAGGING, m.state.phase)
        assertEquals(listOf(SwipeHaptic.ThresholdDisarmed), fx.haptics())
    }

    @Test
    fun dragging_to_settling_back_on_release_below_the_threshold_without_a_fling() {
        val m = machine()
        m.drag(0.2f * W, 0)
        val fx = m.release(500f)
        assertEquals(SwipePhase.SETTLING_BACK, m.state.phase)
        assertEquals(listOf<SwipeEffect>(SwipeEffect.SpringBack), fx)
        assertEquals(0f, m.state.offset, 0f)
    }

    @Test
    fun armed_to_committing_on_release_with_commit_and_the_committed_haptic() {
        val m = machine(ctx(end = SwipeSlot(SwipeAction.ARCHIVE)))
        m.drag(-0.5f * W, 0)
        assertEquals(SwipePhase.ARMED, m.state.phase)
        val fx = m.release(0f)
        assertEquals(SwipePhase.COMMITTING, m.state.phase)
        assertEquals(listOf(SwipeHaptic.Committed), fx.haptics())
        assertTrue(fx.contains(SwipeEffect.Commit(SwipeAction.ARCHIVE, SwipeDirection.END_TO_START)))
        assertEquals(-W, m.state.offset, 0.001f)
    }

    @Test
    fun a_fling_commits_from_twenty_percent_even_when_not_armed() {
        val m = machine()
        m.drag(0.2f * W, 0)
        assertEquals(SwipePhase.DRAGGING, m.state.phase)
        val fx = m.release(1200f)
        assertEquals(SwipePhase.COMMITTING, m.state.phase)
        assertTrue(fx.any { it is SwipeEffect.Commit })
        assertEquals(listOf(SwipeHaptic.Committed), fx.haptics())
    }

    @Test
    fun a_fling_below_twenty_percent_or_below_the_speed_does_not_commit() {
        val slow = machine().also { it.drag(0.2f * W, 0) }
        assertEquals(SwipePhase.SETTLING_BACK, slow.also { it.release(1199f) }.state.phase)
        val short = machine().also { it.drag(0.19f * W, 0) }
        assertEquals(SwipePhase.SETTLING_BACK, short.also { it.release(3000f) }.state.phase)
        val wrongWay = machine().also { it.drag(0.25f * W, 0) }
        assertEquals(SwipePhase.SETTLING_BACK, wrongWay.also { it.release(-3000f) }.state.phase)
    }

    @Test
    fun committing_to_done_when_the_animation_ends_with_the_completed_effect() {
        val m = machine()
        m.drag(0.5f * W, 0)
        m.release()
        val fx = m.send(SwipeInput.AnimationEnd)
        assertEquals(SwipePhase.DONE, m.state.phase)
        assertEquals(listOf<SwipeEffect>(SwipeEffect.Completed(SwipeAction.TOGGLE_READ, SwipeDirection.START_TO_END)), fx)
    }

    @Test
    fun armed_snooze_or_move_to_parks_at_forty_percent_and_the_sheet_decides() {
        val c = ctx(start = SwipeSlot(SwipeAction.MOVE_TO))
        val chosen = machine(c).also { it.drag(0.5f * W, 0) }
        val fx = chosen.release()
        assertEquals(SwipePhase.PARKED, chosen.state.phase)
        assertTrue(fx.contains(SwipeEffect.Park(SwipeAction.MOVE_TO, SwipeDirection.START_TO_END)))
        assertEquals(0.4f * W, chosen.state.offset, 0.001f)
        assertFalse(fx.any { it is SwipeEffect.Commit })

        val done = chosen.send(SwipeInput.SheetResult(true))
        assertEquals(SwipePhase.DONE, chosen.state.phase)
        assertEquals(listOf<SwipeEffect>(SwipeEffect.Completed(SwipeAction.MOVE_TO, SwipeDirection.START_TO_END)), done)

        val dismissed = machine(c).also { it.drag(0.5f * W, 0); it.release() }
        val back = dismissed.send(SwipeInput.SheetResult(false))
        assertEquals(SwipePhase.SETTLING_BACK, dismissed.state.phase)
        assertEquals(listOf<SwipeEffect>(SwipeEffect.SpringBack), back)
    }

    @Test
    fun a_slot_can_park_by_itself_for_delete_forever_in_trash() {
        val m = machine(ctx(end = SwipeSlot(SwipeAction.DELETE, parks = true)))
        m.drag(-0.5f * W, 0)
        val fx = m.release()
        assertEquals(SwipePhase.PARKED, m.state.phase)
        assertTrue(fx.contains(SwipeEffect.Park(SwipeAction.DELETE, SwipeDirection.END_TO_START)))
        assertEquals(-0.4f * W, m.state.offset, 0.001f)
    }

    @Test
    fun any_state_to_settling_back_when_another_pointer_touches_selection_starts_or_the_row_leaves() {
        for (reason in SwipeCancel.entries) {
            val dragging = machine().also { it.drag(0.1f * W, 0) }
            assertEquals(listOf<SwipeEffect>(SwipeEffect.SpringBack), dragging.send(SwipeInput.Cancel(reason)))
            assertEquals(SwipePhase.SETTLING_BACK, dragging.state.phase)

            val armed = machine().also { it.drag(0.5f * W, 0) }
            val fx = armed.send(SwipeInput.Cancel(reason))
            assertEquals(SwipePhase.SETTLING_BACK, armed.state.phase)
            assertFalse("no action on $reason", fx.any { it is SwipeEffect.Commit || it is SwipeEffect.Park || it is SwipeEffect.Completed })

            val parked = machine(ctx(start = SwipeSlot(SwipeAction.MOVE_TO))).also { it.drag(0.5f * W, 0); it.release() }
            parked.send(SwipeInput.Cancel(reason))
            assertEquals(SwipePhase.SETTLING_BACK, parked.state.phase)
        }
    }

    @Test
    fun a_cancel_after_the_point_of_no_return_or_with_nothing_going_on_changes_nothing() {
        val idle = machine()
        assertTrue(idle.send(SwipeInput.Cancel(SwipeCancel.SELECTION)).isEmpty())
        assertEquals(SwipePhase.IDLE, idle.state.phase)
        val committing = machine().also { it.drag(0.5f * W, 0); it.release() }
        assertTrue(committing.send(SwipeInput.Cancel(SwipeCancel.ROW_REMOVED)).isEmpty())
        assertEquals(SwipePhase.COMMITTING, committing.state.phase)
    }

    // ---- Springing back, interruption, reset ----

    @Test
    fun settling_back_to_idle_when_the_animation_ends() {
        val m = machine().also { it.drag(0.1f * W, 0); it.release() }
        assertEquals(SwipePhase.SETTLING_BACK, m.state.phase)
        assertTrue(m.send(SwipeInput.AnimationEnd).isEmpty())
        assertEquals(SwipePhase.IDLE, m.state.phase)
    }

    @Test
    fun a_touch_during_the_spring_back_hands_the_card_to_the_finger() {
        val m = machine().also { it.drag(0.1f * W, 0); it.release() }
        m.drag(0.12f * W, 50)
        assertEquals(SwipePhase.DRAGGING, m.state.phase)
        assertEquals(0.12f * W, m.state.offset, 0.001f)
        m.drag(0.4f * W, 100)
        assertEquals(SwipePhase.ARMED, m.state.phase)
    }

    @Test
    fun a_card_sliding_out_or_parked_ignores_the_finger() {
        val committing = machine().also { it.drag(-0.5f * W, 0); it.release() }
        assertEquals(SwipeAction.DELETE, committing.state.action)
        assertTrue(committing.drag(-0.1f * W, 300).isEmpty())
        assertEquals(SwipePhase.COMMITTING, committing.state.phase)
        val parked = machine(ctx(start = SwipeSlot(SwipeAction.MOVE_TO))).also { it.drag(0.5f * W, 0); it.release() }
        assertTrue(parked.drag(0.1f * W, 300).isEmpty())
        assertEquals(SwipePhase.PARKED, parked.state.phase)
    }

    @Test
    fun a_read_or_star_that_is_springing_back_can_be_swiped_again_at_once() {
        val m = machine().also { it.drag(0.5f * W, 0); it.release() }
        assertEquals(SwipePhase.COMMITTING, m.state.phase)
        assertEquals(SwipeAction.TOGGLE_READ, m.state.action)
        m.drag(0.1f * W, 100)
        assertEquals(SwipePhase.DRAGGING, m.state.phase)
        m.drag(0.5f * W, 200)
        assertEquals(SwipePhase.ARMED, m.state.phase)
        val fx = m.release(0f, 300)
        assertTrue("and it reverses: a second commit", fx.any { it is SwipeEffect.Commit })
    }

    @Test
    fun reset_goes_back_to_idle_from_anywhere_without_effects() {
        val m = machine().also { it.drag(0.5f * W, 0); it.release(); it.send(SwipeInput.AnimationEnd) }
        assertEquals(SwipePhase.DONE, m.state.phase)
        assertTrue(m.send(SwipeInput.Reset).isEmpty())
        assertEquals(SwipeState(), m.state)
    }

    @Test
    fun release_and_animation_end_do_nothing_when_idle() {
        val m = machine()
        assertTrue(m.release(5000f).isEmpty())
        assertTrue(m.send(SwipeInput.AnimationEnd).isEmpty())
        assertTrue(m.send(SwipeInput.SheetResult(true)).isEmpty())
        assertEquals(SwipePhase.IDLE, m.state.phase)
    }

    // ---- Threshold, sensitivity, resistance ----

    @Test
    fun sensitivity_moves_the_arm_point_to_45_35_and_25_percent() {
        for ((s, f) in listOf(SwipeSensitivity.LOW to 0.45f, SwipeSensitivity.NORMAL to 0.35f, SwipeSensitivity.HIGH to 0.25f)) {
            val m = machine(ctx(sensitivity = s))
            m.drag(f * W - 1f, 0)
            assertEquals("$s just below", SwipePhase.DRAGGING, m.state.phase)
            m.drag(f * W, 100)
            assertEquals("$s at the point", SwipePhase.ARMED, m.state.phase)
            assertEquals(f * W, SwipeReducer.threshold(TestTuning, ctx(sensitivity = s)), 0.001f)
        }
    }

    @Test
    fun the_card_follows_one_to_one_then_at_a_third_past_85_percent_and_never_past_the_width() {
        assertEquals(100f, SwipeReducer.resisted(TestTuning, W, 100f), 0.001f)
        assertEquals(0.85f * W, SwipeReducer.resisted(TestTuning, W, 0.85f * W), 0.001f)
        assertEquals(0.85f * W + 30f * 0.3333f, SwipeReducer.resisted(TestTuning, W, 0.85f * W + 30f), 0.001f)
        assertEquals(W, SwipeReducer.resisted(TestTuning, W, 5 * W), 0.001f)
        val m = machine().also { it.drag(0.95f * W, 0) }
        assertEquals(0.85f * W + 0.1f * W * 0.3333f, m.state.offset, 0.01f)
    }

    // ---- Haptics: the 40 ms guard and the order ----

    @Test
    fun drag_across_back_across_and_release_gives_armed_disarmed_armed_committed() {
        val m = machine()
        val all = ArrayList<SwipeEffect>()
        all += m.drag(0.2f * W, 0)
        all += m.drag(0.4f * W, 100)
        all += m.drag(0.2f * W, 200)
        all += m.drag(0.4f * W, 300)
        all += m.release(0f, 400)
        assertEquals(
            listOf(SwipeHaptic.ThresholdArmed, SwipeHaptic.ThresholdDisarmed, SwipeHaptic.ThresholdArmed, SwipeHaptic.Committed),
            all.haptics(),
        )
    }

    @Test
    fun a_crossing_within_40_ms_of_the_last_is_held_back_and_taken_up_by_a_later_event() {
        val m = machine()
        val fx = ArrayList<SwipeEffect>()
        fx += m.drag(0.4f * W, 0)       // arms
        fx += m.drag(0.3f * W, 20)      // would disarm: too soon
        assertEquals(SwipePhase.ARMED, m.state.phase)
        fx += m.drag(0.3f * W, 60)      // now it may
        assertEquals(SwipePhase.DRAGGING, m.state.phase)
        assertEquals(listOf(SwipeHaptic.ThresholdArmed, SwipeHaptic.ThresholdDisarmed), fx.haptics())
    }

    @Test
    fun a_release_looks_at_the_final_distance_even_inside_the_guard() {
        val m = machine()
        m.drag(0.4f * W, 0)
        m.drag(0.3f * W, 10) // held back, still armed
        val fx = m.release(0f, 20)
        assertEquals(SwipePhase.SETTLING_BACK, m.state.phase)
        assertFalse(fx.any { it is SwipeEffect.Commit })
        assertEquals(listOf(SwipeHaptic.ThresholdDisarmed), fx.haptics())
    }

    // ---- Unavailable ----

    @Test
    fun an_unavailable_action_never_arms_plays_rejected_once_and_springs_back_with_a_notice() {
        val m = machine(ctx(end = SwipeSlot(SwipeAction.ARCHIVE, available = false)))
        val fx = ArrayList<SwipeEffect>()
        fx += m.drag(-0.2f * W, 0)
        fx += m.drag(-0.5f * W, 100)
        assertEquals(SwipePhase.DRAGGING, m.state.phase)
        fx += m.drag(-0.6f * W, 200)
        fx += m.release(0f, 300)
        assertEquals(listOf(SwipeHaptic.Rejected), fx.haptics())
        assertTrue(fx.contains(SwipeEffect.NotAvailable(SwipeAction.ARCHIVE)))
        assertTrue(fx.contains(SwipeEffect.SpringBack))
        assertFalse(fx.any { it is SwipeEffect.Commit })
        assertEquals(SwipePhase.SETTLING_BACK, m.state.phase)
    }

    @Test
    fun an_unavailable_action_released_short_of_the_arm_point_just_springs_back() {
        val m = machine(ctx(end = SwipeSlot(SwipeAction.ARCHIVE, available = false)))
        m.drag(-0.1f * W, 0)
        val fx = m.release(0f, 100)
        assertEquals(listOf<SwipeEffect>(SwipeEffect.SpringBack), fx)
    }

    @Test
    fun an_unavailable_action_flung_gets_the_notice_but_never_commits() {
        val m = machine(ctx(end = SwipeSlot(SwipeAction.ARCHIVE, available = false)))
        m.drag(-0.25f * W, 0)
        val fx = m.release(-4000f, 100)
        assertTrue(fx.contains(SwipeEffect.NotAvailable(SwipeAction.ARCHIVE)))
        assertFalse(fx.any { it is SwipeEffect.Commit || it is SwipeEffect.Park })
    }

    // ---- Direction, RTL ----

    @Test
    fun in_ltr_a_drag_right_is_start_to_end_and_in_rtl_it_is_end_to_start() {
        val ltr = machine(ctx(rtl = false)).also { it.drag(0.5f * W, 0) }
        assertEquals(SwipeDirection.START_TO_END, ltr.state.direction)
        assertEquals(SwipeAction.TOGGLE_READ, ltr.state.action)
        val rtl = machine(ctx(rtl = true)).also { it.drag(0.5f * W, 0) }
        assertEquals(SwipeDirection.END_TO_START, rtl.state.direction)
        assertEquals(SwipeAction.DELETE, rtl.state.action)
        assertEquals(0.5f * W, rtl.state.offset, 0.001f)
    }

    @Test
    fun the_stored_direction_is_what_mirrors() {
        assertEquals(1, SwipeDirection.START_TO_END.physicalSign(rtl = false))
        assertEquals(-1, SwipeDirection.START_TO_END.physicalSign(rtl = true))
        assertEquals(-1, SwipeDirection.END_TO_START.physicalSign(rtl = false))
        assertEquals(1, SwipeDirection.END_TO_START.physicalSign(rtl = true))
    }

    @Test
    fun crossing_over_zero_while_armed_disarms_and_follows_the_other_side() {
        val m = machine()
        val fx = ArrayList<SwipeEffect>()
        fx += m.drag(0.4f * W, 0)
        fx += m.drag(-0.1f * W, 200)
        assertEquals(SwipeDirection.END_TO_START, m.state.direction)
        assertEquals(SwipePhase.DRAGGING, m.state.phase)
        assertEquals(listOf(SwipeHaptic.ThresholdArmed, SwipeHaptic.ThresholdDisarmed), fx.haptics())
        assertNotNull(m.state.direction)
        assertEquals(-0.1f * W, m.state.offset, 0.001f)
    }

    @Test
    fun dragging_toward_a_direction_without_an_action_after_the_other_goes_idle_and_the_row_returns() {
        val m = machine(ctx(end = SwipeSlot(SwipeAction.NONE)))
        m.drag(0.1f * W, 0)
        m.drag(-0.1f * W, 100)
        assertEquals(SwipePhase.IDLE, m.state.phase)
        assertEquals(0f, m.state.offset, 0f)
        m.drag(0.2f * W, 200)
        assertEquals(SwipePhase.DRAGGING, m.state.phase)
    }

    // ---- Settings types ----

    @Test
    fun the_picker_leaves_snooze_and_move_to_out_until_their_sheets_exist_and_ends_with_none() {
        assertFalse(SwipeAction.SNOOZE in SwipeAction.picker)
        assertFalse(SwipeAction.MOVE_TO in SwipeAction.picker)
        assertEquals(SwipeAction.NONE, SwipeAction.picker.last())
        assertEquals(
            listOf(SwipeAction.TOGGLE_READ, SwipeAction.ARCHIVE, SwipeAction.DELETE, SwipeAction.TOGGLE_STAR, SwipeAction.SPAM, SwipeAction.NONE),
            SwipeAction.picker,
        )
    }

    @Test
    fun presets_match_the_docs_and_the_default_is_triage() {
        assertEquals(SwipePreset.TRIAGE, SwipeSettings.Default.matchingPreset())
        assertEquals(SwipeAction.ARCHIVE, SwipeSettings().with(SwipeDirection.START_TO_END, SwipeAction.ARCHIVE).startToEnd)
        assertEquals(SwipePreset.TIDY, SwipeSettings(SwipeAction.TOGGLE_READ, SwipeAction.ARCHIVE).matchingPreset())
        assertEquals(SwipePreset.GMAIL_STYLE, SwipeSettings(SwipeAction.ARCHIVE, SwipeAction.ARCHIVE).matchingPreset())
        assertEquals(null, SwipeSettings(SwipeAction.TOGGLE_STAR).matchingPreset())
    }

    @Test
    fun actions_and_sensitivities_round_trip_through_their_ids() {
        for (a in SwipeAction.entries) assertEquals(a, SwipeAction.fromId(a.id))
        for (s in SwipeSensitivity.entries) assertEquals(s, SwipeSensitivity.fromId(s.id))
        assertEquals(null, SwipeAction.fromId("nope"))
    }
}
