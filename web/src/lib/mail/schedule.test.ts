// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

import { test } from 'node:test';
import assert from 'node:assert/strict';
import { MAX_AHEAD_MS, describe, fromLocalInput, presets, problemWith, toLocalInput, utcDate } from './schedule.ts';

const local = (y: number, m: number, d: number, h = 0, mi = 0) => new Date(y, m - 1, d, h, mi);

test('presets: tomorrow morning and afternoon, and the next Monday unless that is tomorrow', () => {
  // A Wednesday.
  const wed = presets(local(2026, 11, 11, 15, 30));
  assert.deepEqual(
    wed.map((p) => [p.label, p.at.getDate(), p.at.getHours(), p.at.getMinutes()]),
    [
      ['Tomorrow morning', 12, 8, 0],
      ['Tomorrow afternoon', 12, 13, 0],
      ['Monday morning', 16, 8, 0]
    ]
  );
  // Sunday: Monday morning is tomorrow morning, listed once.
  assert.deepEqual(presets(local(2026, 11, 15, 9)).map((p) => p.label), ['Tomorrow morning', 'Tomorrow afternoon']);
  // Monday: the Monday after.
  assert.equal(presets(local(2026, 11, 16, 9))[2]!.at.getDate(), 23);
  // Across a month end.
  assert.equal(presets(local(2026, 11, 30, 9))[0]!.at.getMonth(), 11);
});

test('a time is sent as a UTCDate in whole seconds', () => {
  assert.equal(utcDate(new Date(Date.UTC(2026, 10, 14, 8, 0, 0, 567))), '2026-11-14T08:00:00Z');
});

test('a chosen time must be at least a minute away and within a year', () => {
  const now = local(2026, 11, 13, 12);
  assert.equal(problemWith(new Date(now.getTime() + 2 * 60_000), now), null);
  assert.match(problemWith(new Date(now.getTime() + 30_000), now)!, /minute/);
  assert.match(problemWith(local(2026, 11, 12), now)!, /minute/);
  assert.match(problemWith(new Date(now.getTime() + MAX_AHEAD_MS + 1), now)!, /year/);
  assert.match(problemWith(new Date(NaN), now)!, /date/);
});

test('the date and time picker reads strictly and refuses dates that do not exist', () => {
  assert.equal(fromLocalInput('2026-11-14T08:30').getTime(), local(2026, 11, 14, 8, 30).getTime());
  for (const bad of ['', '2026-11-14', '2026-11-31T08:00', '2026-02-29T08:00', '2026-11-14T8:30', 'x']) {
    assert.ok(Number.isNaN(fromLocalInput(bad).getTime()), bad);
  }
  assert.equal(toLocalInput(local(2026, 1, 5, 7, 4)), '2026-01-05T07:04');
});

test('a scheduled time reads as today, tomorrow, a weekday or a date', () => {
  const now = local(2026, 11, 13, 12);
  assert.match(describe(local(2026, 11, 13, 18), now), /^Today, /);
  assert.match(describe(local(2026, 11, 14, 8), now), /^Tomorrow, /);
  assert.doesNotMatch(describe(local(2026, 11, 20, 8), now), /^(Today|Tomorrow)/);
  assert.match(describe(local(2027, 1, 20, 8), now), /2027/);
});
