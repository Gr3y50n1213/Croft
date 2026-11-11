// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// Every source file, test and generated file starts with the MPL-2.0 Exhibit A notice.

import assert from 'node:assert/strict';
import { readFileSync, readdirSync } from 'node:fs';
import { join } from 'node:path';
import { test } from 'node:test';
import { committedDir, here, designDir } from './helpers.ts';

const NOTICE = [
  'This Source Code Form is subject to the terms of the Mozilla Public',
  'License, v. 2.0. If a copy of the MPL was not distributed with this',
  'file, You can obtain one at https://mozilla.org/MPL/2.0/.',
];

function files(dir: string): string[] {
  return readdirSync(dir, { withFileTypes: true }).flatMap((e) =>
    e.isDirectory() ? files(join(dir, e.name)) : [join(dir, e.name)],
  );
}

test('every .ts, .kt, .swift and .css file carries the MPL-2.0 notice', () => {
  const targets = [
    ...files(join(designDir, 'generator/src')),
    ...files(here),
    ...files(committedDir),
  ];
  assert.ok(targets.length >= 15);
  for (const file of targets) {
    const head = readFileSync(file, 'utf8').slice(0, 600);
    for (const line of NOTICE) assert.ok(head.includes(line), `${file} lacks: ${line}`);
  }
});
