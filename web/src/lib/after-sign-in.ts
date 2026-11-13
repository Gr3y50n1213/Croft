// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

/**
 * Where to go after signing in (T34e): "Delete my Companion data" needs a fresh sign-in, after which the person comes back to it.
 * Kept for this tab only (sessionStorage), used once, and only a page on this list: never an address from a link.
 */
export const RETURNS = ["/settings/security"] as const;
export type Return = (typeof RETURNS)[number];

const KEY = "croft-after-sign-in";

export function rememberReturn(storage: Storage, to: Return): void {
  try {
    storage.setItem(KEY, to);
  } catch {
    // Storage refused (a private window): the person lands on the inbox instead.
  }
}

/** The page to go to after signing in, once; `/` unless one on [RETURNS] was remembered. */
export function takeReturn(storage: Storage): string {
  let to: string | null = null;
  try {
    to = storage.getItem(KEY);
    storage.removeItem(KEY);
  } catch {
    return "/";
  }
  return (RETURNS as readonly string[]).includes(to ?? "")
    ? (to as string)
    : "/";
}
