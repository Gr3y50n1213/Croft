// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

/**
 * The swipe state machine (docs/ux/swipe.md, "Swipe state machine"), ported one for one from
 * SwipeMachine.kt (T30b): a pure reducer, no UI types and no clock (times arrive with the inputs).
 * On the web it drives trackpad two-finger swipes the same way it drives touch on Android.
 */

import {
  SwipeAction,
  directionOf,
  physicalSign,
  type SwipeActionInfo,
  type SwipeDirection,
  type SwipeSensitivity
} from './model.ts';

/** The numbers the machine needs (from the tokens; tests give their own). */
export interface SwipeTuning {
  readonly armLow: number;
  readonly armNormal: number;
  readonly armHigh: number;
  /** A release faster than this (px/s, in the swipe direction) commits from `flingMinTravelFraction`. */
  readonly flingMinVelocity: number;
  readonly flingMinTravelFraction: number;
  /** Parking actions hold the card at this fraction of the row width. */
  readonly parkFraction: number;
  /** Beyond this fraction the card follows the finger at `resistanceFactor` speed. */
  readonly resistanceStartFraction: number;
  readonly resistanceFactor: number;
  /** A second arm or disarm within this many milliseconds of the last is held back (jitter). */
  readonly crossingGuardMs: number;
}

export function armFraction(t: SwipeTuning, s: SwipeSensitivity): number {
  return s === 'low' ? t.armLow : s === 'normal' ? t.armNormal : t.armHigh;
}

/** What one direction does for one row right now. */
export interface SwipeSlot {
  readonly action: SwipeActionInfo;
  /** False when the row cannot do it (Archive in Sent). */
  readonly available: boolean;
  readonly parks: boolean;
}

export function slotOf(action: SwipeActionInfo, available = true, parks: boolean = action.parks): SwipeSlot {
  return { action, available, parks };
}

/** The row: its width (the unit of every distance), layout direction, sensitivity and slots. */
export interface SwipeContext {
  readonly width: number;
  readonly rtl: boolean;
  readonly sensitivity: SwipeSensitivity;
  readonly startToEnd: SwipeSlot;
  readonly endToStart: SwipeSlot;
}

export function contextSlot(ctx: SwipeContext, d: SwipeDirection): SwipeSlot {
  return d === 'start-to-end' ? ctx.startToEnd : ctx.endToStart;
}

export type SwipePhase = 'idle' | 'dragging' | 'armed' | 'settling-back' | 'committing' | 'parked' | 'done';
export const swipePhases: readonly SwipePhase[] = [
  'idle',
  'dragging',
  'armed',
  'settling-back',
  'committing',
  'parked',
  'done'
];

/**
 * What the machine knows. `distance` is the finger's distance in `direction` (never negative, never
 * resisted); `offset` is where the card is or is heading (signed, positive to the right).
 */
export interface SwipeState {
  readonly phase: SwipePhase;
  readonly direction: SwipeDirection | null;
  readonly action: SwipeActionInfo;
  readonly distance: number;
  readonly offset: number;
  /** Time of the last arm or disarm, for the guard. */
  readonly lastCrossingMs: number | null;
  /** An unavailable action was dragged past its arm point (Rejected has played). */
  readonly rejectedSignalled: boolean;
}

export const initialSwipeState: SwipeState = Object.freeze({
  phase: 'idle',
  direction: null,
  action: SwipeAction.NONE,
  distance: 0,
  offset: 0,
  lastCrossingMs: null,
  rejectedSignalled: false
});

export type SwipeCancel = 'second-pointer' | 'selection' | 'row-removed' | 'disabled';
export const swipeCancels: readonly SwipeCancel[] = ['second-pointer', 'selection', 'row-removed', 'disabled'];

export type SwipeInput =
  | { readonly type: 'drag'; readonly dx: number; readonly timeMs: number }
  | { readonly type: 'release'; readonly velocity: number; readonly timeMs: number }
  | { readonly type: 'cancel'; readonly reason: SwipeCancel }
  | { readonly type: 'animation-end' }
  | { readonly type: 'sheet-result'; readonly chosen: boolean }
  | { readonly type: 'reset' };

export type SwipeHaptic = 'threshold-armed' | 'threshold-disarmed' | 'committed' | 'rejected';

export type SwipeEffect =
  | { readonly type: 'haptic'; readonly kind: SwipeHaptic }
  | { readonly type: 'commit'; readonly action: SwipeActionInfo; readonly direction: SwipeDirection }
  | { readonly type: 'park'; readonly action: SwipeActionInfo; readonly direction: SwipeDirection }
  | { readonly type: 'spring-back' }
  | { readonly type: 'not-available'; readonly action: SwipeActionInfo }
  | { readonly type: 'completed'; readonly action: SwipeActionInfo; readonly direction: SwipeDirection };

