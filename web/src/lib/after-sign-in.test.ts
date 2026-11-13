// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

import { test } from "node:test";
import assert from "node:assert/strict";
import { rememberReturn, takeReturn } from "./after-sign-in.ts";

class Memory implements Storage {
  private m = new Map<string, string>();
  get length() {
    return this.m.size;
  }
  clear() {
    this.m.clear();
  }
  getItem(k: string) {
    return this.m.get(k) ?? null;
  }
  key(i: number) {
    return [...this.m.keys()][i] ?? null;
  }
  removeItem(k: string) {
    this.m.delete(k);
  }
  setItem(k: string, v: string) {
    this.m.set(k, v);
  }
}

test("the page after signing in is the remembered one, once", () => {
  const s = new Memory();
  rememberReturn(s, "/settings/security");
  assert.equal(takeReturn(s), "/settings/security");
  assert.equal(takeReturn(s), "/");
});

test("the page after signing in is the inbox for anything not on the list", () => {
  const s = new Memory();
  for (const bad of [
    "https://evil.example/",
    "//evil.example",
    "/settings/security/../../x",
    "javascript:alert(1)",
  ]) {
    s.setItem("croft-after-sign-in", bad);
    assert.equal(takeReturn(s), "/");
  }
});
