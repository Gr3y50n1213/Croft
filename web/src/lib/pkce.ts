// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

/**
 * Single sign-on (T25's flow for apps): the app makes a PKCE verifier, sends the browser to the
 * Companion with its S256 challenge, and redeems the one-time code that comes back with the
 * verifier. The verifier is kept in `sessionStorage` for the round trip only (this tab, gone when it
 * closes) and removed as soon as it is read.
 */

import { toB64url } from "./passkey.ts";

const KEY = "croft.sso.verifier";

/** A new verifier: 32 random bytes, base64url (43 characters). */
export function newVerifier(
  random: (b: Uint8Array<ArrayBuffer>) => Uint8Array<ArrayBuffer> = (b) =>
    crypto.getRandomValues(b),
): string {
  return toB64url(random(new Uint8Array(32)));
}

/** The S256 challenge of a verifier. */
export async function challengeOf(verifier: string): Promise<string> {
  const digest = await crypto.subtle.digest(
    "SHA-256",
    new TextEncoder().encode(verifier),
  );
  return toB64url(digest);
}

/**
 * The address that starts single sign-on, coming back to `redirect` (this app's own page). `fresh`: the provider must ask the person
 * to sign in (T34e: "Sign in again" before deleting the account), not reuse its own session.
 */
export function startUrl(
  challenge: string,
  redirect: string,
  fresh = false,
): string {
  const q = new URLSearchParams({
    app_challenge: challenge,
    app_redirect: redirect,
  });
  if (fresh) q.set("fresh", "1");
  return `/api/sign-in/oidc/start?${q.toString()}`;
}

export function keepVerifier(
  store: Pick<Storage, "setItem">,
  verifier: string,
): void {
  store.setItem(KEY, verifier);
}

/** The kept verifier, removed as it is read (a code is redeemed once). */
export function takeVerifier(
  store: Pick<Storage, "getItem" | "removeItem">,
): string | null {
  const v = store.getItem(KEY);
  store.removeItem(KEY);
  return v;
}