export interface SwipeStep {
  readonly state: SwipeState;
  readonly effects: readonly SwipeEffect[];
}

const haptic = (kind: SwipeHaptic): SwipeEffect => ({ type: 'haptic', kind });
const SPRING_BACK: SwipeEffect = Object.freeze({ type: 'spring-back' });
const step = (state: SwipeState, effects: readonly SwipeEffect[] = []): SwipeStep => ({ state, effects });

/** The card's offset for a finger distance: one to one, then resisted, never past the width. */
export function resisted(t: SwipeTuning, width: number, distance: number): number {
  const start = t.resistanceStartFraction * width;
  const d = Math.max(0, distance);
  const moved = d <= start ? d : start + (d - start) * t.resistanceFactor;
  return Math.min(moved, width);
}

/** The distance at which the action arms. */
export function threshold(t: SwipeTuning, ctx: SwipeContext): number {
  return armFraction(t, ctx.sensitivity) * ctx.width;
}

/** A committed read or star only springs back, so a new touch can take the card. */
function isGesture(s: SwipeState): boolean {
  return (
    s.phase === 'idle' ||
    s.phase === 'dragging' ||
    s.phase === 'armed' ||
    s.phase === 'settling-back' ||
    (s.phase === 'committing' && !s.action.removesRow)
  );
}

/** Applies `input`. */
export function reduce(t: SwipeTuning, ctx: SwipeContext, s: SwipeState, input: SwipeInput): SwipeStep {
  switch (input.type) {
    case 'drag':
      return drag(t, ctx, s, input.dx, input.timeMs);
    case 'release':
      return release(t, ctx, s, input.velocity, input.timeMs);
    case 'cancel':
      return cancel(s);
    case 'animation-end':
      return animationEnd(s);
    case 'sheet-result':
      return sheetResult(ctx, s, input.chosen);
    case 'reset':
      return step(initialSwipeState);
  }
}

function drag(t: SwipeTuning, ctx: SwipeContext, s: SwipeState, dx: number, timeMs: number): SwipeStep {
  if (!isGesture(s)) return step(s);
  // A touch during the spring back takes the card over: a new gesture from the card's place.
  const base = s.phase === 'settling-back' || s.phase === 'committing' ? initialSwipeState : s;
  const effects: SwipeEffect[] = [];
  if (dx === 0) {
    const dir = base.direction;
    if (dir === null) return step(base);
    return crossing(t, ctx, { ...base, distance: 0, offset: 0 }, dir, 0, timeMs, true, effects);
  }
  const dir = directionOf(dx, ctx.rtl);
  const slot = contextSlot(ctx, dir);
  let cur = base;
  // Moving over to the other side while armed: the armed action is not under the card any more.
  if (cur.direction !== null && cur.direction !== dir && cur.phase === 'armed') {
    effects.push(haptic('threshold-disarmed'));
    cur = { ...cur, phase: 'dragging', lastCrossingMs: timeMs };
  }
  if (cur.direction !== null && cur.direction !== dir) cur = { ...cur, rejectedSignalled: false };
  if (slot.action === SwipeAction.NONE) {
    return step({ ...initialSwipeState, lastCrossingMs: cur.lastCrossingMs }, effects);
  }
  const distance = Math.abs(dx);
  const moved: SwipeState = {
    ...cur,
    phase: cur.phase === 'idle' ? 'dragging' : cur.phase,
    direction: dir,
    action: slot.action,
    distance
  };
  return crossing(t, ctx, moved, dir, distance, timeMs, true, effects);
}

