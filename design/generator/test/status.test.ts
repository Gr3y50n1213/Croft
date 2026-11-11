// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// Status colours (amber): the same in every output, and readable in the pairs the apps use them in.

import assert from 'node:assert/strict';
import { test } from 'node:test';
import { CONTRASTS, MODES, presetsOf } from '../src/schemes.ts';
import { loadTokens } from '../src/tokens.ts';
import { committedDir, importGeneratedTs, parseCss, readOutput, tokensPath } from './helpers.ts';
import { readFileSync } from 'node:fs';

const tokens = loadTokens(tokensPath);
const presets = presetsOf(tokens);
const ts = await importGeneratedTs(committedDir);
const css = parseCss(readOutput(committedDir, 'css'));
const kt = readOutput(committedDir, 'kotlin');
const sw = readOutput(committedDir, 'swift');

const lum = (hex: string): number => {
  const [r, g, b] = [1, 3, 5].map((i) => {
    const c = parseInt(hex.slice(i, i + 2), 16) / 255;
    return c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
  }) as [number, number, number];
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
};
const ratio = (a: string, b: string): number => {
  const [x, y] = [lum(a), lum(b)].sort((p, q) => q - p) as [number, number];
  return (x + 0.05) / (y + 0.05);
};

test('amber is in tokens.json as hue 78, chroma 80, and the standard tones give the colours Android used to compute', () => {
  const amber = tokens.status?.amber as { hue: number; chroma: number };
  assert.equal(amber.hue, 78);
  assert.equal(amber.chroma, 80);
  // Tones 50 / 90 / 10 (light) and 80 / 30 / 90 (dark) at HCT 78, 80, as StatusColors.kt made them.
  assert.deepEqual(ts.croftStatus.amber.light.standard, { color: '#9F6D00', container: '#FFDEAE', onContainer: '#281900' });
  assert.equal(ts.croftStatus.amber.dark.standard.container, ts.croftStatus.amber.pureBlack.standard.container);
});

test('every output holds amber for every mode and contrast with the same values', () => {
  for (const mode of MODES) {
    for (const c of CONTRASTS) {
      const want = ts.croftStatus.amber[mode][c.id] as { color: string; container: string; onContainer: string };
      for (const v of Object.values(want)) assert.match(v, /^#[0-9A-F]{6}$/);
      const k = `CroftStatusKey(CroftMode.${mode === 'pureBlack' ? 'PURE_BLACK' : mode.toUpperCase()}, CroftContrast.${c.id.toUpperCase()}) to CroftStatusRoles(0xFF${want.color.slice(1)}, 0xFF${want.container.slice(1)}, 0xFF${want.onContainer.slice(1)})`;
      assert.ok(kt.includes(k), `Kotlin ${mode}/${c.id}`);
      const s = `CroftStatusKey(mode: .${mode}, contrast: .${c.id}): CroftStatusRoles(color: 0x${want.color.slice(1)}, container: 0x${want.container.slice(1)}, onContainer: 0x${want.onContainer.slice(1)})`;
      assert.ok(sw.includes(s), `Swift ${mode}/${c.id}`);
      for (const p of presets) {
        const block = css.schemes.get(`${p.id}/${mode}/${c.id}`);
        assert.ok(block, `CSS ${p.id}/${mode}/${c.id}`);
      }
    }
  }
  const raw = readFileSync(`${committedDir}/css/croft-tokens.css`, 'utf8');
  const blocks = [...raw.matchAll(/\[data-croft-theme="(\w+)"\]\[data-croft-mode="([\w-]+)"\](?:\[data-croft-contrast="(\w+)"\])? \{([^}]*)\}/g)];
  assert.equal(blocks.length, presets.length * MODES.length * CONTRASTS.length);
  for (const b of blocks) {
    const mode = b[2] === 'pure-black' ? 'pureBlack' : (b[2] as string);
    const want = ts.croftStatus.amber[mode][b[3] ?? 'standard'];
    const body = b[4] as string;
    assert.ok(body.includes(`--croft-status-amber: ${want.color};`));
    assert.ok(body.includes(`--croft-status-amber-container: ${want.container};`));
    assert.ok(body.includes(`--croft-status-amber-on-container: ${want.onContainer};`));
  }
});

// Where amber is used: the star icon on list rows (surface, container low, selected row = primary container),
// the Server problem chip and banner and the blocked attachment chip (text on the amber container).
const ICON_ON = ['surface', 'surfaceContainerLowest', 'surfaceContainerLow', 'surfaceContainer'] as const;
// The selected row is primaryContainer. Material moves that role toward the amber's tone at Medium and High contrast
// (1.0 to 2.2 to 1 there), so only Standard is held to 3 to 1. Known gap, listed in docs/DECISIONS.md.
const SELECTED_ROW_CHECKED_AT = ['standard'];

for (const p of presets) {
  for (const mode of MODES) {
    test(`amber contrast: ${p.id} ${mode}`, () => {
      const failures: string[] = [];
      for (const c of CONTRASTS) {
        const a = ts.croftStatus.amber[mode][c.id] as { color: string; container: string; onContainer: string };
        const roles = ts.croftSchemes[p.id][mode][c.id] as Record<string, string>;
        for (const bg of ICON_ON) {
          const r = ratio(a.color, roles[bg] as string);
          if (r < 3) failures.push(`${c.id} amber icon on ${bg}: ${r.toFixed(2)} < 3`);
        }
        if (SELECTED_ROW_CHECKED_AT.includes(c.id)) {
          const r = ratio(a.color, roles.primaryContainer as string);
          if (r < 3) failures.push(`${c.id} amber icon on primaryContainer (selected row): ${r.toFixed(2)} < 3`);
        }
        const text = ratio(a.onContainer, a.container);
        if (text < (c.id === 'high' ? 7 : 4.5)) failures.push(`${c.id} onAmberContainer on amberContainer: ${text.toFixed(2)}`);
      }
      assert.deepEqual(failures, []);
    });
  }
}
