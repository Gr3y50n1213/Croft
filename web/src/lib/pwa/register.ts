// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

import { dev } from '$app/env';

/** The service worker's one address (the Companion gives it its own CSP: companion/src/webapp.rs). */
export const WORKER = '/service-worker.js';

interface TrustedTypesLike {
  createPolicy(name: string, rules: { createScriptURL: (s: string) => string }): { createScriptURL(s: string): unknown };
}

// Under the page's Trusted Types a worker's address must come from a policy; this one (named in
// the CSP, `croft-sw`) passes the worker's address and nothing else.
const tt = (globalThis as { trustedTypes?: TrustedTypesLike }).trustedTypes;
const policy = tt?.createPolicy('croft-sw', {
  createScriptURL: (s: string) => {
    if (s !== WORKER) throw new TypeError('not the service worker');
    return s;
  }
});

let registering: Promise<ServiceWorkerRegistration | null> | null = null;

/** Registers the service worker (once per page); null where the browser has none or refuses. */
export function registerWorker(): Promise<ServiceWorkerRegistration | null> {
  if (typeof navigator === 'undefined' || !('serviceWorker' in navigator)) return Promise.resolve(null);
  registering ??= navigator.serviceWorker
    .register((policy ? policy.createScriptURL(WORKER) : WORKER) as string, {
      scope: '/',
      type: dev ? 'module' : 'classic'
    })
    .catch(() => null);
  return registering;
}
