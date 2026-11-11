// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// Pure black: base surfaces are #000000, raised containers stay lighter (docs/ux/theme.md).

import assert from 'node:assert/strict';
import { test } from 'node:test';
import { CONTRASTS, PURE_BLACK_ROLES, presetsOf } from '../src/schemes.ts';
import { loadTokens } from '../src/tokens.ts';
import { committedDir, importGeneratedTs, parseCss, parseKotlin, parseSwift, readOutput, tokensPath, cssRoleName } from './helpers.ts';

const presets = presetsOf(loadTokens(tokensPath));
const ts = await importGeneratedTs(committedDir);
const kotlin = parseKotlin(readOutput(committedDir, 'kotlin'));
const swift = parseSwift(readOutput(committedDir, 'swift'));
const css = parseCss(readOutput(committedDir, 'css'));

const luma = (hex: string): number => parseInt(hex.slice(1), 16);
const lightness = (hex: string): number => {
  const [r, g, b] = [1, 3, 5].map((i) => parseInt(hex.slice(i, i + 2), 16));
  return 0.2126 * (r as number) + 0.7152 * (g as number) + 0.0722 * (b as number);
};

test('pure black sets the darkest surface roles to #000000 in every output', () => {
  assert.deepEqual([...PURE_BLACK_ROLES].sort(), ['background', 'surface', 'surfaceContainerLowest', 'surfaceDim']);
  for (const p of presets) {
    for (const c of CONTRASTS) {
      const key = `${p.id}/pureBlack/${c.id}`;
      for (const role of PURE_BLACK_ROLES) {
        assert.equal(ts.croftSchemes[p.id].pureBlack[c.id][role], '#000000', `TS ${key} ${role}`);
        assert.equal(kotlin.schemes.get(key)?.[role], '#000000', `Kotlin ${key} ${role}`);
        assert.equal(swift.schemes.get(key)?.[role], '#000000', `Swift ${key} ${role}`);
        assert.equal(css.schemes.get(key)?.[cssRoleName(role)], '#000000', `CSS ${key} ${role}`);
      }
    }
  }
});

test('pure black keeps raised surface containers lighter than black, equal to the dark scheme', () => {
  for (const p of presets) {
    for (const c of CONTRASTS) {
      const black = ts.croftSchemes[p.id].pureBlack[c.id] as Record<string, string>;
      const dark = ts.croftSchemes[p.id].dark[c.id] as Record<string, string>;
      for (const role of ['surfaceContainerLow', 'surfaceContainer', 'surfaceContainerHigh', 'surfaceContainerHighest', 'surfaceBright']) {
        assert.notEqual(black[role], '#000000', `${p.id} ${c.id} ${role} must stay visible`);
        assert.ok(lightness(black[role] as string) > 0, role);
        assert.equal(black[role], dark[role], `${p.id} ${c.id} ${role} unchanged from dark`);
      }
      // The containers step up in lightness from the black base.
      const steps = ['surfaceContainerLowest', 'surfaceContainerLow', 'surfaceContainer', 'surfaceContainerHigh', 'surfaceContainerHighest'].map(
        (r) => lightness(black[r] as string),
      );
      assert.deepEqual([...steps].sort((a, b) => a - b), steps, `${p.id} ${c.id} container order`);
    }
  }
});

test('only the pure black base roles differ between dark and pure black', () => {
  for (const p of presets) {
    const black = ts.croftSchemes[p.id].pureBlack.standard as Record<string, string>;
    const dark = ts.croftSchemes[p.id].dark.standard as Record<string, string>;
    const changed = Object.keys(dark).filter((r) => dark[r] !== black[r]);
    assert.deepEqual(changed.sort(), [...PURE_BLACK_ROLES].sort(), p.id);
    assert.ok(luma(dark.background as string) > 0, 'dark is not black');
  }
});

test('light and dark schemes are not black', () => {
  for (const p of presets) {
    assert.notEqual(ts.croftSchemes[p.id].dark.standard.surface, '#000000');
    assert.notEqual(ts.croftSchemes[p.id].light.standard.surface, '#000000');
  }
});
