// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// Spring response curves for the web. CSS has no spring easing, so each spring token is sampled
// into a `linear()` easing. Compose and SwiftUI use the stiffness and damping directly.

const SAMPLES = 32;

/** Position of a unit-mass spring released from rest towards 1. `damping` is the damping ratio. */
export function springPosition(stiffness: number, damping: number, t: number): number {
  const w = Math.sqrt(stiffness);
  if (damping >= 1) {
    return 1 - Math.exp(-w * t) * (1 + w * t);
  }
  const wd = w * Math.sqrt(1 - damping * damping);
  return 1 - Math.exp(-damping * w * t) * (Math.cos(wd * t) + ((damping * w) / wd) * Math.sin(wd * t));
}

/** First time (to 1 ms) after which the spring stays within 0.5 percent of its target. */
export function springDuration(stiffness: number, damping: number): number {
  let last = 0;
  for (let ms = 1; ms <= 3000; ms++) {
    if (Math.abs(springPosition(stiffness, damping, ms / 1000) - 1) > 0.005) last = ms;
  }
  return (last + 1) / 1000;
}

function round4(n: number): string {
  const s = n.toFixed(4).replace(/\.?0+$/, '');
  return s === '' || s === '-0' ? '0' : s;
}

export function springLinearEasing(stiffness: number, damping: number): { duration: number; easing: string } {
  const duration = springDuration(stiffness, damping);
  const points: string[] = [];
  for (let i = 0; i <= SAMPLES; i++) {
    points.push(i === SAMPLES ? '1' : round4(springPosition(stiffness, damping, (duration * i) / SAMPLES)));
  }
  return { duration, easing: `linear(${points.join(', ')})` };
}