/** Looks at `distance` against the arm point and flips arm or reject once if it should (and the guard allows). */
function crossing(
  t: SwipeTuning,
  ctx: SwipeContext,
  s: SwipeState,
  dir: SwipeDirection,
  distance: number,
  timeMs: number,
  guarded: boolean,
  effects: SwipeEffect[]
): SwipeStep {
  const slot = contextSlot(ctx, dir);
  const sign = physicalSign(dir, ctx.rtl);
  const beyond = distance >= threshold(t, ctx) && slot.action !== SwipeAction.NONE;
  const last = s.lastCrossingMs;
  const allowed = !guarded || last === null || timeMs - last >= t.crossingGuardMs;
  let phase: SwipePhase = s.phase === 'idle' || s.phase === 'settling-back' ? 'dragging' : s.phase;
  let rejected = s.rejectedSignalled;
  let lastCrossing = s.lastCrossingMs;
  if (slot.available) {
    const armed = phase === 'armed';
    if (beyond !== armed && allowed) {
      phase = beyond ? 'armed' : 'dragging';
      effects.push(haptic(beyond ? 'threshold-armed' : 'threshold-disarmed'));
      lastCrossing = timeMs;
    }
  } else if (beyond !== rejected && allowed) {
    rejected = beyond;
    if (beyond) effects.push(haptic('rejected'));
    lastCrossing = timeMs;
  }
  const offset = sign * resisted(t, ctx.width, distance);
  return step(
    {
      ...s,
      phase,
      direction: dir,
      action: slot.action,
      distance,
      offset,
      lastCrossingMs: lastCrossing,
      rejectedSignalled: rejected
    },
    effects
  );
}

function release(t: SwipeTuning, ctx: SwipeContext, s: SwipeState, velocity: number, timeMs: number): SwipeStep {
  if (s.phase !== 'dragging' && s.phase !== 'armed') return step(s);
  const dir = s.direction;
  if (dir === null) return step(initialSwipeState);
  const effects: SwipeEffect[] = [];
  // Look once more at where the finger ended, whatever the guard says.
  const settled = crossing(t, ctx, s, dir, s.distance, timeMs, false, effects).state;
  const slot = contextSlot(ctx, dir);
  const sign = physicalSign(dir, ctx.rtl);
  const along = velocity * sign;
  const fling = along >= t.flingMinVelocity && settled.distance >= t.flingMinTravelFraction * ctx.width;
  const armed = settled.phase === 'armed';
  const back: SwipeState = {
    ...initialSwipeState,
    phase: 'settling-back',
    direction: dir,
    action: slot.action
  };
  if (!slot.available) {
    if (settled.rejectedSignalled || fling) effects.push({ type: 'not-available', action: slot.action });
    effects.push(SPRING_BACK);
    return step(back, effects);
  }
  if (!armed && !fling) {
    effects.push(SPRING_BACK);
    return step(back, effects);
  }
  effects.push(haptic('committed'));
  if (slot.parks) {
    effects.push({ type: 'park', action: slot.action, direction: dir });
    return step({ ...settled, phase: 'parked', offset: sign * t.parkFraction * ctx.width }, effects);
  }
  effects.push({ type: 'commit', action: slot.action, direction: dir });
  return step({ ...settled, phase: 'committing', offset: sign * ctx.width }, effects);
}

function cancel(s: SwipeState): SwipeStep {
  if (s.phase === 'dragging' || s.phase === 'armed' || s.phase === 'parked') {
    return step({ ...initialSwipeState, phase: 'settling-back', direction: s.direction, action: s.action }, [SPRING_BACK]);
  }
  // Nothing to undo, already going back, or past the point of no return (Undo covers that).
  return step(s);
}

function animationEnd(s: SwipeState): SwipeStep {
  if (s.phase === 'settling-back') return step(initialSwipeState);
  if (s.phase === 'committing') {
    const dir = s.direction;
    if (dir === null) return step(initialSwipeState);
    return step({ ...s, phase: 'done' }, [{ type: 'completed', action: s.action, direction: dir }]);
  }
  return step(s);
}

function sheetResult(ctx: SwipeContext, s: SwipeState, chosen: boolean): SwipeStep {
  if (s.phase !== 'parked') return step(s);
  const dir = s.direction;
  if (dir === null) return step(initialSwipeState);
  if (chosen) {
    const full = physicalSign(dir, ctx.rtl) * ctx.width;
    return step({ ...s, phase: 'done', offset: full }, [{ type: 'completed', action: s.action, direction: dir }]);
  }
  return step({ ...initialSwipeState, phase: 'settling-back', direction: dir, action: s.action }, [SPRING_BACK]);
}

/** The reducer with the state kept: one per row. */
export class SwipeMachine {
  readonly #tuning: SwipeTuning;
  context: SwipeContext;
  #state: SwipeState = initialSwipeState;

  constructor(tuning: SwipeTuning, context: SwipeContext) {
    this.#tuning = tuning;
    this.context = context;
  }

  get state(): SwipeState {
    return this.#state;
  }

  /** Feeds `input`; the effects it caused. */
  send(input: SwipeInput): readonly SwipeEffect[] {
    const next = reduce(this.#tuning, this.context, this.#state, input);
    this.#state = next.state;
    return next.effects;
  }
}
