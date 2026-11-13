// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

/**
 * What the service worker does with a request (T32b), kept pure so it is tested here. It answers
 * only for the app's own files (from its cache) and for opening an app page (the network first,
 * the cached page when offline). Everything else, and above all the Companion's API, mail and
 * sign-in paths, goes to the network untouched: mail and tokens are never cached or seen by it.
 */

/** The Companion's own paths (companion/src/webapp.rs, `RESERVED`): never handled by the worker. */
export const SERVER_PATHS = ['/api', '/oidc', '/jmap', '/.well-known', '/enrol', '/healthz', '/version'];

export type Route = 'asset' | 'page' | 'network';

export interface RequestLike {
  readonly method: string;
  readonly url: string;
  readonly mode: string;
}

export function isServerPath(path: string): boolean {
  const lower = path.toLowerCase();
  return SERVER_PATHS.some((p) => lower === p || lower.startsWith(`${p}/`));
}

/**
 * `origin`: the app's origin; `assets`: the paths of the app's files in the cache.
 */
export function route(req: RequestLike, origin: string, assets: ReadonlySet<string>): Route {
  if (req.method !== 'GET') return 'network';
  let url: URL;
  try {
    url = new URL(req.url);
  } catch {
    return 'network';
  }
  if (url.origin !== origin || isServerPath(url.pathname)) return 'network';
  // Opening a page of the app (a path without a file name, the Companion gives every app path the
  // same page): the network first, so an updated app is seen at once (its old files are gone).
  const last = url.pathname.split('/').pop() ?? '';
  if (req.mode === 'navigate') return last.includes('.') ? 'network' : 'page';
  // The app's files, exactly (no query: a query means something else is asked for).
  if (!url.search && assets.has(url.pathname)) return 'asset';
  return 'network';
}

/** Messages from the service worker to the app's tabs. */
export const NEW_MAIL = 'croft-new-mail';
export const PUSH_CHANGED = 'croft-push-changed';
