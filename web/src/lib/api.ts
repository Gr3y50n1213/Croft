// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

/**
 * Talking to the Companion (T30). The access token (15 minutes) lives in this module's memory only;
 * the refresh token is an HttpOnly cookie the browser sends to `/api/web/refresh` and no script can
 * read. Every request carries `X-Croft-Web: 1`, which the Companion requires for anything that
 * touches the cookie (a page on another origin cannot send it).
 */

/** An error from the Companion: its code (`invalid`, `rate_limited`, ...) and HTTP status. */
export class ApiError extends Error {
  readonly code: string;
  readonly status: number;
  readonly retryAfterS: number | undefined;

  constructor(code: string, status: number, retryAfterS?: number) {
    super(code);
    this.code = code;
    this.status = status;
    this.retryAfterS = retryAfterS;
  }
}

/** What a sign-in or a refresh gives the app. */
export interface Session {
  accessToken: string;
  expiresMs: number;
  recovery: boolean;
}

export type Fetch = typeof fetch;

/** Renew this long before the access token expires. */
export const RENEW_BEFORE_MS = 60_000;

/** The client: one per page. */
export class Api {
  #fetch: Fetch;
  #session: Session | null = null;
  #renewing: Promise<Session | null> | null = null;
  #now: () => number;

  constructor(fetchFn: Fetch, now: () => number = () => Date.now()) {
    this.#fetch = fetchFn;
    this.#now = now;
  }

  get session(): Session | null {
    return this.#session;
  }

  /** Takes a sign-in's answer (the refresh cookie is already set by the browser). */
  adopt(answer: unknown): Session {
    const s = toSession(answer);
    this.#session = s;
    return s;
  }

  forget(): void {
    this.#session = null;
  }

  /** A request to the API. Signed in, it carries the access token (renewed first when due). */
  async call<T = unknown>(method: string, path: string, body?: unknown): Promise<T> {
    const session = await this.current();
    return this.raw<T>(method, path, body, session?.accessToken);
  }

  /** A request without renewing (sign-in, refresh, sign-out). */
  async raw<T = unknown>(method: string, path: string, body?: unknown, token?: string): Promise<T> {
    const headers: Record<string, string> = { 'x-croft-web': '1' };
    if (body !== undefined) headers['content-type'] = 'application/json';
    if (token) headers['authorization'] = `Bearer ${token}`;
    const init: RequestInit = {
      method,
      headers,
      credentials: 'same-origin',
      cache: 'no-store',
      redirect: 'error'
    };
    if (body !== undefined) init.body = JSON.stringify(body);
    let res: Response;
    try {
      res = await this.#fetch(path, init);
    } catch {
      throw new ApiError('offline', 0);
    }
    const data: unknown = await res.json().catch(() => ({}));
    if (!res.ok) {
      const code = isRecord(data) && typeof data['error'] === 'string' ? data['error'] : 'server';
      const retry = Number(res.headers.get('retry-after'));
      throw new ApiError(code, res.status, Number.isFinite(retry) && retry > 0 ? retry : undefined);
    }
    return data as T;
  }

  /**
   * The session, renewed through the cookie when it is near its end (or absent: a reload). One
   * renewal at a time. `null` when the browser has no session: the app shows sign-in.
   */
  async current(): Promise<Session | null> {
    const s = this.#session;
    if (s && !s.recovery && s.expiresMs - RENEW_BEFORE_MS > this.#now()) return s;
    if (s?.recovery) return s.expiresMs > this.#now() ? s : null;
    return this.renew();
  }

  /** Renews through the cookie now. */
  renew(): Promise<Session | null> {
    if (!this.#renewing) {
      this.#renewing = this.raw('POST', '/api/web/refresh')
        .then((a) => this.adopt(a))
        .catch((e: unknown) => {
          // Only a refusal ends the session; a network failure keeps what we have.
          if (e instanceof ApiError && e.status !== 0) {
            this.#session = null;
            return null;
          }
          return this.#session;
        })
        .finally(() => {
          this.#renewing = null;
        });
    }
    return this.#renewing;
  }

  async signOut(): Promise<void> {
    try {
      await this.raw('POST', '/api/web/sign-out');
    } finally {
      this.#session = null;
    }
  }
}

function isRecord(v: unknown): v is Record<string, unknown> {
  return typeof v === 'object' && v !== null;
}

/** A sign-in or refresh answer as a session; never keeps a refresh token if one is sent. */
export function toSession(answer: unknown): Session {
  if (
    !isRecord(answer) ||
    typeof answer['access_token'] !== 'string' ||
    typeof answer['access_expires_ms'] !== 'number'
  ) {
    throw new ApiError('server', 500);
  }
  return {
    accessToken: answer['access_token'],
    expiresMs: answer['access_expires_ms'],
    recovery: answer['recovery'] === true
  };
}

/** Plain words for an error, for the sign-in screens. */
export function explain(e: unknown): string {
  const code = e instanceof ApiError ? e.code : e instanceof DOMException ? e.name : 'server';
  switch (code) {
    case 'invalid':
      return "That didn't work. Check what you entered and try again.";
    case 'locked':
      return 'Too many wrong tries. Wait 15 minutes, then try again.';
    case 'rate_limited':
      return 'Too many tries from here. Wait a few minutes, then try again.';
    case 'offline':
      return "Can't reach your Companion. Check your connection.";
    case 'NotAllowedError':
      return 'The passkey request was cancelled or timed out. Try again.';
    case 'InvalidStateError':
      return 'This device already has a passkey for your account.';
    case 'forbidden':
      return 'This page could not sign you in. Reload it and try again.';
    default:
      return 'Something went wrong. Try again.';
  }
}
