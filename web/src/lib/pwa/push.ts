// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

/**
 * Push in this browser (T32b): a Web Push subscription with the Companion's VAPID key, given to
 * the Companion (T28), which sends a content-free ping when mail arrives. The subscription belongs
 * to this browser's session on the Companion: signing out ends it there, and the app gives it again
 * after the next sign-in. This browser remembers only that push is on, the Companion's id for the
 * subscription and a hash of its endpoint (to notice when the browser replaced it), never the
 * endpoint or its keys.
 */

import { ApiError } from '../api.ts';
import { registerWorker } from './register.ts';
import { keyBytes, parseStored, sha256Hex, type Stored } from './push-data.ts';

export const PUSH_KEY = 'croft.push';

export interface PushCaller {
  call<T = unknown>(method: string, path: string, body?: unknown): Promise<T>;
}

export type PushState = 'unsupported' | 'blocked' | 'off' | 'on';

function read(): Stored | null {
  try {
    return parseStored(globalThis.localStorage?.getItem(PUSH_KEY) ?? null);
  } catch {
    return null;
  }
}

function write(s: Stored | null): void {
  try {
    if (s) globalThis.localStorage?.setItem(PUSH_KEY, JSON.stringify(s));
    else globalThis.localStorage?.removeItem(PUSH_KEY);
  } catch {
    // Not kept: push is given again on the next start that can store it.
  }
}

function supported(): boolean {
  return (
    typeof window !== 'undefined' &&
    'serviceWorker' in navigator &&
    'PushManager' in window &&
    'Notification' in window
  );
}

/** Whether push is on in this browser (as far as this browser knows). */
export function pushState(): PushState {
  if (!supported()) return 'unsupported';
  if (Notification.permission === 'denied') return 'blocked';
  return read() && Notification.permission === 'granted' ? 'on' : 'off';
}

async function subscription(api: PushCaller): Promise<PushSubscription> {
  const reg = await registerWorker();
  if (!reg) throw new ApiError('unsupported', 0);
  await navigator.serviceWorker.ready;
  const existing = await reg.pushManager.getSubscription();
  if (existing) return existing;
  const { key } = await api.call<{ key: string }>('GET', '/api/push/vapid');
  return reg.pushManager.subscribe({ userVisibleOnly: true, applicationServerKey: keyBytes(key) });
}

/** Gives the subscription to the Companion; the id it answers, stored. */
async function give(api: PushCaller, sub: PushSubscription): Promise<void> {
  const json = sub.toJSON();
  const { id } = await api.call<{ id: number }>('POST', '/api/push/subscriptions', {
    endpoint: json.endpoint,
    keys: { p256dh: json.keys?.['p256dh'], auth: json.keys?.['auth'] }
  });
  write({ id, endpointHash: await sha256Hex(sub.endpoint) });
}

async function forget(api: PushCaller, id: number): Promise<void> {
  await api.call('DELETE', `/api/push/subscriptions/${id}`).catch((e: unknown) => {
    // Already gone (signed out, or removed elsewhere) is what was wanted.
    if (!(e instanceof ApiError && e.status === 404)) throw e;
  });
}

/** Turns push on: asks the browser's permission, subscribes and tells the Companion. */
export async function enablePush(api: PushCaller): Promise<PushState> {
  if (!supported()) return 'unsupported';
  const permission = await Notification.requestPermission();
  if (permission !== 'granted') return permission === 'denied' ? 'blocked' : 'off';
  const old = read();
  await give(api, await subscription(api));
  if (old) await forget(api, old.id).catch(() => {});
  return 'on';
}

/** Turns push off: the Companion forgets the subscription and the browser drops it. */
export async function disablePush(api: PushCaller): Promise<PushState> {
  const stored = read();
  if (stored) await forget(api, stored.id);
  write(null);
  if (supported()) {
    const reg = await navigator.serviceWorker.getRegistration('/');
    await (await reg?.pushManager.getSubscription())?.unsubscribe().catch(() => false);
  }
  return pushState();
}

/**
 * On every start, and when the worker says the browser replaced the subscription: when push is on,
 * makes sure the Companion holds this browser's current subscription for this session (signing
 * out ends it there; browsers rotate subscriptions).
 */
export async function keepPush(api: PushCaller): Promise<void> {
  const stored = read();
  if (!stored || !supported()) return;
  if (Notification.permission !== 'granted') {
    // The person took the permission back in the browser: push is off.
    await forget(api, stored.id).catch(() => {});
    write(null);
    return;
  }
  const sub = await subscription(api);
  const { subscriptions } = await api.call<{ subscriptions: { id: number; current_device: boolean }[] }>(
    'GET',
    '/api/push/subscriptions'
  );
  const held = subscriptions.some((s) => s.id === stored.id && s.current_device);
  if (held && stored.endpointHash === (await sha256Hex(sub.endpoint))) return;
  await give(api, sub);
  if (held) await forget(api, stored.id).catch(() => {});
}
