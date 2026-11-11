// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// The committed design/generated/ must match a fresh run, and a run must be deterministic.

import assert from 'node:assert/strict';
import { mkdtempSync, readFileSync, readdirSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { test } from 'node:test';
import { generate, render } from '../src/generate.ts';
import { committedDir, tokensPath } from './helpers.ts';

test('design/generated matches a fresh run (run `npm run generate` after editing tokens.json)', () => {
  const fresh = render(tokensPath);
  for (const [rel, text] of Object.entries(fresh)) {
    assert.equal(readFileSync(join(committedDir, rel), 'utf8'), text, `${rel} is stale`);
  }
});

test('design/generated holds exactly the four generated files', () => {
  const found: string[] = [];
  for (const dir of readdirSync(committedDir)) {
    for (const file of readdirSync(join(committedDir, dir))) found.push(`${dir}/${file}`);
  }
  assert.deepEqual(found.sort(), Object.keys(render(tokensPath)).sort());
});

test('generation is deterministic: two runs write byte-identical files', () => {
  const a = mkdtempSync(join(tmpdir(), 'croft-gen-a-'));
  const b = mkdtempSync(join(tmpdir(), 'croft-gen-b-'));
  try {
    generate(tokensPath, a);
    generate(tokensPath, b);
    for (const rel of Object.keys(render(tokensPath))) {
      assert.equal(readFileSync(join(a, rel), 'utf8'), readFileSync(join(b, rel), 'utf8'), rel);
    }
  } finally {
    rmSync(a, { recursive: true, force: true });
    rmSync(b, { recursive: true, force: true });
  }
});

test('generated files carry no timestamps or machine paths', () => {
  for (const text of Object.values(render(tokensPath))) {
    assert.doesNotMatch(text, /\/home\/|\b20\d\d-\d\d-\d\d\b/);
  }
});
