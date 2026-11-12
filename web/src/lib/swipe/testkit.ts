// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// The test kit of android/swipe (SwipeTestKit.kt), ported: the numbers of docs/ux/swipe.md written out.

import { SwipeAction, type SwipeSensitivity } from './model.ts';
import { SwipeMachine, slotOf, type SwipeContext, type SwipeEffect, type SwipeHaptic, type SwipeSlot, type SwipeTuning } from './machine.ts';
import type { SwipeGeometryConfig } from './geometry.ts';

export const TestTuning: SwipeTuning = {
  armLow: 0.45,
  armNormal: 0.35,
  armHigh: 0.25,
  flingMinVelocity: 1200,
  flingMinTravelFraction: 0.2,
  parkFraction: 0.4,
  resistanceStartFraction: 0.85,
  resistanceFactor: 0.3333,
  crossingGuardMs: 40
};

export const TestGeometry: SwipeGeometryConfig = {
  gapToCard: 4,
  insetFromEdge: 8,
  widthIsDragMinus: 12,
  cardEaseInDistance: 16,
  iconSize: 24
};

export const W = 400;

export function ctx(o: {
  start?: SwipeSlot;
  end?: SwipeSlot;
  rtl?: boolean;
  sensitivity?: SwipeSensitivity;
  width?: number;
} = {}): SwipeContext {
  return {
    width: o.width ?? W,
    rtl: o.rtl ?? false,
    sensitivity: o.sensitivity ?? 'normal',
    startToEnd: o.start ?? slotOf(SwipeAction.TOGGLE_READ),
    endToStart: o.end ?? slotOf(SwipeAction.DELETE)
  };
}

export function machine(c: SwipeContext = ctx()): SwipeMachine {
  return new SwipeMachine(TestTuning, c);
}

export const drag = (m: SwipeMachine, dx: number, t: number) => m.send({ type: 'drag', dx, timeMs: t });
export const release = (m: SwipeMachine, v = 0, t = 10_000) => m.send({ type: 'release', velocity: v, timeMs: t });

export function haptics(fx: readonly SwipeEffect[]): SwipeHaptic[] {
  return fx.flatMap((e) => (e.type === 'haptic' ? [e.kind] : []));
}

export const close = (actual: number, expected: number, eps: number) => Math.abs(actual - expected) <= eps;
