// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// Changing one value in tokens.json changes all four outputs.

import assert from 'node:assert/strict';
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { test } from 'node:test';
import { render } from '../src/generate.ts';
import { tokensPath } from './helpers.ts';

type Tokens = Record<string, any>;

const baseline = render(tokensPath);
const original = JSON.parse(readFileSync(tokensPath, 'utf8')) as Tokens;

function renderWithChange(change: (t: Tokens) => void): Record<string, string> {
  const dir = mkdtempSync(join(tmpdir(), 'croft-change-'));
  try {
    const copy = structuredClone(original);
    change(copy);
    mkdirSync(dir, { recursive: true });
    const file = join(dir, 'tokens.json');
    writeFileSync(file, JSON.stringify(copy, null, 2));
    return render(file);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
}

const changes: [string, (t: Tokens) => void][] = [
  ['a preset seed (Moss #4F6F52 to #4F6F60)', (t) => { t.palettes.presets.moss = '#4F6F60'; }],
  ['a brand colour (Wax)', (t) => { t.brand.wax = '#B4472F'; }],
  ['a spring stiffness (spatial fast 800 to 810)', (t) => { t.motion.spatial.fast.stiffness = 810; }],
  ['a spring damping (effects default 1.0 to 0.95)', (t) => { t.motion.effects.default.damping = 0.95; }],
  ['a density value (default row 72 to 76)', (t) => { t.density.default.rowDp = 76; }],
  ['a swipe threshold (normal arm point 0.35 to 0.36)', (t) => { t.swipe.armAtFractionOfRowWidth.normal = 0.36; }],
  ['a shape scale (round 1.4 to 1.5)', (t) => { t.shape.roundnessScale.round = 1.5; }],
  ['the touch target (48 to 52)', (t) => { t.touch.minTargetDp = 52; }],
  ['the haptics gap (40 to 50 ms)', (t) => { t.haptics.minGapMs = 50; }],
];

for (const [name, change] of changes) {
  test(`changing ${name} changes Kotlin, Swift, CSS and TypeScript`, () => {
    const changed = renderWithChange(change);
    for (const rel of Object.keys(baseline)) {
      assert.notEqual(changed[rel], baseline[rel], `${rel} did not change`);
    }
  });
}

test('changing a note (underscore key) changes nothing', () => {
  const changed = renderWithChange((t) => { t._note = 'something else'; t.motion._note = 'edited'; });
  assert.deepEqual(changed, baseline);
});

test('a seed change touches only that preset and leaves the others identical', () => {
  const changed = renderWithChange((t) => { t.palettes.presets.wax = '#B4472F'; });
  const base = baseline['ts/croft-tokens.ts'] as string;
  const next = changed['ts/croft-tokens.ts'] as string;
  const block = (text: string, preset: string): string => {
    const start = text.indexOf(`  ${preset}: {\n    light:`);
    return text.slice(start, text.indexOf('\n  },\n', start));
  };
  for (const preset of ['moss', 'slate', 'heather', 'ochre', 'peat', 'tide', 'rowan']) {
    assert.equal(block(next, preset), block(base, preset), preset);
  }
  assert.notEqual(block(next, 'wax'), block(base, 'wax'));
});

test('a broken tokens.json is refused, not half-generated', () => {
  assert.throws(() => renderWithChange((t) => { t.palettes.presets.moss = 'green'; }), /not #RRGGBB/);
  assert.throws(() => renderWithChange((t) => { t.palettes.default = 'nope'; }), /palettes.default/);
  assert.throws(() => renderWithChange((t) => { t.palettes.styles = { moss: 'sparkly' }; }), /unknown style/);
});
