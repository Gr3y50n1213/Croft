// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// Golden test: one colour role gives the same value in Kotlin, Swift, CSS and TypeScript output,
// for every preset in light, dark and pure black, at every contrast level.

import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { test } from 'node:test';
import { CONTRASTS, MODES, ROLES, presetsOf } from '../src/schemes.ts';
import { loadTokens } from '../src/tokens.ts';
import {
  committedDir, cssRoleName, importGeneratedTs, parseCss, parseKotlin, parseSwift, readOutput, tokensPath,
} from './helpers.ts';

const tokens = loadTokens(tokensPath);
const presets = presetsOf(tokens);
const kotlin = parseKotlin(readOutput(committedDir, 'kotlin'));
const swift = parseSwift(readOutput(committedDir, 'swift'));
const css = parseCss(readOutput(committedDir, 'css'));
const ts = await importGeneratedTs(committedDir);

test('there are eight presets, as in docs/ux/theme.md', () => {
  assert.deepEqual(
    presets.map((p) => p.id),
    ['moss', 'wax', 'slate', 'heather', 'ochre', 'peat', 'tide', 'rowan'],
  );
});

test('every output holds every preset, mode and contrast, with every role', () => {
  const expected = presets.length * MODES.length * CONTRASTS.length;
  assert.equal(expected, 72);
  for (const [name, schemes] of [['kotlin', kotlin.schemes], ['swift', swift.schemes], ['css', css.schemes]] as const) {
    assert.equal(schemes.size, expected, `${name} scheme count`);
    for (const roles of schemes.values()) {
      assert.equal(Object.keys(roles).length, ROLES.length, `${name} role count`);
    }
  }
});

for (const preset of presets) {
  for (const mode of MODES) {
    test(`${preset.id} ${mode}: all roles identical in Kotlin, Swift, CSS and TypeScript`, () => {
      for (const contrast of CONTRASTS) {
        const key = `${preset.id}/${mode}/${contrast.id}`;
        const fromTs = ts.croftSchemes[preset.id][mode][contrast.id] as Record<string, string>;
        const fromKotlin = kotlin.schemes.get(key);
        const fromSwift = swift.schemes.get(key);
        const fromCss = css.schemes.get(key);
        assert.ok(fromKotlin && fromSwift && fromCss, `${key} missing from an output`);
        for (const role of ROLES) {
          const want = fromTs[role];
          assert.match(want as string, /^#[0-9A-F]{6}$/, `${key} ${role}`);
          assert.equal(fromKotlin[role], want, `${key} ${role} Kotlin`);
          assert.equal(fromSwift[role], want, `${key} ${role} Swift`);
          assert.equal(fromCss[cssRoleName(role)], want, `${key} ${role} CSS`);
        }
      }
    });
  }
}

test('seeds and styles in every output match tokens.json', () => {
  const kt = readFileSync(`${committedDir}/kotlin/CroftTokens.kt`, 'utf8');
  const sw = readFileSync(`${committedDir}/swift/CroftTokens.swift`, 'utf8');
  for (const p of presets) {
    assert.equal(tokens.palettes.presets[p.id]?.toUpperCase(), p.seed);
    assert.ok(kt.includes(`("${p.id}", 0xFF${p.seed.slice(1)}, "${p.style}")`), `Kotlin preset ${p.id}`);
    assert.ok(sw.includes(`case .${p.id}: return 0x${p.seed.slice(1)}`), `Swift preset ${p.id}`);
    assert.deepEqual(ts.croftPresets[p.id], { seed: p.seed, style: p.style });
  }
  assert.equal(ts.croftDefaultPreset, tokens.palettes.default);
});

test('known anchor: Moss, light, standard contrast, primary is #35693F (HCT tonal spot, tone 40)', () => {
  assert.equal(ts.croftSchemes.moss.light.standard.primary, '#35693F');
});

test('brand colours match in every output', () => {
  const want = Object.fromEntries(Object.entries(tokens.brand).map(([k, v]) => [k, v.toUpperCase()]));
  assert.deepEqual(kotlin.brand, want);
  assert.deepEqual(swift.brand, want);
  assert.deepEqual(ts.croftBrand, want);
  for (const [name, hex] of Object.entries(want)) {
    const cssName = `--croft-brand-${name.replace(/([A-Z])/g, '-$1').toLowerCase()}`;
    assert.equal(css.vars.get(cssName), hex);
  }
});
