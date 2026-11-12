// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

/** The swipe machine's numbers, from the design tokens (never written out here). */

import { croftTokens } from '../../../../design/generated/ts/croft-tokens.ts';
import type { SwipeTuning } from './machine.ts';

const s = croftTokens.swipe;

export const tuning: SwipeTuning = {
  armLow: s.armAtFractionOfRowWidth.low,
  armNormal: s.armAtFractionOfRowWidth.normal,
  armHigh: s.armAtFractionOfRowWidth.high,
  flingMinVelocity: s.flingMinVelocityDpPerSec,
  flingMinTravelFraction: s.flingMinTravelFraction,
  parkFraction: s.parkAtFractionOfRowWidth,
  resistanceStartFraction: s.resistance.startsAtFraction,
  resistanceFactor: s.resistance.factor,
  crossingGuardMs: croftTokens.haptics.minGapMs
};
