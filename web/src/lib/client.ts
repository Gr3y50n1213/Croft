// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

import { Api } from './api.ts';

/** The page's one client (the access token lives in it, in memory only). */
export const api = new Api((input, init) => fetch(input, init));

/** Who is signed in (`GET /api/me`). */
export interface Me {
  id: number;
  name: string;
  role: 'user' | 'admin';
  recovery: boolean;
  oidc_linked: boolean;
}
