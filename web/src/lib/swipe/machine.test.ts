// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// SwipeMachineTest.kt, ported one for one: every row of the transition table in docs/ux/swipe.md,
// then the special cases. The names are the Kotlin ones.

import { test } from 'node:test';
import assert from 'node:assert/strict';
import {
  SwipeAction,
  SwipePresets,
  actionFromId,
  defaultSwipeSettings,
  matchingPreset,
  physicalSign,
  sensitivityFromId,
  swipeActions,
  swipePicker,
  swipeSensitivities,
  withAction
} from './model.ts';
import { initialSwipeState, resisted, slotOf, swipeCancels, threshold, type SwipeEffect } from './machine.ts';
import { TestTuning, W, close, ctx, drag, haptics, machine, release } from './testkit.ts';

const approx = (a: number, e: number, eps: number, msg?: string) =>
  assert.ok(close(a, e, eps), `${msg ?? ''} expected ${e}, got ${a}`);
const has = (fx: readonly SwipeEffect[], want: SwipeEffect) =>
  fx.some((e) => JSON.stringify(e) === JSON.stringify(want));
const any = (fx: readonly SwipeEffect[], ...types: SwipeEffect['type'][]) => fx.some((e) => types.includes(e.type));

// ---- The table ----

test('idle_to_dragging_past_the_start_when_an_action_is_set', () => {
  const m = machine();
  const fx = drag(m, 30, 0);
  assert.equal(m.state.phase, 'dragging');
  assert.equal(m.state.action, SwipeAction.TOGGLE_READ);
  approx(m.state.offset, 30, 0.001);
  assert.equal(fx.length, 0);
});

test('idle_stays_idle_and_the_row_does_not_move_when_the_direction_has_no_action', () => {
  const m = machine(ctx({ start: slotOf(SwipeAction.NONE) }));
  const fx = drag(m, 120, 0);
  assert.equal(m.state.phase, 'idle');
  assert.equal(m.state.offset, 0);
  assert.equal(fx.length, 0);
  assert.equal(release(m, 0, 100).length, 0);
});

test('dragging_to_armed_at_the_threshold_with_the_arm_haptic', () => {
  const m = machine();
  drag(m, 0.34 * W, 0);
  assert.equal(m.state.phase, 'dragging');
  const fx = drag(m, 0.35 * W, 16);
  assert.equal(m.state.phase, 'armed');
  assert.deepEqual(haptics(fx), ['threshold-armed']);
});

test('armed_to_dragging_when_the_distance_falls_back_with_the_disarm_haptic', () => {
  const m = machine();
  drag(m, 0.4 * W, 0);
  const fx = drag(m, 0.3 * W, 100);
  assert.equal(m.state.phase, 'dragging');
  assert.deepEqual(haptics(fx), ['threshold-disarmed']);
});

test('dragging_to_settling_back_on_release_below_the_threshold_without_a_fling', () => {
  const m = machine();
  drag(m, 0.2 * W, 0);
  const fx = release(m, 500);
  assert.equal(m.state.phase, 'settling-back');
  assert.deepEqual(fx, [{ type: 'spring-back' }]);
  assert.equal(m.state.offset, 0);
});

test('armed_to_committing_on_release_with_commit_and_the_committed_haptic', () => {
  const m = machine(ctx({ end: slotOf(SwipeAction.ARCHIVE) }));
  drag(m, -0.5 * W, 0);
  assert.equal(m.state.phase, 'armed');
  const fx = release(m, 0);
  assert.equal(m.state.phase, 'committing');
  assert.deepEqual(haptics(fx), ['committed']);
  assert.ok(has(fx, { type: 'commit', action: SwipeAction.ARCHIVE, direction: 'end-to-start' }));
  approx(m.state.offset, -W, 0.001);
});

test('a_fling_commits_from_twenty_percent_even_when_not_armed', () => {
  const m = machine();
  drag(m, 0.2 * W, 0);
  assert.equal(m.state.phase, 'dragging');
  const fx = release(m, 1200);
  assert.equal(m.state.phase, 'committing');
  assert.ok(any(fx, 'commit'));
  assert.deepEqual(haptics(fx), ['committed']);
});

