// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// Spring values in every output equal the motion table in docs/ux/motion.md.

import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { test } from 'node:test';
import { springLinearEasing, springPosition } from '../src/spring.ts';
import {
  committedDir, importGeneratedTs, parseCss, parseKotlin, parseSwift, readOutput, repoRoot,
} from './helpers.ts';

interface Row {
  group: 'spatial' | 'effects' | 'standardSpatialForReducedMotion';
  size: 'fast' | 'default' | 'slow';
  stiffness: number;
  damping: number;
  settle: number;
  response: number;
}

const num = (s: string): number => Number(s.replace(/,/g, '').replace(/s$/, '').trim());
const list = (s: string): number[] => s.replace(/\s*s\s*$/, '').split(/,\s+/).map(num);

/** Reads the "Spring tokens" table of docs/ux/motion.md. */
function motionTable(): Row[] {
  const text = readFileSync(join(repoRoot, 'docs/ux/motion.md'), 'utf8');
  const section = text.split('### Spring tokens')[1]?.split('###')[0] ?? '';
  const rows: Row[] = [];
  for (const line of section.split('\n')) {
    if (!line.startsWith('|') || /^\|\s*(Token|---)/.test(line)) continue;
    const cells = line.split('|').slice(1, -1).map((c) => c.trim());
    const [token, stiff, damp, settle, response] = cells as [string, string, string, string, string];
    const m = /^(Spatial|Effects) (fast|default|slow)$/.exec(token);
    if (m) {
      rows.push({
        group: m[1] === 'Spatial' ? 'spatial' : 'effects',
        size: m[2] as Row['size'],
        stiffness: num(stiff),
        damping: num(damp),
        settle: num(settle.replace('s', '').replace('about', '')),
        response: num(response),
      });
    } else if (token.startsWith('Standard spatial')) {
      const stiffs = list(stiff);
      const settles = list(settle);
      const responses = list(response);
      (['fast', 'default', 'slow'] as const).forEach((size, i) => {
        rows.push({
          group: 'standardSpatialForReducedMotion',
          size,
          stiffness: stiffs[i] as number,
          damping: num(damp),
          settle: settles[i] as number,
          response: responses[i] as number,
        });
      });
    }
  }
  return rows;
}

const table = motionTable();

test('the motion table was read: nine springs', () => {
  assert.equal(table.length, 9);
  assert.deepEqual(
    table.find((r) => r.group === 'spatial' && r.size === 'fast'),
    { group: 'spatial', size: 'fast', stiffness: 800, damping: 0.6, settle: 0.24, response: 0.22 },
  );
  assert.equal(table.find((r) => r.group === 'effects' && r.size === 'fast')?.stiffness, 3800);
  assert.equal(table.find((r) => r.group === 'standardSpatialForReducedMotion' && r.size === 'slow')?.stiffness, 300);
});

test('SwiftUI response in the table is 2 pi / sqrt(stiffness) to two decimals', () => {
  for (const r of table) {
    assert.equal(Number((2 * Math.PI / Math.sqrt(r.stiffness)).toFixed(2)), r.response, `${r.group} ${r.size}`);
  }
});

test('Kotlin, Swift, CSS and TypeScript springs equal docs/ux/motion.md', async () => {
  const kotlin = parseKotlin(readOutput(committedDir, 'kotlin'));
  const swift = parseSwift(readOutput(committedDir, 'swift'));
  const css = parseCss(readOutput(committedDir, 'css'));
  const ts = await importGeneratedTs(committedDir);
  const cssGroup = (g: string) => (g === 'standardSpatialForReducedMotion' ? 'standard-spatial-for-reduced-motion' : g);
  for (const r of table) {
    const where = `${r.group} ${r.size}`;
    for (const [name, leaves] of [['Kotlin', kotlin.leaves], ['Swift', swift.leaves]] as const) {
      const get = (field: string) => leaves.find((l) => l.path.join('.') === `motion.${r.group}.${r.size}.${field}`)?.value;
      assert.equal(get('stiffness'), r.stiffness, `${name} ${where} stiffness`);
      assert.equal(get('damping'), r.damping, `${name} ${where} damping`);
      assert.equal(get('swiftUIResponse'), r.response, `${name} ${where} response`);
      assert.equal(get('settleSeconds'), r.settle, `${name} ${where} settle`);
    }
    const base = `--croft-motion-${cssGroup(r.group)}-${r.size}`;
    assert.equal(css.vars.get(`${base}-stiffness`), String(r.stiffness), `CSS ${where} stiffness`);
    assert.equal(css.vars.get(`${base}-damping`), String(r.damping), `CSS ${where} damping`);
    assert.equal(css.vars.get(`${base}-swift-ui-response`), String(r.response), `CSS ${where} response`);
    assert.equal(css.vars.get(`${base}-settle-seconds`), `${r.settle}s`, `CSS ${where} settle`);
    const spring = ts.croftTokens.motion[r.group][r.size];
    assert.deepEqual(
      [spring.stiffness, spring.damping, spring.swiftUIResponse, spring.settleSeconds],
      [r.stiffness, r.damping, r.response, r.settle],
      `TS ${where}`,
    );
  }
});

test('the Kotlin and Swift motion wrappers read the same token paths the table maps to', () => {
  const kt = readOutput(committedDir, 'kotlin');
  const sw = readOutput(committedDir, 'swift');
  for (const [wrapper, group] of [['spatial', 'Spatial'], ['effects', 'Effects'], ['reducedSpatial', 'StandardSpatialForReducedMotion']]) {
    for (const size of ['Fast', 'Default', 'Slow']) {
      const path = `CroftTokens.Motion.${group}.${size}`;
      assert.ok(kt.includes(`val ${wrapper}${size} = CroftSpring(${path}.stiffness.toFloat(), ${path}.damping.toFloat())`), `Kotlin ${wrapper}${size}`);
      assert.ok(sw.includes(`static let ${wrapper}${size} = CroftSpring(stiffness: Double(${path}.stiffness), damping: Double(${path}.damping), response: Double(${path}.swiftUIResponse))`), `Swift ${wrapper}${size}`);
    }
  }
});

test('web easings are springs: start at 0, end at 1, overshoot only when under-damped', () => {
  for (const r of table) {
    const { easing, duration } = springLinearEasing(r.stiffness, r.damping);
    const points = /^linear\((.*)\)$/.exec(easing)?.[1]?.split(', ').map(Number) ?? [];
    assert.equal(points[0], 0);
    assert.equal(points[points.length - 1], 1);
    const peak = Math.max(...points);
    if (r.damping >= 1) assert.ok(peak <= 1, `${r.group} ${r.size} must not overshoot`);
    else if (r.damping <= 0.8) assert.ok(peak > 1, `${r.group} ${r.size} should overshoot`);
    assert.ok(duration > 0.05 && duration < 1, `${r.group} ${r.size} duration ${duration}`);
    assert.ok(Math.abs(springPosition(r.stiffness, r.damping, duration) - 1) <= 0.005);
  }
});
