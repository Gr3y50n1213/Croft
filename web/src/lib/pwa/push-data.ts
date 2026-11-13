// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

/** The pure parts of push in this browser (push.ts), tested on their own. */

/**
 * What this browser keeps: whose push it is (the user who turned it on), and while it is given to
 * the Companion, the Companion's id for it and a hash of its endpoint. After that user signs out
 * only `user` is left (push is wanted, the subscription is dropped): it is made again when the same
 * user signs in, and never for anyone else (T32b review).
 */
export interface Stored {
  user: number;
  id?: number;
  endpointHash?: string;
}

const positive = (v: unknown): v is number => typeof v === 'number' && Number.isSafeInteger(v) && v > 0;

/** What was stored, or null when nothing (or something malformed) is. */
export function parseStored(s: string | null): Stored | null {
  if (!s) return null;
  try {
    const v: unknown = JSON.parse(s);
    if (typeof v !== 'object' || v === null || Array.isArray(v)) return null;
    const { user, id, endpointHash } = v as Record<string, unknown>;
    if (!positive(user)) return null;
    if (id === undefined && endpointHash === undefined) return { user };
    if (positive(id) && typeof endpointHash === 'string' && /^[0-9a-f]{64}$/.test(endpointHash)) {
      return { user, id, endpointHash };
    }
  } catch {
    // Malformed: as if nothing were stored.
  }
  return null;
}

export async function sha256Hex(s: string): Promise<string> {
  const d = await crypto.subtle.digest('SHA-256', new TextEncoder().encode(s));
  return [...new Uint8Array(d)].map((b) => b.toString(16).padStart(2, '0')).join('');
}

/** The VAPID key (base64url) as the bytes `PushManager.subscribe` takes. */
export function keyBytes(b64url: string): Uint8Array<ArrayBuffer> {
  const bin = atob(b64url.replace(/-/g, '+').replace(/_/g, '/'));
  const out = new Uint8Array(new ArrayBuffer(bin.length));
  for (let i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i);
  return out;
}

