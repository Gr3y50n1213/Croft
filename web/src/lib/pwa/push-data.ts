// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

/** The pure parts of push in this browser (push.ts), tested on their own. */

/** What this browser keeps: the Companion's id for the subscription and a hash of its endpoint. */
export interface Stored {
  id: number;
  endpointHash: string;
}

/** What was stored, or null when nothing (or something malformed) is. */
export function parseStored(s: string | null): Stored | null {
  if (!s) return null;
  try {
    const v: unknown = JSON.parse(s);
    if (typeof v !== 'object' || v === null) return null;
    const { id, endpointHash } = v as Record<string, unknown>;
    if (typeof id === 'number' && Number.isSafeInteger(id) && id > 0 && typeof endpointHash === 'string' && /^[0-9a-f]{64}$/.test(endpointHash)) {
      return { id, endpointHash };
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

