// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { SERVER_PATHS, isServerPath, notifyFor, route } from './route.ts';
import { keyBytes, parseStored, sha256Hex } from './push-data.ts';

const ORIGIN = 'https://companion.test';
const ASSETS = new Set(['/', '/theme.js', '/_app/immutable/entry/app.abc.js', '/icon-192.png']);
const get = (path: string, mode = 'no-cors', method = 'GET') => route({ method, url: `${ORIGIN}${path}`, mode }, ORIGIN, ASSETS);

test("the worker's list of the Companion's paths is the Companion's", () => {
  // companion/src/webapp.rs, RESERVED: the paths that never get the app's page.
  const rs = readFileSync(new URL('../../../../companion/src/webapp.rs', import.meta.url), 'utf8');
  const block = rs.slice(rs.indexOf('const RESERVED'), rs.indexOf('];', rs.indexOf('const RESERVED')));
  const listed = [...block.matchAll(/"(\/[^"]*)"/g)].map((m) => m[1]);
  assert.deepEqual([...listed].sort(), [...SERVER_PATHS].sort());
});

test('the API, mail and sign-in paths always go to the network, whatever the request', () => {
  for (const p of [
    '/api/me',
    '/api/web/refresh',
    '/API/web/refresh',
    '/jmap/api',
    '/jmap/session',
    '/oidc/callback?code=x',
    '/enrol/abc',
    '/.well-known/jmap',
    '/healthz',
    '/version'
  ]) {
    for (const mode of ['navigate', 'cors', 'same-origin', 'no-cors']) {
      for (const method of ['GET', 'POST', 'DELETE']) assert.equal(get(p, mode, method), 'network', `${method} ${p} ${mode}`);
    }
    assert.ok(isServerPath(p.split('?')[0]!), p);
  }
});

test("the app's files come from the cache; anything else is the network", () => {
  assert.equal(get('/theme.js'), 'asset');
  assert.equal(get('/_app/immutable/entry/app.abc.js'), 'asset');
  assert.equal(get('/icon-192.png'), 'asset');
  // Not a cached file, a query, another origin, or not a GET: the network.
  assert.equal(get('/_app/immutable/entry/other.js'), 'network');
  assert.equal(get('/theme.js?x=1'), 'network');
  assert.equal(get('/theme.js', 'no-cors', 'POST'), 'network');
  assert.equal(route({ method: 'GET', url: 'https://elsewhere.test/theme.js', mode: 'no-cors' }, ORIGIN, ASSETS), 'network');
  assert.equal(route({ method: 'GET', url: 'not a url', mode: 'navigate' }, ORIGIN, ASSETS), 'network');
});

test('opening an app page is a page (network first, offline the cached page); files are not', () => {
  for (const p of ['/', '/settings', '/settings/appearance', '/sign-in', '/sso/back?code=1']) {
    assert.equal(get(p, 'navigate'), 'page', p);
  }
  // A page fetched by script is not a navigation.
  assert.equal(get('/settings', 'cors'), 'network');
  // A file opened in a tab (cached or not) is the network's answer, not the app's page.
  assert.equal(get('/missing.txt', 'navigate'), 'network');
  assert.equal(get('/theme.js', 'navigate'), 'network');
});

test("the stored push record is the Companion's id and an endpoint hash, read back safely", () => {
  const ok = { id: 7, endpointHash: 'a'.repeat(64) };
  assert.deepEqual(parseStored(JSON.stringify(ok)), ok);
  for (const bad of [null, '', 'x', '[]', '{}', '{"id":"7","endpointHash":"' + 'a'.repeat(64) + '"}', '{"id":0,"endpointHash":"' + 'a'.repeat(64) + '"}', '{"id":7,"endpointHash":"https://push.example/x"}']) {
    assert.equal(parseStored(bad), null, String(bad));
  }
});

test('the VAPID key becomes its bytes; endpoints are hashed, never kept', async () => {
  // A P-256 public key is 65 bytes starting with 4 (base64url, no padding).
  const raw = new Uint8Array(65).map((_, i) => (i === 0 ? 4 : i));
  const b64 = Buffer.from(raw).toString('base64url');
  assert.deepEqual([...keyBytes(b64)], [...raw]);
  const h = await sha256Hex('https://push.example.org/made-up-endpoint');
  assert.match(h, /^[0-9a-f]{64}$/);
  assert.equal(h, await sha256Hex('https://push.example.org/made-up-endpoint'));
});

test('a ping shows "New mail" only when no app tab is in view', () => {
  assert.equal(notifyFor([]), true);
  assert.equal(notifyFor([{ visibilityState: 'hidden' }]), true);
  assert.equal(notifyFor([{ visibilityState: 'hidden' }, { visibilityState: 'visible' }]), false);
});
