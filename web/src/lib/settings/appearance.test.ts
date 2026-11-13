// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { runInNewContext } from 'node:vm';
import {
  CONTRASTS,
  DENSITIES,
  LIST_STYLES,
  MODES,
  MOTIONS,
  PALETTES,
  ROUNDNESSES,
  attributesOf,
  defaultAppearance,
  parseAppearance,
  serializeAppearance,
  type Appearance
} from './appearance.ts';
import { parseSwipe, serializeSwipe } from './swipe.ts';
import { SwipeAction, defaultSwipeSettings } from '../swipe/model.ts';

test('nothing stored, or anything unreadable, is the defaults', () => {
  for (const s of [null, undefined, '', 'not json', '[]', '42', 'null', '"moss"', '{']) {
    assert.deepEqual(parseAppearance(s), defaultAppearance, String(s));
  }
});

test('the defaults are the inventory defaults (docs/ux/customisation.md)', () => {
  assert.deepEqual(defaultAppearance, {
    mode: 'system',
    pureBlack: false,
    palette: 'moss',
    contrast: 'standard',
    density: 'default',
    roundness: 'standard',
    list: 'container',
    motion: 'system'
  });
});

test('every palette from the tokens is offered', () => {
  assert.deepEqual(PALETTES, ['moss', 'wax', 'slate', 'heather', 'ochre', 'peat', 'tide', 'rowan']);
});

test('unknown values fall back field by field; known ones are kept', () => {
  const a = parseAppearance(
    JSON.stringify({ mode: 'dark', palette: 'neon', contrast: 'high', density: 7, roundness: 'round', pureBlack: 'yes', extra: 1 })
  );
  assert.deepEqual(a, { ...defaultAppearance, mode: 'dark', contrast: 'high', roundness: 'round' });
});

test('stored and read back is the same; only changes are stored', () => {
  assert.equal(serializeAppearance(defaultAppearance), '{}');
  const a: Appearance = { ...defaultAppearance, palette: 'tide', density: 'compact', pureBlack: true };
  assert.deepEqual(JSON.parse(serializeAppearance(a)), { palette: 'tide', density: 'compact', pureBlack: true });
  assert.deepEqual(parseAppearance(serializeAppearance(a)), a);
});

test('mode follows the system only when it is System; pure black only in dark', () => {
  const at = (a: Partial<Appearance>, dark: boolean) => attributesOf({ ...defaultAppearance, ...a }, dark, false)['data-croft-mode'];
  assert.equal(at({}, false), 'light');
  assert.equal(at({}, true), 'dark');
  assert.equal(at({ mode: 'light' }, true), 'light');
  assert.equal(at({ mode: 'dark' }, false), 'dark');
  assert.equal(at({ mode: 'dark', pureBlack: true }, false), 'pure-black');
  assert.equal(at({ pureBlack: true }, true), 'pure-black');
  assert.equal(at({ mode: 'light', pureBlack: true }, true), 'light');
});

test('animation follows the system only when it is Follow system', () => {
  const at = (motion: Appearance['motion'], reduced: boolean) =>
    attributesOf({ ...defaultAppearance, motion }, false, reduced)['data-croft-motion'];
  assert.equal(at('system', false), 'full');
  assert.equal(at('system', true), 'reduced');
  assert.equal(at('full', true), 'full');
  assert.equal(at('off', false), 'off');
});

/** Runs static/theme.js against a fake page and returns the attributes it set. */
function themeJs(stored: string | null, dark: boolean, reduced: boolean): Record<string, string> {
  const src = readFileSync(new URL('../../../static/theme.js', import.meta.url), 'utf8');
  const attrs: Record<string, string> = {};
  const window = {
    localStorage: {
      getItem: (k: string) => (k === 'croft.appearance' ? stored : null)
    },
    matchMedia: (q: string) => ({ matches: q.includes('dark') ? dark : q.includes('reduce') ? reduced : false })
  };
  const document = { documentElement: { setAttribute: (k: string, v: string) => (attrs[k] = v) } };
  runInNewContext(src, { window, document });
  return attrs;
}

test('theme.js paints the first frame exactly as the app applies it', () => {
  // Every value of every field once, against both system states, plus junk.
  const cases: (string | null)[] = [null, '', 'junk', '[1]', '{"mode":"neon","palette":1}'];
  const fields: [keyof Appearance, readonly unknown[]][] = [
    ['mode', MODES],
    ['palette', PALETTES],
    ['contrast', CONTRASTS],
    ['density', DENSITIES],
    ['roundness', ROUNDNESSES],
    ['list', LIST_STYLES],
    ['motion', MOTIONS],
    ['pureBlack', [true, false, 'true']]
  ];
  for (const [k, values] of fields) for (const v of values) cases.push(JSON.stringify({ [k]: v, mode: k === 'mode' ? v : 'dark' }));
  for (const stored of cases) {
    for (const dark of [false, true]) {
      for (const reduced of [false, true]) {
        assert.deepEqual(themeJs(stored, dark, reduced), attributesOf(parseAppearance(stored), dark, reduced), `${stored} ${dark} ${reduced}`);
      }
    }
  }
});

test('theme.js survives storage that throws (blocked site data)', () => {
  const src = readFileSync(new URL('../../../static/theme.js', import.meta.url), 'utf8');
  const attrs: Record<string, string> = {};
  const window = {
    get localStorage(): never {
      throw new Error('SecurityError');
    },
    matchMedia: () => ({ matches: false })
  };
  runInNewContext(src, { window, document: { documentElement: { setAttribute: (k: string, v: string) => (attrs[k] = v) } } });
  assert.deepEqual(attrs, attributesOf(defaultAppearance, false, false));
});

test('swipe settings are stored by action id and read back safely', () => {
  assert.deepEqual(parseSwipe(null), defaultSwipeSettings);
  assert.deepEqual(parseSwipe('{"startToEnd":"bogus","endToStart":5,"sensitivity":"max"}'), defaultSwipeSettings);
  // Snooze is not offered yet, so a stored Snooze (from a newer app) falls back.
  assert.deepEqual(parseSwipe('{"startToEnd":"snooze"}'), defaultSwipeSettings);
  const s = { startToEnd: SwipeAction.ARCHIVE, endToStart: SwipeAction.SPAM, sensitivity: 'high' as const };
  assert.equal(serializeSwipe(s), '{"startToEnd":"archive","endToStart":"spam","sensitivity":"high"}');
  assert.deepEqual(parseSwipe(serializeSwipe(s)), s);
});
