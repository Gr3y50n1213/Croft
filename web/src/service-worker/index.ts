// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

/**
 * The service worker (T32b): the app opens offline from its cached files, and push pings wake it.
 * It keeps the app's own files only (the page, scripts, styles, icons: nothing personal); requests
 * to the Companion's API, mail and sign-in paths are never handled (#lib/pwa/route.ts). A ping has
 * no content (T28): it tells open tabs to refresh, or shows "New mail".
 */

import { self as sw } from '$app/service-worker';
import { immutable, assets } from '$app/manifest';
import { version } from '$app/env';
import { NEW_MAIL, PUSH_CHANGED, notifyFor, route } from '../lib/pwa/route.ts';

const CACHE = `croft-app-${version}`;
const PAGE = '/';
/** The manifest's paths are relative to the app's base, which is the origin's root (the Companion's rule). */
const abs = (p: string) => `/${p.replace(/^\/+/, '')}`;
const ASSETS = new Set<string>([PAGE, ...immutable.map((f) => abs(f.path)), ...assets.map((f) => abs(f.path))]);
ASSETS.delete('/service-worker.js');

sw.addEventListener('install', (event) => {
  event.waitUntil(
    (async () => {
      const cache = await caches.open(CACHE);
      await cache.addAll([...ASSETS]);
      await sw.skipWaiting();
    })()
  );
});

sw.addEventListener('activate', (event) => {
  event.waitUntil(
    (async () => {
      for (const key of await caches.keys()) if (key !== CACHE) await caches.delete(key);
      await sw.clients.claim();
    })()
  );
});

sw.addEventListener('fetch', (event) => {
  const req = event.request;
  switch (route(req, sw.location.origin, ASSETS)) {
    case 'asset':
      event.respondWith(fromCache(new URL(req.url).pathname, req));
      return;
    case 'page':
      event.respondWith(page(req));
      return;
    default:
      // The browser does what it would without a service worker.
      return;
  }
});

async function fromCache(path: string, req: Request): Promise<Response> {
  const cache = await caches.open(CACHE);
  return (await cache.match(path)) ?? fetch(req);
}

/** The network first (a new version of the page is seen at once); offline, the cached page. */
async function page(req: Request): Promise<Response> {
  try {
    return await fetch(req);
  } catch (e) {
    const cached = await (await caches.open(CACHE)).match(PAGE);
    if (cached) return cached;
    throw e;
  }
}

// ---- Push: a content-free ping (whatever the message holds is ignored). ----

sw.addEventListener('push', (event) => {
  event.waitUntil(ping());
});

async function ping(): Promise<void> {
  const tabs = await sw.clients.matchAll({ type: 'window', includeUncontrolled: true });
  for (const tab of tabs) tab.postMessage({ type: NEW_MAIL });
  if (!notifyFor(tabs)) return;
  await sw.registration
    .showNotification('New mail', { body: 'Open Croft to read it.', tag: NEW_MAIL, icon: '/icon-192.png' })
    .catch(() => {
      // Notifications were turned off in the browser meanwhile: nothing to show.
    });
}

sw.addEventListener('notificationclick', (event) => {
  event.notification.close();
  event.waitUntil(
    (async () => {
      const tabs = await sw.clients.matchAll({ type: 'window', includeUncontrolled: true });
      const tab = tabs[0];
      if (tab) {
        await tab.focus();
        return;
      }
      await sw.clients.openWindow('/');
    })()
  );
});

// The browser replaced the subscription: the worker holds no token to tell the Companion, so an
// open tab does it (#lib/pwa/push.ts), and every tab does it when it next starts.
sw.addEventListener('pushsubscriptionchange', (event) => {
  (event as ExtendableEvent).waitUntil(
    (async () => {
      const tabs = await sw.clients.matchAll({ type: 'window', includeUncontrolled: true });
      for (const tab of tabs) tab.postMessage({ type: PUSH_CHANGED });
    })()
  );
});
