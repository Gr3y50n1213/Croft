// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

import { test } from 'node:test';
import assert from 'node:assert/strict';
import { formatAddresses, parseAddresses } from './addresses.ts';

test('recipient fields split into addresses and back', () => {
  assert.deepEqual(parseAddresses('Ann Example <ann@example.org>, bob@example.org; '), [
    { name: 'Ann Example', email: 'ann@example.org' },
    { name: null, email: 'bob@example.org' }
  ]);
  assert.deepEqual(parseAddresses('"Example, Ann" <ann@example.org>'), [{ name: 'Example, Ann', email: 'ann@example.org' }]);
  assert.deepEqual(parseAddresses(''), []);
  assert.deepEqual(parseAddresses('not an address'), [{ name: null, email: 'not an address' }]);
  const list = [
    { name: 'Example, Ann', email: 'ann@example.org' },
    { name: null, email: 'bob@example.org' }
  ];
  assert.equal(formatAddresses(list), '"Example, Ann" <ann@example.org>, bob@example.org');
  assert.deepEqual(parseAddresses(formatAddresses(list)), list);
});
