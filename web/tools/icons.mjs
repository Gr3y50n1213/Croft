// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// Draws the installable app's icons (static/icon-*.png) from the brand's envelope mark, with
// Chromium (Playwright). Run after changing the mark: `node tools/icons.mjs`. The PNGs are committed.
import { chromium } from '@playwright/test';
import { writeFileSync } from 'node:fs';

const MOSS = '#4F6F52';
const PAPER = '#F6F3EE';

/** The envelope on a moss square; `inset` is the share of each side left empty (maskable icons keep their mark in the middle 80 percent). */
function svg(size, rounded, inset) {
  const m = size * inset;
  const s = (size - 2 * m) / 32;
  const r = rounded ? size * 0.22 : 0;
  return `<svg xmlns="http://www.w3.org/2000/svg" width="${size}" height="${size}" viewBox="0 0 ${size} ${size}">
<rect width="${size}" height="${size}" rx="${r}" fill="${MOSS}"/>
<g transform="translate(${m} ${m}) scale(${s})" fill="none" stroke="${PAPER}" stroke-width="2.2" stroke-linejoin="round">
<path d="M7 11h18v12H7z"/><path d="M7 11l9 7 9-7"/></g></svg>`;
}

const icons = [
  ['icon-192.png', 192, true, 0],
  ['icon-512.png', 512, true, 0],
  ['icon-maskable-512.png', 512, false, 0.1]
];

const browser = await chromium.launch();
const page = await browser.newPage({ deviceScaleFactor: 1 });
for (const [name, size, rounded, inset] of icons) {
  await page.setViewportSize({ width: size, height: size });
  await page.setContent(`<style>html,body{margin:0;background:transparent}</style>${svg(size, rounded, inset)}`);
  writeFileSync(new URL(`../static/${name}`, import.meta.url), await page.screenshot({ omitBackground: true }));
}
await browser.close();
