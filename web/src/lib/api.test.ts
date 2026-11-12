// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

import { test } from 'node:test';
import assert from 'node:assert/strict';
import { Api, ApiError, RENEW_BEFORE_MS, toSession, explain } from './api.ts';

interface Call {
  url: string;
  init: RequestInit;
}

/** A fake Companion: answers by path, records each request. */
function fake(answers: Record<string, () => Response>) {
  const calls: Call[] = [];
  const fn = async (input: RequestInfo | URL, init?: RequestInit): Promise<Response> => {
    const url = String(input);
    calls.push({ url, init: init ?? {} });
    const a = answers[url];
    if (!a) return new Response(JSON.stringify({ error: 'not_found' }), { status: 404 });
    return a();
  };
  return { fn: fn as typeof fetch, calls };
}

const json = (v: unknown, status = 200, headers: Record<string, string> = {}) =>
  new Response(JSON.stringify(v), { status, headers });

test('every request is marked as the web app, never follows redirects, and sends no token unless signed in', async () => {
  const f = fake({ '/api/oidc': () => json({ enabled: false }) });
  const api = new Api(f.fn, () => 0);
  await api.raw('GET', '/api/oidc');
  const h = f.calls[0]!.init.headers as Record<string, string>;
  assert.equal(h['x-croft-web'], '1');
  assert.equal(h['authorization'], undefined);
  assert.equal(f.calls[0]!.init.redirect, 'error');
  assert.equal(f.calls[0]!.init.credentials, 'same-origin');
});

test('a sign-in answer keeps the access token only, and a refresh token is never kept', () => {
  const s = toSession({ access_token: 'a', refresh_token: 'never-kept', access_expires_ms: 5, recovery: false });
  assert.deepEqual(s, { accessToken: 'a', expiresMs: 5, recovery: false });
  assert.throws(() => toSession({ access_token: 1 }), ApiError);
});

test('the token is renewed through the cookie when it is near its end, once for many callers', async () => {
  let now = 1_000_000;
  let n = 0;
  const f = fake({
    '/api/web/refresh': () => json({ access_token: `t${++n}`, access_expires_ms: now + 900_000, recovery: false }),
    '/api/me': () => json({ name: 'ann' })
  });
  const api = new Api(f.fn, () => now);
  // No session yet (a reload): renewed from the cookie.
  const [a, b] = await Promise.all([api.current(), api.current()]);
  assert.equal(a?.accessToken, 't1');
  assert.equal(b?.accessToken, 't1');
  assert.equal(n, 1, 'one renewal for both');
  await api.call('GET', '/api/me');
  const auth = (f.calls.at(-1)!.init.headers as Record<string, string>)['authorization'];
  assert.equal(auth, 'Bearer t1');
  // Still valid: no renewal. Near its end: renewed.
  now += 900_000 - RENEW_BEFORE_MS - 1;
  await api.call('GET', '/api/me');
  assert.equal(n, 1);
  now += 2;
  await api.call('GET', '/api/me');
  assert.equal(n, 2);
});

test('a refused renewal ends the session; a network failure keeps it', async () => {
  let mode: 'refuse' | 'offline' = 'offline';
  const now = 0;
  const f = fake({
    '/api/web/refresh': () => {
      if (mode === 'offline') throw new TypeError('network');
      return json({ error: 'invalid' }, 401);
    }
  });
  const api = new Api(f.fn, () => now);
  api.adopt({ access_token: 'a', access_expires_ms: 10, recovery: false });
  assert.equal((await api.renew())?.accessToken, 'a', 'kept while offline');
  mode = 'refuse';
  assert.equal(await api.renew(), null);
  assert.equal(api.session, null);
});

test('a recovery session is never renewed (it only adds a passkey)', async () => {
  let now = 0;
  const f = fake({ '/api/web/refresh': () => json({ error: 'invalid' }, 401) });
  const api = new Api(f.fn, () => now);
  api.adopt({ access_token: 'r', access_expires_ms: 100_000, recovery: true });
  assert.equal((await api.current())?.accessToken, 'r');
  now = 100_001;
  assert.equal(await api.current(), null);
  assert.equal(f.calls.length, 0);
});

test('errors carry the code and the wait, and read as plain words', async () => {
  const f = fake({ '/api/sign-in/recovery': () => json({ error: 'rate_limited' }, 429, { 'retry-after': '30' }) });
  const api = new Api(f.fn);
  await assert.rejects(api.raw('POST', '/api/sign-in/recovery', {}), (e: unknown) => {
    assert.ok(e instanceof ApiError);
    assert.equal(e.code, 'rate_limited');
    assert.equal(e.retryAfterS, 30);
    assert.match(explain(e), /Too many tries/);
    return true;
  });
  assert.match(explain(new ApiError('offline', 0)), /Can't reach/);
  assert.match(explain(new DOMException('x', 'NotAllowedError')), /cancelled/);
});

test('signing out forgets the token even when the request fails', async () => {
  const f = fake({});
  const api = new Api(async () => {
    throw new TypeError('down');
  });
  api.adopt({ access_token: 'a', access_expires_ms: 10, recovery: false });
  await assert.rejects(api.signOut());
  assert.equal(api.session, null);
  void f;
});
