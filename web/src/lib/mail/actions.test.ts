// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

import { test } from 'node:test';
import assert from 'node:assert/strict';
import { applied, plan } from './actions.ts';
import type { EmailSummary, Mailbox } from '../jmap/client.ts';

const mb = (id: string, role: Mailbox['role'], name = id): Mailbox => ({ id, name, role, totalEmails: 0, unreadEmails: 0 });
const boxes = [mb('f1', 'inbox', 'INBOX'), mb('f2', 'archive', 'Archive'), mb('f3', 'trash', 'Trash'), mb('f4', 'junk', 'Spam'), mb('f5', null, 'Work')];
const email = (o: Partial<EmailSummary> = {}): EmailSummary => ({
  id: 'm1',
  threadId: 't1',
  mailboxIds: { f1: true },
  keywords: {},
  from: null,
  to: null,
  cc: null,
  subject: 's',
  receivedAt: '2026-01-01T00:00:00Z',
  preview: '',
  hasAttachment: false,
  ...o
});

test('archive, spam and move change the mailbox; Undo puts it back', () => {
  const a = plan('archive', email(), 'f1', boxes)!;
  assert.deepEqual(a.change, { kind: 'update', patch: { mailboxIds: { f2: true } } });
  assert.deepEqual(a.undo, { kind: 'update', patch: { mailboxIds: { f1: true } } });
  assert.equal(a.removesRow, true);
  assert.equal(a.message, 'Archived');
  assert.equal(plan('archive', email({ mailboxIds: { f2: true } }), 'f2', boxes), null, 'not in Archive');
  assert.equal(plan('archive', email(), 'f1', [boxes[0]!]), null, 'no Archive folder');
  assert.equal(plan('spam', email(), 'f1', boxes)!.message, 'Moved to Spam');
  assert.equal(plan('move', email(), 'f1', boxes, boxes[4])!.message, 'Moved to Work');
  assert.equal(plan('move', email(), 'f1', boxes, boxes[0]), null, 'already there');
});

test('delete moves to Trash with Undo; in Trash it is for good without Undo', () => {
  const d = plan('delete', email(), 'f1', boxes)!;
  assert.deepEqual(d.change, { kind: 'update', patch: { mailboxIds: { f3: true } } });
  assert.ok(d.undo);
  const forGood = plan('delete', email({ mailboxIds: { f3: true } }), 'f3', boxes)!;
  assert.deepEqual(forGood.change, { kind: 'destroy' });
  assert.equal(forGood.undo, null);
});

test('read and star toggle keywords, stay in the list and need no snackbar', () => {
  const r = plan('toggle-read', email(), 'f1', boxes)!;
  assert.deepEqual(r.change, { kind: 'update', patch: { 'keywords/$seen': true } });
  assert.deepEqual(r.undo, { kind: 'update', patch: { 'keywords/$seen': null } });
  assert.equal(r.removesRow, false);
  assert.equal(r.message, null);
  assert.equal(plan('mark-read', email({ keywords: { $seen: true } }), 'f1', boxes), null);
  assert.ok(plan('mark-unread', email({ keywords: { $seen: true } }), 'f1', boxes));
  const s = plan('toggle-star', email({ keywords: { $flagged: true } }), 'f1', boxes)!;
  assert.deepEqual(s.change, { kind: 'update', patch: { 'keywords/$flagged': null } });
});

test('a change applies to the shown email before the server confirms it', () => {
  const e = applied(email(), { kind: 'update', patch: { 'keywords/$seen': true, 'keywords/$flagged': true } });
  assert.deepEqual(e.keywords, { $seen: true, $flagged: true });
  const back = applied(e, { kind: 'update', patch: { 'keywords/$seen': null } });
  assert.deepEqual(back.keywords, { $flagged: true });
  assert.deepEqual(applied(email(), { kind: 'update', patch: { mailboxIds: { f2: true } } }).mailboxIds, { f2: true });
});

test('delete for good is decided by where the email is, not the folder shown', async () => {
  const { destroys } = await import('./actions.ts');
  // An Inbox email shown while Trash is the folder (a race): moved to Trash, not destroyed.
  const p = plan('delete', email(), 'f3', boxes)!;
  assert.equal(p.change.kind, 'update');
  assert.equal(destroys('delete', [email()], boxes), false);
  assert.equal(destroys('delete', [email({ mailboxIds: { f3: true } })], boxes), true);
  assert.equal(destroys('archive', [email({ mailboxIds: { f3: true } })], boxes), false);
});
