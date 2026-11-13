// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

import { test } from 'node:test';
import assert from 'node:assert/strict';
import { segments } from './highlight.ts';

const show = (text: string, r: [number, number][]) => segments(text, r).map((s) => (s.mark ? `[${s.text}]` : s.text)).join('');

test('matches are marked where the ranges say', () => {
  assert.equal(show('Seed swap on Saturday', [[0, 4], [13, 21]]), '[Seed] swap on [Saturday]');
  assert.equal(show('Message 2', [[0, 9]]), '[Message 2]');
  assert.equal(show('Plain', []), 'Plain');
  assert.equal(show('', []), '');
});

test('ranges are counted in UTF-16 code units, as the Companion sends them', () => {
  // The emoji is two code units.
  assert.equal(show('🌱 seeds', [[3, 8]]), '🌱 [seeds]');
});

test('bad ranges are clamped or skipped, never trusted', () => {
  assert.equal(show('abcdef', [[4, 2]]), 'abcdef');
  assert.equal(show('abcdef', [[2, 4], [3, 5]]), 'ab[cd][e]f');
  assert.equal(show('abc', [[1, 99]]), 'a[bc]');
  assert.equal(show('abc', [[50, 60]]), 'abc');
  assert.equal(show('abc', [[-5, 1]]), '[a]bc');
  assert.equal(show('abc', [[0.5, 2] as [number, number]]), 'abc');
});
