// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

/**
 * This browser's settings, live: read once, applied to `<html>` at once on every change, saved to
 * `localStorage`, and followed across tabs (`storage` events). Storage can be missing or refuse
 * (private windows, blocked site data): the app then works with the defaults and keeps changes for
 * this page only.
 */

import {
  APPEARANCE_KEY,
  attributesOf,
  parseAppearance,
  serializeAppearance,
  type Appearance
} from './appearance.ts';
import { SWIPE_KEY, parseSwipe, serializeSwipe } from './swipe.ts';
import type { SwipeSettings } from '../swipe/model.ts';

function read(key: string): string | null {
  try {
    return globalThis.localStorage?.getItem(key) ?? null;
  } catch {
    return null;
  }
}

function write(key: string, value: string): void {
  try {
    globalThis.localStorage?.setItem(key, value);
  } catch {
    // Not kept past this page; nothing else to do.
  }
}

class Settings {
  appearance: Appearance = $state(parseAppearance(read(APPEARANCE_KEY)));
  swipe: SwipeSettings = $state(parseSwipe(read(SWIPE_KEY)));
  #started = false;

  setAppearance(change: Partial<Appearance>): void {
    this.appearance = { ...this.appearance, ...change };
    write(APPEARANCE_KEY, serializeAppearance(this.appearance));
    this.apply();
  }

  setSwipe(s: SwipeSettings): void {
    this.swipe = s;
    write(SWIPE_KEY, serializeSwipe(s));
  }

  /** Back to every default (both groups). */
  reset(): void {
    this.setAppearance(parseAppearance(null));
    this.setSwipe(parseSwipe(null));
  }

  /** Sets the attributes on `<html>`. */
  apply(): void {
    if (typeof document === 'undefined') return;
    const dark = matchMedia('(prefers-color-scheme: dark)').matches;
    const reduced = matchMedia('(prefers-reduced-motion: reduce)').matches;
    const root = document.documentElement;
    for (const [k, v] of Object.entries(attributesOf(this.appearance, dark, reduced))) root.setAttribute(k, v);
  }

  /** Follows the system and other tabs. Once per page. */
  start(): void {
    if (this.#started || typeof window === 'undefined') return;
    this.#started = true;
    this.apply();
    matchMedia('(prefers-color-scheme: dark)').addEventListener('change', () => this.apply());
    matchMedia('(prefers-reduced-motion: reduce)').addEventListener('change', () => this.apply());
    window.addEventListener('storage', (e) => {
      if (e.key === APPEARANCE_KEY || e.key === null) {
        this.appearance = parseAppearance(read(APPEARANCE_KEY));
        this.apply();
      }
      if (e.key === SWIPE_KEY || e.key === null) this.swipe = parseSwipe(read(SWIPE_KEY));
    });
  }
}

/** The page's settings. */
export const settings = new Settings();