test('a_fling_below_twenty_percent_or_below_the_speed_does_not_commit', () => {
  const slow = machine();
  drag(slow, 0.2 * W, 0);
  release(slow, 1199);
  assert.equal(slow.state.phase, 'settling-back');
  const short = machine();
  drag(short, 0.19 * W, 0);
  release(short, 3000);
  assert.equal(short.state.phase, 'settling-back');
  const wrongWay = machine();
  drag(wrongWay, 0.25 * W, 0);
  release(wrongWay, -3000);
  assert.equal(wrongWay.state.phase, 'settling-back');
});

test('committing_to_done_when_the_animation_ends_with_the_completed_effect', () => {
  const m = machine();
  drag(m, 0.5 * W, 0);
  release(m);
  const fx = m.send({ type: 'animation-end' });
  assert.equal(m.state.phase, 'done');
  assert.deepEqual(fx, [{ type: 'completed', action: SwipeAction.TOGGLE_READ, direction: 'start-to-end' }]);
});

test('armed_snooze_or_move_to_parks_at_forty_percent_and_the_sheet_decides', () => {
  const c = ctx({ start: slotOf(SwipeAction.MOVE_TO) });
  const chosen = machine(c);
  drag(chosen, 0.5 * W, 0);
  const fx = release(chosen);
  assert.equal(chosen.state.phase, 'parked');
  assert.ok(has(fx, { type: 'park', action: SwipeAction.MOVE_TO, direction: 'start-to-end' }));
  approx(chosen.state.offset, 0.4 * W, 0.001);
  assert.ok(!any(fx, 'commit'));

  const done = chosen.send({ type: 'sheet-result', chosen: true });
  assert.equal(chosen.state.phase, 'done');
  assert.deepEqual(done, [{ type: 'completed', action: SwipeAction.MOVE_TO, direction: 'start-to-end' }]);

  const dismissed = machine(c);
  drag(dismissed, 0.5 * W, 0);
  release(dismissed);
  const back = dismissed.send({ type: 'sheet-result', chosen: false });
  assert.equal(dismissed.state.phase, 'settling-back');
  assert.deepEqual(back, [{ type: 'spring-back' }]);
});

test('a_slot_can_park_by_itself_for_delete_forever_in_trash', () => {
  const m = machine(ctx({ end: slotOf(SwipeAction.DELETE, true, true) }));
  drag(m, -0.5 * W, 0);
  const fx = release(m);
  assert.equal(m.state.phase, 'parked');
  assert.ok(has(fx, { type: 'park', action: SwipeAction.DELETE, direction: 'end-to-start' }));
  approx(m.state.offset, -0.4 * W, 0.001);
});

test('any_state_to_settling_back_when_another_pointer_touches_selection_starts_or_the_row_leaves', () => {
  for (const reason of swipeCancels) {
    const dragging = machine();
    drag(dragging, 0.1 * W, 0);
    assert.deepEqual(dragging.send({ type: 'cancel', reason }), [{ type: 'spring-back' }]);
    assert.equal(dragging.state.phase, 'settling-back');

    const armed = machine();
    drag(armed, 0.5 * W, 0);
    const fx = armed.send({ type: 'cancel', reason });
    assert.equal(armed.state.phase, 'settling-back');
    assert.ok(!any(fx, 'commit', 'park', 'completed'), `no action on ${reason}`);

    const parked = machine(ctx({ start: slotOf(SwipeAction.MOVE_TO) }));
    drag(parked, 0.5 * W, 0);
    release(parked);
    parked.send({ type: 'cancel', reason });
    assert.equal(parked.state.phase, 'settling-back');
  }
});

