// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// SwipePropertyTest.kt, ported: the same properties on random gestures (a random row width, layout
// direction, sensitivity and slots, a random walk of drags with random gaps, and a random end). The
// generator is seeded (mulberry32) so a failure reproduces; the failing gesture is printed.

import { test } from 'node:test';
import assert from 'node:assert/strict';
import { directionOf, physicalSign, swipeActions, swipeSensitivities } from './model.ts';
import {
  SwipeMachine,
  contextSlot,
  swipeCancels,
  swipePhases,
  threshold,
  type SwipeCancel,
  type SwipeContext,
  type SwipeEffect,
  type SwipeInput,
  type SwipePhase,
  type SwipeSlot,
  type SwipeState
} from './machine.ts';
import { TestTuning, haptics } from './testkit.ts';

const ITERATIONS = 2_000;

/** A small seeded generator (mulberry32). */
class Random {
  #s: number;
  constructor(seed: number) {
    this.#s = seed >>> 0;
  }
  next(): number {
    let t = (this.#s += 0x6d2b79f5);
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  }
  int(from: number, until?: number): number {
    const [lo, hi] = until === undefined ? [0, from] : [from, until];
    return lo + Math.floor(this.next() * (hi - lo));
  }
  bool(): boolean {
    return this.next() < 0.5;
  }
  pick<T>(xs: readonly T[]): T {
    return xs[this.int(xs.length)]!;
  }
}

type End = { type: 'release'; velocity: number } | { type: 'cancel'; reason: SwipeCancel } | { type: 'none' };

interface Gesture {
  ctx: SwipeContext;
  drags: [number, number][];
  end: End;
}

function randomSlot(r: Random): SwipeSlot {
  const action = r.pick(swipeActions);
  return { action, available: r.int(4) !== 0, parks: action.parks || r.int(8) === 0 };
}

function randomContext(r: Random, rtl: boolean = r.bool()): SwipeContext {
  return {
    width: r.int(200, 900),
    rtl,
    sensitivity: r.pick(swipeSensitivities),
    startToEnd: randomSlot(r),
    endToStart: randomSlot(r)
  };
}

function randomGesture(r: Random): Gesture {
  const ctx = randomContext(r);
  let x = 0;
  let t = 0;
  const drags: [number, number][] = [];
  for (let i = r.int(1, 40); i > 0; i--) {
    x = Math.min(1.2 * ctx.width, Math.max(-1.2 * ctx.width, x + (r.next() - 0.5) * 0.6 * ctx.width));
    if (r.int(10) === 0) x = 0;
    t += r.int(0, 120);
    drags.push([x, t]);
  }
  const k = r.int(6);
  const end: End =
    k === 0 ? { type: 'cancel', reason: r.pick(swipeCancels) } : k === 1 ? { type: 'none' } : { type: 'release', velocity: (r.next() - 0.5) * 6000 };
  return { ctx, drags, end };
}

interface Run {
  effects: SwipeEffect[];
  beforeEnd: SwipeState;
  machine: SwipeMachine;
}

function run(g: Gesture): Run {
  const m = new SwipeMachine(TestTuning, g.ctx);
  const effects: SwipeEffect[] = [];
  let t = 0;
  for (const [dx, time] of g.drags) {
    effects.push(...m.send({ type: 'drag', dx, timeMs: time }));
    t = time;
  }
  const before = m.state;
  if (g.end.type === 'release') effects.push(...m.send({ type: 'release', velocity: g.end.velocity, timeMs: t + 5 }));
  if (g.end.type === 'cancel') effects.push(...m.send({ type: 'cancel', reason: g.end.reason }));
  return { effects, beforeEnd: before, machine: m };
}

const show = (g: Gesture) => JSON.stringify(g);

function check(seed: number, property: (g: Gesture, r: Run) => void): void {
  const r = new Random(seed);
  for (let i = 0; i < ITERATIONS; i++) {
    const g = randomGesture(r);
    property(g, run(g));
  }
}

const SEED = 20261111;

function flung(g: Gesture, r: Run, velocity: number): boolean {
  const dir = r.beforeEnd.direction!;
  const along = velocity * physicalSign(dir, g.ctx.rtl);
  return along >= TestTuning.flingMinVelocity && r.beforeEnd.distance >= TestTuning.flingMinTravelFraction * g.ctx.width;
}

test('neverCommitsBelowTheThresholdWithoutAFling', () =>
  check(SEED, (g, r) => {
    const committed = r.effects.filter((e) => e.type === 'commit' || e.type === 'park');
    const e = g.end.type === 'release' ? g.end : null;
    if (committed.length > 0) {
      assert.ok(e, `a commit without a release: ${show(g)}`);
      assert.ok(r.beforeEnd.direction, show(g));
      const th = threshold(TestTuning, g.ctx);
      assert.ok(r.beforeEnd.distance >= th || flung(g, r, e.velocity), `committed at ${r.beforeEnd.distance} without a fling: ${show(g)}`);
    }
    if (e && r.beforeEnd.phase !== 'idle') {
      const th = threshold(TestTuning, g.ctx);
      if (r.beforeEnd.distance < th && !flung(g, r, e.velocity)) assert.equal(committed.length, 0, `committed below the threshold: ${show(g)}`);
    }
  }));

test('neverTwoArmEventsWithoutADisarmBetween', () =>
  check(SEED + 1, (g, r) => {
    let armed = false;
    for (const h of haptics(r.effects)) {
      if (h === 'threshold-armed') {
        assert.ok(!armed, `two arm events in a row: ${show(g)}`);
        armed = true;
      } else if (h === 'threshold-disarmed') {
        assert.ok(armed, `a disarm without an arm: ${show(g)}`);
        armed = false;
      }
    }
  }));

test('neverCommitsParksOrArmsWhenTheActionIsUnavailable', () =>
  check(SEED + 2, (g, r) => {
    const dir = r.beforeEnd.direction;
    if (dir && !contextSlot(g.ctx, dir).available) {
      assert.ok(!r.effects.some((e) => e.type === 'commit' || e.type === 'park' || e.type === 'completed'), `acted on an unavailable action: ${show(g)}`);
    }
    if (!g.ctx.startToEnd.available && !g.ctx.endToStart.available) {
      assert.ok(!haptics(r.effects).includes('threshold-armed'), `armed with nothing available: ${show(g)}`);
    }
  }));

test('neverCommitsAnActionThatIsNone', () =>
  check(SEED + 3, (_g, r) => {
    for (const e of r.effects) {
      if (e.type === 'commit' || e.type === 'park') assert.notEqual(e.action.id, 'none');
    }
  }));

test('rtlIsTheMirrorImageOfLtrForTheSameStoredSettingsAndTheOppositePhysicalDrag', () => {
  const rnd = new Random(SEED + 4);
  for (let i = 0; i < ITERATIONS; i++) {
    const g = randomGesture(rnd);
    const mirrorEnd = (e: End, flip: boolean): End => (flip && e.type === 'release' ? { type: 'release', velocity: -e.velocity } : e);
    const a = run({ ctx: { ...g.ctx, rtl: false }, drags: g.drags, end: mirrorEnd(g.end, false) });
    const b = run({
      ctx: { ...g.ctx, rtl: true },
      drags: g.drags.map(([dx, t]) => [-dx, t] as [number, number]),
      end: mirrorEnd(g.end, true)
    });
    assert.deepEqual(a.effects, b.effects, `effects: ${show(g)}`);
    assert.equal(a.machine.state.phase, b.machine.state.phase, `phase: ${show(g)}`);
    assert.equal(a.machine.state.direction, b.machine.state.direction, `direction: ${show(g)}`);
    assert.ok(Math.abs(a.machine.state.offset + b.machine.state.offset) <= 0.001, `offset: ${show(g)}`);
  }
});

test('theSamePhysicalDragInRtlRunsTheOtherDirectionsAction', () => {
  const rnd = new Random(SEED + 5);
  for (let i = 0; i < ITERATIONS; i++) {
    const g = randomGesture(rnd);
    const dx = g.drags.map(([d]) => d).find((d) => d !== 0);
    if (dx === undefined) continue;
    const ltrDir = directionOf(dx, false);
    const rtlDir = directionOf(dx, true);
    assert.notEqual(ltrDir, rtlDir);
    assert.equal(physicalSign(ltrDir, false), physicalSign(rtlDir, true));
  }
});

test('theCardNeverLeavesTheRowAndIsOnTheSideOfTheFinger', () => {
  const rnd = new Random(SEED + 6);
  for (let i = 0; i < ITERATIONS; i++) {
    const g = randomGesture(rnd);
    const m = new SwipeMachine(TestTuning, g.ctx);
    for (const [dx, t] of g.drags) {
      m.send({ type: 'drag', dx, timeMs: t });
      const s = m.state;
      assert.ok(Math.abs(s.offset) <= g.ctx.width + 0.001, `offset ${s.offset} outside the row in ${show(g)}`);
      if (s.phase === 'dragging' || s.phase === 'armed') {
        assert.ok(s.offset === 0 || s.offset > 0 === dx > 0, `card on the wrong side in ${show(g)}`);
      }
      if (s.phase === 'idle') assert.equal(s.offset, 0);
    }
  }
});

function randomInput(r: Random, ctx: SwipeContext, t: number, kinds: 10 | 8): SwipeInput {
  const k = r.int(kinds);
  if (kinds === 10) {
    if (k <= 4) return { type: 'drag', dx: (r.next() - 0.5) * 2.4 * ctx.width, timeMs: t };
    if (k === 5) return { type: 'release', velocity: (r.next() - 0.5) * 6000, timeMs: t };
    if (k === 6) return { type: 'cancel', reason: r.pick(swipeCancels) };
    if (k === 7) return { type: 'animation-end' };
    if (k === 8) return { type: 'sheet-result', chosen: r.bool() };
    return { type: 'reset' };
  }
  if (k <= 3) return { type: 'drag', dx: (r.next() - 0.5) * 2.4 * ctx.width, timeMs: t };
  if (k === 4) return { type: 'release', velocity: (r.next() - 0.5) * 6000, timeMs: t };
  if (k === 5) return { type: 'animation-end' };
  if (k === 6) return { type: 'sheet-result', chosen: r.bool() };
  return { type: 'cancel', reason: 'second-pointer' };
}

test('anyInputSequenceKeepsTheInvariants', () => {
  const seeds = new Random(SEED + 7);
  for (let i = 0; i < ITERATIONS; i++) {
    const seed = seeds.int(0, 2 ** 31);
    const r = new Random(seed);
    const ctx = randomContext(r);
    const m = new SwipeMachine(TestTuning, ctx);
    let t = 0;
    let committedOrParked = false;
    for (let n = r.int(5, 60); n > 0; n--) {
      t += r.int(0, 100);
      const input = randomInput(r, ctx, t, 10);
      const before = m.state.phase;
      const beforeAction = m.state.action;
      const fx = m.send(input);
      for (const e of fx) {
        if (e.type === 'commit' || e.type === 'park') committedOrParked = true;
        if (e.type === 'completed') assert.ok(committedOrParked, `Completed without a commit (seed ${seed})`);
      }
      if (input.type === 'reset') committedOrParked = false;
      if (m.state.phase === 'done') {
        assert.ok(['committing', 'parked', 'done'].includes(before), `done from ${before} (seed ${seed})`);
      }
      assert.ok(Math.abs(m.state.offset) <= ctx.width + 0.001, `offset out of range (seed ${seed})`);
      const keepsRow = !beforeAction.removesRow;
      if ((before === 'committing' && !keepsRow) || before === 'parked' || before === 'done') {
        assert.ok(input.type !== 'drag' || m.state.phase === before, `the finger moved a card that is leaving (seed ${seed})`);
      }
    }
  }
});

test('everyStateIsReachable', () => {
  const seen = new Set<SwipePhase>();
  const seeds = new Random(7);
  for (let i = 0; i < 3_000; i++) {
    const r = new Random(seeds.int(0, 2 ** 31));
    const ctx = randomContext(r);
    const m = new SwipeMachine(TestTuning, ctx);
    let t = 0;
    for (let n = r.int(5, 40); n > 0; n--) {
      t += r.int(0, 100);
      m.send(randomInput(r, ctx, t, 8));
      seen.add(m.state.phase);
    }
  }
  assert.deepEqual([...seen].sort(), [...swipePhases].sort(), 'not every state was reached');
});
