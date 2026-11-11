// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// Every non-colour token in tokens.json reaches every output with the same value.

import assert from 'node:assert/strict';
import { test } from 'node:test';
import { cssName, cssUnit } from '../src/emit-css.ts';
import { flatten, genericTree, loadTokens } from '../src/tokens.ts';
import type { LeafValue } from '../src/tokens.ts';
import {
  committedDir, importGeneratedTs, kebabForTest, parseCss, parseKotlin, parseSwift, readOutput, tokensPath,
} from './helpers.ts';

const tokens = loadTokens(tokensPath);
const expected = flatten(genericTree(tokens));
const key = (path: string[]): string => path.join('.');

test('tokens.json has leaves to check, and notes are not among them', () => {
  assert.ok(expected.length >= 70);
  assert.ok(expected.every((l) => !l.path.some((p) => p.startsWith('_'))));
});

for (const [name, parse] of [['Kotlin', parseKotlin], ['Swift', parseSwift]] as const) {
  test(`${name}: every token value matches tokens.json`, () => {
    const parsed = parse(readOutput(committedDir, name === 'Kotlin' ? 'kotlin' : 'swift'));
    const got = new Map(parsed.leaves.map((l) => [key(l.path), l.value]));
    assert.equal(got.size, expected.length, 'same number of tokens');
    for (const leaf of expected) assert.deepEqual(got.get(key(leaf.path)), leaf.value, key(leaf.path));
  });
}

test('CSS: every token value matches tokens.json (dp as px, seconds as s)', () => {
  const css = parseCss(readOutput(committedDir, 'css'));
  for (const leaf of expected) {
    const name = `--croft-${leaf.path.map(kebabForTest).join('-')}`;
    assert.equal(name, cssName(leaf.path));
    const raw = css.vars.get(name);
    assert.ok(raw !== undefined, `${name} missing`);
    const unit = cssUnit(leaf.path);
    const want = (v: number | string | boolean) => (typeof v === 'number' ? `${v}${unit}` : String(v));
    const wantAll = Array.isArray(leaf.value) ? leaf.value.map(want).join(', ') : want(leaf.value);
    assert.equal(raw, wantAll, name);
  }
});

test('TypeScript: croftTokens deep-equals tokens.json without its notes', async () => {
  const ts = await importGeneratedTs(committedDir);
  const got = new Map<string, LeafValue>(flatten(ts.croftTokens).map((l) => [key(l.path), l.value]));
  assert.equal(got.size, expected.length);
  for (const leaf of expected) assert.deepEqual(got.get(key(leaf.path)), leaf.value, key(leaf.path));
});