test('a_cancel_after_the_point_of_no_return_or_with_nothing_going_on_changes_nothing', () => {
  const idle = machine();
  assert.equal(idle.send({ type: 'cancel', reason: 'selection' }).length, 0);
  assert.equal(idle.state.phase, 'idle');
  const committing = machine();
  drag(committing, 0.5 * W, 0);
  release(committing);
  assert.equal(committing.send({ type: 'cancel', reason: 'row-removed' }).length, 0);
  assert.equal(committing.state.phase, 'committing');
});

// ---- Springing back, interruption, reset ----

test('settling_back_to_idle_when_the_animation_ends', () => {
  const m = machine();
  drag(m, 0.1 * W, 0);
  release(m);
  assert.equal(m.state.phase, 'settling-back');
  assert.equal(m.send({ type: 'animation-end' }).length, 0);
  assert.equal(m.state.phase, 'idle');
});

test('a_touch_during_the_spring_back_hands_the_card_to_the_finger', () => {
  const m = machine();
  drag(m, 0.1 * W, 0);
  release(m);
  drag(m, 0.12 * W, 50);
  assert.equal(m.state.phase, 'dragging');
  approx(m.state.offset, 0.12 * W, 0.001);
  drag(m, 0.4 * W, 100);
  assert.equal(m.state.phase, 'armed');
});

test('a_card_sliding_out_or_parked_ignores_the_finger', () => {
  const committing = machine();
  drag(committing, -0.5 * W, 0);
  release(committing);
  assert.equal(committing.state.action, SwipeAction.DELETE);
  assert.equal(drag(committing, -0.1 * W, 300).length, 0);
  assert.equal(committing.state.phase, 'committing');
  const parked = machine(ctx({ start: slotOf(SwipeAction.MOVE_TO) }));
  drag(parked, 0.5 * W, 0);
  release(parked);
  assert.equal(drag(parked, 0.1 * W, 300).length, 0);
  assert.equal(parked.state.phase, 'parked');
});

test('a_read_or_star_that_is_springing_back_can_be_swiped_again_at_once', () => {
  const m = machine();
  drag(m, 0.5 * W, 0);
  release(m);
  assert.equal(m.state.phase, 'committing');
  assert.equal(m.state.action, SwipeAction.TOGGLE_READ);
  drag(m, 0.1 * W, 100);
  assert.equal(m.state.phase, 'dragging');
  drag(m, 0.5 * W, 200);
  assert.equal(m.state.phase, 'armed');
  const fx = release(m, 0, 300);
  assert.ok(any(fx, 'commit'), 'and it reverses: a second commit');
});

test('reset_goes_back_to_idle_from_anywhere_without_effects', () => {
  const m = machine();
  drag(m, 0.5 * W, 0);
  release(m);
  m.send({ type: 'animation-end' });
  assert.equal(m.state.phase, 'done');
  assert.equal(m.send({ type: 'reset' }).length, 0);
  assert.deepEqual(m.state, initialSwipeState);
});

test('release_and_animation_end_do_nothing_when_idle', () => {
  const m = machine();
  assert.equal(release(m, 5000).length, 0);
  assert.equal(m.send({ type: 'animation-end' }).length, 0);
  assert.equal(m.send({ type: 'sheet-result', chosen: true }).length, 0);
  assert.equal(m.state.phase, 'idle');
});

// ---- Threshold, sensitivity, resistance ----

test('sensitivity_moves_the_arm_point_to_45_35_and_25_percent', () => {
  for (const [s, f] of [
    ['low', 0.45],
    ['normal', 0.35],
    ['high', 0.25]
  ] as const) {
    const m = machine(ctx({ sensitivity: s }));
    drag(m, f * W - 1, 0);
    assert.equal(m.state.phase, 'dragging', `${s} just below`);
    drag(m, f * W, 100);
    assert.equal(m.state.phase, 'armed', `${s} at the point`);
    approx(threshold(TestTuning, ctx({ sensitivity: s })), f * W, 0.001);
  }
});

