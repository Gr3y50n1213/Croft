// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

/**
 * Passkeys: the Companion's WebAuthn options (JSON, base64url) to what the browser takes, and the
 * browser's answer back to JSON (as the enrolment page does, T24).
 */

export function fromB64url(s: string): ArrayBuffer {
  const bin = atob(s.replace(/-/g, '+').replace(/_/g, '/'));
  const out = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i);
  return out.buffer;
}

export function toB64url(buf: ArrayBuffer | ArrayBufferView): string {
  const bytes =
    buf instanceof ArrayBuffer ? new Uint8Array(buf) : new Uint8Array(buf.buffer, buf.byteOffset, buf.byteLength);
  let s = '';
  for (const b of bytes) s += String.fromCharCode(b);
  return btoa(s).replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, '');
}

type Json = Record<string, unknown>;

function credList(list: unknown): PublicKeyCredentialDescriptor[] {
  if (!Array.isArray(list)) return [];
  return list.map((c: Json) => ({ ...c, id: fromB64url(String(c['id'])) }) as PublicKeyCredentialDescriptor);
}

/** Sign-in options (`options.publicKey` of `/api/sign-in/passkey/start`). */
export function requestOptions(publicKey: Json): PublicKeyCredentialRequestOptions {
  return {
    ...publicKey,
    challenge: fromB64url(String(publicKey['challenge'])),
    allowCredentials: credList(publicKey['allowCredentials'])
  } as PublicKeyCredentialRequestOptions;
}

/** Registration options (`options.publicKey` of `/api/passkeys/start`). */
export function creationOptions(publicKey: Json): PublicKeyCredentialCreationOptions {
  const user = publicKey['user'] as Json;
  return {
    ...publicKey,
    challenge: fromB64url(String(publicKey['challenge'])),
    user: { ...user, id: fromB64url(String(user['id'])) },
    excludeCredentials: credList(publicKey['excludeCredentials'])
  } as PublicKeyCredentialCreationOptions;
}

/** A sign-in answer as JSON. */
export function assertionJson(cred: PublicKeyCredential): Json {
  const r = cred.response as AuthenticatorAssertionResponse;
  return {
    id: cred.id,
    rawId: toB64url(cred.rawId),
    type: cred.type,
    response: {
      authenticatorData: toB64url(r.authenticatorData),
      clientDataJSON: toB64url(r.clientDataJSON),
      signature: toB64url(r.signature),
      userHandle: r.userHandle ? toB64url(r.userHandle) : null
    },
    extensions: cred.getClientExtensionResults()
  };
}

/** A registration answer as JSON. */
export function attestationJson(cred: PublicKeyCredential): Json {
  const r = cred.response as AuthenticatorAttestationResponse;
  return {
    id: cred.id,
    rawId: toB64url(cred.rawId),
    type: cred.type,
    response: {
      attestationObject: toB64url(r.attestationObject),
      clientDataJSON: toB64url(r.clientDataJSON),
      transports: typeof r.getTransports === 'function' ? r.getTransports() : undefined
    },
    extensions: cred.getClientExtensionResults()
  };
}

/** What `makePasskey` needs from the API client. */
export interface PasskeyCaller {
  call<T = unknown>(method: string, path: string, body?: unknown): Promise<T>;
}

/**
 * Makes a passkey on this device for the signed-in person (`/api/passkeys/start`, the browser's
 * prompt, `/api/passkeys/finish`). Throws `NotAllowedError` when the person cancels.
 */
export async function makePasskey(api: PasskeyCaller): Promise<void> {
  const start = await api.call<{ ceremony: string; options: { publicKey: Json } }>('POST', '/api/passkeys/start');
  const cred = (await navigator.credentials.create({
    publicKey: creationOptions(start.options.publicKey)
  })) as PublicKeyCredential | null;
  if (!cred) throw new DOMException('cancelled', 'NotAllowedError');
  await api.call('POST', '/api/passkeys/finish', { ceremony: start.ceremony, credential: attestationJson(cred) });
}
