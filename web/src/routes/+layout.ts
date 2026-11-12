// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// A single-page app: nothing is rendered on a server, and the Companion gives every app path the
// same page.
export const ssr = false;
export const prerender = false;
export const trailingSlash = 'never';
