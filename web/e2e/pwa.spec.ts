// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// The installable app (T32b) against a real Companion: the manifest, the service worker and its
// cache, opening offline, and push in the browser. Chromium cannot reach a real push service here,
// so the browser's subscription is a made-up one (a fresh P-256 key, an endpoint on example.org that
// nothing ever answers), and a ping is handed to the service worker through DevTools, as a push
// service would. Keys and endpoints are made up, for this test only.

import { test, expect, type Page } from '@playwright/test';
import AxeBuilder from '@axe-core/playwright';
import { Companion, virtualAuthenticator, watchForErrors } from './companion.ts';

let companion: Companion;

test.beforeEach(async () => {
  companion = await Companion.start();
});

test.afterEach(() => {
  companion.stop();
});

let users = 0;

async function signedIn(page: Page): Promise<void> {
  await virtualAuthenticator(page);
  await page.goto(companion.addUser(`user${++users}`));
  await page.getByRole('button', { name: /passkey/i }).click();
  await expect(page.locator('#step-backup')).toBeVisible();
  await page.goto(`${companion.url}/sign-in`);
  await page.getByRole('button', { name: 'Sign in with a passkey' }).click();
  await expect(page.locator('[data-screen="W01"]')).toBeVisible();
}

/** Waits until the service worker is active and controls the page. */
async function controlled(page: Page): Promise<void> {
  await page.evaluate(async () => {
    await navigator.serviceWorker.ready;
    if (!navigator.serviceWorker.controller) {
      await new Promise((r) => navigator.serviceWorker.addEventListener('controllerchange', r, { once: true }));
    }
  });
}

/** Every URL in every cache the page's origin holds. */
const cached = (page: Page) =>
  page.evaluate(async () => {
    const out: string[] = [];
    for (const name of await caches.keys()) {
      for (const req of await (await caches.open(name)).keys()) out.push(new URL(req.url).pathname);
    }
    return out;
  });

test('the manifest and service worker are served and install; the worker caches only the app', async ({ page, request }) => {
  const errors = watchForErrors(page);
  await signedIn(page);
  await controlled(page);

  const manifest = await request.get(`${companion.url}/manifest.webmanifest`);
  expect(manifest.headers()['content-type']).toBe('application/manifest+json');
  const m = await manifest.json();
  expect(m).toMatchObject({ name: 'Croft', display: 'standalone', start_url: '/', scope: '/' });
  for (const icon of m.icons) expect((await request.get(`${companion.url}${icon.src}`)).headers()['content-type']).toBe('image/png');
  expect(m.icons.some((i: { purpose: string }) => i.purpose === 'maskable')).toBe(true);

  const sw = await request.get(`${companion.url}/service-worker.js`);
  expect(sw.headers()['content-security-policy']).toContain("connect-src 'self'");
  expect(sw.headers()['cache-control']).toBe('no-store');

  // The page and its files, and nothing from the API, mail or sign-in: use the app a little first.
  await page.getByRole('link', { name: 'Settings' }).click();
  await expect(page.locator('[data-screen="W04"]')).toBeVisible();
  await page.getByRole('link', { name: 'Back to mail' }).click();
  await expect(page.locator('[data-screen="W01"]')).toBeVisible();
  const paths = await cached(page);
  expect(paths).toContain('/');
  expect(paths).toContain('/theme.js');
  expect(paths.some((p) => p.startsWith('/_app/immutable/'))).toBe(true);
  for (const p of paths) {
    expect(p, p).not.toMatch(/^\/(api|jmap|oidc|enrol|\.well-known|healthz|version)(\/|$)/);
    expect(p).not.toBe('/service-worker.js');
  }
  // API answers never come from the worker.
  const me = page.waitForResponse((r) => r.url().endsWith('/api/me'));
  await page.reload();
  expect((await me).fromServiceWorker()).toBe(false);
  expect(errors).toEqual([]);
});

