// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

import { defineConfig, devices } from '@playwright/test';

// The end-to-end tests start a real Companion (CROFT_COMPANION_BIN, built with `cargo build`) that
// serves `build/` (run `npm run build` first). Chromium only: its virtual authenticator stands in
// for a passkey.
export default defineConfig({
  testDir: 'e2e',
  workers: 1,
  timeout: 60_000,
  reporter: 'list',
  use: { ...devices['Desktop Chrome'], trace: 'off' }
});
