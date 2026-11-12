// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// SwipeGeometryTest.kt, ported: golden tests on the pill (docs/ux/swipe.md, "The pill") for drag
// distances from 0 to 100 percent of the row, at all three roundness settings.

import { test } from 'node:test';
import assert from 'node:assert/strict';
import { cardRadius, committedPill, pill } from './geometry.ts';
import { TestGeometry, close } from './testkit.ts';

const rowWidth = 400;
const rowHeight = 88;
const radii: Record<string, number> = { tight: 10, standard: 17, round: 24 };
const approx = (a: number, e: number, eps: number, msg: string) => assert.ok(close(a, e, eps), `${msg}: expected ${e}, got ${a}`);

test('literalGoldenRows', () => {
  // offset, width, radius, icon visible (hand-computed from the rules, not by the code under test)
  const golden = [
    [0, 0, 0, 0],
    [12, 0, 0, 0],
    [20, 8, 4, 0],
    [36, 24, 12, 1],
    [100, 88, 44, 1],
    [140, 128, 44, 1],
    [400, 388, 44, 1]
  ] as const;
  for (const [offset, width, radius, icon] of golden) {
    const p = pill(TestGeometry, offset, rowHeight);
    approx(p.width, width, 0.0001, `width at ${offset}`);
    approx(p.radius, radius, 0.0001, `radius at ${offset}`);
    assert.equal(p.iconVisible, icon === 1, `icon at ${offset}`);
    assert.equal(p.height, rowHeight);
    assert.equal(p.gapToCard, 4);
    assert.equal(p.insetFromEdge, 8);
  }
});

test('everyDragDistanceFromZeroToTheFullWidthAtEveryRoundness', () => {
  for (const [name, cardR] of Object.entries(radii)) {
    for (let percent = 0; percent <= 100; percent++) {
      const offset = (rowWidth * percent) / 100;
      const p = pill(TestGeometry, offset, rowHeight);
      const tag = `${name} ${percent}%`;
      approx(p.width, Math.max(0, offset - 12), 0.0001, `${tag} width`);
      assert.equal(p.height, rowHeight, `${tag} height`);
      approx(p.radius, Math.min(p.width, rowHeight) / 2, 0.0001, `${tag} radius`);
      assert.equal(p.gapToCard, 4);
      assert.equal(p.insetFromEdge, 8);
      assert.ok(p.width + p.gapToCard + p.insetFromEdge <= Math.max(offset, 12) + 0.0001, `${tag} fits`);
      if (p.width <= rowHeight) approx(p.radius, p.width / 2, 0.0001, `${tag} capsule`);
      else approx(p.radius, rowHeight / 2, 0.0001, `${tag} stadium`);
      const corner = cardRadius(TestGeometry, offset, 0, cardR);
      if (offset >= 16) approx(corner, cardR, 0.0001, `${tag} corner`);
      assert.ok(corner >= 0 && corner <= cardR, `${tag} corner range`);
    }
  }
});

test('theCardsCornerEasesUpOverTheFirst16dp', () => {
  for (const [name, r] of Object.entries(radii)) {
    assert.equal(cardRadius(TestGeometry, 0, 0, r), 0, name);
    approx(cardRadius(TestGeometry, 8, 0, r), r / 2, 0.0001, name);
    approx(cardRadius(TestGeometry, 16, 0, r), r, 0.0001, name);
    approx(cardRadius(TestGeometry, 300, 0, r), r, 0.0001, name);
  }
  assert.equal(cardRadius(TestGeometry, 0, 17, 17), 17);
});

test('theIconAppearsOnceThePillIsWideEnoughToHoldIt', () => {
  assert.equal(pill(TestGeometry, 35.9, rowHeight).iconVisible, false);
  assert.equal(pill(TestGeometry, 36, rowHeight).iconVisible, true);
});

test('theCommittedPillFillsTheRowEightDpInFromEachEdge', () => {
  const p = committedPill(TestGeometry, rowWidth, rowHeight);
  assert.equal(p.width, 384);
  assert.equal(p.radius, 44);
  assert.ok(p.iconVisible);
});
