// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

import { Api, type Exclusive } from './api.ts';
import { connection } from './pwa/connection.svelte.ts';

/** Refreshes never overlap across this origin's tabs (see `Exclusive`). */
const exclusive: Exclusive = (fn) =>
  typeof navigator !== 'undefined' && navigator.locks
    ? navigator.locks.request('croft-refresh', { mode: 'exclusive' }, fn)
    : fn();

/** The page's one client (the access token lives in it, in memory only). */
export const api = new Api((input, init) => fetch(input, init), () => Date.now(), exclusive);
api.onReach = (reached) => (connection.companionReached = reached);

/** Who is signed in (`GET /api/me`). */
export interface Me {
  id: number;
  name: string;
  role: 'user' | 'admin';
  recovery: boolean;
  oidc_linked: boolean;
}