test('the_card_follows_one_to_one_then_at_a_third_past_85_percent_and_never_past_the_width', () => {
  approx(resisted(TestTuning, W, 100), 100, 0.001);
  approx(resisted(TestTuning, W, 0.85 * W), 0.85 * W, 0.001);
  approx(resisted(TestTuning, W, 0.85 * W + 30), 0.85 * W + 30 * 0.3333, 0.001);
  approx(resisted(TestTuning, W, 5 * W), W, 0.001);
  const m = machine();
  drag(m, 0.95 * W, 0);
  approx(m.state.offset, 0.85 * W + 0.1 * W * 0.3333, 0.01);
});

// ---- Haptics: the 40 ms guard and the order ----

test('drag_across_back_across_and_release_gives_armed_disarmed_armed_committed', () => {
  const m = machine();
  const all: SwipeEffect[] = [];
  all.push(...drag(m, 0.2 * W, 0));
  all.push(...drag(m, 0.4 * W, 100));
  all.push(...drag(m, 0.2 * W, 200));
  all.push(...drag(m, 0.4 * W, 300));
  all.push(...release(m, 0, 400));
  assert.deepEqual(haptics(all), ['threshold-armed', 'threshold-disarmed', 'threshold-armed', 'committed']);
});

test('a_crossing_within_40_ms_of_the_last_is_held_back_and_taken_up_by_a_later_event', () => {
  const m = machine();
  const fx: SwipeEffect[] = [];
  fx.push(...drag(m, 0.4 * W, 0)); // arms
  fx.push(...drag(m, 0.3 * W, 20)); // would disarm: too soon
  assert.equal(m.state.phase, 'armed');
  fx.push(...drag(m, 0.3 * W, 60)); // now it may
  assert.equal(m.state.phase, 'dragging');
  assert.deepEqual(haptics(fx), ['threshold-armed', 'threshold-disarmed']);
});

test('a_release_looks_at_the_final_distance_even_inside_the_guard', () => {
  const m = machine();
  drag(m, 0.4 * W, 0);
  drag(m, 0.3 * W, 10); // held back, still armed
  const fx = release(m, 0, 20);
  assert.equal(m.state.phase, 'settling-back');
  assert.ok(!any(fx, 'commit'));
  assert.deepEqual(haptics(fx), ['threshold-disarmed']);
});

// ---- Unavailable ----

test('an_unavailable_action_never_arms_plays_rejected_once_and_springs_back_with_a_notice', () => {
  const m = machine(ctx({ end: slotOf(SwipeAction.ARCHIVE, false) }));
  const fx: SwipeEffect[] = [];
  fx.push(...drag(m, -0.2 * W, 0));
  fx.push(...drag(m, -0.5 * W, 100));
  assert.equal(m.state.phase, 'dragging');
  fx.push(...drag(m, -0.6 * W, 200));
  fx.push(...release(m, 0, 300));
  assert.deepEqual(haptics(fx), ['rejected']);
  assert.ok(has(fx, { type: 'not-available', action: SwipeAction.ARCHIVE }));
  assert.ok(has(fx, { type: 'spring-back' }));
  assert.ok(!any(fx, 'commit'));
  assert.equal(m.state.phase, 'settling-back');
});

test('an_unavailable_action_released_short_of_the_arm_point_just_springs_back', () => {
  const m = machine(ctx({ end: slotOf(SwipeAction.ARCHIVE, false) }));
  drag(m, -0.1 * W, 0);
  assert.deepEqual(release(m, 0, 100), [{ type: 'spring-back' }]);
});

test('an_unavailable_action_flung_gets_the_notice_but_never_commits', () => {
  const m = machine(ctx({ end: slotOf(SwipeAction.ARCHIVE, false) }));
  drag(m, -0.25 * W, 0);
  const fx = release(m, -4000, 100);
  assert.ok(has(fx, { type: 'not-available', action: SwipeAction.ARCHIVE }));
  assert.ok(!any(fx, 'commit', 'park'));
});

// ---- Direction, RTL ----

