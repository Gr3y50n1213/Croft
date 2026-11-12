// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

import adapter from '@sveltejs/adapter-static';
import { sveltekit } from '@sveltejs/kit/vite';
import { defineConfig } from 'vite';

export default defineConfig({
  plugins: [
    sveltekit({
      // A static single-page app: the Companion serves `build/` and gives every app path index.html.
      adapter: adapter({ pages: 'build', assets: 'build', fallback: 'index.html', strict: true }),
      compilerOptions: { runes: true }
    })
  ],
  server: {
    // The design tokens are read in place from design/generated (never copied).
    fs: { allow: ['..'] },
    // Development: the API of a Companion on this machine.
    proxy: {
      '/api': 'http://127.0.0.1:8480',
      '/oidc': 'http://127.0.0.1:8480',
      '/enrol': 'http://127.0.0.1:8480'
    }
  },
  build: { assetsInlineLimit: 0 }
});
