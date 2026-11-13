// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

import { test } from "node:test";
import assert from "node:assert/strict";
import {
  challengeOf,
  keepVerifier,
  newVerifier,
  startUrl,
  takeVerifier,
} from "./pkce.ts";
import { fromB64url, toB64url } from "./passkey.ts";

test("PKCE: a 43-character verifier and RFC 7636's S256 challenge", async () => {
  const v = newVerifier();
  assert.match(v, /^[A-Za-z0-9_-]{43}$/);
  assert.notEqual(v, newVerifier());
  // RFC 7636, Appendix B.
  assert.equal(
    await challengeOf("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
    "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM",
  );
  assert.equal(
    startUrl("abc", "https://c.example/sso/back"),
    "/api/sign-in/oidc/start?app_challenge=abc&app_redirect=https%3A%2F%2Fc.example%2Fsso%2Fback",
  );
});

test("the verifier is kept for one round trip and read once", () => {
  const m = new Map<string, string>();
  const store = {
    setItem: (k: string, v: string) => void m.set(k, v),
    getItem: (k: string) => m.get(k) ?? null,
    removeItem: (k: string) => void m.delete(k),
  };
  keepVerifier(store, "v1");
  assert.equal(takeVerifier(store), "v1");
  assert.equal(takeVerifier(store), null);
});

test("base64url round trips without padding", () => {
  for (const n of [0, 1, 2, 3, 31, 32, 65]) {
    const bytes = new Uint8Array(n).map((_, i) => (i * 37 + 11) & 255);
    const s = toB64url(bytes);
    assert.doesNotMatch(s, /[+/=]/);
    assert.deepEqual(new Uint8Array(fromB64url(s)), bytes);
  }
});

test("a fresh single sign-on start asks the Companion to make the provider ask", () => {
  assert.ok(
    startUrl("abc", "https://c.example/sso/back", true).endsWith("&fresh=1"),
  );
  assert.ok(!startUrl("abc", "https://c.example/sso/back").includes("fresh"));
});