test('in_ltr_a_drag_right_is_start_to_end_and_in_rtl_it_is_end_to_start', () => {
  const ltr = machine(ctx({ rtl: false }));
  drag(ltr, 0.5 * W, 0);
  assert.equal(ltr.state.direction, 'start-to-end');
  assert.equal(ltr.state.action, SwipeAction.TOGGLE_READ);
  const rtl = machine(ctx({ rtl: true }));
  drag(rtl, 0.5 * W, 0);
  assert.equal(rtl.state.direction, 'end-to-start');
  assert.equal(rtl.state.action, SwipeAction.DELETE);
  approx(rtl.state.offset, 0.5 * W, 0.001);
});

test('the_stored_direction_is_what_mirrors', () => {
  assert.equal(physicalSign('start-to-end', false), 1);
  assert.equal(physicalSign('start-to-end', true), -1);
  assert.equal(physicalSign('end-to-start', false), -1);
  assert.equal(physicalSign('end-to-start', true), 1);
});

test('crossing_over_zero_while_armed_disarms_and_follows_the_other_side', () => {
  const m = machine();
  const fx: SwipeEffect[] = [];
  fx.push(...drag(m, 0.4 * W, 0));
  fx.push(...drag(m, -0.1 * W, 200));
  assert.equal(m.state.direction, 'end-to-start');
  assert.equal(m.state.phase, 'dragging');
  assert.deepEqual(haptics(fx), ['threshold-armed', 'threshold-disarmed']);
  approx(m.state.offset, -0.1 * W, 0.001);
});

test('dragging_toward_a_direction_without_an_action_after_the_other_goes_idle_and_the_row_returns', () => {
  const m = machine(ctx({ end: slotOf(SwipeAction.NONE) }));
  drag(m, 0.1 * W, 0);
  drag(m, -0.1 * W, 100);
  assert.equal(m.state.phase, 'idle');
  assert.equal(m.state.offset, 0);
  drag(m, 0.2 * W, 200);
  assert.equal(m.state.phase, 'dragging');
});

// ---- Settings types ----

test('the_picker_offers_move_to_since_its_sheet_exists_leaves_snooze_out_and_ends_with_none', () => {
  assert.ok(!swipePicker.includes(SwipeAction.SNOOZE));
  assert.ok(swipePicker.includes(SwipeAction.MOVE_TO));
  assert.equal(swipePicker.at(-1), SwipeAction.NONE);
  assert.deepEqual(swipePicker, [
    SwipeAction.TOGGLE_READ,
    SwipeAction.ARCHIVE,
    SwipeAction.DELETE,
    SwipeAction.MOVE_TO,
    SwipeAction.TOGGLE_STAR,
    SwipeAction.SPAM,
    SwipeAction.NONE
  ]);
});

test('presets_match_the_docs_and_the_default_is_triage', () => {
  assert.equal(matchingPreset(defaultSwipeSettings), SwipePresets.TRIAGE);
  assert.equal(withAction(defaultSwipeSettings, 'start-to-end', SwipeAction.ARCHIVE).startToEnd, SwipeAction.ARCHIVE);
  const s = (a: typeof SwipeAction.NONE, b: typeof SwipeAction.NONE = SwipeAction.DELETE) => ({
    startToEnd: a,
    endToStart: b,
    sensitivity: 'normal' as const
  });
  assert.equal(matchingPreset(s(SwipeAction.TOGGLE_READ, SwipeAction.ARCHIVE)), SwipePresets.TIDY);
  assert.equal(matchingPreset(s(SwipeAction.ARCHIVE, SwipeAction.ARCHIVE)), SwipePresets.GMAIL_STYLE);
  assert.equal(matchingPreset(s(SwipeAction.TOGGLE_STAR)), null);
});

test('actions_and_sensitivities_round_trip_through_their_ids', () => {
  for (const a of swipeActions) assert.equal(actionFromId(a.id), a);
  for (const s of swipeSensitivities) assert.equal(sensitivityFromId(s), s);
  assert.equal(actionFromId('nope'), null);
});