test('the app opens offline with its CSP and says so; it carries on when the connection is back', async ({ page, context }) => {
  const errors = watchForErrors(page);
  await signedIn(page);
  await controlled(page);
  const csp = (await page.reload())!.headers()['content-security-policy'];
  await expect(page.locator('[data-screen="W01"]')).toBeVisible();

  await context.setOffline(true);
  const offline = await page.reload();
  // The cached page, with the Companion's CSP kept.
  expect(offline!.fromServiceWorker()).toBe(true);
  expect(offline!.headers()['content-security-policy']).toBe(csp);
  await expect(page.getByRole('heading', { name: "You're offline" })).toBeVisible();
  await expect(page.getByRole('status').filter({ hasText: 'Offline.' })).toBeVisible();
  expect((await new AxeBuilder({ page }).analyze()).violations.map((v) => v.id)).toEqual([]);
  // Another app page opens offline too.
  await page.goto(`${companion.url}/settings/about`);
  await expect(page.getByRole('status').filter({ hasText: 'Offline.' })).toBeVisible();
  await page.goto(companion.url);

  await context.setOffline(false);
  await expect(page.locator('[data-screen="W01"]')).toBeVisible();
  await expect(page.getByRole('status').filter({ hasText: 'Offline.' })).toHaveCount(0);
  // The only errors are the browser's own about requests that could not leave while offline.
  expect(errors.filter((e) => !/ERR_INTERNET_DISCONNECTED|Failed to fetch/.test(e))).toEqual([]);
});

