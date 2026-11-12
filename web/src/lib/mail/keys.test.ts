// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

import { test } from 'node:test';
import assert from 'node:assert/strict';
import { SEQUENCE_MS, Shortcuts, type Key } from './keys.ts';

const k = (key: string, o: Partial<Key> = {}): Key => ({ key, shift: false, modified: false, typing: false, timeMs: 0, ...o });

test('the table of docs/UX.md', () => {
  const s = new Shortcuts();
  const expect: [Key, string | null][] = [
    [k('j'), 'next'],
    [k('k'), 'previous'],
    [k('Enter'), 'open'],
    [k('o'), 'open'],
    [k('u'), 'back'],
    [k('x'), 'select'],
    [k('e'), 'archive'],
    [k('#', { shift: true }), 'delete'],
    [k('I', { shift: true }), 'mark-read'],
    [k('U', { shift: true }), 'mark-unread'],
    [k('s'), 'star'],
    [k('r'), 'reply'],
    [k('a'), 'reply-all'],
    [k('f'), 'forward'],
    [k('c'), 'compose'],
    [k('/'), 'search'],
    [k('z'), 'undo'],
    [k('?', { shift: true }), 'help'],
    [k('q'), null],
    [k('E', { shift: true }), null]
  ];
  for (const [key, want] of expect) assert.equal(s.press(key), want, JSON.stringify(key));
});

test('g then i, d or s goes to a folder, within the time', () => {
  const s = new Shortcuts();
  assert.equal(s.press(k('g', { timeMs: 0 })), null);
  assert.equal(s.press(k('i', { timeMs: 500 })), 'go-inbox');
  assert.equal(s.press(k('g', { timeMs: 1000 })), null);
  assert.equal(s.press(k('d', { timeMs: 1100 })), 'go-drafts');
  assert.equal(s.press(k('g', { timeMs: 2000 })), null);
  assert.equal(s.press(k('s', { timeMs: 2100 })), 'go-sent');
  // Too late: `s` is star again.
  s.press(k('g', { timeMs: 3000 }));
  assert.equal(s.press(k('s', { timeMs: 3000 + SEQUENCE_MS + 1 })), 'star');
  // g then an unknown key does nothing, and the next key is itself.
  s.press(k('g', { timeMs: 9000 }));
  assert.equal(s.press(k('q', { timeMs: 9100 })), null);
  assert.equal(s.press(k('e', { timeMs: 9200 })), 'archive');
});

test('never while typing or with Ctrl, Alt or Meta', () => {
  const s = new Shortcuts();
  assert.equal(s.press(k('e', { typing: true })), null);
  assert.equal(s.press(k('c', { modified: true })), null);
  s.press(k('g'));
  assert.equal(s.press(k('i', { typing: true })), null);
  assert.equal(s.press(k('i')), null, 'the sequence was dropped');
});
