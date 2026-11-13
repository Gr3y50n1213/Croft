// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

/**
 * The look (W04's Appearance, docs/ux/customisation.md): what is stored, its defaults, and the
 * attributes on `<html>` that the generated token CSS and app.css read. Kept in this browser
 * (`localStorage`) until the Companion syncs settings; `static/theme.js` reads the same key with
 * the same rules before the first paint, so a reload never flashes the default look.
 */

import { croftPresets, croftDefaultPreset } from '../../../../design/generated/ts/croft-tokens.ts';

export type Palette = keyof typeof croftPresets;
export type Mode = 'system' | 'light' | 'dark';
export type Contrast = 'standard' | 'medium' | 'high';
export type Density = 'comfortable' | 'default' | 'compact';
export type Roundness = 'tight' | 'standard' | 'round';
export type ListStyle = 'container' | 'flat';
export type Motion = 'system' | 'full' | 'reduced' | 'off';

export interface Appearance {
  readonly mode: Mode;
  /** Dark mode uses true black (saves power on OLED screens). */
  readonly pureBlack: boolean;
  readonly palette: Palette;
  readonly contrast: Contrast;
  readonly density: Density;
  readonly roundness: Roundness;
  readonly list: ListStyle;
  readonly motion: Motion;
}

export const PALETTES = Object.keys(croftPresets) as Palette[];
export const MODES: readonly Mode[] = ['system', 'light', 'dark'];
export const CONTRASTS: readonly Contrast[] = ['standard', 'medium', 'high'];
export const DENSITIES: readonly Density[] = ['comfortable', 'default', 'compact'];
export const ROUNDNESSES: readonly Roundness[] = ['tight', 'standard', 'round'];
export const LIST_STYLES: readonly ListStyle[] = ['container', 'flat'];
export const MOTIONS: readonly Motion[] = ['system', 'full', 'reduced', 'off'];

/** The palette's seed colour, for the swatches. */
export const seedOf = (p: Palette): string => croftPresets[p].seed;

export const defaultAppearance: Appearance = Object.freeze({
  mode: 'system',
  pureBlack: false,
  palette: croftDefaultPreset,
  contrast: 'standard',
  density: 'default',
  roundness: 'standard',
  list: 'container',
  motion: 'system'
});

/** The `localStorage` key (static/theme.js reads it too). */
export const APPEARANCE_KEY = 'croft.appearance';

function pick<T extends string>(allowed: readonly T[], v: unknown, fallback: T): T {
  return typeof v === 'string' && (allowed as readonly string[]).includes(v) ? (v as T) : fallback;
}

/**
 * What was stored, made safe: anything unknown, missing or malformed falls back to its default
 * (stored text is never trusted to be well formed: another version of the app may have written it).
 */
export function parseAppearance(stored: string | null | undefined): Appearance {
  let v: Record<string, unknown> = {};
  if (stored) {
    try {
      const parsed: unknown = JSON.parse(stored);
      if (typeof parsed === 'object' && parsed !== null && !Array.isArray(parsed)) v = parsed as Record<string, unknown>;
    } catch {
      // Unreadable: the defaults.
    }
  }
  const d = defaultAppearance;
  return {
    mode: pick(MODES, v['mode'], d.mode),
    pureBlack: typeof v['pureBlack'] === 'boolean' ? v['pureBlack'] : d.pureBlack,
    palette: pick(PALETTES, v['palette'], d.palette),
    contrast: pick(CONTRASTS, v['contrast'], d.contrast),
    density: pick(DENSITIES, v['density'], d.density),
    roundness: pick(ROUNDNESSES, v['roundness'], d.roundness),
    list: pick(LIST_STYLES, v['list'], d.list),
    motion: pick(MOTIONS, v['motion'], d.motion)
  };
}

/** What is stored: only the fields that differ from the defaults (so new defaults reach everyone). */
export function serializeAppearance(a: Appearance): string {
  const out: Record<string, unknown> = {};
  for (const [k, v] of Object.entries(a)) {
    if (defaultAppearance[k as keyof Appearance] !== v) out[k] = v;
  }
  return JSON.stringify(out);
}

/** The attributes on `<html>`, given whether the system is dark and asks for reduced motion. */
export function attributesOf(a: Appearance, systemDark: boolean, systemReducedMotion: boolean): Record<string, string> {
  const dark = a.mode === 'dark' || (a.mode === 'system' && systemDark);
  const motion = a.motion === 'system' ? (systemReducedMotion ? 'reduced' : 'full') : a.motion;
  return {
    'data-croft-theme': a.palette,
    'data-croft-mode': dark ? (a.pureBlack ? 'pure-black' : 'dark') : 'light',
    'data-croft-contrast': a.contrast,
    'data-croft-density': a.density,
    'data-croft-roundness': a.roundness,
    'data-croft-list': a.list,
    'data-croft-motion': motion
  };
}

/** Words for each choice, as W04 shows them. */
export const LABELS: Record<string, string> = {
  system: 'System',
  light: 'Light',
  dark: 'Dark',
  standard: 'Standard',
  medium: 'Medium',
  high: 'High',
  comfortable: 'Comfortable',
  default: 'Default',
  compact: 'Compact',
  tight: 'Tight',
  round: 'Round',
  container: 'Container',
  flat: 'Flat',
  full: 'Full',
  reduced: 'Reduced',
  off: 'Off'
};

/** A palette's name, capitalised ("moss" is "Moss"). */
export const paletteName = (p: Palette): string => p.charAt(0).toUpperCase() + p.slice(1);