/** A made-up push subscription in place of the browser's (no push service is reachable here). */
async function fakePush(page: Page): Promise<void> {
  await page.context().grantPermissions(['notifications'], { origin: companion.url });
  await page.addInitScript(() => {
    let current: PushSubscription | null = null;
    const b64 = (b: ArrayBuffer | Uint8Array) =>
      btoa(String.fromCharCode(...new Uint8Array(b instanceof Uint8Array ? b : new Uint8Array(b))))
        .replace(/\+/g, '-')
        .replace(/\//g, '_')
        .replace(/=+$/, '');
    PushManager.prototype.getSubscription = async () => current;
    PushManager.prototype.subscribe = async (options) => {
      if (!options?.applicationServerKey || !options.userVisibleOnly) throw new DOMException('bad options', 'NotAllowedError');
      const pair = await crypto.subtle.generateKey({ name: 'ECDH', namedCurve: 'P-256' }, true, ['deriveBits']);
      const p256dh = b64(await crypto.subtle.exportKey('raw', pair.publicKey));
      const auth = b64(crypto.getRandomValues(new Uint8Array(16)));
      const endpoint = `https://push.example.org/made-up/${crypto.randomUUID()}`;
      const sub = {
        endpoint,
        expirationTime: null,
        options,
        getKey: () => null,
        toJSON: () => ({ endpoint, expirationTime: null, keys: { p256dh, auth } }),
        unsubscribe: async () => {
          current = null;
          return true;
        }
      } as unknown as PushSubscription;
      current = sub;
      (window as unknown as { __subscribed: number }).__subscribed = ((window as unknown as { __subscribed?: number }).__subscribed ?? 0) + 1;
      return sub;
    };
  });
}

/** The Companion's list of this user's subscriptions, through the app's own session. */
const subscriptions = (page: Page) =>
  page.evaluate(async () => {
    const r = await fetch('/api/web/refresh', { method: 'POST', headers: { 'x-croft-web': '1' } });
    const { access_token } = await r.json();
    const list = await fetch('/api/push/subscriptions', { headers: { authorization: `Bearer ${access_token}` } });
    return (await list.json()).subscriptions as { id: number; current_device: boolean }[];
  });

test('push is turned on and off in W04, and given again after signing in again', async ({ page }) => {
  const errors = watchForErrors(page);
  await fakePush(page);
  await signedIn(page);
  await page.goto(`${companion.url}/settings/notifications`);
  const toggle = page.getByRole('checkbox', { name: /New mail in this browser/ });
  await expect(toggle).not.toBeChecked();
  expect((await new AxeBuilder({ page }).analyze()).violations.map((v) => v.id)).toEqual([]);
  await toggle.check();
  await expect(toggle).toBeChecked();
  await expect(toggle).toBeEnabled();
  let subs = await subscriptions(page);
  expect(subs).toHaveLength(1);
  expect(subs[0]!.current_device).toBe(true);
  // Only the id and a hash are kept in the browser, never the endpoint.
  const stored = await page.evaluate(() => localStorage.getItem('croft.push'));
  expect(stored).not.toContain('example.org');
  expect(JSON.parse(stored!)).toEqual({ id: subs[0]!.id, endpointHash: expect.stringMatching(/^[0-9a-f]{64}$/) });

  await toggle.uncheck();
  await expect(toggle).not.toBeChecked();
  expect(await subscriptions(page)).toHaveLength(0);
  expect(await page.evaluate(() => localStorage.getItem('croft.push'))).toBeNull();

  // On again, then sign out: the Companion drops it with the session; the next sign-in gives it again.
  await toggle.check();
  await expect(toggle).toBeEnabled();
  expect(await subscriptions(page)).toHaveLength(1);
  await page.goto(companion.url);
  await page.getByRole('button', { name: 'Sign out' }).click();
  await expect(page).toHaveURL(/\/sign-in$/);
  await page.getByRole('button', { name: 'Sign in with a passkey' }).click();
  await expect(page.locator('[data-screen="W01"]')).toBeVisible();
  await expect(async () => {
    subs = await subscriptions(page);
    expect(subs.filter((s) => s.current_device)).toHaveLength(1);
  }).toPass({ timeout: 10_000 });
  expect(errors).toEqual([]);
});

test('a ping refreshes an open tab; with no tab in view it shows "New mail"', async ({ page }) => {
  const errors = watchForErrors(page);
  await signedIn(page);
  await controlled(page);
  const cdp = await page.context().newCDPSession(page);
  const registrations = new Map<string, string>();
  cdp.on('ServiceWorker.workerRegistrationUpdated', (e) => {
    for (const r of e.registrations) registrations.set(r.scopeURL, r.registrationId);
  });
  await cdp.send('ServiceWorker.enable');
  await expect.poll(() => registrations.get(`${companion.url}/`)).toBeTruthy();
  const registrationId = registrations.get(`${companion.url}/`)!;

  // A tab in view: the ping makes it ask the Companion for its mail again, at once.
  const refreshed = page.waitForRequest((r) => r.url().endsWith('/jmap/api') && r.method() === 'POST', { timeout: 5_000 });
  await cdp.send('ServiceWorker.deliverPushMessage', { origin: companion.url, registrationId, data: '' });
  await refreshed;
  // Whatever a message holds is ignored (pings have no content).
  const again = page.waitForRequest((r) => r.url().endsWith('/jmap/api') && r.method() === 'POST', { timeout: 5_000 });
  await cdp.send('ServiceWorker.deliverPushMessage', { origin: companion.url, registrationId, data: '{"from":"x"}' });
  await again;

  expect(errors).toEqual([]);

  // No app tab open: the worker shows the content-free notice (the browser requires one for a push).
  const context = page.context();
  await context.grantPermissions(['notifications'], { origin: companion.url });
  const other = await context.newPage();
  await other.goto('about:blank');
  const cdp2 = await context.newCDPSession(other);
  await cdp2.send('ServiceWorker.enable');
  await page.close();
  await cdp2.send('ServiceWorker.deliverPushMessage', { origin: companion.url, registrationId, data: '' });
  const back = await context.newPage();
  await back.goto(companion.url);
  await expect
    .poll(
      () =>
        back.evaluate(async () => {
          const reg = await navigator.serviceWorker.ready;
          return (await reg.getNotifications()).map((n) => `${n.title}|${n.body}`);
        }),
      { timeout: 5_000 }
    )
    .toEqual(['New mail|Open Croft to read it.']);
});
